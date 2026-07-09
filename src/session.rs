use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{
    cli::ResumeArgs,
    layout::GridSize,
    profiles::Profile,
    pty::PtyPane,
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
    pub grid: SavedGrid,
    #[serde(default)]
    pub panes: Vec<SavedPane>,
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
    #[serde(default)]
    pub history: SavedPaneHistory,
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
        let grid = GridSize::new(self.grid.rows, self.grid.columns).ok_or_else(|| {
            anyhow!(
                "saved session {} has invalid grid {}x{}",
                self.id,
                self.grid.rows,
                self.grid.columns
            )
        })?;

        let panes = self
            .panes
            .iter()
            .map(|pane| PaneLaunchSpec {
                profile_name: pane.profile_name.clone(),
                command: pane.command.clone(),
                env: BTreeMap::new(),
                cwd: pane.cwd.clone(),
                folder_name: pane.folder_name.clone(),
                worktree_name: pane.worktree_name.clone(),
                auth_name: None,
                auth_kind: None,
                auth_dir: None,
            })
            .collect::<Vec<_>>();

        if panes.is_empty() {
            bail!("saved session {} has no panes", self.id);
        }

        Ok(LaunchPlan { panes, grid })
    }

    pub fn pane_histories(&self) -> Vec<SavedPaneHistory> {
        self.panes.iter().map(|pane| pane.history.clone()).collect()
    }

    fn new(id: String, plan: &LaunchPlan) -> Self {
        let now = now_seconds();
        Self {
            version: SESSION_VERSION,
            id,
            started_at: now,
            updated_at: now,
            grid: plan.grid.into(),
            panes: plan
                .panes
                .iter()
                .enumerate()
                .map(|(index, spec)| SavedPane::from_spec(index, spec, SavedPaneHistory::default()))
                .collect(),
        }
    }

    fn update_from_live(&mut self, plan: &LaunchPlan, panes: &[PtyPane]) {
        self.version = SESSION_VERSION;
        self.updated_at = now_seconds();
        self.grid = plan.grid.into();
        self.panes = plan
            .panes
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                let history = panes
                    .get(index)
                    .map(SavedPaneHistory::from_pane)
                    .unwrap_or_default();
                SavedPane::from_spec(index, spec, history)
            })
            .collect();
    }

    fn summary(&self) -> String {
        let folders = compact_labels(
            self.panes
                .iter()
                .map(|pane| pane.folder_name.as_str())
                .filter(|name| !name.is_empty()),
        );
        let profiles = compact_labels(
            self.panes
                .iter()
                .map(|pane| pane.profile_name.as_str())
                .filter(|name| !name.is_empty()),
        );

        format!(
            "{} | {}x{} | {} pane{} | {} | {}",
            age_label(self.updated_at),
            self.grid.rows,
            self.grid.columns,
            self.panes.len(),
            if self.panes.len() == 1 { "" } else { "s" },
            folders.unwrap_or_else(|| "unknown folders".into()),
            profiles.unwrap_or_else(|| "unknown profiles".into())
        )
    }
}

impl SavedPane {
    fn from_spec(index: usize, spec: &PaneLaunchSpec, history: SavedPaneHistory) -> Self {
        Self {
            index,
            profile_name: spec.profile_name.clone(),
            command: spec.command.clone(),
            cwd: spec.cwd.clone(),
            folder_name: spec.folder_name.clone(),
            worktree_name: spec.worktree_name.clone(),
            history,
        }
    }
}

impl SavedPaneHistory {
    fn from_pane(pane: &PtyPane) -> Self {
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
    pub fn start_new(plan: &LaunchPlan) -> Result<Self> {
        let id = new_session_id();
        let path = session_file_path(&id)?;
        let recorder = Self {
            path,
            session: SavedSession::new(id, plan),
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

    pub fn update(&mut self, plan: &LaunchPlan, panes: &[PtyPane]) {
        self.session.update_from_live(plan, panes);
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create session directory {}", parent.display())
            })?;
        }

        let raw = toml::to_string_pretty(&self.session).context("failed to serialize session")?;
        fs::write(&self.path, raw)
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
        let plan = LaunchPlan::legacy(
            "cmd".into(),
            Profile {
                command: "cmd".into(),
                args: Vec::new(),
                title: Some("cmd".into()),
                agent_kind: None,
            },
            cwd.clone(),
            2,
            grid,
        );

        let session = SavedSession::new("test".into(), &plan);
        let restored = session.launch_plan().expect("launch plan");

        assert_eq!(restored.grid, grid);
        assert_eq!(restored.panes.len(), 2);
        assert_eq!(restored.panes[0].profile_name, "cmd");
        assert_eq!(restored.panes[0].cwd, cwd);
    }

    #[test]
    fn summarizes_unique_folders_and_profiles() {
        let session = SavedSession {
            version: SESSION_VERSION,
            id: "test".into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            grid: SavedGrid {
                rows: 2,
                columns: 2,
            },
            panes: vec![
                pane("one", "claude"),
                pane("two", "codex"),
                pane("one", "claude"),
            ],
        };

        let summary = session.summary();

        assert!(summary.contains("2x2"));
        assert!(summary.contains("3 panes"));
        assert!(summary.contains("one, two"));
        assert!(summary.contains("claude, codex"));
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
            history: SavedPaneHistory::default(),
        }
    }
}
