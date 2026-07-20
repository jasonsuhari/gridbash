use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use fs4::FileExt;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use crate::{
    auth::AgentKind,
    cli::ResumeArgs,
    layout::GridSize,
    pane_host::{PtyHostRef, PtyPane},
    ports::roots_with_descendant_named,
    profiles::Profile,
    setup::{LaunchPlan, PaneLaunchSpec, folder_display_name},
};

const SESSION_VERSION: u16 = 1;
const MAX_SAVED_SESSIONS: usize = 50;
const MAX_RECOVERED_PANES_PER_TAB: usize = 100;

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub path: PathBuf,
    pub session: SavedSession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub version: u16,
    pub id: String,
    pub started_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub grid: SavedGrid,
    #[serde(default)]
    pub panes: Vec<SavedPane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_panes: Vec<SavedBackgroundPane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<SavedTab>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SavedGrid {
    pub rows: usize,
    pub columns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPane {
    pub index: usize,
    pub profile_name: String,
    pub command: Profile,
    pub cwd: PathBuf,
    pub folder_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_kind: Option<AgentKind>,
    #[serde(default)]
    pub history: SavedPaneHistory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<PtyHostRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub grid: SavedGrid,
    #[serde(default)]
    pub panes: Vec<SavedPane>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedBackgroundPane {
    pub id: u64,
    pub source_tab: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub pane: SavedPane,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedPaneHistory {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_history: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output_tail: String,
}

#[derive(Debug, Clone)]
pub struct InterruptedRecovery {
    pub tabs: Vec<SavedTab>,
    pub session_count: usize,
    pub pane_count: usize,
}

pub struct SessionRecorder {
    path: PathBuf,
    session: SavedSession,
}

impl SessionRecord {
    pub fn summary(&self) -> String {
        self.session.summary()
    }
}

impl SavedSession {
    pub fn launch_plan(&self) -> Result<LaunchPlan> {
        launch_plan_from_saved(&self.id, self.grid, &self.panes)
    }

    pub fn pane_histories(&self) -> Vec<SavedPaneHistory> {
        self.panes.iter().map(|pane| pane.history.clone()).collect()
    }

    pub fn pane_hosts(&self) -> Vec<Option<PtyHostRef>> {
        self.panes.iter().map(|pane| pane.host.clone()).collect()
    }

    fn new(id: String, title: &str, plan: &LaunchPlan) -> Self {
        let now = now_seconds();
        Self {
            version: SESSION_VERSION,
            id,
            started_at: now,
            updated_at: now,
            title: title.to_string(),
            grid: plan.grid.into(),
            panes: plan
                .panes
                .iter()
                .enumerate()
                .map(|(index, spec)| {
                    SavedPane::from_spec(index, spec, SavedPaneHistory::default(), None, None)
                })
                .collect(),
            background_panes: Vec::new(),
            tabs: Vec::new(),
            running: true,
            owner_pid: Some(std::process::id()),
            recovered_at: None,
        }
    }

    fn begin_run(&mut self) {
        self.running = true;
        self.owner_pid = Some(std::process::id());
        self.recovered_at = None;
        self.updated_at = now_seconds();
    }

    fn finish_run(&mut self) {
        self.running = false;
        self.owner_pid = None;
        self.updated_at = now_seconds();
    }

    fn mark_recovered(&mut self) {
        self.running = false;
        self.owner_pid = None;
        self.recovered_at = Some(now_seconds());
    }

    fn has_agent_pane(&self) -> bool {
        self.panes
            .iter()
            .chain(self.tabs.iter().flat_map(|tab| tab.panes.iter()))
            .chain(self.background_panes.iter().map(|job| &job.pane))
            .any(|pane| pane.launch_spec().agent_label().is_some())
    }

    fn update_from_live(
        &mut self,
        title: &str,
        plan: &LaunchPlan,
        panes: &[PtyPane],
        tabs: Vec<SavedTab>,
        background_panes: Vec<SavedBackgroundPane>,
    ) {
        self.version = SESSION_VERSION;
        self.updated_at = now_seconds();
        self.title = title.to_string();
        self.grid = plan.grid.into();
        self.panes = saved_panes_from_live(plan, panes);
        self.tabs = tabs;
        self.background_panes = background_panes;
    }

    fn summary(&self) -> String {
        let panes = self
            .panes
            .iter()
            .chain(self.tabs.iter().flat_map(|tab| tab.panes.iter()))
            .collect::<Vec<_>>();
        let folders = compact_labels(
            panes
                .iter()
                .map(|pane| pane.folder_name.as_str())
                .filter(|name| !name.is_empty()),
        );
        let profiles = compact_labels(
            panes
                .iter()
                .map(|pane| pane.profile_name.as_str())
                .filter(|name| !name.is_empty()),
        );
        let pane_count = panes.len();
        let tab_suffix = (!self.tabs.is_empty()).then(|| {
            let tab_count = self.tabs.len() + 1;
            format!(" / {tab_count} tabs")
        });

        let background = if self.background_panes.is_empty() {
            String::new()
        } else {
            format!(" | {} background", self.background_panes.len())
        };

        format!(
            "{} | {}x{} | {} pane{}{} | {} | {}{}",
            age_label(self.updated_at),
            self.grid.rows,
            self.grid.columns,
            pane_count,
            if pane_count == 1 { "" } else { "s" },
            tab_suffix.unwrap_or_default(),
            folders.unwrap_or_else(|| "unknown folders".into()),
            profiles.unwrap_or_else(|| "unknown profiles".into()),
            background,
        )
    }
}

impl SavedTab {
    pub fn from_live(title: &str, plan: &LaunchPlan, panes: &[PtyPane]) -> Self {
        Self {
            title: title.to_string(),
            grid: plan.grid.into(),
            panes: saved_panes_from_live(plan, panes),
        }
    }

    pub fn launch_plan(&self) -> Result<LaunchPlan> {
        launch_plan_from_saved(&self.title, self.grid, &self.panes)
    }

    pub fn pane_histories(&self) -> Vec<SavedPaneHistory> {
        self.panes.iter().map(|pane| pane.history.clone()).collect()
    }

    pub fn pane_hosts(&self) -> Vec<Option<PtyHostRef>> {
        self.panes.iter().map(|pane| pane.host.clone()).collect()
    }
}

fn launch_plan_from_saved(
    id: &str,
    saved_grid: SavedGrid,
    panes: &[SavedPane],
) -> Result<LaunchPlan> {
    let grid = GridSize::new(saved_grid.rows, saved_grid.columns).ok_or_else(|| {
        anyhow!(
            "saved session {id} has invalid grid {}x{}",
            saved_grid.rows,
            saved_grid.columns
        )
    })?;
    let panes = panes.iter().map(SavedPane::launch_spec).collect::<Vec<_>>();
    if panes.is_empty() {
        bail!("saved session {id} has no panes");
    }
    Ok(LaunchPlan { panes, grid })
}

fn saved_panes_from_live(plan: &LaunchPlan, panes: &[PtyPane]) -> Vec<SavedPane> {
    let host_processes = panes
        .iter()
        .filter_map(PtyPane::host_process_id)
        .collect::<Vec<_>>();
    let codex_roots = roots_with_descendant_named(&host_processes, "codex").ok();
    plan.panes
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let live = panes.get(index);
            let history = live.map(SavedPaneHistory::from_pane).unwrap_or_default();
            let host = live.map(PtyPane::host_ref);
            let codex_running = live.is_some_and(|pane| {
                pane.host_process_id().is_some_and(|process_id| {
                    codex_roots
                        .as_ref()
                        .is_some_and(|roots| roots.contains(&process_id))
                }) || (spec.command.agent_kind == Some(AgentKind::Codex) && !pane.exited)
            });
            let codex_thread_id = codex_running
                .then(|| {
                    codex_resume_id(&spec.command, &history.input_history)
                        .or_else(|| live.and_then(|pane| pane.codex_thread_id(pane.cwd())))
                })
                .flatten();
            let mut saved = SavedPane::from_spec(index, spec, history, codex_thread_id, host);
            if let Some(live) = live {
                saved.cwd = live.cwd().to_path_buf();
                saved.folder_name = folder_display_name(&saved.cwd);
            }
            saved
        })
        .collect()
}

impl SavedPane {
    pub fn launch_spec(&self) -> PaneLaunchSpec {
        let (profile_name, command) = self
            .codex_thread_id
            .as_deref()
            .map(|thread_id| codex_resume_profile(&self.profile_name, &self.command, thread_id))
            .unwrap_or_else(|| (self.profile_name.clone(), self.command.clone()));
        PaneLaunchSpec {
            profile_name,
            command,
            env: BTreeMap::new(),
            cwd: self.cwd.clone(),
            folder_name: self.folder_name.clone(),
            worktree_name: self.worktree_name.clone(),
            auth_name: self.auth_name.clone(),
            auth_kind: self.auth_kind,
            auth_dir: None,
        }
    }

    pub fn from_background(
        spec: &PaneLaunchSpec,
        history: SavedPaneHistory,
        host: Option<PtyHostRef>,
    ) -> Self {
        Self::from_spec(0, spec, history, None, host)
    }

    fn from_spec(
        index: usize,
        spec: &PaneLaunchSpec,
        history: SavedPaneHistory,
        codex_thread_id: Option<String>,
        host: Option<PtyHostRef>,
    ) -> Self {
        Self {
            index,
            profile_name: spec.profile_name.clone(),
            command: spec.command.clone(),
            cwd: spec.cwd.clone(),
            folder_name: spec.folder_name.clone(),
            worktree_name: spec.worktree_name.clone(),
            auth_name: spec.auth_name.clone(),
            auth_kind: spec.auth_kind,
            history,
            codex_thread_id,
            host,
        }
    }
}

fn codex_resume_profile(
    profile_name: &str,
    command: &Profile,
    thread_id: &str,
) -> (String, Profile) {
    if command.agent_kind == Some(AgentKind::Codex) && is_direct_codex_command(&command.command) {
        let mut command = command.clone();
        if let Some(resume_index) = command
            .args
            .iter()
            .position(|argument| argument == "resume")
        {
            if command
                .args
                .get(resume_index + 1)
                .is_some_and(|argument| looks_like_thread_id(argument))
            {
                command.args[resume_index + 1] = thread_id.to_string();
            } else {
                command.args.insert(resume_index + 1, thread_id.to_string());
            }
        } else {
            command.args.extend(["resume".into(), thread_id.into()]);
        }
        return (profile_name.to_string(), command);
    }
    if codex_resume_id(command, &[]).as_deref() == Some(thread_id) {
        return (profile_name.to_string(), command.clone());
    }

    (
        "codex".into(),
        Profile {
            command: "codex".into(),
            args: vec!["resume".into(), thread_id.into()],
            title: Some("Codex".into()),
            agent_kind: Some(AgentKind::Codex),
        },
    )
}

fn codex_resume_id(command: &Profile, input_history: &[String]) -> Option<String> {
    command
        .args
        .windows(2)
        .find_map(|arguments| {
            (arguments[0] == "resume" && looks_like_thread_id(&arguments[1]))
                .then(|| arguments[1].clone())
        })
        .or_else(|| {
            command
                .args
                .iter()
                .rev()
                .find_map(|argument| resume_id_in_text(argument))
        })
        .or_else(|| {
            input_history
                .iter()
                .rev()
                .find_map(|input| resume_id_in_text(input))
        })
}

fn resume_id_in_text(value: &str) -> Option<String> {
    let tokens = value
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| matches!(character, '"' | '\'' | ';' | ','))
        })
        .collect::<Vec<_>>();
    tokens.windows(2).find_map(|tokens| {
        (tokens[0] == "resume" && looks_like_thread_id(tokens[1])).then(|| tokens[1].to_string())
    })
}

