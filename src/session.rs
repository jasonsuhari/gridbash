use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::{BaseDirs, ProjectDirs};
use fs4::FileExt;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use crate::{
    auth::AgentKind,
    cli::ResumeArgs,
    layout::{GridSize, MAX_PANES},
    pane_host::{PtyHostRef, PtyPane, terminate_saved_host},
    ports::roots_with_descendant_named,
    profiles::Profile,
    setup::{LaunchPlan, PaneLaunchSpec, folder_display_name},
};

/// Snapshot format GridBash writes. Version 2 added the state that made a
/// resumed workspace identical to the one that closed: pane names, the focused
/// pane, zoom, divider weights, tab order, and the Claude conversation each
/// agent pane was talking to.
const SESSION_VERSION: u16 = 2;
/// Oldest format still understood. Older snapshots load with the version 2
/// fields defaulted, so upgrading GridBash never strands a saved workspace.
const MIN_SESSION_VERSION: u16 = 1;
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
    /// Where the grid held in `panes` sits among `tabs`. Without it a resumed
    /// workspace reordered every grid so the active one came first.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub active_tab: usize,
    /// Number the next new grid should take, so resuming does not hand out a
    /// name an existing grid already uses.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub next_tab_number: usize,
    pub grid: SavedGrid,
    #[serde(default, skip_serializing_if = "SavedView::is_default")]
    pub view: SavedView,
    #[serde(default)]
    pub panes: Vec<SavedPane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_panes: Vec<SavedBackgroundPane>,
    /// Every grid except the active one, in workspace order.
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
    /// Grid slot this pane occupied, counting left to right then top to bottom.
    /// Restores read it back so a pane never lands in a different cell.
    pub index: usize,
    pub profile_name: String,
    pub command: Profile,
    pub cwd: PathBuf,
    pub folder_name: String,
    /// Name the user typed for this pane, kept separate from the profile title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_kind: Option<AgentKind>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sleeping: bool,
    #[serde(default)]
    pub history: SavedPaneHistory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<PtyHostRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub grid: SavedGrid,
    #[serde(default, skip_serializing_if = "SavedView::is_default")]
    pub view: SavedView,
    #[serde(default)]
    pub panes: Vec<SavedPane>,
}

/// How a grid looked, beyond which panes it held: which pane had focus, whether
/// it was zoomed, and where the user dragged its dividers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedView {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub focus: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    pub zoomed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_weights: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_weights: Vec<u16>,
}

impl SavedView {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
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
    /// Every grid from every interrupted workspace, in the order it was shown.
    pub tabs: Vec<SavedTab>,
    /// Grid to open on, so recovery lands where the user left off.
    pub active_tab: usize,
    pub background_panes: Vec<SavedBackgroundPane>,
    pub next_tab_number: usize,
    pub session_count: usize,
    pub pane_count: usize,
    pub claim: InterruptedRecoveryClaim,
}

#[derive(Debug, Clone)]
pub struct InterruptedRecoveryClaim {
    sources: Vec<RecoverySource>,
}

#[derive(Debug, Clone)]
struct RecoverySource {
    path: PathBuf,
    id: String,
}

pub struct SessionRecorder {
    path: PathBuf,
    session: SavedSession,
    /// Digest of the last session written to disk.
    ///
    /// The serialized session embeds every pane's output tail, so keeping the
    /// text itself pinned a multi-megabyte copy for the life of the process.
    /// Only the change check needs it, and a digest answers that in 8 bytes.
    last_saved_digest: Option<u64>,
}

impl SessionRecord {
    pub fn summary(&self) -> String {
        self.session.summary()
    }
}

impl SavedSession {
    /// The active grid described the same way as every other grid, so restore
    /// paths only ever handle one shape.
    pub fn active_grid(&self) -> SavedTab {
        SavedTab {
            title: self.title.clone(),
            grid: self.grid,
            view: self.view.clone(),
            panes: self.panes.clone(),
        }
    }

    /// Every grid in the order the workspace showed them, plus the position the
    /// active grid held.
    pub fn ordered_grids(&self) -> (Vec<SavedTab>, usize) {
        let mut grids = self.tabs.clone();
        let active = self.active_tab.min(grids.len());
        grids.insert(active, self.active_grid());
        (grids, active)
    }

    /// Number the workspace should give its next new grid. Older snapshots did
    /// not record it, so fall back to one past the grids they do describe.
    pub fn next_grid_number(&self) -> usize {
        self.next_tab_number.max(self.tabs.len() + 2)
    }

    pub fn all_panes(&self) -> impl Iterator<Item = &SavedPane> {
        self.panes
            .iter()
            .chain(self.tabs.iter().flat_map(|tab| tab.panes.iter()))
            .chain(self.background_panes.iter().map(|job| &job.pane))
    }

