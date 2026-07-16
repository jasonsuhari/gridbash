use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use crate::{
    auth::AgentKind,
    cli::ResumeArgs,
    layout::GridSize,
    pane_host::{PtyHostRef, PtyPane},
    profiles::Profile,
    setup::{LaunchPlan, PaneLaunchSpec},
};

const SESSION_VERSION: u16 = 1;
const MAX_SAVED_SESSIONS: usize = 50;

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
                    SavedPane::from_spec(index, spec, SavedPaneHistory::default(), None)
                })
                .collect(),
            background_panes: Vec::new(),
            tabs: Vec::new(),
        }
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
    let panes = panes
        .iter()
        .map(|pane| PaneLaunchSpec {
            profile_name: pane.profile_name.clone(),
            command: pane.command.clone(),
            env: BTreeMap::new(),
            cwd: pane.cwd.clone(),
            folder_name: pane.folder_name.clone(),
            worktree_name: pane.worktree_name.clone(),
            auth_name: pane.auth_name.clone(),
            auth_kind: pane.auth_kind,
            auth_dir: None,
        })
        .collect::<Vec<_>>();
    if panes.is_empty() {
        bail!("saved session {id} has no panes");
    }
    Ok(LaunchPlan { panes, grid })
}

fn saved_panes_from_live(plan: &LaunchPlan, panes: &[PtyPane]) -> Vec<SavedPane> {
    plan.panes
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let history = panes
                .get(index)
                .map(SavedPaneHistory::from_pane)
                .unwrap_or_default();
            let host = panes.get(index).map(PtyPane::host_ref);
            SavedPane::from_spec(index, spec, history, host)
        })
        .collect()
}

impl SavedPane {
    pub fn launch_spec(&self) -> PaneLaunchSpec {
        PaneLaunchSpec {
            profile_name: self.profile_name.clone(),
            command: self.command.clone(),
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
        Self::from_spec(0, spec, history, host)
    }

    fn from_spec(
        index: usize,
        spec: &PaneLaunchSpec,
        history: SavedPaneHistory,
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
            host,
        }
    }
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

    pub fn continue_record(record: SessionRecord) -> Self {
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
        if let Some(parent) = self.path.parent() {
            create_private_dir_all(parent).with_context(|| {
                format!("failed to create session directory {}", parent.display())
            })?;
        }

        let raw = toml::to_string_pretty(&self.session).context("failed to serialize session")?;
        write_private_file(&self.path, raw.as_bytes())
            .with_context(|| format!("failed to write session {}", self.path.display()))
    }
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

    if let Some(query) = args.session.as_deref() {
        return find_session(&sessions, query).map(Some);
    }

    if args.latest || sessions.len() == 1 {
        return Ok(sessions.into_iter().next());
    }

    prompt_for_session(&sessions)
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
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
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
            host: None,
        }
    }
}