fn is_direct_codex_command(command: &str) -> bool {
    Path::new(command)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("codex"))
}

fn looks_like_thread_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

impl SavedPaneHistory {
    pub fn from_pane(pane: &PtyPane) -> Self {
        Self {
            input_history: pane.input_history().to_vec(),
            output_tail: pane.output_tail().to_string(),
        }
    }
}

impl From<GridSize> for SavedGrid {
    fn from(grid: GridSize) -> Self {
        Self {
            rows: grid.rows,
            columns: grid.columns,
        }
    }
}

impl SessionRecorder {
    pub fn start_new(title: &str, plan: &LaunchPlan) -> Result<Self> {
        let id = new_session_id();
        let path = session_file_path(&id)?;
        let recorder = Self {
            path,
            session: SavedSession::new(id, title, plan),
        };
        recorder.save()?;
        prune_old_sessions()?;
        Ok(recorder)
    }

    pub fn continue_record(mut record: SessionRecord) -> Self {
        record.session.begin_run();
        Self {
            path: record.path,
            session: record.session,
        }
    }

    pub fn update(
        &mut self,
        title: &str,
        plan: &LaunchPlan,
        panes: &[PtyPane],
        tabs: Vec<SavedTab>,
        background_panes: Vec<SavedBackgroundPane>,
    ) {
        self.session
            .update_from_live(title, plan, panes, tabs, background_panes);
    }