    fn new(id: String, title: &str, plan: &LaunchPlan) -> Self {
        let now = now_seconds();
        Self {
            version: SESSION_VERSION,
            id,
            started_at: now,
            updated_at: now,
            title: title.to_string(),
            active_tab: 0,
            next_tab_number: 2,
            grid: plan.grid.into(),
            view: SavedView::default(),
            panes: plan
                .panes
                .iter()
                .enumerate()
                .map(|(index, spec)| {
                    SavedPane::from_spec(index, spec, SavedPaneHistory::default(), None, None, None)
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
        self.updated_at = now_seconds();
    }

    fn has_agent_pane(&self) -> bool {
        self.all_panes().any(SavedPane::runs_an_agent)
    }

    /// Takes the captured workspace by value: it carries every pane's output
    /// tail, and this runs on a timer, so borrowing it here would copy megabytes
    /// on each save for no reason.
    fn update_from_live(&mut self, live: LiveWorkspace<'_>) {
        let active = SavedTab::from_live(&live.active);
        self.version = SESSION_VERSION;
        self.title = active.title;
        self.grid = active.grid;
        self.view = active.view;
        self.panes = active.panes;
        self.tabs = live.tabs;
        self.background_panes = live.background_panes;
        self.active_tab = live.active_tab;
        self.next_tab_number = live.next_tab_number;
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
    pub fn from_live(live: &LiveGrid<'_>) -> Self {
        Self {
            title: live.title.to_string(),
            grid: live.plan.grid.into(),
            view: live.view.clone(),
            panes: saved_panes_from_live(live),
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

    pub fn pane_names(&self) -> Vec<Option<String>> {
        self.panes.iter().map(|pane| pane.name.clone()).collect()
    }

    pub fn sleeping_panes(&self) -> BTreeSet<usize> {
        self.panes
            .iter()
            .enumerate()
            .filter_map(|(index, pane)| pane.sleeping.then_some(index))
            .collect()
    }

    /// Focused pane, clamped to the panes that actually came back.
    pub fn focus(&self) -> usize {
        self.view.focus.min(self.panes.len().saturating_sub(1))
    }
}

/// The live workspace a snapshot is taken from. Grouping it keeps every caller
/// honest about the state a resumed workspace has to reproduce.
pub struct LiveWorkspace<'a> {
    pub active: LiveGrid<'a>,
    pub tabs: Vec<SavedTab>,
    pub background_panes: Vec<SavedBackgroundPane>,
    pub active_tab: usize,
    pub next_tab_number: usize,
}

/// One live grid: what it runs, what the user named it and its panes, and how it
/// was arranged on screen.
pub struct LiveGrid<'a> {
    pub title: &'a str,
    pub plan: &'a LaunchPlan,
    pub panes: &'a [PtyPane],
    pub pane_names: &'a [Option<String>],
    pub sleeping: &'a BTreeSet<usize>,
    pub view: SavedView,
}

/// Rebuild a grid exactly as it was saved.
///
/// The saved dimensions are used verbatim: a 2x3 grid comes back 2x3 even when
/// some of its cells were empty, because a grid recomputed from its pane count
/// silently reshapes the workspace the user arranged.
fn launch_plan_from_saved(
    id: &str,
    saved_grid: SavedGrid,
    panes: &[SavedPane],
) -> Result<LaunchPlan> {
    let mut grid = GridSize::new(saved_grid.rows, saved_grid.columns).ok_or_else(|| {
        anyhow!(
            "saved session {id} has invalid grid {}x{}",
            saved_grid.rows,
            saved_grid.columns
        )
    })?;
    let panes = ordered_panes(panes)
        .into_iter()
        .map(SavedPane::launch_spec)
        .collect::<Vec<_>>();
    if panes.is_empty() {
        bail!("saved session {id} has no panes");
    }
    // A snapshot damaged into claiming more panes than cells would hide the
    // extras behind the grid, which loses work. Growing the grid keeps every
    // pane reachable; a consistent snapshot never reaches this.
    if panes.len() > grid.count() {
        grid = GridSize::from_count(panes.len());
    }
    // A grid can never hold more than `MAX_PANES`, so a file claiming more is
    // damaged. Refusing it here keeps the count from becoming per-pane
    // allocations and PTY spawns.
    if panes.len() > MAX_PANES {
        bail!(
            "saved session {id} claims {} panes; the maximum is {MAX_PANES}",
            panes.len()
        );
    }
    Ok(LaunchPlan { panes, grid })
}

/// Panes in grid order, so a snapshot written out of order still restores every
/// pane to the cell it occupied.
fn ordered_panes(panes: &[SavedPane]) -> Vec<&SavedPane> {
    let mut ordered = panes.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|pane| pane.index);
    ordered
}

fn saved_panes_from_live(live: &LiveGrid<'_>) -> Vec<SavedPane> {
    let host_processes = live
        .panes
        .iter()
        .filter_map(PtyPane::host_process_id)
        .collect::<Vec<_>>();
    let codex_roots = roots_with_descendant_named(&host_processes, "codex").ok();
    live.plan
        .panes
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let pane = live.panes.get(index);
            let history = pane.map(SavedPaneHistory::from_pane).unwrap_or_default();
            let host = pane.map(PtyPane::host_ref);
            let codex_running = pane.is_some_and(|pane| {
                pane.host_process_id().is_some_and(|process_id| {
                    codex_roots
                        .as_ref()
                        .is_some_and(|roots| roots.contains(&process_id))
                }) || (spec.command.agent_kind == Some(AgentKind::Codex) && !pane.exited)
            });
            let codex_thread_id = codex_running
                .then(|| {
                    codex_resume_id(&spec.command, &history.input_history)
                        .or_else(|| pane.and_then(|pane| pane.codex_thread_id(pane.cwd())))
                })
                .flatten();
            let claude_session_id = claude_session_from_live(spec, pane, &history.input_history);
            let mut saved = SavedPane::from_spec(
                index,
                spec,
                history,
                codex_thread_id,
                claude_session_id,
                host,
            );
            saved.name = live
                .pane_names
                .get(index)
                .cloned()
                .flatten()
                .filter(|name| !name.trim().is_empty());
            saved.sleeping = live.sleeping.contains(&index);
            if let Some(pane) = pane {
                saved.cwd = pane.cwd().to_path_buf();
                saved.folder_name = folder_display_name(&saved.cwd);
            }
            saved
        })
        .collect()
}

/// Claude conversation a live pane is attached to.
///
/// A pane GridBash launched carries the id it pinned at spawn, which cannot be
/// confused by a sibling pane in the same directory. A pane where the user
/// started Claude themselves is read back out of what they typed.
fn claude_session_from_live(
    spec: &PaneLaunchSpec,
    pane: Option<&PtyPane>,
    input_history: &[String],
) -> Option<String> {
    pane.and_then(PtyPane::agent_session_id)
        .map(str::to_string)
        .or_else(|| claude_resume_id(&spec.command, input_history))
}

impl SavedPane {
    pub fn launch_spec(&self) -> PaneLaunchSpec {
        let (profile_name, command) = self.resume_command();
        self.spec_for(profile_name, command)
    }

    /// Whether this pane runs an agent, without working out which conversation
    /// it would resume. Checking every saved pane happens on every launch, and
    /// resolving conversations there would put a filesystem lookup per pane in
    /// front of startup.
    fn runs_an_agent(&self) -> bool {
        self.codex_thread_id.is_some()
            || self.claude_session_id.is_some()
            || self
                .spec_for(self.profile_name.clone(), self.command.clone())
                .agent_label()
                .is_some()
    }