    pub fn save(&self) -> Result<()> {
        save_session_to_path(&self.path, &self.session)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.session.finish_run();
        self.save()
    }
}

pub fn claim_interrupted_recovery() -> Result<Option<InterruptedRecovery>> {
    let directory = sessions_dir()?;
    create_private_dir_all(&directory)
        .with_context(|| format!("failed to create session directory {}", directory.display()))?;
    let lock_path = directory.join(".recovery.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options
        .open(&lock_path)
        .with_context(|| format!("failed to open recovery lock {}", lock_path.display()))?;
    FileExt::lock(&lock)
        .with_context(|| format!("failed to lock recovery state {}", lock_path.display()))?;

    let result = claim_interrupted_recovery_locked();
    let unlock_result = FileExt::unlock(&lock)
        .with_context(|| format!("failed to unlock recovery state {}", lock_path.display()));
    match (result, unlock_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(recovery), Ok(())) => Ok(recovery),
    }
}

fn claim_interrupted_recovery_locked() -> Result<Option<InterruptedRecovery>> {
    let mut records = load_recent_sessions()?
        .into_iter()
        .filter(is_interrupted_agent_session)
        .collect::<Vec<_>>();
    let Some(recovery) = build_interrupted_recovery(&records) else {
        return Ok(None);
    };

    for record in &mut records {
        record.session.mark_recovered();
        save_session_to_path(&record.path, &record.session)?;
    }
    Ok(Some(recovery))
}

fn is_interrupted_agent_session(record: &SessionRecord) -> bool {
    let session = &record.session;
    session.running
        && session.recovered_at.is_none()
        && session
            .owner_pid
            .is_some_and(|owner_pid| !process_is_running(owner_pid))
        && session.has_agent_pane()
}

fn build_interrupted_recovery(records: &[SessionRecord]) -> Option<InterruptedRecovery> {
    let mut groups = Vec::<(String, PathBuf, Vec<SavedPane>)>::new();
    for record in records {
        let session = &record.session;
        let panes = session
            .panes
            .iter()
            .chain(session.tabs.iter().flat_map(|tab| tab.panes.iter()))
            .chain(session.background_panes.iter().map(|job| &job.pane));
        for pane in panes {
            let key = directory_group_key(&pane.cwd);
            if let Some((_, _, panes)) = groups.iter_mut().find(|(group, _, _)| group == &key) {
                panes.push(pane.clone());
            } else {
                groups.push((key, pane.cwd.clone(), vec![pane.clone()]));
            }
        }
    }

    let pane_count = groups.iter().map(|(_, _, panes)| panes.len()).sum();
    if pane_count == 0 {
        return None;
    }

    let mut title_counts = BTreeMap::<String, usize>::new();
    let mut tabs = Vec::new();
    for (_, cwd, panes) in groups {
        let base_title = folder_display_name(&cwd);
        for chunk in panes.chunks(MAX_RECOVERED_PANES_PER_TAB) {
            let occurrence = title_counts.entry(base_title.clone()).or_default();
            *occurrence += 1;
            let title = if *occurrence == 1 {
                base_title.clone()
            } else {
                format!("{base_title} ({occurrence})")
            };
            let mut panes = chunk.to_vec();
            for (index, pane) in panes.iter_mut().enumerate() {
                pane.index = index;
            }
            tabs.push(SavedTab {
                title,
                grid: GridSize::from_count(panes.len()).into(),
                panes,
            });
        }
    }

    Some(InterruptedRecovery {
        tabs,
        session_count: records.len(),
        pane_count,
    })
}

fn directory_group_key(path: &Path) -> String {
    #[cfg(windows)]
    return path.to_string_lossy().replace('\\', "/").to_lowercase();

    #[cfg(not(windows))]
    path.to_string_lossy().into_owned()
}

pub fn select_resume_session(args: &ResumeArgs) -> Result<Option<SessionRecord>> {
    let sessions = load_recent_sessions()?;
    if args.list {
        print_sessions(&sessions);
        return Ok(None);
    }

    if sessions.is_empty() {
        println!("gridbash: no saved sessions found");
        return Ok(None);
    }

    let selected = if let Some(query) = args.session.as_deref() {
        Some(find_session(&sessions, query)?)
    } else if args.latest {
        sessions
            .iter()
            .find(|record| live_owner_pid(record).is_none())
            .cloned()
            .or_else(|| sessions.first().cloned())
    } else if sessions.len() == 1 {
        sessions.first().cloned()
    } else {
        prompt_for_session(&sessions)?
    };

    selected.map(ensure_session_is_resumable).transpose()
}