    fn spec_for(&self, profile_name: String, command: Profile) -> PaneLaunchSpec {
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

    /// Command that puts this pane back in the conversation it left.
    ///
    /// A Claude transcript that is no longer on disk falls back to a plain
    /// launch, because `claude --resume` on a missing session fails outright and
    /// would leave the pane with no agent at all.
    fn resume_command(&self) -> (String, Profile) {
        if let Some(thread_id) = self.codex_thread_id.as_deref() {
            return codex_resume_profile(&self.profile_name, &self.command, thread_id);
        }
        if let Some(session_id) = self.claude_session_id.as_deref()
            && claude_session_exists(&self.cwd, session_id)
        {
            return claude_resume_profile(&self.profile_name, &self.command, session_id);
        }

        (
            self.profile_name.clone(),
            without_claude_session_pin(&self.command),
        )
    }

    /// Conversation this pane will actually be put back into, for callers that
    /// describe a snapshot rather than launch it. Reports nothing when the
    /// transcript is gone, matching what a resume would really do.
    pub fn resumable_conversation(&self) -> Option<&str> {
        if let Some(thread_id) = self.codex_thread_id.as_deref() {
            return Some(thread_id);
        }
        self.claude_session_id
            .as_deref()
            .filter(|session_id| claude_session_exists(&self.cwd, session_id))
    }

    pub fn from_background(
        spec: &PaneLaunchSpec,
        history: SavedPaneHistory,
        host: Option<PtyHostRef>,
    ) -> Self {
        Self::from_spec(0, spec, history, None, None, host)
    }

    fn from_spec(
        index: usize,
        spec: &PaneLaunchSpec,
        history: SavedPaneHistory,
        codex_thread_id: Option<String>,
        claude_session_id: Option<String>,
        host: Option<PtyHostRef>,
    ) -> Self {
        Self {
            index,
            profile_name: spec.profile_name.clone(),
            command: spec.command.clone(),
            cwd: spec.cwd.clone(),
            folder_name: spec.folder_name.clone(),
            name: None,
            worktree_name: spec.worktree_name.clone(),
            auth_name: spec.auth_name.clone(),
            auth_kind: spec.auth_kind,
            sleeping: false,
            history,
            codex_thread_id,
            claude_session_id,
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

/// Flags that already decide which conversation Claude opens. A pane launched
/// with one of these is left exactly as the user wrote it.
const CLAUDE_SESSION_FLAGS: &[&str] = &[
    "--session-id",
    "--resume",
    "-r",
    "--continue",
    "-c",
    "--fork-session",
    "--from-pr",
    "--no-session-persistence",
];

/// Pin the conversation a Claude pane is about to open.
///
/// Codex records its thread in a state database GridBash can read afterwards,
/// but Claude leaves nothing that ties a transcript to a particular pane. So
/// GridBash chooses the session id up front and passes it in: the snapshot then
/// names the conversation exactly, even with several Claude panes in one folder.
///
/// Returns the conversation this pane will be talking to, if it is knowable.
pub fn pin_claude_session(command: &mut Profile) -> Option<String> {
    if command.agent_kind != Some(AgentKind::Claude) || !is_direct_claude_command(&command.command)
    {
        return None;
    }
    if let Some(session_id) = claude_session_id_in_args(&command.args) {
        return Some(session_id);
    }
    if command
        .args
        .iter()
        .any(|argument| CLAUDE_SESSION_FLAGS.contains(&argument.as_str()))
    {
        // The user asked for a specific conversation without naming an id, such
        // as `--continue`. Overriding that would open the wrong one.
        return None;
    }

    let session_id = new_session_uuid()?;
    command.args.push("--session-id".into());
    command.args.push(session_id.clone());
    Some(session_id)
}

/// Conversation a command already names, without choosing a new one. Used when
/// reattaching to a terminal that is still running: its agent is already in a
/// conversation, so inventing an id would misname it.
pub fn claude_session_in_command(command: &Profile) -> Option<String> {
    claude_session_id_in_args(&command.args)
}

fn claude_resume_profile(
    profile_name: &str,
    command: &Profile,
    session_id: &str,
) -> (String, Profile) {
    if command.agent_kind == Some(AgentKind::Claude) && is_direct_claude_command(&command.command) {
        let mut command = command.clone();
        command.args = claude_resume_args(&command.args, session_id);
        return (profile_name.to_string(), command);
    }
    if claude_resume_id(command, &[]).as_deref() == Some(session_id) {
        return (profile_name.to_string(), command.clone());
    }

    (
        "claude".into(),
        Profile {
            command: "claude".into(),
            args: vec!["--resume".into(), session_id.into()],
            title: Some("Claude".into()),
            agent_kind: Some(AgentKind::Claude),
        },
    )
}

/// Swap whatever conversation selection the pane carried for a resume of
/// `session_id`. Re-passing `--session-id` would ask Claude to create a session
/// that already exists, which it refuses.
fn claude_resume_args(args: &[String], session_id: &str) -> Vec<String> {
    let mut resumed = args_without_claude_session_selection(args);
    resumed.push("--resume".into());
    resumed.push(session_id.to_string());
    resumed
}

fn without_claude_session_pin(command: &Profile) -> Profile {
    if claude_session_id_in_args(&command.args).is_none() {
        return command.clone();
    }

    let mut command = command.clone();
    command.args = args_without_claude_session_selection(&command.args);
    command
}

fn args_without_claude_session_selection(args: &[String]) -> Vec<String> {
    let mut kept = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        if CLAUDE_SESSION_FLAGS.contains(&argument) {
            index += 1;
            if args
                .get(index)
                .is_some_and(|value| looks_like_thread_id(value))
            {
                index += 1;
            }
            continue;
        }
        if let Some(value) = argument.strip_prefix("--session-id=")
            && looks_like_thread_id(value)
        {
            index += 1;
            continue;
        }
        kept.push(args[index].clone());
        index += 1;
    }
    kept
}

fn claude_resume_id(command: &Profile, input_history: &[String]) -> Option<String> {
    claude_session_id_in_args(&command.args).or_else(|| {
        input_history
            .iter()
            .rev()
            .find_map(|input| claude_session_id_in_text(input))
    })
}

fn claude_session_id_in_args(args: &[String]) -> Option<String> {
    args.iter()
        .enumerate()
        .find_map(|(index, argument)| {
            (CLAUDE_SESSION_FLAGS.contains(&argument.as_str()))
                .then(|| args.get(index + 1))
                .flatten()
                .filter(|value| looks_like_thread_id(value))
                .cloned()
        })
        .or_else(|| {
            args.iter().find_map(|argument| {
                argument
                    .strip_prefix("--session-id=")
                    .filter(|value| looks_like_thread_id(value))
                    .map(str::to_string)
            })
        })
}

/// Read a Claude conversation id out of a command the user typed into a shell
/// pane, so panes GridBash did not launch itself are still preserved.
fn claude_session_id_in_text(value: &str) -> Option<String> {
    let tokens = value
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| matches!(character, '"' | '\'' | ';' | ','))
        })
        .collect::<Vec<_>>();
    if !tokens.iter().any(|token| {
        Path::new(token)
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("claude"))
    }) {
        return None;
    }

    tokens.windows(2).find_map(|tokens| {
        (CLAUDE_SESSION_FLAGS.contains(&tokens[0]) && looks_like_thread_id(tokens[1]))
            .then(|| tokens[1].to_string())
    })
}

fn is_direct_claude_command(command: &str) -> bool {
    Path::new(command)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("claude"))
}

/// Number of Claude project folders scanned when the encoded folder name does
/// not match. Bounded so a huge history cannot stall a resume.
const MAX_CLAUDE_PROJECT_SCAN: usize = 4096;

/// Number of transcripts examined when following a pane onto a new
/// conversation. Bounded for the same reason.
const MAX_CLAUDE_TRANSCRIPT_SCAN: usize = 4096;