pub fn load_recent_sessions() -> Result<Vec<SessionRecord>> {
    let directory = sessions_dir()?;
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("failed to read session directory {}", directory.display()))?
    {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }

        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(session) = toml::from_str::<SavedSession>(&raw) else {
            continue;
        };
        if session.version == SESSION_VERSION && !session.id.is_empty() {
            sessions.push(SessionRecord { path, session });
        }
    }

    sessions.sort_by(|left, right| {
        right
            .session
            .updated_at
            .cmp(&left.session.updated_at)
            .then_with(|| right.session.started_at.cmp(&left.session.started_at))
            .then_with(|| right.session.id.cmp(&left.session.id))
    });
    Ok(sessions)
}

fn print_sessions(sessions: &[SessionRecord]) {
    if sessions.is_empty() {
        println!("gridbash: no saved sessions found");
        return;
    }

    for record in sessions {
        println!("{}\t{}", record.session.id, record.summary());
    }
}

fn find_session(sessions: &[SessionRecord], query: &str) -> Result<SessionRecord> {
    let matches = sessions
        .iter()
        .filter(|record| record.session.id == query || record.session.id.starts_with(query))
        .cloned()
        .collect::<Vec<_>>();

    match matches.len() {
        0 => bail!("no saved session matches '{query}'"),
        1 => Ok(matches[0].clone()),
        _ => bail!("session id prefix '{query}' is ambiguous"),
    }
}

fn prompt_for_session(sessions: &[SessionRecord]) -> Result<Option<SessionRecord>> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return crate::resume_picker::select_session(sessions);
    }

    prompt_for_session_plain(sessions)
}

fn live_owner_pid(record: &SessionRecord) -> Option<u32> {
    record
        .session
        .running
        .then_some(record.session.owner_pid)
        .flatten()
        .filter(|owner_pid| process_is_running(*owner_pid))
}

fn ensure_session_is_resumable(record: SessionRecord) -> Result<SessionRecord> {
    if let Some(owner_pid) = live_owner_pid(&record) {
        bail!(
            "session {} is already open in GridBash (PID {owner_pid}); switch to that client or close it before resuming",
            record.session.id
        );
    }
    Ok(record)
}

fn prompt_for_session_plain(sessions: &[SessionRecord]) -> Result<Option<SessionRecord>> {
    println!("Recent GridBash sessions:");
    for (index, record) in sessions.iter().take(20).enumerate() {
        println!(
            "{:>2}. {}  {}",
            index + 1,
            record.session.id,
            record.summary()
        );
    }

    print!("Select session [1], or q to cancel: ");
    io::stdout().flush().context("failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read session selection")?;
    let input = input.trim();
    if input.eq_ignore_ascii_case("q") {
        return Ok(None);
    }

    let selected = if input.is_empty() {
        1
    } else {
        input
            .parse::<usize>()
            .with_context(|| format!("invalid session selection '{input}'"))?
    };

    if selected == 0 || selected > sessions.len().min(20) {
        bail!("session selection out of range: {selected}");
    }

    Ok(Some(sessions[selected - 1].clone()))
}

fn prune_old_sessions() -> Result<()> {
    let sessions = load_recent_sessions()?;
    for record in sessions.into_iter().skip(MAX_SAVED_SESSIONS) {
        let _ = fs::remove_file(record.path);
    }
    Ok(())
}

fn sessions_dir() -> Result<PathBuf> {
    ProjectDirs::from("", "", "GridBash")
        .map(|dirs| dirs.data_local_dir().join("sessions"))
        .ok_or_else(|| anyhow!("failed to resolve GridBash session directory"))
}

fn create_private_dir_all(path: &std::path::Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn write_private_file(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("session.toml");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.flush()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn save_session_to_path(path: &Path, session: &SavedSession) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)
            .with_context(|| format!("failed to create session directory {}", parent.display()))?;
    }

    let raw = toml::to_string_pretty(session).context("failed to serialize session")?;
    write_private_file(path, raw.as_bytes())
        .with_context(|| format!("failed to write session {}", path.display()))
}

fn session_file_path(id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("{id}.toml")))
}