/// Conversation a pane has moved on to since it launched.
///
/// Clearing a conversation starts a new one inside the same terminal, so the id
/// pinned at launch goes stale and the snapshot would resume the abandoned
/// conversation. The pane's current one is the newest transcript in its folder
/// that appeared after the pane started and that no other pane is following.
///
/// Returns nothing when the pane is still on the conversation it had.
pub fn latest_claude_session(
    cwd: &Path,
    current: Option<&str>,
    started_at_ms: Option<u64>,
    followed: &BTreeSet<String>,
) -> Option<String> {
    let directory = claude_projects_dir()?.join(claude_project_folder(cwd));
    latest_claude_session_in(&directory, current, started_at_ms, followed)
}

fn latest_claude_session_in(
    directory: &Path,
    current: Option<&str>,
    started_at_ms: Option<u64>,
    followed: &BTreeSet<String>,
) -> Option<String> {
    // Only a transcript newer than the one the pane already has can be a
    // conversation it moved to.
    let floor = current
        .and_then(|session_id| modified_ms(&directory.join(format!("{session_id}.jsonl"))))
        .or(started_at_ms)
        .unwrap_or_default();

    let mut newest: Option<(u64, String)> = None;
    for entry in fs::read_dir(directory)
        .ok()?
        .flatten()
        .take(MAX_CLAUDE_TRANSCRIPT_SCAN)
    {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(session_id) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        // A conversation another pane is following belongs to that pane.
        if !looks_like_thread_id(session_id)
            || Some(session_id) == current
            || followed.contains(session_id)
        {
            continue;
        }
        let Some(modified) = modified_ms(&path) else {
            continue;
        };
        if modified <= floor || started_at_ms.is_some_and(|started| modified < started) {
            continue;
        }
        if newest
            .as_ref()
            .is_none_or(|(newest_ms, _)| modified > *newest_ms)
        {
            newest = Some((modified, session_id.to_string()));
        }
    }

    newest.map(|(_, session_id)| session_id)
}

fn modified_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
}

/// Whether Claude can still open this conversation.
///
/// `claude --resume` on a transcript that is gone exits immediately, which would
/// leave the pane empty instead of running an agent. Checking first lets the
/// restore fall back to a fresh Claude in the right folder.
fn claude_session_exists(cwd: &Path, session_id: &str) -> bool {
    if !looks_like_thread_id(session_id) {
        return false;
    }
    let Some(root) = claude_projects_dir() else {
        return false;
    };

    let transcript = format!("{session_id}.jsonl");
    if root
        .join(claude_project_folder(cwd))
        .join(&transcript)
        .is_file()
    {
        return true;
    }

    // Claude derives the folder name from the working directory, and that
    // encoding has changed shape before. Scanning keeps a resumable
    // conversation from being dropped over a spelling difference.
    claude_project_dirs()
        .iter()
        .any(|directory| directory.join(&transcript).is_file())
}

/// Folders Claude keeps transcripts in, listed once.
///
/// The fallback above runs for every pane whose transcript is not where it was
/// expected, and re-reading the directory each time would put a listing per pane
/// in front of a resume.
fn claude_project_dirs() -> &'static [PathBuf] {
    static DIRECTORIES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRECTORIES.get_or_init(|| {
        let Some(root) = claude_projects_dir() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(root) else {
            return Vec::new();
        };
        entries
            .flatten()
            .take(MAX_CLAUDE_PROJECT_SCAN)
            .map(|entry| entry.path())
            .collect()
    })
}

fn claude_projects_dir() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Some(configured.join("projects"));
    }

    BaseDirs::new().map(|dirs| dirs.home_dir().join(".claude").join("projects"))
}

/// Folder Claude keeps a directory's transcripts in: the path with everything
/// that is not a letter, digit, or dash folded to a dash.
fn claude_project_folder(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// Random version 4 UUID, the form Claude requires for `--session-id`.
fn new_session_uuid() -> Option<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
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
        let mut recorder = Self {
            path,
            session: SavedSession::new(id, title, plan),
            last_saved_digest: None,
        };
        recorder.save()?;
        prune_old_sessions()?;
        Ok(recorder)
    }

    pub fn continue_record(mut record: SessionRecord) -> Self {
        let last_saved_digest = toml::to_string_pretty(&record.session)
            .ok()
            .map(|raw| session_digest(&raw));
        record.session.begin_run();
        Self {
            path: record.path,
            session: record.session,
            last_saved_digest,
        }
    }

    pub fn update(&mut self, live: LiveWorkspace<'_>) {
        self.session.update_from_live(live);
    }

    pub fn save(&mut self) -> Result<()> {
        self.save_if_changed().map(|_| ())
    }

    pub fn save_if_changed(&mut self) -> Result<bool> {
        let raw = toml::to_string_pretty(&self.session).context("failed to serialize session")?;
        let unchanged = self.last_saved_digest == Some(session_digest(&raw));
        // Release the probe before serializing again; the two copies of a
        // session carrying every pane's output tail are the peak of this path.
        drop(raw);
        if unchanged {
            return Ok(false);
        }

        self.session.updated_at = now_seconds();
        let raw = toml::to_string_pretty(&self.session).context("failed to serialize session")?;
        write_session_raw(&self.path, raw.as_bytes())?;
        self.last_saved_digest = Some(session_digest(&raw));
        Ok(true)
    }

    pub fn resume_command(&self) -> String {
        format!("gridbash resume {}", self.session.id)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.session.finish_run();
        self.save()
    }
}

pub fn claim_interrupted_recovery() -> Result<Option<InterruptedRecovery>> {
    with_session_state_lock(claim_interrupted_recovery_locked)
}

pub fn complete_interrupted_recovery(claim: &InterruptedRecoveryClaim) -> Result<()> {
    with_session_state_lock(|| {
        for source in &claim.sources {
            let mut record = load_session_record(&source.path)?;
            if record.session.id != source.id {
                bail!(
                    "saved recovery session {} changed identity while it was claimed",
                    source.id
                );
            }
            if record.session.running
                && record.session.owner_pid == Some(std::process::id())
                && record.session.recovered_at.is_none()
            {
                record.session.mark_recovered();
                save_session_to_path(&record.path, &record.session)?;
            }
        }
        Ok(())
    })
}

fn with_session_state_lock<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let directory = sessions_dir()?;
    create_private_dir_all(&directory)
        .with_context(|| format!("failed to create session directory {}", directory.display()))?;
    let lock_path = directory.join(".sessions.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options
        .open(&lock_path)
        .with_context(|| format!("failed to open session lock {}", lock_path.display()))?;
    FileExt::lock(&lock)
        .with_context(|| format!("failed to lock session state {}", lock_path.display()))?;

    let result = operation();
    let unlock_result = FileExt::unlock(&lock)
        .with_context(|| format!("failed to unlock session state {}", lock_path.display()));
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
        record.session.begin_run();
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

/// Rebuild what the interrupted workspaces looked like.
///
/// Every grid is carried over as it was: its own dimensions, its name, and its
/// panes in the cells they occupied. Earlier versions regrouped loose panes by
/// working directory and sized the result from the pane count, which reshaped a
/// 2x3 grid into 3x3 and replaced grid names with folder names.
fn build_interrupted_recovery(records: &[SessionRecord]) -> Option<InterruptedRecovery> {
    let mut tabs = Vec::new();
    let mut background_panes = Vec::new();
    let mut used_titles = BTreeMap::<String, usize>::new();
    let mut active_tab = 0;
    let mut next_tab_number = 0;

    for (record_index, record) in records.iter().enumerate() {
        let (grids, active) = record.session.ordered_grids();
        let mut recovered_active = None;
        let first = tabs.len();
        for (index, mut grid) in grids.into_iter().enumerate() {
            // A grid with no panes cannot be rebuilt, so it is dropped without
            // moving the grid the user was looking at.
            if grid.panes.is_empty() {
                continue;
            }
            if recovered_active.is_none() && index >= active {
                recovered_active = Some(tabs.len());
            }
            grid.title = unique_grid_title(&mut used_titles, &grid.title);
            tabs.push(grid);
        }

        // The most recently updated workspace is listed first, so opening on its
        // active grid puts the user back where the crash happened.
        if record_index == 0 {
            active_tab = recovered_active.unwrap_or(first);
        }
        background_panes.extend(record.session.background_panes.iter().cloned());
        next_tab_number = next_tab_number.max(record.session.next_grid_number());
    }

    let pane_count = tabs.iter().map(|tab| tab.panes.len()).sum::<usize>() + background_panes.len();
    if tabs.is_empty() {
        return None;
    }

    Some(InterruptedRecovery {
        active_tab: active_tab.min(tabs.len().saturating_sub(1)),
        next_tab_number: next_tab_number.max(tabs.len() + 1),
        tabs,
        background_panes,
        session_count: records.len(),
        pane_count,
        claim: InterruptedRecoveryClaim {
            sources: records
                .iter()
                .map(|record| RecoverySource {
                    path: record.path.clone(),
                    id: record.session.id.clone(),
                })
                .collect(),
        },
    })
}

/// Keep grid names when several interrupted workspaces are recovered together,
/// numbering the duplicates rather than renaming them after a folder.
fn unique_grid_title(used: &mut BTreeMap<String, usize>, title: &str) -> String {
    let base = match title.trim() {
        "" => "Grid".to_string(),
        trimmed => trimmed.to_string(),
    };
    let occurrence = used.entry(base.clone()).or_default();
    *occurrence += 1;
    if *occurrence == 1 {
        base
    } else {
        format!("{base} ({occurrence})")
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

    if args.delete {
        let query = args
            .session
            .as_deref()
            .ok_or_else(|| anyhow!("--delete requires a session id or unique id prefix"))?;
        let record = find_session(&sessions, query)?;
        let id = record.session.id.clone();
        delete_saved_session(&record)?;
        println!("gridbash: deleted saved session {id}");
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

    selected.map(claim_resume_session).transpose()
}

fn claim_resume_session(record: SessionRecord) -> Result<SessionRecord> {
    with_session_state_lock(|| claim_resume_session_locked(record))
}

fn claim_resume_session_locked(record: SessionRecord) -> Result<SessionRecord> {
    let mut current = load_session_record(&record.path)?;
    if current.session.id != record.session.id {
        bail!(
            "saved session {} changed identity while it was selected",
            record.session.id
        );
    }
    current = ensure_session_is_resumable(current)?;
    current.session.begin_run();
    save_session_to_path(&current.path, &current.session)?;
    Ok(current)
}

pub fn delete_saved_session(record: &SessionRecord) -> Result<()> {
    with_session_state_lock(|| {
        // Check the caller's snapshot before touching the filesystem. A session
        // whose owner is still alive must be refused even when its file is
        // already gone, otherwise a live workspace loses its saved state.
        ensure_session_is_closed(record)?;
        let current = match load_session_record(&record.path) {
            Ok(current) => current,
            Err(_) if !record.path.exists() => return Ok(()),
            Err(error) => return Err(error),
        };
        if current.session.id != record.session.id {
            bail!(
                "saved session {} changed identity while it was selected for deletion",
                record.session.id
            );
        }
        delete_saved_session_locked(&current)
    })
}

fn ensure_session_is_closed(record: &SessionRecord) -> Result<()> {
    if let Some(owner_pid) = live_owner_pid(record) {
        bail!(
            "session {} is open in GridBash (PID {owner_pid}); close that client before deleting it",
            record.session.id
        );
    }
    Ok(())
}

fn delete_saved_session_locked(record: &SessionRecord) -> Result<()> {
    ensure_session_is_closed(record)?;

    let mut hosts = BTreeMap::<String, PtyHostRef>::new();
    for pane in record.session.all_panes() {
        if let Some(host) = pane.host.as_ref() {
            hosts
                .entry(host.endpoint.clone())
                .or_insert_with(|| host.clone());
        }
    }

    for host in hosts.values() {
        terminate_saved_host(host).with_context(|| {
            format!(
                "failed to stop a detached terminal for session {}",
                record.session.id
            )
        })?;
    }

    let _ = fs::remove_file(backup_path(&record.path));
    match fs::remove_file(&record.path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to delete saved session {}", record.session.id)),
    }
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

        // Reads the snapshot, falling back to its backup when a crash left the
        // primary copy unreadable.
        if let Ok(record) = load_session_record(&path) {
            sessions.push(record);
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
    with_session_state_lock(|| {
        let sessions = load_recent_sessions()?;
        let mut excess = sessions.len().saturating_sub(MAX_SAVED_SESSIONS);
        if excess == 0 {
            return Ok(());
        }

        for record in sessions.into_iter().rev() {
            if excess == 0 {
                break;
            }
            if live_owner_pid(&record).is_some() || session_has_live_host(&record.session) {
                continue;
            }
            let _ = fs::remove_file(backup_path(&record.path));
            match fs::remove_file(&record.path) {
                Ok(()) => excess -= 1,
                Err(error) if error.kind() == io::ErrorKind::NotFound => excess -= 1,
                Err(_) => {}
            }
        }
        Ok(())
    })
}

fn session_has_live_host(session: &SavedSession) -> bool {
    session
        .all_panes()
        .filter_map(|pane| pane.host.as_ref())
        .any(|host| host.process_id.map(process_is_running).unwrap_or(true))
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
    // Flushing only hands the bytes to the OS. A machine that loses power in
    // that window leaves a snapshot that parses as truncated TOML, so the
    // contents are on disk before the rename publishes them.
    if let Err(error) = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;

    // Keep the snapshot being replaced as a fallback, in case this write or a
    // later one leaves the primary copy unreadable. Moving it aside rather than
    // copying it matters: a busy workspace saves every few seconds, and its
    // snapshot carries every pane's output.
    let _ = fs::rename(path, backup_path(path));
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// Companion file holding the previous good snapshot.
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

/// Fingerprint a serialized session so autosave can skip unchanged writes
/// without keeping the serialized text around.
fn session_digest(raw: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    hasher.finish()
}

fn save_session_to_path(path: &Path, session: &SavedSession) -> Result<()> {
    let raw = toml::to_string_pretty(session).context("failed to serialize session")?;
    write_session_raw(path, raw.as_bytes())
}

fn write_session_raw(path: &Path, raw: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)
            .with_context(|| format!("failed to create session directory {}", parent.display()))?;
    }
    write_private_file(path, raw)
        .with_context(|| format!("failed to write session {}", path.display()))
}

fn load_session_record(path: &Path) -> Result<SessionRecord> {
    // A snapshot damaged by a crash mid-write must not cost the workspace, so
    // the previous good copy is tried before giving up.
    let error = match read_session_file(path) {
        Ok(session) => {
            return Ok(SessionRecord {
                path: path.to_path_buf(),
                session,
            });
        }
        Err(error) => error,
    };
    match read_session_file(&backup_path(path)) {
        Ok(session) => Ok(SessionRecord {
            path: path.to_path_buf(),
            session,
        }),
        Err(_) => Err(error),
    }
}

fn read_session_file(path: &Path) -> Result<SavedSession> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read saved session {}", path.display()))?;
    let session = toml::from_str::<SavedSession>(&raw)
        .with_context(|| format!("failed to parse saved session {}", path.display()))?;
    if !is_supported_session(&session) {
        bail!("saved session {} has an unsupported format", path.display());
    }
    Ok(session)
}

/// Snapshots from older GridBash versions still load; their newer fields simply
/// default, and the next save writes the current format.
fn is_supported_session(session: &SavedSession) -> bool {
    (MIN_SESSION_VERSION..=SESSION_VERSION).contains(&session.version) && !session.id.is_empty()
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

fn is_zero(value: &usize) -> bool {
    *value == 0
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
        let restored = session.active_grid().launch_plan().expect("launch plan");

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
            active_tab: 0,
            next_tab_number: 2,
            grid: SavedGrid {
                rows: 2,
                columns: 2,
            },
            view: SavedView::default(),
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
            process_id: None,
            codex_sqlite_home: None,
            started_at_ms: None,
        });
        let mut session = SavedSession {
            version: SESSION_VERSION,
            id: "test".into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            title: "Grid 1".into(),
            active_tab: 0,
            next_tab_number: 2,
            grid: SavedGrid {
                rows: 1,
                columns: 1,
            },
            view: SavedView::default(),
            panes: vec![pane("active", "cmd")],
            background_panes: Vec::new(),
            tabs: vec![SavedTab {
                title: "Long build".into(),
                grid: SavedGrid {
                    rows: 1,
                    columns: 1,
                },
                view: SavedView::default(),
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
            active_tab: 0,
            next_tab_number: 2,
            grid: SavedGrid {
                rows: 1,
                columns: 1,
            },
            view: SavedView::default(),
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
    fn resume_claim_is_persisted_before_the_session_is_returned() {
        let directory = env::temp_dir().join(format!(
            "gridbash-resume-claim-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("claim.toml");
        let record = SessionRecord {
            path: path.clone(),
            session: recovery_session("claim", vec![pane("alpha", "codex")]),
        };
        save_session_to_path(&path, &record.session).expect("save unclaimed session");

        let claimed = claim_resume_session_locked(record.clone()).expect("claim session");
        assert!(claimed.session.running);
        assert_eq!(claimed.session.owner_pid, Some(std::process::id()));

        let error = claim_resume_session_locked(record).expect_err("second claim rejected");
        assert!(error.to_string().contains("already open"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deleting_a_saved_session_removes_its_snapshot() {
        let directory = env::temp_dir().join(format!(
            "gridbash-delete-session-test-{}-{}",
            std::process::id(),
            now_seconds()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("saved.toml");
        let record = SessionRecord {
            path: path.clone(),
            session: recovery_session("saved", vec![pane("alpha", "codex")]),
        };
        save_session_to_path(&path, &record.session).expect("write session snapshot");

        delete_saved_session(&record).expect("delete saved session");

        assert!(!path.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deleting_an_open_session_is_refused() {
        let mut session = recovery_session("open", vec![pane("alpha", "codex")]);
        session.running = true;
        session.owner_pid = Some(std::process::id());
        let record = SessionRecord {
            path: PathBuf::from("open.toml"),
            session,
        };

        let error = delete_saved_session(&record).expect_err("open session must be protected");

        assert!(error.to_string().contains("close that client"));
    }

    #[test]
    fn session_recorder_skips_unchanged_snapshots() {
        let directory = env::temp_dir().join(format!(
            "gridbash-session-dirty-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("session.toml");
        let mut recorder = SessionRecorder {
            path: path.clone(),
            session: recovery_session("dirty", vec![pane("alpha", "codex")]),
            last_saved_digest: None,
        };

        assert!(recorder.save_if_changed().expect("first save"));
        let first = fs::read(&path).expect("first snapshot");
        assert!(!recorder.save_if_changed().expect("unchanged save"));
        assert_eq!(fs::read(&path).expect("unchanged snapshot"), first);

        recorder.session.title = "Changed".into();
        assert!(recorder.save_if_changed().expect("changed save"));
        assert_ne!(fs::read(&path).expect("changed snapshot"), first);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn live_and_legacy_hosts_are_protected_from_pruning() {
        let mut session = recovery_session("host", vec![pane("alpha", "codex")]);
        session.panes[0].host = Some(PtyHostRef {
            endpoint: "127.0.0.1:12345".into(),
            token: "token".into(),
            process_id: Some(std::process::id()),
            codex_sqlite_home: None,
            started_at_ms: None,
        });
        assert!(session_has_live_host(&session));

        session.panes[0].host.as_mut().expect("host").process_id = Some(u32::MAX);
        assert!(!session_has_live_host(&session));

        session.panes[0].host.as_mut().expect("host").process_id = None;
        assert!(session_has_live_host(&session));
    }

    fn recovery_session(id: &str, panes: Vec<SavedPane>) -> SavedSession {
        SavedSession {
            version: SESSION_VERSION,
            id: id.into(),
            started_at: now_seconds(),
            updated_at: now_seconds(),
            title: "Grid 1".into(),
            active_tab: 0,
            next_tab_number: 2,
            grid: GridSize::from_count(panes.len()).into(),
            view: SavedView::default(),
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
            name: None,
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            sleeping: false,
            history: SavedPaneHistory::default(),
            codex_thread_id: None,
            claude_session_id: None,
            host: None,
        }
    }

    fn grid(rows: usize, columns: usize) -> SavedGrid {
        SavedGrid { rows, columns }
    }

    /// A grid keeps the size the user chose. Sizing it from the pane count is
    /// what turned a 2x3 grid with four panes into a 2x2 one.
    #[test]
    fn saved_grid_dimensions_survive_a_partly_filled_grid() {
        let panes = (0..4)
            .map(|index| {
                let mut pane = pane("fluent", "codex");
                pane.index = index;
                pane
            })
            .collect::<Vec<_>>();

        let plan = launch_plan_from_saved("test", grid(2, 3), &panes).expect("launch plan");

        assert_eq!(
            plan.grid,
            GridSize {
                rows: 2,
                columns: 3
            }
        );
        assert_eq!(plan.panes.len(), 4);
    }

    /// Panes are placed by the cell they recorded, not by where they happen to
    /// sit in the file.
    #[test]
    fn panes_are_restored_into_their_saved_cells() {
        let mut third = pane("third", "codex");
        third.index = 2;
        let mut first = pane("first", "claude");
        first.index = 0;
        let mut second = pane("second", "git-bash");
        second.index = 1;

        let plan = launch_plan_from_saved("test", grid(1, 3), &[third, first, second])
            .expect("launch plan");

        assert_eq!(
            plan.panes
                .iter()
                .map(|pane| pane.folder_name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

    /// The grid that was open goes back where it was, instead of being moved to
    /// the front of the tab strip.
    #[test]
    fn tab_order_and_active_grid_round_trip() {
        let mut session = recovery_session("order", vec![pane("middle", "codex")]);
        session.title = "Middle".into();
        session.active_tab = 1;
        session.tabs = vec![
            SavedTab {
                title: "First".into(),
                grid: grid(1, 1),
                view: SavedView::default(),
                panes: vec![pane("first", "codex")],
            },
            SavedTab {
                title: "Last".into(),
                grid: grid(1, 1),
                view: SavedView::default(),
                panes: vec![pane("last", "codex")],
            },
        ];

        let (grids, active) = session.ordered_grids();

        assert_eq!(
            grids
                .iter()
                .map(|grid| grid.title.as_str())
                .collect::<Vec<_>>(),
            ["First", "Middle", "Last"]
        );
        assert_eq!(active, 1);
    }

    /// Pane names, sleeping panes, focus, zoom, and dragged dividers are part of
    /// what "the same workspace" means.
    #[test]
    fn workspace_details_round_trip_through_toml() {
        let mut named = pane("fluent", "codex");
        named.name = Some("planner".into());
        named.sleeping = true;
        let mut session = recovery_session("details", vec![named, pane("fluent", "claude")]);
        session.grid = grid(1, 2);
        session.view = SavedView {
            focus: 1,
            zoomed: true,
            row_weights: vec![1000],
            column_weights: vec![1400, 600],
        };

        let raw = toml::to_string_pretty(&session).expect("serialize session");
        let restored: SavedSession = toml::from_str(&raw).expect("parse session");

        assert_eq!(restored.grid.rows, 1);
        assert_eq!(restored.grid.columns, 2);
        assert_eq!(restored.view, session.view);
        assert_eq!(restored.panes[0].name.as_deref(), Some("planner"));
        assert!(restored.panes[0].sleeping);
        assert!(!restored.panes[1].sleeping);

        let grid = restored.active_grid();
        assert_eq!(grid.focus(), 1);
        assert_eq!(grid.pane_names(), [Some("planner".into()), None]);
        assert_eq!(grid.sleeping_panes(), BTreeSet::from([0]));
    }

    /// Recovery rebuilds the workspaces that were interrupted, rather than
    /// pouring their panes into new folder-named grids.
    #[test]
    fn recovery_preserves_grid_sizes_names_and_positions() {
        let mut first_pane = pane("alpha", "codex");
        first_pane.name = Some("planner".into());
        first_pane.history.output_tail = "first conversation".into();
        let mut second_pane = pane("alpha", "claude");
        second_pane.index = 1;

        let mut first = recovery_session("first", vec![first_pane, second_pane]);
        first.title = "Main".into();
        first.grid = grid(2, 3);
        first.active_tab = 1;
        first.tabs = vec![SavedTab {
            title: "Reviews".into(),
            grid: grid(1, 2),
            view: SavedView::default(),
            panes: vec![pane("beta", "claude")],
        }];
        let mut second = recovery_session("second", vec![pane("gamma", "codex")]);
        second.title = "Main".into();
        second.grid = grid(1, 1);

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
        // Grid names are kept, and the duplicate is numbered rather than renamed
        // after its folder.
        assert_eq!(
            recovery
                .tabs
                .iter()
                .map(|tab| tab.title.as_str())
                .collect::<Vec<_>>(),
            ["Reviews", "Main", "Main (2)"]
        );
        // Each grid keeps the size it had.
        assert_eq!(recovery.tabs[0].grid.columns, 2);
        assert_eq!(
            (recovery.tabs[1].grid.rows, recovery.tabs[1].grid.columns),
            (2, 3)
        );
        assert_eq!(recovery.tabs[2].grid.rows, 1);
        // Recovery opens on the grid the crash interrupted.
        assert_eq!(recovery.active_tab, 1);
        assert_eq!(recovery.tabs[1].panes[0].name.as_deref(), Some("planner"));
        assert_eq!(
            recovery.tabs[1].panes[0].history.output_tail,
            "first conversation"
        );
        assert_eq!(recovery.claim.sources.len(), 2);
    }

    /// Background panes were dropped entirely by recovery before.
    #[test]
    fn recovery_keeps_background_panes() {
        let mut session = recovery_session("background", vec![pane("alpha", "codex")]);
        session.background_panes.push(SavedBackgroundPane {
            id: 7,
            source_tab: "Main".into(),
            name: Some("long build".into()),
            pane: pane("alpha", "codex"),
        });
        let records = vec![SessionRecord {
            path: PathBuf::from("background.toml"),
            session,
        }];

        let recovery = build_interrupted_recovery(&records).expect("recovery");

        assert_eq!(recovery.background_panes.len(), 1);
        assert_eq!(recovery.background_panes[0].id, 7);
        assert_eq!(recovery.pane_count, 2);
    }

    /// A Claude pane is pinned to a conversation at launch, which is what lets a
    /// snapshot name it later.
    #[test]
    fn claude_panes_are_pinned_to_a_session_at_launch() {
        let mut command = Profile {
            command: "claude".into(),
            args: vec!["--model".into(), "opus".into()],
            title: Some("Claude".into()),
            agent_kind: Some(AgentKind::Claude),
        };

        let session_id = pin_claude_session(&mut command).expect("pinned session");

        assert!(looks_like_thread_id(&session_id), "{session_id}");
        assert_eq!(
            command.args,
            ["--model", "opus", "--session-id", session_id.as_str()]
        );
        // Pinning is idempotent: relaunching the same pane keeps its conversation.
        assert_eq!(
            pin_claude_session(&mut command).as_deref(),
            Some(session_id.as_str())
        );
        assert_eq!(command.args.len(), 4);
    }

    /// A conversation the user already chose is left alone.
    #[test]
    fn pinning_leaves_a_users_own_session_choice_alone() {
        let mut continued = Profile {
            command: "claude".into(),
            args: vec!["--continue".into()],
            title: None,
            agent_kind: Some(AgentKind::Claude),
        };
        assert_eq!(pin_claude_session(&mut continued), None);
        assert_eq!(continued.args, ["--continue"]);

        let mut shell = Profile {
            command: "bash".into(),
            args: Vec::new(),
            title: None,
            agent_kind: None,
        };
        assert_eq!(pin_claude_session(&mut shell), None);
        assert!(shell.args.is_empty());
    }

    /// Restoring a Claude pane resumes its conversation instead of asking Claude
    /// to create a session id that already exists.
    #[test]
    fn saved_claude_session_relaunches_with_resume() {
        let session_id = "019f7b81-de49-7782-8186-a3dc2c644c61";
        let mut command = Profile {
            command: "claude".into(),
            args: vec!["--model".into(), "opus".into()],
            title: Some("Claude".into()),
            agent_kind: Some(AgentKind::Claude),
        };
        command
            .args
            .extend(["--session-id".into(), session_id.into()]);

        let (profile_name, resumed) = claude_resume_profile("claude", &command, session_id);

        assert_eq!(profile_name, "claude");
        assert_eq!(resumed.args, ["--model", "opus", "--resume", session_id]);
    }

    /// A pane where the user started Claude in a shell is still preserved.
    #[test]
    fn extracts_claude_session_id_from_shell_history() {
        let command = Profile {
            command: "bash".into(),
            args: Vec::new(),
            title: Some("Git Bash".into()),
            agent_kind: None,
        };

        assert_eq!(
            claude_resume_id(
                &command,
                &["claude --resume 019f7b81-e2cd-71c1-84dd-9f09622cf74e".into()]
            )
            .as_deref(),
            Some("019f7b81-e2cd-71c1-84dd-9f09622cf74e")
        );
        // An unrelated command that merely mentions a uuid is not a Claude
        // conversation.
        assert_eq!(
            claude_resume_id(
                &command,
                &["git switch --resume 019f7b81-e2cd-71c1-84dd-9f09622cf74e".into()]
            ),
            None
        );
    }

    /// A Claude conversation whose transcript is gone falls back to a plain
    /// launch, because resuming a missing session leaves the pane with no agent.
    #[test]
    fn a_missing_claude_transcript_falls_back_to_a_plain_launch() {
        let mut pane = pane("fluent", "claude");
        pane.command.agent_kind = Some(AgentKind::Claude);
        pane.command.command = "claude".into();
        pane.claude_session_id = Some("019f7b81-0000-4000-8000-000000000000".into());

        let launch = pane.launch_spec();

        assert_eq!(launch.command.command, "claude");
        assert!(launch.command.args.is_empty(), "{:?}", launch.command.args);
        assert_eq!(pane.resumable_conversation(), None);
    }

    /// Snapshots written by older GridBash versions still load, with their newer
    /// fields defaulted.
    #[test]
    fn version_one_snapshots_still_load() {
        let raw = r#"
version = 1
id = "legacy"
started_at = 1
updated_at = 2
title = "Grid 1"

[grid]
rows = 2
columns = 3

[[panes]]
index = 0
profile_name = "codex"
cwd = "."
folder_name = "fluent"

[panes.command]
command = "codex"
"#;
        let session: SavedSession = toml::from_str(raw).expect("parse legacy session");

        assert!(is_supported_session(&session));
        assert_eq!(session.active_tab, 0);
        assert_eq!(session.view, SavedView::default());
        assert!(session.panes[0].name.is_none());
        let plan = session.active_grid().launch_plan().expect("launch plan");
        assert_eq!(
            plan.grid,
            GridSize {
                rows: 2,
                columns: 3
            }
        );
    }

    /// Clearing a conversation starts a new one in the same pane, and that is
    /// the conversation a resume has to reopen.
    #[test]
    fn a_pane_follows_the_conversation_it_moved_to() {
        let directory = env::temp_dir().join(format!(
            "gridbash-claude-follow-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let launched = "019f7b81-0000-4000-8000-00000000000a";
        let cleared = "019f7b81-0000-4000-8000-00000000000b";
        fs::write(directory.join(format!("{launched}.jsonl")), b"{}").expect("first transcript");
        // File timestamps decide which conversation is current, so the second
        // transcript has to land measurably later.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(directory.join(format!("{cleared}.jsonl")), b"{}").expect("second transcript");

        let none_followed = BTreeSet::new();
        assert_eq!(
            latest_claude_session_in(&directory, Some(launched), None, &none_followed).as_deref(),
            Some(cleared)
        );
        // A conversation another pane is already following is left to that pane.
        let taken = BTreeSet::from([cleared.to_string()]);
        assert_eq!(
            latest_claude_session_in(&directory, Some(launched), None, &taken),
            None
        );
        // A pane already on the newest conversation stays put.
        assert_eq!(
            latest_claude_session_in(&directory, Some(cleared), None, &none_followed),
            None
        );
        // A transcript that predates the terminal belongs to an earlier run.
        let after_everything = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64
            + 60_000;
        assert_eq!(
            latest_claude_session_in(
                &directory,
                Some(launched),
                Some(after_everything),
                &none_followed
            ),
            None
        );
        let _ = fs::remove_dir_all(directory);
    }

    /// A snapshot damaged by a crash mid-write must not cost the workspace.
    #[test]
    fn a_damaged_snapshot_falls_back_to_its_backup() {
        let directory = env::temp_dir().join(format!(
            "gridbash-session-backup-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("session.toml");
        let mut session = recovery_session("backup", vec![pane("alpha", "codex")]);
        session.title = "Good".into();
        save_session_to_path(&path, &session).expect("write first snapshot");

        // A second write keeps the first as the backup.
        session.title = "Newer".into();
        save_session_to_path(&path, &session).expect("write second snapshot");
        assert!(backup_path(&path).is_file());

        fs::write(&path, b"version = 2\nid = \"trunc").expect("damage the snapshot");
        let recovered = load_session_record(&path).expect("recover from backup");

        assert_eq!(recovered.session.title, "Good");
        assert_eq!(recovered.path, path);
        let _ = fs::remove_dir_all(directory);
    }
}