fn new_session_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{millis}-{}", std::process::id())
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(unix)]
pub(crate) fn process_is_running(process_id: u32) -> bool {
    let Ok(process_id) = i32::try_from(process_id) else {
        return false;
    };
    if process_id <= 0 {
        return false;
    }

    let result = unsafe { libc::kill(process_id, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub(crate) fn process_is_running(process_id: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED, STILL_ACTIVE},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };

    if process_id == 0 {
        return false;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
    }

    let mut exit_code = 0;
    let queried = unsafe { GetExitCodeProcess(process, &mut exit_code) };
    unsafe { CloseHandle(process) };
    queried != 0 && exit_code == STILL_ACTIVE as u32
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn process_is_running(_process_id: u32) -> bool {
    true
}

fn age_label(updated_at: u64) -> String {
    let elapsed = now_seconds().saturating_sub(updated_at);
    if elapsed < 60 {
        return format!("{elapsed}s ago");
    }

    let minutes = elapsed / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }

    let hours = minutes / 60;
    if hours < 48 {
        return format!("{hours}h ago");
    }

    let days = hours / 24;
    format!("{days}d ago")
}

fn compact_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Option<String> {
    let unique = labels.collect::<BTreeSet<_>>();
    if unique.is_empty() {
        return None;
    }

    let shown = unique.iter().take(3).copied().collect::<Vec<_>>();
    let extra = unique.len().saturating_sub(shown.len());
    let mut label = shown.join(", ");
    if extra > 0 {
        label.push_str(&format!(" +{extra}"));
    }
    Some(label)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn saved_session_restores_launch_plan() {
        let cwd = env::current_dir().expect("cwd");
        let grid = GridSize {
            rows: 1,
            columns: 2,
        };
        let mut plan = LaunchPlan::legacy(
            "cmd".into(),
            Profile {
                command: "cmd".into(),
                args: Vec::new(),
                title: Some("cmd".into()),
                agent_kind: Some(AgentKind::Codex),
            },
            cwd.clone(),
            2,
            grid,
        );
        plan.panes[0].auth_name = Some("codex-2".into());
        plan.panes[0].auth_kind = Some(AgentKind::Codex);

        let session = SavedSession::new("test".into(), "Grid 1", &plan);
        let restored = session.launch_plan().expect("launch plan");

        assert_eq!(restored.grid, grid);
        assert_eq!(restored.panes.len(), 2);
        assert_eq!(restored.panes[0].profile_name, "cmd");
        assert_eq!(restored.panes[0].cwd, cwd);
        assert_eq!(restored.panes[0].auth_name.as_deref(), Some("codex-2"));
        assert_eq!(restored.panes[0].auth_kind, Some(AgentKind::Codex));
    }

    #[test]
    fn saved_codex_thread_relaunches_with_resume() {
        let mut pane = pane("fluent", "codex");
        pane.command.agent_kind = Some(AgentKind::Codex);
        pane.command.args = vec!["--dangerously-bypass-approvals-and-sandbox".into()];
        pane.codex_thread_id = Some("019f7b81-de49-7782-8186-a3dc2c644c61".into());

        let launch = pane.launch_spec();

        assert_eq!(launch.profile_name, "codex");
        assert_eq!(
            launch.command.args,
            [
                "--dangerously-bypass-approvals-and-sandbox",
                "resume",
                "019f7b81-de49-7782-8186-a3dc2c644c61",
            ]
        );
    }

    #[test]
    fn shell_running_codex_becomes_a_resumable_codex_pane() {
        let mut pane = pane("fluent", "git-bash");
        pane.codex_thread_id = Some("019f7b81-e026-7d12-a013-25f4763f4bce".into());

        let launch = pane.launch_spec();

        assert_eq!(launch.profile_name, "codex");
        assert_eq!(launch.command.command, "codex");
        assert_eq!(
            launch.command.args,
            ["resume", "019f7b81-e026-7d12-a013-25f4763f4bce"]
        );
        assert_eq!(launch.command.agent_kind, Some(AgentKind::Codex));
    }

    #[test]
    fn extracts_codex_resume_id_from_shell_history() {
        let command = Profile {
            command: "bash".into(),
            args: Vec::new(),
            title: Some("Git Bash".into()),
            agent_kind: None,
        };

        assert_eq!(
            codex_resume_id(
                &command,
                &["codex resume 019f7b81-e2cd-71c1-84dd-9f09622cf74e".into()]
            )
            .as_deref(),
            Some("019f7b81-e2cd-71c1-84dd-9f09622cf74e")
        );
    }

    #[test]
    fn preserves_a_wrapper_that_already_resumes_the_saved_thread() {
        let thread_id = "019f7b81-de49-7782-8186-a3dc2c644c61";
        let command = Profile {
            command: "powershell.exe".into(),
            args: vec![
                "-Command".into(),
                format!("& codex --dangerously-bypass-approvals-and-sandbox resume {thread_id}"),
            ],
            title: Some("Codex".into()),
            agent_kind: Some(AgentKind::Codex),
        };

        let (profile_name, restored) = codex_resume_profile("codex", &command, thread_id);
        assert_eq!(profile_name, "codex");
        assert_eq!(restored.command, command.command);
        assert_eq!(restored.args, command.args);
    }

    #[test]
    fn summarizes_unique_folders_and_profiles() {
        let session = SavedSession {
            version: SESSION_VERSION,
            id: "test".into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            title: "Grid 1".into(),
            grid: SavedGrid {
                rows: 2,
                columns: 2,
            },
            panes: vec![
                pane("one", "claude"),
                pane("two", "codex"),
                pane("one", "claude"),
            ],
            background_panes: Vec::new(),
            tabs: Vec::new(),
            running: false,
            owner_pid: None,
            recovered_at: None,
        };

        let summary = session.summary();

        assert!(summary.contains("2x2"));
        assert!(summary.contains("3 panes"));
        assert!(summary.contains("one, two"));
        assert!(summary.contains("claude, codex"));
    }

    #[test]
    fn saved_session_round_trips_background_tabs() {
        let mut background_pane = pane("background", "cmd");
        background_pane.host = Some(PtyHostRef {
            endpoint: "127.0.0.1:32123".into(),
            token: "secret".into(),
            codex_sqlite_home: None,
            started_at_ms: None,
        });
        let mut session = SavedSession {
            version: SESSION_VERSION,
            id: "test".into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            title: "Grid 1".into(),
            grid: SavedGrid {
                rows: 1,
                columns: 1,
            },
            panes: vec![pane("active", "cmd")],
            background_panes: Vec::new(),
            tabs: vec![SavedTab {
                title: "Long build".into(),
                grid: SavedGrid {
                    rows: 1,
                    columns: 1,
                },
                panes: vec![background_pane],
            }],
            running: false,
            owner_pid: None,
            recovered_at: None,
        };

        let raw = toml::to_string(&session).expect("serialize session");
        session = toml::from_str(&raw).expect("parse session");

        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tabs[0].title, "Long build");
        assert!(session.tabs[0].panes[0].host.is_some());
        assert_eq!(
            session.tabs[0]
                .launch_plan()
                .expect("tab launch plan")
                .panes
                .len(),
            1
        );
    }

    #[test]
    fn background_panes_round_trip_and_default_for_older_sessions() {
        let mut session = SavedSession {
            version: SESSION_VERSION,
            id: "background-test".into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            title: "Grid 1".into(),
            grid: SavedGrid {
                rows: 1,
                columns: 1,
            },
            panes: vec![pane("visible", "cmd")],
            background_panes: vec![SavedBackgroundPane {
                id: 9,
                source_tab: "Grid 2".into(),
                name: Some("auth fix".into()),
                pane: pane("hidden", "codex"),
            }],
            tabs: Vec::new(),
            running: false,
            owner_pid: None,
            recovered_at: None,
        };
        session.background_panes[0].pane.history.output_tail = "tests passing".into();

        let raw = toml::to_string(&session).expect("serialize session");
        let restored: SavedSession = toml::from_str(&raw).expect("restore session");
        assert_eq!(restored.background_panes.len(), 1);
        assert_eq!(restored.background_panes[0].id, 9);
        assert_eq!(restored.background_panes[0].source_tab, "Grid 2");
        assert_eq!(
            restored.background_panes[0].pane.history.output_tail,
            "tests passing"
        );

        let without_background = raw
            .split("[[background_panes]]")
            .next()
            .expect("visible session prefix");
        let restored: SavedSession =
            toml::from_str(without_background).expect("restore old session");
        assert!(restored.background_panes.is_empty());
    }

    #[test]
    fn running_metadata_defaults_to_closed_for_older_sessions() {
        let plan = LaunchPlan::legacy(
            "codex".into(),
            Profile {
                command: "codex".into(),
                args: Vec::new(),
                title: Some("Codex".into()),
                agent_kind: Some(AgentKind::Codex),
            },
            env::current_dir().expect("cwd"),
            1,
            GridSize::from_count(1),
        );
        let session = SavedSession::new("running-test".into(), "Grid 1", &plan);
        assert!(session.running);
        assert_eq!(session.owner_pid, Some(std::process::id()));

        let raw = toml::to_string(&session)
            .expect("serialize")
            .lines()
            .filter(|line| !line.starts_with("running =") && !line.starts_with("owner_pid ="))
            .collect::<Vec<_>>()
            .join("\n");
        let old_session: SavedSession = toml::from_str(&raw).expect("parse old session");
        assert!(!old_session.running);
        assert!(old_session.owner_pid.is_none());
        assert!(old_session.recovered_at.is_none());
    }

    #[test]
    fn interruption_detection_ignores_live_owners() {
        let mut session = recovery_session("live", vec![pane("alpha", "codex")]);
        session.panes[0].command.agent_kind = Some(AgentKind::Codex);
        session.running = true;
        session.owner_pid = Some(std::process::id());
        let mut record = SessionRecord {
            path: PathBuf::from("live.toml"),
            session,
        };
        assert!(!is_interrupted_agent_session(&record));

        record.session.owner_pid = Some(u32::MAX);
        assert!(is_interrupted_agent_session(&record));

        record.session.recovered_at = Some(now_seconds());
        assert!(!is_interrupted_agent_session(&record));
    }

    #[test]
    fn resume_rejects_only_sessions_with_live_owners() {
        let mut session = recovery_session("live", vec![pane("alpha", "codex")]);
        session.running = true;
        session.owner_pid = Some(std::process::id());
        let record = SessionRecord {
            path: PathBuf::from("live.toml"),
            session,
        };

        let error = ensure_session_is_resumable(record.clone()).expect_err("live owner rejected");
        assert!(error.to_string().contains("already open"));

        let mut interrupted = record;
        interrupted.session.owner_pid = Some(u32::MAX);
        ensure_session_is_resumable(interrupted).expect("dead owner can be recovered");
    }

    #[test]
    fn interrupted_sessions_are_grouped_into_directory_named_tabs() {
        let alpha = PathBuf::from("workspaces").join("alpha");
        let beta = PathBuf::from("workspaces").join("beta");
        let mut alpha_one = pane("ignored", "codex");
        alpha_one.cwd = alpha.clone();
        alpha_one.history.output_tail = "first conversation".into();
        let mut beta_one = pane("ignored", "claude");
        beta_one.cwd = beta.clone();
        let mut alpha_two = pane("ignored", "codex");
        alpha_two.cwd = alpha;
        let mut beta_two = pane("ignored", "claude");
        beta_two.cwd = beta;

        let mut first = recovery_session("first", vec![alpha_one]);
        first.tabs.push(SavedTab {
            title: "old title".into(),
            grid: GridSize::from_count(1).into(),
            panes: vec![beta_one],
        });
        let mut second = recovery_session("second", vec![alpha_two]);
        second.background_panes.push(SavedBackgroundPane {
            id: 4,
            source_tab: "other".into(),
            name: None,
            pane: beta_two,
        });
        let records = vec![
            SessionRecord {
                path: PathBuf::from("first.toml"),
                session: first,
            },
            SessionRecord {
                path: PathBuf::from("second.toml"),
                session: second,
            },
        ];

        let recovery = build_interrupted_recovery(&records).expect("recovery");

        assert_eq!(recovery.session_count, 2);
        assert_eq!(recovery.pane_count, 4);
        assert_eq!(
            recovery
                .tabs
                .iter()
                .map(|tab| tab.title.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(recovery.tabs[0].panes.len(), 2);
        assert_eq!(recovery.tabs[1].panes.len(), 2);
        assert_eq!(recovery.tabs[0].panes[0].index, 0);
        assert_eq!(recovery.tabs[0].panes[1].index, 1);
        assert_eq!(
            recovery.tabs[0].panes[0].history.output_tail,
            "first conversation"
        );
    }

    fn recovery_session(id: &str, panes: Vec<SavedPane>) -> SavedSession {
        SavedSession {
            version: SESSION_VERSION,
            id: id.into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            title: "Grid 1".into(),
            grid: GridSize::from_count(panes.len()).into(),
            panes,
            background_panes: Vec::new(),
            tabs: Vec::new(),
            running: false,
            owner_pid: None,
            recovered_at: None,
        }
    }

    fn pane(folder_name: &str, profile_name: &str) -> SavedPane {
        SavedPane {
            index: 0,
            profile_name: profile_name.into(),
            command: Profile {
                command: profile_name.into(),
                args: Vec::new(),
                title: None,
                agent_kind: None,
            },
            cwd: PathBuf::from("."),
            folder_name: folder_name.into(),
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            history: SavedPaneHistory::default(),
            codex_thread_id: None,
            host: None,
        }
    }
}
