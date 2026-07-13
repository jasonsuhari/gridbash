use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    ffi::OsString,
    io::{self, Stdout, Write},
    mem,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc as std_mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect, style::Color, text::Line};
use tokio::sync::mpsc;
use vt100::{MouseProtocolEncoding, MouseProtocolMode, Screen};

use crate::{
    auth::{self, AgentKind, AuthProfile},
    cli::{Cli, GridMode},
    composer::{Composer, GridPicker, GridPickerAction},
    config::{Config, PaletteColor, PaneWorkloadPolicy, UiConfig, UiPalette},
    control::{self, ControlCommand, ControlEnvelope, ControlHandle, ControlResponse},
    image_preview::{self, ImagePreview},
    layout::{GridLayout, GridSize, PaneId, pane_at},
    manager::{self, ManagerCommand, ManagerDecision},
    process_priority::PaneWorkloadClass,
    profiles::{default_profile_name, find_profile},
    pty::{PtyEvent, PtyPane, PtyWriteToken},
    session::{SavedPaneHistory, SessionRecord, SessionRecorder},
    setup::{LaunchPlan, PaneLaunchSpec},
    ui,
    usage::{self, UsageEvent, UsageTarget},
    voice::{VoiceInput, VoiceOutcome, VoiceStart},
    worktrees::ManagedWorktreeOptions,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const LARGE_GRID_FRAME_INTERVAL: Duration = Duration::from_millis(33);
const PTY_EVENT_CHANNEL_CAPACITY: usize = 256;
const PTY_DRAIN_MAX_EVENTS: usize = 64;
const PTY_DRAIN_MAX_BYTES: usize = 512 * 1024;
const PTY_DRAIN_MAX_TIME: Duration = Duration::from_millis(4);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const PANE_GOAL_OUTPUT_MAX_BYTES: usize = 12_000;
const PANE_GOAL_REVIEW_IDLE: Duration = Duration::from_secs(2);
const PANE_GOAL_RETRY_DELAY: Duration = Duration::from_secs(30);
const PANE_GOAL_MAX_FAILURES: u8 = 5;
const MAX_MANAGER_SETTING_CHARS: usize = 2048;
const MAX_PANE_NAME_CHARS: usize = 32;
const MAX_TAB_TITLE_CHARS: usize = 40;
const CONVERSATION_SUMMARY_MAX_CHARS: usize = 120;
const TODO_INPUT_LIMIT: usize = 240;
const MIN_TODO_IDLE_SECONDS: u64 = 15;
const MAX_TODO_IDLE_SECONDS: u64 = 600;
const TODO_IDLE_STEP_SECONDS: u64 = 15;
const COMMAND_OUTPUT_MAX_LINES: usize = 2000;
const ACTIVITY_DECAY_INTERVAL: Duration = Duration::from_millis(250);
const OUTPUT_QUIET_AFTER: Duration = Duration::from_secs(3);
const PANE_SCROLL_ROWS: isize = 3;

pub struct App {
    config: Config,
    config_path: Option<PathBuf>,
    worktrees: Option<ManagedWorktreeOptions>,
    tabs: Vec<Option<GridTabSnapshot>>,
    active_tab: usize,
    tab_title: String,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    pane_idle: Vec<PaneIdleState>,
    focus: usize,
    selected: BTreeSet<usize>,
    pane_names: Vec<Option<String>>,
    text_selection: Option<MouseSelection>,
    sleeping: BTreeSet<usize>,
    manager_goal: Option<ManagerGoal>,
    goal_editor: Option<GoalEditorState>,
    next_goal_id: u64,
    goal_tx: std_mpsc::Sender<GoalReviewEvent>,
    goal_rx: std_mpsc::Receiver<GoalReviewEvent>,
    rects: Vec<Rect>,
    mouse_enabled: bool,
    command_line: CommandLineState,
    command_tx: mpsc::UnboundedSender<CommandRunEvent>,
    command_rx: mpsc::UnboundedReceiver<CommandRunEvent>,
    voice: VoiceInput,
    voice_destination: Option<VoiceDestination>,
    control_handle: Option<ControlHandle>,
    control_rx: Option<std_mpsc::Receiver<ControlEnvelope>>,
    image_overlay: Option<ImagePreview>,
    grid_resizer: Option<GridPicker>,
    help_open: bool,
    quit_confirmation_pending: bool,
    settings: SettingsState,
    rename: RenamePaneState,
    tab_rename: RenameTabState,
    previous_panes: PreviousPanesState,
    follow_up: Option<FollowUpPromptState>,
    auth_profiles: Vec<AuthProfile>,
    auth_refresh_rx: Option<std_mpsc::Receiver<Result<Vec<AuthProfile>, String>>>,
    pane_settings: PaneSettingsState,
    status: String,
    restored_histories: Vec<SavedPaneHistory>,
    session_recorder: Option<SessionRecorder>,
    next_pane_id: usize,
    next_tab_number: usize,
    previous_panes_button: Option<Rect>,
    previous_pane_rows: Vec<(usize, Rect)>,
    pane_settings_button: Option<Rect>,
    pane_settings_rename_button: Option<Rect>,
    pane_settings_reload_button: Option<Rect>,
    pane_settings_sleep_button: Option<Rect>,
    pane_settings_goal_button: Option<Rect>,
    pane_settings_stop_goal_button: Option<Rect>,
    event_tx: mpsc::Sender<PtyEvent>,
    event_rx: mpsc::Receiver<PtyEvent>,
    pane_render_cache: RefCell<HashMap<PaneId, ui::PaneRenderCache>>,
    conversation_cache: RefCell<HashMap<PaneId, ConversationCache>>,
    applied_workloads: HashMap<PaneId, (PaneWorkloadPolicy, PaneWorkloadClass)>,
    terminal_focused: bool,
    workload_warning_shown: bool,
    usage_tx: std_mpsc::Sender<UsageEvent>,
    usage_rx: std_mpsc::Receiver<UsageEvent>,
    profile_usage: BTreeMap<String, String>,
    api_spend_label: Option<String>,
    last_activity_decay: Instant,
    last_exit_poll: Instant,
}

struct AppInit {
    config: Config,
    config_path: Option<PathBuf>,
    worktrees: Option<ManagedWorktreeOptions>,
    launch_plan: Option<LaunchPlan>,
    grid: GridSize,
    mouse_enabled: bool,
    command_cwd: PathBuf,
    control_handle: Option<ControlHandle>,
    control_rx: Option<std_mpsc::Receiver<ControlEnvelope>>,
    settings: SettingsState,
    restored_histories: Vec<SavedPaneHistory>,
    session_recorder: Option<SessionRecorder>,
    status: String,
}

struct GridTabSnapshot {
    title: String,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    panes: Vec<PtyPane>,
    pane_idle: Vec<PaneIdleState>,
    focus: usize,
    selected: BTreeSet<usize>,
    pane_names: Vec<Option<String>>,
    text_selection: Option<MouseSelection>,
    sleeping: BTreeSet<usize>,
    manager_goal: Option<ManagerGoal>,
    rects: Vec<Rect>,
}

#[derive(Debug, Clone)]
pub struct TabLabel {
    pub title: String,
    pub active: bool,
    pub activity: bool,
    pub exited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyOutcome {
    Continue,
    Render,
    AuthLogin(AuthProfile),
    Quit,
}

#[derive(Debug, Clone)]
enum VoiceDestination {
    CommandLine,
    Panes { tab: usize, panes: Vec<PaneId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellPoint {
    row: u16,
    column: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaneCell {
    pane: usize,
    point: CellPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MouseSelection {
    pane: usize,
    anchor: CellPoint,
    cursor: CellPoint,
    active: bool,
    moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSelection {
    pub start_row: u16,
    pub start_column: u16,
    pub end_row: u16,
    pub end_column: u16,
}

#[derive(Debug, Clone, Default)]
struct ConversationCache {
    revision: u64,
    summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneRoute {
    Visible(usize),
    Inactive { tab: usize, pane: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneOverlayShortcut {
    Summary,
    Previous,
}

struct PtyDrainBudget {
    started: Instant,
    events: usize,
    bytes: usize,
}

impl PtyDrainBudget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            events: 0,
            bytes: 0,
        }
    }

    fn allows_more(&self) -> bool {
        self.within_size_limits() && self.started.elapsed() < PTY_DRAIN_MAX_TIME
    }

    fn within_size_limits(&self) -> bool {
        self.events < PTY_DRAIN_MAX_EVENTS && self.bytes < PTY_DRAIN_MAX_BYTES
    }

    fn record(&mut self, bytes: usize) {
        self.events += 1;
        self.bytes += bytes;
    }
}

impl MouseSelection {
    fn range(self) -> PaneSelection {
        let (start, end) =
            if (self.anchor.row, self.anchor.column) <= (self.cursor.row, self.cursor.column) {
                (self.anchor, self.cursor)
            } else {
                (self.cursor, self.anchor)
            };

        PaneSelection {
            start_row: start.row,
            start_column: start.column,
            end_row: end.row,
            end_column: end.column,
        }
    }
}

impl PaneSelection {
    pub fn contains(self, row: u16, column: u16) -> bool {
        if row < self.start_row || row > self.end_row {
            return false;
        }

        if self.start_row == self.end_row {
            return column >= self.start_column && column <= self.end_column;
        }

        if row == self.start_row {
            return column >= self.start_column;
        }
        if row == self.end_row {
            return column <= self.end_column;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwapSelection {
    NeedsMore,
    TooMany,
    Pair(usize, usize),
}

#[derive(Debug, Clone)]
struct ManagerGoal {
    id: u64,
    objective: String,
    active: bool,
    output_buffer: String,
    last_output_at: Option<Instant>,
    in_flight: bool,
    retry_after: Option<Instant>,
    review_notice: Option<String>,
    dispatch_retry: Option<GoalDispatchRetry>,
    next_dispatch_sequence: u64,
    failure_count: u8,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GoalCommandKey {
    pane_id: PaneId,
    pane_generation: u64,
    command: String,
}

#[derive(Debug, Clone, Default)]
struct GoalDispatchRetry {
    successful: BTreeSet<GoalCommandKey>,
    pending: BTreeMap<PtyWriteToken, GoalCommandKey>,
    failed: BTreeMap<GoalCommandKey, String>,
    summary: String,
}

#[derive(Debug, Clone)]
struct GoalEditorState {
    input: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GoalTarget {
    pane_number: usize,
    pane_id: PaneId,
    pane_generation: u64,
    screen_revision: u64,
    input_revision: u64,
}

#[derive(Debug, Clone, Copy)]
struct GoalPaneState {
    pane_id: PaneId,
    pane_generation: u64,
    screen_revision: u64,
    input_revision: u64,
    unavailable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoalPaneContext {
    pane_number: usize,
    state: &'static str,
    metadata: String,
    output: String,
}

#[derive(Debug)]
struct GoalReviewEvent {
    goal_id: u64,
    targets: Vec<GoalTarget>,
    result: Result<ManagerDecision, String>,
}

#[derive(Debug, Clone)]
pub struct GoalEditorView {
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct PaneGoalView {
    pub objective: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitedRecoveryAction {
    Restart,
    Sleep,
    HoldAltPrefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteRole {
    Accent,
    Focus,
    Selected,
    Quiet,
    Exited,
}

impl PaletteRole {
    const ALL: [Self; 5] = [
        Self::Accent,
        Self::Focus,
        Self::Selected,
        Self::Quiet,
        Self::Exited,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Accent => "Accent color",
            Self::Focus => "Focus border",
            Self::Selected => "Selected border",
            Self::Quiet => "Quiet border",
            Self::Exited => "Exited border",
        }
    }
}

impl PaletteColor {
    const ALL: [Self; 13] = [
        Self::Cyan,
        Self::Sky,
        Self::Blue,
        Self::Teal,
        Self::Green,
        Self::Yellow,
        Self::Amber,
        Self::Orange,
        Self::Red,
        Self::Magenta,
        Self::DarkGray,
        Self::Gray,
        Self::White,
    ];

    fn color(self) -> Color {
        match self {
            Self::Cyan => Color::Cyan,
            Self::Sky => Color::Rgb(88, 166, 255),
            Self::Blue => Color::Blue,
            Self::Teal => Color::Rgb(54, 211, 153),
            Self::Green => Color::Green,
            Self::Yellow => Color::Yellow,
            Self::Amber => Color::Rgb(245, 158, 11),
            Self::Orange => Color::Rgb(249, 115, 22),
            Self::Red => Color::Red,
            Self::Magenta => Color::Magenta,
            Self::DarkGray => Color::DarkGray,
            Self::Gray => Color::Gray,
            Self::White => Color::White,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cyan => "cyan",
            Self::Sky => "sky",
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Amber => "amber",
            Self::Orange => "orange",
            Self::Red => "red",
            Self::Magenta => "magenta",
            Self::DarkGray => "dark gray",
            Self::Gray => "gray",
            Self::White => "white",
        }
    }

    fn adjust(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|color| *color == self)
            .unwrap_or_default();
        let next = (index as isize + delta).rem_euclid(Self::ALL.len() as isize) as usize;
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPalette {
    accent: PaletteColor,
    focus: PaletteColor,
    selected: PaletteColor,
    quiet: PaletteColor,
    exited: PaletteColor,
}

impl Default for GridPalette {
    fn default() -> Self {
        Self::from(UiPalette::default())
    }
}

impl From<UiPalette> for GridPalette {
    fn from(palette: UiPalette) -> Self {
        Self {
            accent: palette.accent,
            focus: palette.focus,
            selected: palette.selected,
            quiet: palette.quiet,
            exited: palette.exited,
        }
    }
}

impl From<GridPalette> for UiPalette {
    fn from(palette: GridPalette) -> Self {
        Self {
            accent: palette.accent,
            focus: palette.focus,
            selected: palette.selected,
            quiet: palette.quiet,
            exited: palette.exited,
        }
    }
}

impl GridPalette {
    pub fn accent(&self) -> Color {
        self.accent.color()
    }

    pub fn focus(&self) -> Color {
        self.focus.color()
    }

    pub fn selected(&self) -> Color {
        self.selected.color()
    }

    pub fn quiet(&self) -> Color {
        self.quiet.color()
    }

    pub fn exited(&self) -> Color {
        self.exited.color()
    }

    fn color_for(self, role: PaletteRole) -> PaletteColor {
        match role {
            PaletteRole::Accent => self.accent,
            PaletteRole::Focus => self.focus,
            PaletteRole::Selected => self.selected,
            PaletteRole::Quiet => self.quiet,
            PaletteRole::Exited => self.exited,
        }
    }

    fn adjust(&mut self, role: PaletteRole, delta: isize) {
        let target = match role {
            PaletteRole::Accent => &mut self.accent,
            PaletteRole::Focus => &mut self.focus,
            PaletteRole::Selected => &mut self.selected,
            PaletteRole::Quiet => &mut self.quiet,
            PaletteRole::Exited => &mut self.exited,
        };
        *target = (*target).adjust(delta);
    }
}

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub selected: bool,
    pub group: SettingsGroup,
    pub value_kind: SettingsValueKind,
    pub editing: bool,
    pub label: String,
    pub value: String,
    pub value_color: Option<Color>,
    pub hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsGroup {
    Display,
    Workflow,
    Todo,
    Performance,
    Theme,
    Manager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsValueKind {
    Switch,
    Stepper,
    Choice,
    Text,
    Action,
}

#[derive(Debug, Clone)]
pub struct FollowUpDialog {
    pub pane_number: usize,
    pub prompt: String,
    pub todo_position: usize,
    pub todo_count: usize,
    pub quiet_seconds: u64,
}

#[derive(Debug, Clone, Copy)]
struct PaneIdleState {
    last_output_at: Instant,
    snoozed_until: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct FollowUpPromptState {
    pane_index: usize,
    todo_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTarget {
    CompactTitles,
    ActivityBadges,
    ConfirmQuit,
    IdleFollowups,
    IdleSeconds,
    Todo(usize),
    AddTodo,
    Scrollback,
    RefreshMs,
    PaneWorkload,
    Palette(PaletteRole),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoEditState {
    target: TodoEditTarget,
    buffer: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TodoEditTarget {
    Existing(usize),
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsChange {
    None,
    Render,
    SaveTodos,
    SaveUi,
    SaveWorkload,
}

#[derive(Debug, Clone)]
pub struct RenamePaneView {
    pub pane_index: usize,
    pub pane_label: String,
    pub value: String,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct RenameTabView {
    pub title: String,
    pub value: String,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct PreviousPanesView {
    pub cursor: usize,
    pub panes: Vec<PreviousPaneView>,
}

#[derive(Debug, Clone)]
pub struct PreviousPaneView {
    pub index: usize,
    pub label: String,
    pub folder: String,
    pub worktree: Option<String>,
    pub summary: String,
    pub focused: bool,
    pub selected: bool,
    pub sleeping: bool,
    pub exited: bool,
}

#[derive(Debug, Clone, Default)]
struct PreviousPanesState {
    open: bool,
    cursor: usize,
}

impl PreviousPanesState {
    fn begin(&mut self, focus: usize, pane_count: usize) {
        self.open = true;
        self.cursor = focus.min(pane_count.saturating_sub(1));
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn move_cursor(&mut self, delta: isize, pane_count: usize) {
        if pane_count == 0 {
            self.cursor = 0;
            return;
        }

        self.cursor = (self.cursor as isize + delta).clamp(0, pane_count as isize - 1) as usize;
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self, pane_count: usize) {
        self.cursor = pane_count.saturating_sub(1);
    }
}

#[derive(Debug, Clone)]
pub struct PaneSettingsView {
    pub index: usize,
    pub label: String,
    pub folder: String,
    pub worktree: Option<String>,
    pub history_summary: String,
    pub focused: bool,
    pub selected: bool,
    pub sleeping: bool,
    pub exited: bool,
    pub auth_kind: Option<AgentKind>,
    pub auth_options: Vec<PaneAuthOption>,
    pub auth_cursor: usize,
    pub goal: Option<PaneGoalView>,
    pub manager_configured: bool,
}

#[derive(Debug, Clone)]
pub struct PaneAuthOption {
    pub name: String,
    pub account_label: Option<String>,
    pub ready: bool,
    pub current: bool,
}

#[derive(Debug, Clone, Default)]
struct PaneSettingsState {
    open: bool,
    pane_index: usize,
    history_summary: Option<String>,
    auth_cursor: usize,
}

impl PaneSettingsState {
    fn open(&mut self, pane_index: usize, history_summary: String, auth_cursor: usize) {
        self.open = true;
        self.pane_index = pane_index;
        self.history_summary = Some(history_summary);
        self.auth_cursor = auth_cursor;
    }

    fn close(&mut self) {
        self.open = false;
        self.history_summary = None;
    }

    fn refresh_history(&mut self, history_summary: String) {
        self.history_summary = Some(history_summary);
    }
}

#[derive(Debug, Clone)]
pub struct ExitedPaneRecoveryView {
    pub pane_index: usize,
    pub pane_label: String,
    pub target_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Auth,
    Manager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerSettingTarget {
    Endpoint,
    Model,
    ApiKey,
}

impl ManagerSettingTarget {
    const ALL: [Self; 3] = [Self::Endpoint, Self::Model, Self::ApiKey];
}

#[derive(Debug, Clone)]
struct ManagerSettingEdit {
    target: ManagerSettingTarget,
    buffer: String,
}

#[derive(Debug, Clone)]
pub struct AuthCreateState {
    pub kind: AgentKind,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
struct RenamePaneState {
    open: bool,
    pane_index: usize,
    value: String,
    cursor: usize,
}

impl RenamePaneState {
    fn begin(&mut self, pane_index: usize, current_name: Option<&str>) {
        self.open = true;
        self.pane_index = pane_index;
        self.value = current_name.unwrap_or_default().to_string();
        self.cursor = self.value.chars().count();
    }

    fn close(&mut self) {
        self.open = false;
        self.value.clear();
        self.cursor = 0;
    }

    fn move_cursor(&mut self, delta: isize) {
        let count = self.value.chars().count() as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, count) as usize;
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    fn insert_char(&mut self, ch: char) {
        if ch.is_control() || self.value.chars().count() >= MAX_PANE_NAME_CHARS {
            return;
        }

        let index = char_to_byte_index(&self.value, self.cursor);
        self.value.insert(index, ch);
        self.cursor += 1;
    }

    fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert_char(ch);
        }
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let start = char_to_byte_index(&self.value, self.cursor - 1);
        let end = char_to_byte_index(&self.value, self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        let count = self.value.chars().count();
        if self.cursor >= count {
            return;
        }

        let start = char_to_byte_index(&self.value, self.cursor);
        let end = char_to_byte_index(&self.value, self.cursor + 1);
        self.value.replace_range(start..end, "");
    }
}

#[derive(Debug, Clone, Default)]
struct RenameTabState {
    open: bool,
    value: String,
    cursor: usize,
}

impl RenameTabState {
    fn begin(&mut self, title: &str) {
        self.open = true;
        self.value = title.to_string();
        self.cursor = self.value.chars().count();
    }

    fn close(&mut self) {
        self.open = false;
        self.value.clear();
        self.cursor = 0;
    }

    fn move_cursor(&mut self, delta: isize) {
        let count = self.value.chars().count() as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, count) as usize;
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    fn insert_char(&mut self, ch: char) {
        if ch.is_control() || self.value.chars().count() >= MAX_TAB_TITLE_CHARS {
            return;
        }

        let index = char_to_byte_index(&self.value, self.cursor);
        self.value.insert(index, ch);
        self.cursor += 1;
    }

    fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert_char(ch);
        }
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let start = char_to_byte_index(&self.value, self.cursor - 1);
        let end = char_to_byte_index(&self.value, self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        let count = self.value.chars().count();
        if self.cursor >= count {
            return;
        }

        let start = char_to_byte_index(&self.value, self.cursor);
        let end = char_to_byte_index(&self.value, self.cursor + 1);
        self.value.replace_range(start..end, "");
    }
}

#[derive(Debug, Clone)]
struct SettingsState {
    open: bool,
    tab: SettingsTab,
    cursor: usize,
    auth_cursor: usize,
    auth_refreshing: bool,
    create_auth: Option<AuthCreateState>,
    compact_titles: bool,
    activity_badges: bool,
    confirm_quit: bool,
    idle_followups: bool,
    idle_seconds: u64,
    todo_prompts: Vec<String>,
    todo_edit: Option<TodoEditState>,
    scrollback: i32,
    refresh_ms: i32,
    pane_workload: PaneWorkloadPolicy,
    palette: GridPalette,
    manager_cursor: usize,
    manager_edit: Option<ManagerSettingEdit>,
}

#[derive(Debug, Clone)]
struct CommandLineState {
    focused: bool,
    cwd: PathBuf,
    input: String,
    cursor: usize,
    output_lines: Vec<String>,
    running: bool,
}

#[derive(Debug, Clone)]
struct CommandRunEvent {
    command: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    error: Option<String>,
}

impl CommandLineState {
    fn new(cwd: PathBuf) -> Self {
        Self {
            focused: false,
            cwd,
            input: String::new(),
            cursor: 0,
            output_lines: Vec::new(),
            running: false,
        }
    }

    fn toggle_focus(&mut self) {
        self.focused = !self.focused;
    }

    fn output_expanded(&self) -> bool {
        self.focused
    }

    fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            if matches!(ch, '\r' | '\n') {
                self.insert_char(' ');
            } else if !ch.is_control() {
                self.insert_char(ch);
            }
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    fn backspace(&mut self) -> bool {
        let Some(previous) = previous_char_boundary(&self.input, self.cursor) else {
            return false;
        };
        self.input.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        true
    }

    fn delete(&mut self) -> bool {
        if self.cursor >= self.input.len() {
            return false;
        }
        let next = next_char_boundary(&self.input, self.cursor);
        self.input.replace_range(self.cursor..next, "");
        true
    }

    fn move_left(&mut self) -> bool {
        let Some(previous) = previous_char_boundary(&self.input, self.cursor) else {
            return false;
        };
        self.cursor = previous;
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.input.len() {
            return false;
        }
        self.cursor = next_char_boundary(&self.input, self.cursor);
        true
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        if self.cursor == self.input.len() {
            return false;
        }
        self.cursor = self.input.len();
        true
    }

    fn clear_input(&mut self) -> bool {
        if self.input.is_empty() {
            return false;
        }
        self.input.clear();
        self.cursor = 0;
        true
    }

    fn take_submission(&mut self) -> Option<String> {
        let command = self.input.trim().to_string();
        self.input.clear();
        self.cursor = 0;
        (!command.is_empty()).then_some(command)
    }

    fn cursor_chars(&self) -> usize {
        self.input[..self.cursor].chars().count()
    }

    fn push_output_line(&mut self, line: impl Into<String>) {
        self.output_lines.push(line.into());
        if self.output_lines.len() > COMMAND_OUTPUT_MAX_LINES {
            let excess = self.output_lines.len() - COMMAND_OUTPUT_MAX_LINES;
            self.output_lines.drain(0..excess);
        }
    }

    fn push_output_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
            self.push_output_line(line.to_string());
        }
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            tab: SettingsTab::General,
            cursor: 0,
            auth_cursor: 0,
            auth_refreshing: false,
            create_auth: None,
            compact_titles: false,
            activity_badges: true,
            confirm_quit: false,
            idle_followups: true,
            idle_seconds: crate::config::TodoSettings::default_idle_seconds(),
            todo_prompts: Vec::new(),
            todo_edit: None,
            scrollback: UiConfig::default_scrollback_rows() as i32,
            refresh_ms: UiConfig::default_refresh_ms() as i32,
            pane_workload: PaneWorkloadPolicy::Adaptive,
            palette: GridPalette::default(),
            manager_cursor: 0,
            manager_edit: None,
        }
    }
}

impl SettingsState {
    fn from_config(config: &Config) -> Self {
        Self {
            compact_titles: config.ui.compact_titles,
            activity_badges: config.ui.activity_badges,
            confirm_quit: config.ui.confirm_quit,
            idle_followups: config.todos.enabled,
            idle_seconds: config
                .todos
                .idle_seconds
                .clamp(MIN_TODO_IDLE_SECONDS, MAX_TODO_IDLE_SECONDS),
            todo_prompts: config.todos.normalized_prompts(),
            scrollback: config.ui.scrollback_rows.clamp(1_000, 50_000) as i32,
            refresh_ms: config.ui.refresh_ms.clamp(8, 100) as i32,
            pane_workload: config.defaults.pane_workload,
            palette: GridPalette::from(config.ui.palette),
            ..Self::default()
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        self.cancel_todo_edit();
        let row_count = self.row_targets().len();
        if row_count == 0 {
            self.cursor = 0;
            return;
        }

        let current = self.cursor as isize;
        self.cursor = (current + delta).clamp(0, row_count as isize - 1) as usize;
    }

    fn activate(&mut self) -> SettingsChange {
        match self.selected_target() {
            Some(SettingsTarget::CompactTitles) => {
                self.compact_titles = !self.compact_titles;
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::ActivityBadges) => {
                self.activity_badges = !self.activity_badges;
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::ConfirmQuit) => {
                self.confirm_quit = !self.confirm_quit;
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::IdleFollowups) => {
                self.idle_followups = !self.idle_followups;
                SettingsChange::SaveTodos
            }
            Some(SettingsTarget::Todo(index)) => {
                self.start_todo_edit(TodoEditTarget::Existing(index));
                SettingsChange::Render
            }
            Some(SettingsTarget::AddTodo) => {
                self.start_todo_edit(TodoEditTarget::New);
                SettingsChange::Render
            }
            Some(_) => self.adjust(1),
            None => SettingsChange::None,
        }
    }

    fn adjust(&mut self, delta: i32) -> SettingsChange {
        self.cancel_todo_edit();
        match self.selected_target() {
            Some(SettingsTarget::CompactTitles) => {
                if delta != 0 {
                    self.compact_titles = !self.compact_titles;
                }
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::ActivityBadges) => {
                if delta != 0 {
                    self.activity_badges = !self.activity_badges;
                }
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::ConfirmQuit) => {
                if delta != 0 {
                    self.confirm_quit = !self.confirm_quit;
                }
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::IdleFollowups) => {
                if delta != 0 {
                    self.idle_followups = !self.idle_followups;
                }
                SettingsChange::SaveTodos
            }
            Some(SettingsTarget::IdleSeconds) => {
                let step = (delta as i64) * (TODO_IDLE_STEP_SECONDS as i64);
                let next = (self.idle_seconds as i64 + step)
                    .clamp(MIN_TODO_IDLE_SECONDS as i64, MAX_TODO_IDLE_SECONDS as i64)
                    as u64;
                self.idle_seconds = next;
                SettingsChange::SaveTodos
            }
            Some(SettingsTarget::Scrollback) => {
                self.scrollback = (self.scrollback + delta * 1000).clamp(1_000, 50_000);
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::RefreshMs) => {
                self.refresh_ms = (self.refresh_ms + delta * 4).clamp(8, 100);
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::PaneWorkload) => {
                if delta != 0 {
                    self.pane_workload = match self.pane_workload {
                        PaneWorkloadPolicy::Adaptive => PaneWorkloadPolicy::Unrestricted,
                        PaneWorkloadPolicy::Unrestricted => PaneWorkloadPolicy::Adaptive,
                    };
                }
                SettingsChange::SaveWorkload
            }
            Some(SettingsTarget::Palette(role)) => {
                self.palette.adjust(role, delta as isize);
                SettingsChange::SaveUi
            }
            Some(SettingsTarget::Todo(index)) => {
                self.start_todo_edit(TodoEditTarget::Existing(index));
                SettingsChange::Render
            }
            Some(SettingsTarget::AddTodo) => {
                self.start_todo_edit(TodoEditTarget::New);
                SettingsChange::Render
            }
            None => SettingsChange::None,
        }
    }

    fn delete_selected_todo(&mut self) -> SettingsChange {
        self.cancel_todo_edit();
        if let Some(SettingsTarget::Todo(index)) = self.selected_target()
            && index < self.todo_prompts.len()
        {
            self.todo_prompts.remove(index);
            self.clamp_cursor();
            return SettingsChange::SaveTodos;
        }

        SettingsChange::None
    }

    fn editing_todo(&self) -> bool {
        self.todo_edit.is_some()
    }

    fn cancel_todo_edit(&mut self) {
        self.todo_edit = None;
    }

    fn insert_todo_text(&mut self, text: &str) -> bool {
        let Some(edit) = self.todo_edit.as_mut() else {
            return false;
        };

        let available = TODO_INPUT_LIMIT.saturating_sub(edit.buffer.len());
        if available == 0 {
            return false;
        }

        edit.buffer.extend(text.chars().take(available));
        true
    }

    fn backspace_todo_text(&mut self) -> bool {
        self.todo_edit
            .as_mut()
            .and_then(|edit| edit.buffer.pop())
            .is_some()
    }

    fn commit_todo_edit(&mut self) -> bool {
        let Some(edit) = self.todo_edit.take() else {
            return false;
        };

        let prompt = edit.buffer.trim().to_string();
        match edit.target {
            TodoEditTarget::Existing(index) if prompt.is_empty() => {
                if index < self.todo_prompts.len() {
                    self.todo_prompts.remove(index);
                    self.clamp_cursor();
                    return true;
                }
            }
            TodoEditTarget::Existing(index) => {
                if let Some(existing) = self.todo_prompts.get_mut(index) {
                    *existing = prompt;
                    return true;
                }
            }
            TodoEditTarget::New if !prompt.is_empty() => {
                self.todo_prompts.push(prompt);
                self.cursor = self.row_targets().len().saturating_sub(1);
                return true;
            }
            TodoEditTarget::New => {}
        }

        false
    }

    fn todo_settings(&self) -> crate::config::TodoSettings {
        crate::config::TodoSettings {
            enabled: self.idle_followups,
            idle_seconds: self.idle_seconds,
            prompts: self.todo_prompts.clone(),
        }
    }

    fn ui_config(&self) -> UiConfig {
        UiConfig {
            compact_titles: self.compact_titles,
            activity_badges: self.activity_badges,
            confirm_quit: self.confirm_quit,
            scrollback_rows: self.scrollback.clamp(1_000, 50_000) as usize,
            refresh_ms: self.refresh_ms.clamp(8, 100) as u64,
            palette: UiPalette::from(self.palette),
        }
    }

    fn idle_delay(&self) -> Duration {
        Duration::from_secs(
            self.idle_seconds
                .clamp(MIN_TODO_IDLE_SECONDS, MAX_TODO_IDLE_SECONDS),
        )
    }

    fn move_manager_cursor(&mut self, delta: isize) {
        if self.manager_edit.is_some() {
            return;
        }
        self.manager_cursor = (self.manager_cursor as isize + delta)
            .clamp(0, ManagerSettingTarget::ALL.len() as isize - 1)
            as usize;
    }

    fn selected_manager_target(&self) -> ManagerSettingTarget {
        ManagerSettingTarget::ALL[self
            .manager_cursor
            .min(ManagerSettingTarget::ALL.len().saturating_sub(1))]
    }

    fn begin_manager_edit(&mut self, config: &crate::config::ManagerConfig) {
        let target = self.selected_manager_target();
        let buffer = match target {
            ManagerSettingTarget::Endpoint => config.endpoint.clone(),
            ManagerSettingTarget::Model => config.model.clone(),
            ManagerSettingTarget::ApiKey => String::new(),
        };
        self.manager_edit = Some(ManagerSettingEdit { target, buffer });
    }

    fn editing_manager(&self) -> bool {
        self.manager_edit.is_some()
    }

    fn insert_manager_text(&mut self, text: &str) -> bool {
        let Some(edit) = &mut self.manager_edit else {
            return false;
        };
        let remaining = MAX_MANAGER_SETTING_CHARS.saturating_sub(edit.buffer.chars().count());
        let text = text
            .chars()
            .filter(|ch| !ch.is_control())
            .take(remaining)
            .collect::<String>();
        if text.is_empty() {
            return false;
        }
        edit.buffer.push_str(&text);
        true
    }

    fn backspace_manager_text(&mut self) -> bool {
        self.manager_edit
            .as_mut()
            .is_some_and(|edit| edit.buffer.pop().is_some())
    }

    fn manager_rows(&self, config: &crate::config::ManagerConfig) -> Vec<SettingsRow> {
        ManagerSettingTarget::ALL
            .into_iter()
            .map(|target| {
                let editing = self
                    .manager_edit
                    .as_ref()
                    .is_some_and(|edit| edit.target == target);
                let raw = self
                    .manager_edit
                    .as_ref()
                    .filter(|edit| edit.target == target)
                    .map(|edit| edit.buffer.as_str())
                    .unwrap_or_else(|| match target {
                        ManagerSettingTarget::Endpoint => config.endpoint.as_str(),
                        ManagerSettingTarget::Model => config.model.as_str(),
                        ManagerSettingTarget::ApiKey => config.api_key.as_str(),
                    });
                let (label, value, hint) = match target {
                    ManagerSettingTarget::Endpoint => (
                        "API endpoint",
                        format!("{}{}", raw, if editing { "_" } else { "" }),
                        "OpenAI-compatible chat completions URL",
                    ),
                    ManagerSettingTarget::Model => (
                        "Model",
                        format!("{}{}", raw, if editing { "_" } else { "" }),
                        "model name sent with manager reviews",
                    ),
                    ManagerSettingTarget::ApiKey => (
                        "API key",
                        format!(
                            "{}{}",
                            if raw.is_empty() {
                                "not set"
                            } else {
                                "********"
                            },
                            if editing { "_" } else { "" }
                        ),
                        "stored in the local GridBash config",
                    ),
                };
                SettingsRow {
                    selected: self.selected_manager_target() == target,
                    group: SettingsGroup::Manager,
                    value_kind: SettingsValueKind::Text,
                    editing,
                    label: label.into(),
                    value,
                    value_color: None,
                    hint: if editing {
                        "Enter save | Esc cancel".into()
                    } else {
                        hint.into()
                    },
                }
            })
            .collect()
    }

    fn selected_target(&self) -> Option<SettingsTarget> {
        self.row_targets().get(self.cursor).copied()
    }

    fn row_targets(&self) -> Vec<SettingsTarget> {
        let mut targets = vec![
            SettingsTarget::CompactTitles,
            SettingsTarget::ActivityBadges,
            SettingsTarget::ConfirmQuit,
            SettingsTarget::IdleFollowups,
            SettingsTarget::IdleSeconds,
        ];
        targets.extend(
            self.todo_prompts
                .iter()
                .enumerate()
                .map(|(index, _)| SettingsTarget::Todo(index)),
        );
        targets.extend([
            SettingsTarget::AddTodo,
            SettingsTarget::Scrollback,
            SettingsTarget::RefreshMs,
            SettingsTarget::PaneWorkload,
        ]);
        targets.extend(
            PaletteRole::ALL
                .iter()
                .copied()
                .map(SettingsTarget::Palette),
        );
        targets
    }

    fn clamp_cursor(&mut self) {
        let last = self.row_targets().len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
    }

    fn start_todo_edit(&mut self, target: TodoEditTarget) {
        let buffer = match target {
            TodoEditTarget::Existing(index) => {
                self.todo_prompts.get(index).cloned().unwrap_or_default()
            }
            TodoEditTarget::New => String::new(),
        };
        self.todo_edit = Some(TodoEditState { target, buffer });
    }

    fn todo_edit_value(&self, target: SettingsTarget) -> Option<String> {
        let edit = self.todo_edit.as_ref()?;
        let matches = match (target, edit.target) {
            (SettingsTarget::Todo(row), TodoEditTarget::Existing(editing)) => row == editing,
            (SettingsTarget::AddTodo, TodoEditTarget::New) => true,
            _ => false,
        };

        matches.then(|| format!("{}_", edit.buffer))
    }

    fn is_editing_target(&self, target: SettingsTarget) -> bool {
        self.todo_edit_value(target).is_some()
    }

    fn rows(&self) -> Vec<SettingsRow> {
        let mut rows = Vec::new();

        rows.push(self.row(
            SettingsTarget::CompactTitles,
            SettingsGroup::Display,
            SettingsValueKind::Switch,
            "Compact pane titles",
            switch_value(self.compact_titles),
            "shorter labels in pane chrome",
        ));
        rows.push(self.row(
            SettingsTarget::ActivityBadges,
            SettingsGroup::Display,
            SettingsValueKind::Switch,
            "Activity badges",
            switch_value(self.activity_badges),
            "show live and selected state",
        ));
        rows.push(self.row(
            SettingsTarget::ConfirmQuit,
            SettingsGroup::Workflow,
            SettingsValueKind::Switch,
            "Confirm before quit",
            switch_value(self.confirm_quit),
            "extra guard for Alt+q",
        ));
        rows.push(self.row(
            SettingsTarget::IdleFollowups,
            SettingsGroup::Todo,
            SettingsValueKind::Switch,
            "Idle todo prompts",
            switch_value(self.idle_followups),
            "ask before sending queued follow-ups",
        ));
        rows.push(self.row(
            SettingsTarget::IdleSeconds,
            SettingsGroup::Todo,
            SettingsValueKind::Stepper,
            "Quiet delay",
            format!("{} s", self.idle_seconds),
            "time since last terminal output",
        ));

        for (index, prompt) in self.todo_prompts.iter().enumerate() {
            let target = SettingsTarget::Todo(index);
            let value = self
                .todo_edit_value(target)
                .unwrap_or_else(|| prompt.to_string());
            let hint = if self.is_editing_target(target) {
                "Enter save | Esc cancel"
            } else {
                "Enter edit | Del remove"
            };
            rows.push(self.row(
                target,
                SettingsGroup::Todo,
                SettingsValueKind::Text,
                format!("Todo {}", index + 1),
                value,
                hint,
            ));
        }

        let add_target = SettingsTarget::AddTodo;
        let add_value = self
            .todo_edit_value(add_target)
            .unwrap_or_else(|| "new prompt".to_string());
        let add_hint = if self.is_editing_target(add_target) {
            "Enter save | Esc cancel"
        } else {
            "Enter add"
        };
        rows.push(self.row(
            add_target,
            SettingsGroup::Todo,
            if self.is_editing_target(add_target) {
                SettingsValueKind::Text
            } else {
                SettingsValueKind::Action
            },
            "Add todo",
            add_value,
            add_hint,
        ));

        rows.push(self.row(
            SettingsTarget::Scrollback,
            SettingsGroup::Performance,
            SettingsValueKind::Stepper,
            "Scrollback rows",
            self.scrollback.to_string(),
            "history budget for newly launched panes",
        ));
        rows.push(self.row(
            SettingsTarget::RefreshMs,
            SettingsGroup::Performance,
            SettingsValueKind::Stepper,
            "Minimum refresh delay",
            format!("{} ms", self.refresh_ms),
            "output frame throttle",
        ));
        rows.push(
            self.row(
                SettingsTarget::PaneWorkload,
                SettingsGroup::Performance,
                SettingsValueKind::Choice,
                "Workload policy",
                match self.pane_workload {
                    PaneWorkloadPolicy::Adaptive => "adaptive",
                    PaneWorkloadPolicy::Unrestricted => "unrestricted",
                }
                .into(),
                "keep the desktop responsive under load",
            ),
        );
        rows.extend(
            PaletteRole::ALL
                .iter()
                .copied()
                .map(|role| self.palette_row(role)),
        );
        rows
    }

    fn palette_row(&self, role: PaletteRole) -> SettingsRow {
        let color = self.palette.color_for(role);
        let target = SettingsTarget::Palette(role);
        SettingsRow {
            selected: self.selected_target() == Some(target),
            group: SettingsGroup::Theme,
            value_kind: SettingsValueKind::Choice,
            editing: false,
            label: role.label().into(),
            value: color.name().to_string(),
            value_color: Some(color.color()),
            hint: "-/+ color".into(),
        }
    }

    fn row(
        &self,
        target: SettingsTarget,
        group: SettingsGroup,
        value_kind: SettingsValueKind,
        label: impl Into<String>,
        value: String,
        hint: impl Into<String>,
    ) -> SettingsRow {
        SettingsRow {
            selected: self.selected_target() == Some(target),
            group,
            value_kind,
            editing: self.is_editing_target(target),
            label: label.into(),
            value,
            value_color: None,
            hint: hint.into(),
        }
    }
}

impl SettingsChange {
    fn render(self) -> bool {
        !matches!(self, Self::None)
    }

    fn save_todos(self) -> bool {
        matches!(self, Self::SaveTodos)
    }

    fn save_ui(self) -> bool {
        matches!(self, Self::SaveUi)
    }

    fn save_workload(self) -> bool {
        matches!(self, Self::SaveWorkload)
    }
}

impl PaneIdleState {
    fn new(now: Instant) -> Self {
        Self {
            last_output_at: now,
            snoozed_until: None,
        }
    }
}

fn switch_value(enabled: bool) -> String {
    if enabled { "on".into() } else { "off".into() }
}

fn default_status(mouse_enabled: bool) -> String {
    if mouse_enabled {
        "Drag copies within pane | Wheel scrolls selected panes locally | Alt+arrows move | Alt+l resize | Alt+n new tab | Alt+t tab | Alt+Shift+t restart | Alt+c command line | Alt+Shift+V voice | Alt+p pane summary | Alt+r rename | Alt+x swap | Alt+z sleep | Alt+g grid goal | Alt+u stop goal | Alt+o settings | Alt+h help"
            .into()
    } else {
        "Alt+arrows move | Alt+l resize | Alt+n new tab | Alt+t tab | Alt+Shift+t restart | Alt+s select | Alt+c command line | Alt+Shift+V voice | Alt+p pane summary | Alt+r rename | Alt+x swap | Alt+z sleep | Alt+g grid goal | Alt+u stop goal | Alt+o settings | Alt+h help"
            .into()
    }
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let startup_cwd = resolved_current_dir()?;
        let worktrees = cli
            .worktrees
            .then(|| ManagedWorktreeOptions::new(cli.worktree_prefix.clone()))
            .transpose()?;
        let mut launch_plan = resolve_direct_launch_plan(&cli, &config, worktrees.as_ref())?;
        let config_path = cli.config.clone();
        if let Some(plan) = launch_plan.as_mut() {
            apply_auth_defaults(plan, &config)?;
        }
        let mouse_enabled = !cli.no_mouse;
        let grid = launch_plan
            .as_ref()
            .map(|plan| plan.grid)
            .unwrap_or(GridSize {
                rows: 2,
                columns: 3,
            });
        let agent_api_enabled = cli.agent_api || cli.agent_api_port != 0;
        let (control_handle, control_rx) = if agent_api_enabled {
            let (control_tx, control_rx) = std_mpsc::channel();
            (
                Some(control::start_control_server(
                    cli.agent_api_port,
                    control_tx,
                )?),
                Some(control_rx),
            )
        } else {
            (None, None)
        };
        let base_status = default_status(mouse_enabled);
        let status = control_handle
            .as_ref()
            .map(|control| format!("agent API {} | {base_status}", control.endpoint()))
            .unwrap_or(base_status);
        let settings = SettingsState::from_config(&config);

        Ok(Self::from_parts(AppInit {
            config,
            config_path,
            worktrees,
            launch_plan,
            grid,
            mouse_enabled,
            command_cwd: startup_cwd,
            control_handle,
            control_rx,
            settings,
            restored_histories: Vec::new(),
            session_recorder: None,
            status,
        }))
    }

    pub fn resume(config: Config, record: SessionRecord, mouse_enabled: bool) -> Result<Self> {
        let mut launch_plan = record.session.launch_plan()?;
        apply_auth_defaults(&mut launch_plan, &config)?;
        let grid = launch_plan.grid;
        let restored_histories = record.session.pane_histories();
        let session_id = record.session.id.clone();
        let recorder = SessionRecorder::continue_record(record);
        let settings = SettingsState::from_config(&config);
        let command_cwd = launch_plan
            .panes
            .first()
            .map(|pane| pane.cwd.clone())
            .unwrap_or(resolved_current_dir()?);

        Ok(Self::from_parts(AppInit {
            config,
            config_path: None,
            worktrees: None,
            launch_plan: Some(launch_plan),
            grid,
            mouse_enabled,
            command_cwd,
            control_handle: None,
            control_rx: None,
            settings,
            restored_histories,
            session_recorder: Some(recorder),
            status: format!("resumed session {session_id}"),
        }))
    }

    fn from_parts(init: AppInit) -> Self {
        let (event_tx, event_rx) = mpsc::channel(PTY_EVENT_CHANNEL_CAPACITY);
        let (usage_tx, usage_rx) = std_mpsc::channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (goal_tx, goal_rx) = std_mpsc::channel();

        Self {
            config: init.config,
            config_path: init.config_path,
            worktrees: init.worktrees,
            tabs: vec![None],
            active_tab: 0,
            tab_title: "Grid 1".into(),
            launch_plan: init.launch_plan,
            layout: GridLayout::new(init.grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            pane_idle: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            pane_names: Vec::new(),
            text_selection: None,
            sleeping: BTreeSet::new(),
            manager_goal: None,
            goal_editor: None,
            next_goal_id: 1,
            goal_tx,
            goal_rx,
            rects: Vec::new(),
            mouse_enabled: init.mouse_enabled,
            command_line: CommandLineState::new(init.command_cwd),
            command_tx,
            command_rx,
            voice: VoiceInput::new(),
            voice_destination: None,
            control_handle: init.control_handle,
            control_rx: init.control_rx,
            image_overlay: None,
            grid_resizer: None,
            help_open: false,
            quit_confirmation_pending: false,
            settings: init.settings,
            rename: RenamePaneState::default(),
            tab_rename: RenameTabState::default(),
            previous_panes: PreviousPanesState::default(),
            follow_up: None,
            auth_profiles: Vec::new(),
            auth_refresh_rx: None,
            pane_settings: PaneSettingsState::default(),
            status: init.status,
            restored_histories: init.restored_histories,
            session_recorder: init.session_recorder,
            next_pane_id: 0,
            next_tab_number: 2,
            previous_panes_button: None,
            previous_pane_rows: Vec::new(),
            pane_settings_button: None,
            pane_settings_rename_button: None,
            pane_settings_reload_button: None,
            pane_settings_sleep_button: None,
            pane_settings_goal_button: None,
            pane_settings_stop_goal_button: None,
            event_tx,
            event_rx,
            pane_render_cache: RefCell::new(HashMap::new()),
            conversation_cache: RefCell::new(HashMap::new()),
            applied_workloads: HashMap::new(),
            terminal_focused: true,
            workload_warning_shown: false,
            usage_tx,
            usage_rx,
            profile_usage: BTreeMap::new(),
            api_spend_label: None,
            last_activity_decay: Instant::now(),
            last_exit_poll: Instant::now(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal(self.mouse_enabled)?;
        let result = self.run_in_terminal(&mut terminal);
        teardown_terminal(&mut terminal, self.mouse_enabled)?;
        result
    }

    fn run_in_terminal(&mut self, terminal: &mut Tui) -> Result<()> {
        if self.launch_plan.is_none() {
            let current_dir = resolved_current_dir()?;
            let mut composer = Composer::new(current_dir, self.worktrees.clone());
            let Some(plan) = composer.run(terminal, &self.config)? else {
                return Ok(());
            };
            self.set_launch_plan(plan)?;
        }

        self.spawn_initial_panes()?;
        self.sync_initial_pane_sizes(terminal)?;

        let run_result = self.run_loop(terminal);
        let save_result = self.save_session_snapshot();
        match (run_result, save_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn set_launch_plan(&mut self, mut plan: LaunchPlan) -> Result<()> {
        apply_auth_defaults(&mut plan, &self.config)?;
        self.layout = GridLayout::new(plan.grid);
        self.launch_plan = Some(plan);
        self.restored_histories.clear();
        Ok(())
    }

    fn spawn_initial_panes(&mut self) -> Result<()> {
        let plan = self
            .launch_plan
            .clone()
            .ok_or_else(|| anyhow!("no launch plan selected"))?;
        self.tabs.clear();
        self.tabs.push(None);
        self.active_tab = 0;
        self.tab_title = "Grid 1".into();
        self.next_tab_number = 2;
        self.tab_rename.close();
        self.layout = GridLayout::new(plan.grid);
        self.panes.clear();
        self.pane_names = vec![None; plan.panes.len()];
        self.text_selection = None;
        self.sleeping.clear();
        self.manager_goal = None;
        self.goal_editor = None;
        self.next_pane_id = 0;
        self.pane_idle.clear();
        self.last_exit_poll = Instant::now();
        self.follow_up = None;
        self.start_session_recorder(&plan)?;

        for (index, spec) in plan.panes.iter().enumerate() {
            self.spawn_pane_spec(index, spec)?;
        }
        self.restored_histories.clear();
        self.start_usage_monitor(&plan);

        self.save_session_snapshot()
    }

    fn next_tab_title(&mut self) -> String {
        let title = format!("Grid {}", self.next_tab_number);
        self.next_tab_number += 1;
        title
    }

    fn take_current_tab_snapshot(&mut self) -> GridTabSnapshot {
        let placeholder_layout = GridLayout::new(self.layout.size());
        GridTabSnapshot {
            title: mem::take(&mut self.tab_title),
            launch_plan: self.launch_plan.take(),
            layout: mem::replace(&mut self.layout, placeholder_layout),
            panes: mem::take(&mut self.panes),
            pane_idle: mem::take(&mut self.pane_idle),
            focus: self.focus,
            selected: mem::take(&mut self.selected),
            pane_names: mem::take(&mut self.pane_names),
            text_selection: self.text_selection.take(),
            sleeping: mem::take(&mut self.sleeping),
            manager_goal: self.manager_goal.take(),
            rects: mem::take(&mut self.rects),
        }
    }

    fn restore_tab_snapshot(&mut self, tab: GridTabSnapshot) {
        self.tab_title = tab.title;
        self.launch_plan = tab.launch_plan;
        self.layout = tab.layout;
        self.panes = tab.panes;
        self.pane_idle = tab.pane_idle;
        self.focus = tab.focus.min(self.panes.len().saturating_sub(1));
        self.selected = tab.selected;
        self.pane_names = tab.pane_names;
        self.text_selection = tab.text_selection;
        self.sleeping = tab.sleeping;
        self.manager_goal = tab.manager_goal;
        self.rects = tab.rects;
    }

    fn save_current_tab(&mut self) {
        if self.active_tab >= self.tabs.len() {
            self.tabs.resize_with(self.active_tab + 1, || None);
        }
        let snapshot = self.take_current_tab_snapshot();
        self.tabs[self.active_tab] = Some(snapshot);
    }

    fn close_tab_modals(&mut self) {
        self.rename.close();
        self.tab_rename.close();
        self.previous_panes.close();
        self.pane_settings.close();
        self.follow_up = None;
        self.goal_editor = None;
        self.text_selection = None;
        self.command_line.focused = false;
        self.grid_resizer = None;
    }

    fn activate_plan_as_tab(&mut self, title: String, mut plan: LaunchPlan) -> Result<()> {
        apply_auth_defaults(&mut plan, &self.config)?;
        self.tab_title = title.clone();
        self.launch_plan = Some(plan.clone());
        self.layout = GridLayout::new(plan.grid);
        self.panes.clear();
        self.pane_idle.clear();
        self.focus = 0;
        self.selected.clear();
        self.pane_names = vec![None; plan.panes.len()];
        self.text_selection = None;
        self.sleeping.clear();
        self.manager_goal = None;
        self.goal_editor = None;
        self.rects.clear();
        self.follow_up = None;
        self.restored_histories.clear();
        if let Some(cwd) = plan.panes.first().map(|pane| pane.cwd.clone()) {
            self.command_line.cwd = cwd;
        }

        for (index, spec) in plan.panes.iter().enumerate() {
            self.spawn_pane_spec(index, spec)?;
        }
        self.start_usage_monitor(&plan);
        self.status = format!("opened tab {title}");
        Ok(())
    }

    fn add_tab_from_plan(&mut self, plan: LaunchPlan) -> Result<()> {
        self.save_current_tab();
        self.tabs.push(None);
        self.active_tab = self.tabs.len() - 1;
        let title = self.next_tab_title();
        self.close_tab_modals();
        self.activate_plan_as_tab(title, plan)
    }

    fn start_usage_monitor(&mut self, plan: &LaunchPlan) {
        self.profile_usage.clear();
        self.api_spend_label = None;

        let targets = plan
            .panes
            .iter()
            .map(|spec| UsageTarget {
                profile_name: spec.profile_name.clone(),
                command: spec.command.command.clone(),
                auth_kind: spec.auth_kind,
                auth_dir: spec.auth_dir.clone(),
            })
            .collect::<Vec<_>>();
        usage::spawn_usage_monitor(targets, self.usage_tx.clone());
    }

    fn spawn_pane_spec(&mut self, index: usize, spec: &PaneLaunchSpec) -> Result<()> {
        let mut pane = self.spawn_pane_instance(spec, index)?;
        if let Some(history) = self.restored_histories.get(index) {
            pane.restore_history_display(&history.output_tail, &history.input_history);
        }
        self.panes.push(pane);
        self.pane_idle.push(PaneIdleState::new(Instant::now()));
        Ok(())
    }

    fn spawn_pane_instance(&mut self, spec: &PaneLaunchSpec, pane_index: usize) -> Result<PtyPane> {
        let launch = spec.resolved_command()?;
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let extra_env = self.pane_env(pane_index);
        PtyPane::spawn(
            &spec.profile_name,
            id,
            0,
            &launch.command,
            &launch.args,
            &spec.env,
            &spec.cwd,
            &extra_env,
            self.config.ui.scrollback_rows,
            self.config.defaults.pane_priority,
            self.config.defaults.pane_workload,
            self.event_tx.clone(),
        )
    }

    fn pane_env(&self, pane_index: usize) -> Vec<(String, String)> {
        let Some(control) = &self.control_handle else {
            return Vec::new();
        };

        vec![
            (
                "GRIDBASH_CONTROL_ADDR".into(),
                control.endpoint().to_string(),
            ),
            ("GRIDBASH_CONTROL_TOKEN".into(), control.token().to_string()),
            ("GRIDBASH_PANE_INDEX".into(), (pane_index + 1).to_string()),
        ]
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut immediate_render = true;
        let mut output_render = false;
        let mut last_render = Instant::now();
        let mut mouse_capture_enabled = self.mouse_enabled;

        loop {
            let frame_interval = self.output_frame_interval();
            let until_frame = frame_interval.saturating_sub(last_render.elapsed());
            let wait = if immediate_render || !self.event_rx.is_empty() {
                Duration::ZERO
            } else if output_render {
                until_frame.min(INPUT_POLL_INTERVAL)
            } else {
                INPUT_POLL_INTERVAL
            };

            if event::poll(wait)? {
                let event = event::read()?;
                if self.handle_terminal_event(
                    terminal,
                    event,
                    &mut immediate_render,
                    &mut mouse_capture_enabled,
                )? {
                    break;
                }
            }

            output_render |= self.drain_pty_events();
            immediate_render |= self.drain_usage_events();
            immediate_render |= self.drain_auth_refresh();
            immediate_render |= self.drain_command_events();
            immediate_render |= self.drain_goal_reviews();
            immediate_render |= self.drain_voice_events()?;
            immediate_render |= self.drain_control_events();
            immediate_render |= self.decay_activity();
            immediate_render |= self.update_follow_up_prompt();
            immediate_render |= self.schedule_goal_reviews();
            immediate_render |= self.refresh_workload_classes();

            if immediate_render || (output_render && last_render.elapsed() >= frame_interval) {
                terminal.draw(|frame| {
                    let draw_state = ui::draw(frame, self);
                    self.grid_area = draw_state.grid_area;
                    self.rects = draw_state.pane_rects;
                    self.previous_panes_button = draw_state.previous_panes_button;
                    self.previous_pane_rows = draw_state.previous_pane_rows;
                    self.pane_settings_button = draw_state.pane_settings_button;
                    self.pane_settings_rename_button = draw_state.pane_settings_rename_button;
                    self.pane_settings_reload_button = draw_state.pane_settings_reload_button;
                    self.pane_settings_sleep_button = draw_state.pane_settings_sleep_button;
                    self.pane_settings_goal_button = draw_state.pane_settings_goal_button;
                    self.pane_settings_stop_goal_button = draw_state.pane_settings_stop_goal_button;
                })?;
                self.sync_pane_sizes();
                immediate_render = false;
                output_render = false;
                last_render = Instant::now();
            }
            self.sync_mouse_capture(terminal, &mut mouse_capture_enabled)?;
        }

        if mouse_capture_enabled {
            execute!(terminal.backend_mut(), DisableMouseCapture)?;
        }

        Ok(())
    }

    fn handle_terminal_event(
        &mut self,
        terminal: &mut Tui,
        event: Event,
        needs_render: &mut bool,
        mouse_capture_enabled: &mut bool,
    ) -> Result<bool> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match self.handle_key(terminal, key)? {
                    KeyOutcome::Continue => {}
                    KeyOutcome::Render => *needs_render = true,
                    KeyOutcome::AuthLogin(profile) => {
                        self.run_auth_login(terminal, profile)?;
                        *mouse_capture_enabled = false;
                        *needs_render = true;
                    }
                    KeyOutcome::Quit => return Ok(true),
                }
            }
            Event::Resize(_, _) => *needs_render = true,
            Event::FocusGained => {
                self.terminal_focused = true;
                *needs_render = true;
            }
            Event::FocusLost => self.terminal_focused = false,
            Event::Paste(text) if self.tab_rename.open => {
                self.tab_rename.insert_text(&text);
                *needs_render = true;
            }
            Event::Paste(text) if self.rename.open => {
                self.rename.insert_text(&text);
                *needs_render = true;
            }
            Event::Paste(text) if self.settings.editing_manager() => {
                *needs_render |= self.settings.insert_manager_text(&text);
            }
            Event::Paste(text) if self.settings.editing_todo() => {
                *needs_render |= self.settings.insert_todo_text(&text);
            }
            Event::Paste(text) if self.goal_editor.is_some() => {
                if let Some(editor) = &mut self.goal_editor {
                    let remaining = TODO_INPUT_LIMIT.saturating_sub(editor.input.chars().count());
                    editor.input.extend(text.chars().take(remaining));
                    *needs_render = true;
                }
            }
            Event::Paste(text)
                if !self.settings.open
                    && self.grid_resizer.is_none()
                    && !self.rename.open
                    && !self.previous_panes.open
                    && !self.pane_settings.open
                    && self.image_overlay.is_none()
                    && self.follow_up.is_none()
                    && self.goal_editor.is_none() =>
            {
                if self.command_line.focused {
                    self.command_line.insert_text(&text);
                    *needs_render = true;
                } else {
                    self.route_input(text.as_bytes())?;
                }
            }
            Event::Mouse(mouse)
                if (self.mouse_enabled || !self.sleeping.is_empty())
                    && !self.settings.open
                    && self.grid_resizer.is_none()
                    && self.follow_up.is_none()
                    && self.goal_editor.is_none() =>
            {
                *needs_render |= self.handle_mouse(mouse, terminal)?;
            }
            _ => {}
        }
        Ok(false)
    }

    fn output_frame_interval(&self) -> Duration {
        adaptive_output_frame_interval(self.settings.refresh_ms, self.panes.len())
    }

    fn refresh_workload_classes(&mut self) -> bool {
        let policy = self.config.defaults.pane_workload;
        let mut seen = BTreeSet::new();
        let mut failure = None;

        for (index, pane) in self.panes.iter().enumerate() {
            let class = if !self.terminal_focused || self.sleeping.contains(&index) {
                PaneWorkloadClass::Background
            } else if index == self.focus {
                PaneWorkloadClass::Focused
            } else if self.selected.contains(&index) {
                PaneWorkloadClass::Selected
            } else {
                PaneWorkloadClass::Visible
            };
            seen.insert(pane.id());
            if self.applied_workloads.get(&pane.id()) != Some(&(policy, class)) {
                if let Err(error) = pane.apply_workload(policy, class) {
                    failure.get_or_insert(error);
                }
                self.applied_workloads.insert(pane.id(), (policy, class));
            }
        }

        for tab in self.tabs.iter().filter_map(Option::as_ref) {
            for pane in &tab.panes {
                let class = PaneWorkloadClass::Background;
                seen.insert(pane.id());
                if self.applied_workloads.get(&pane.id()) != Some(&(policy, class)) {
                    if let Err(error) = pane.apply_workload(policy, class) {
                        failure.get_or_insert(error);
                    }
                    self.applied_workloads.insert(pane.id(), (policy, class));
                }
            }
        }

        self.applied_workloads.retain(|id, _| seen.contains(id));
        self.pane_render_cache
            .borrow_mut()
            .retain(|id, _| seen.contains(id));
        self.conversation_cache
            .borrow_mut()
            .retain(|id, _| seen.contains(id));
        if let Some(error) = failure
            && !self.workload_warning_shown
        {
            self.status = format!("adaptive workload fallback: {error:#}");
            self.workload_warning_shown = true;
            return true;
        }
        false
    }

    fn sync_mouse_capture(&self, terminal: &mut Tui, enabled: &mut bool) -> Result<()> {
        let should_enable = self.mouse_enabled;
        if should_enable == *enabled {
            return Ok(());
        }

        if should_enable {
            execute!(terminal.backend_mut(), EnableMouseCapture)?;
        } else {
            execute!(terminal.backend_mut(), DisableMouseCapture)?;
        }
        *enabled = should_enable;
        Ok(())
    }

    fn drain_pty_events(&mut self) -> bool {
        let mut changed = false;
        let routes = self.pane_routes();
        let mut pending_output = BTreeMap::<(PaneId, u64), Vec<u8>>::new();
        let mut exited = Vec::new();
        let mut budget = PtyDrainBudget::new();

        while budget.allows_more() {
            let Ok(event) = self.event_rx.try_recv() else {
                break;
            };
            match event {
                PtyEvent::Output {
                    pane,
                    generation,
                    bytes,
                } => {
                    budget.record(bytes.len());
                    pending_output
                        .entry((pane, generation))
                        .or_default()
                        .extend(bytes);
                }
                PtyEvent::Exited { pane, generation } => {
                    budget.record(0);
                    exited.push((pane, generation));
                }
                PtyEvent::WriteFailed {
                    pane,
                    generation,
                    token,
                    error,
                } => {
                    budget.record(0);
                    let handled = token.is_some_and(|token| {
                        self.apply_manager_write_result(pane, generation, token, Err(error.clone()))
                    });
                    if !handled
                        && let Some(PaneRoute::Visible(index)) =
                            routes.get(&(pane, generation)).copied()
                    {
                        self.status = format!("pane {} input failed: {error}", index + 1);
                        changed = true;
                    }
                    changed |= handled;
                }
                PtyEvent::WriteSucceeded {
                    pane,
                    generation,
                    token,
                } => {
                    budget.record(0);
                    changed |= self.apply_manager_write_result(pane, generation, token, Ok(()));
                }
            }
        }

        for ((pane, generation), bytes) in pending_output {
            match routes.get(&(pane, generation)).copied() {
                Some(PaneRoute::Visible(index)) => {
                    let target = &mut self.panes[index];
                    let plain = target.process_output(&bytes);
                    self.capture_goal_output(index, &plain);
                    self.mark_pane_touched(index);
                    changed |= !self.sleeping.contains(&index);
                }
                Some(PaneRoute::Inactive { tab, pane }) => {
                    if let Some(tab) = self.tabs.get_mut(tab).and_then(Option::as_mut) {
                        let was_active = tab.panes[pane].active;
                        let plain = tab.panes[pane].process_output(&bytes);
                        capture_goal_text(&mut tab.manager_goal, &tab.sleeping, pane, &plain);
                        changed |= !was_active;
                    }
                }
                None => {}
            }
        }

        for (pane, generation) in exited {
            match routes.get(&(pane, generation)).copied() {
                Some(PaneRoute::Visible(index)) => {
                    let target = &mut self.panes[index];
                    if !target.exited {
                        target.exited = true;
                        changed = true;
                    }
                    if self
                        .follow_up
                        .is_some_and(|prompt| prompt.pane_index == index)
                    {
                        self.follow_up = None;
                    }
                }
                Some(PaneRoute::Inactive { tab, pane }) => {
                    if let Some(target) = self
                        .tabs
                        .get_mut(tab)
                        .and_then(Option::as_mut)
                        .and_then(|tab| tab.panes.get_mut(pane))
                        && !target.exited
                    {
                        target.exited = true;
                        changed = true;
                    }
                }
                None => {}
            }
        }

        if self.last_exit_poll.elapsed() >= EXIT_POLL_INTERVAL {
            for (index, pane) in self.panes.iter_mut().enumerate() {
                if pane.poll_exit() {
                    if self
                        .follow_up
                        .is_some_and(|prompt| prompt.pane_index == index)
                    {
                        self.follow_up = None;
                    }
                    changed = true;
                }
            }
            for tab in self.tabs.iter_mut().filter_map(Option::as_mut) {
                for pane in &mut tab.panes {
                    changed |= pane.poll_exit();
                }
            }
            self.last_exit_poll = Instant::now();
        }

        changed
    }

    fn pane_routes(&self) -> HashMap<(PaneId, u64), PaneRoute> {
        let mut routes = HashMap::new();
        for (index, pane) in self.panes.iter().enumerate() {
            routes.insert((pane.id(), pane.generation()), PaneRoute::Visible(index));
        }
        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let Some(tab) = tab else {
                continue;
            };
            for (pane_index, pane) in tab.panes.iter().enumerate() {
                routes.insert(
                    (pane.id(), pane.generation()),
                    PaneRoute::Inactive {
                        tab: tab_index,
                        pane: pane_index,
                    },
                );
            }
        }
        routes
    }

    fn apply_manager_write_result(
        &mut self,
        pane: PaneId,
        generation: u64,
        token: PtyWriteToken,
        result: Result<(), String>,
    ) -> bool {
        let current = goal_pane_states(&self.panes, &self.sleeping);
        if let Some(status) = apply_goal_dispatch_result(
            &mut self.manager_goal,
            &current,
            pane,
            generation,
            token,
            result.clone(),
        ) {
            self.status = status;
            return true;
        }

        for tab in self.tabs.iter_mut().filter_map(Option::as_mut) {
            let current = goal_pane_states(&tab.panes, &tab.sleeping);
            if apply_goal_dispatch_result(
                &mut tab.manager_goal,
                &current,
                pane,
                generation,
                token,
                result.clone(),
            )
            .is_some()
            {
                return true;
            }
        }
        false
    }

    fn capture_goal_output(&mut self, pane_index: usize, output: &str) {
        capture_goal_text(&mut self.manager_goal, &self.sleeping, pane_index, output);
    }

    fn schedule_goal_reviews(&mut self) -> bool {
        let now = Instant::now();
        let Some(goal) = self.manager_goal.as_mut() else {
            return false;
        };
        if !goal.active
            || goal.in_flight
            || goal
                .dispatch_retry
                .as_ref()
                .is_some_and(|dispatch| !dispatch.pending.is_empty())
            || goal.output_buffer.trim().is_empty()
            || goal.retry_after.is_some_and(|retry| now < retry)
            || goal
                .last_output_at
                .is_none_or(|last| now.duration_since(last) < PANE_GOAL_REVIEW_IDLE)
        {
            return false;
        }

        let targets = goal_targets(&self.panes, &self.sleeping);
        if targets.is_empty() {
            let changed = goal.status != "waiting for an awake pane";
            goal.status = "waiting for an awake pane".into();
            return changed;
        }

        let pane_metadata =
            manager_goal_pane_metadata(&self.panes, &self.pane_names, self.launch_plan.as_ref());
        let mut context = manager_goal_context(&self.panes, &pane_metadata, &self.sleeping);
        if let Some(dispatch) = goal.dispatch_retry.as_ref() {
            let current = goal_pane_states(&self.panes, &self.sleeping);
            context.push_str("\n--- LOCALLY GENERATED PRIOR-DISPATCH RECORD ---\n");
            context.push_str(&bounded_prefix(
                &format_goal_dispatch_record(dispatch, &current),
                2_048,
            ));
            context.push('\n');
        }
        if let Some(notice) = goal.review_notice.as_deref() {
            context.push_str("\n--- LOCALLY GENERATED MANAGER ERROR RECORD ---\n");
            context.push_str(&bounded_prefix(notice, 2_048));
            context.push('\n');
        }
        let goal_id = goal.id;
        let objective = goal.objective.clone();
        goal.output_buffer.clear();
        goal.last_output_at = None;
        goal.in_flight = true;
        goal.retry_after = None;
        goal.status = "reviewing grid output".into();

        let config = self.config.manager.clone();
        let tx = self.goal_tx.clone();
        thread::spawn(move || {
            let result = manager::review(&config, &objective, &context)
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(GoalReviewEvent {
                goal_id,
                targets,
                result,
            });
        });

        true
    }

    fn drain_goal_reviews(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.goal_rx.try_recv() {
            if let Some(status) = apply_goal_review(
                &mut self.panes,
                &mut self.manager_goal,
                &self.sleeping,
                &event,
            ) {
                self.status = status;
                changed = true;
                continue;
            }

            for tab in self.tabs.iter_mut().filter_map(Option::as_mut) {
                if apply_goal_review(&mut tab.panes, &mut tab.manager_goal, &tab.sleeping, &event)
                    .is_some()
                {
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    fn drain_usage_events(&mut self) -> bool {
        let mut changed = false;

        while let Ok(event) = self.usage_rx.try_recv() {
            match event {
                UsageEvent::Profile { usage_key, label } => match label {
                    Some(label) => {
                        changed |= self.profile_usage.get(&usage_key) != Some(&label);
                        self.profile_usage.insert(usage_key, label);
                    }
                    None => {
                        changed |= self.profile_usage.remove(&usage_key).is_some();
                    }
                },
                UsageEvent::ApiSpend { label } => {
                    changed |= self.api_spend_label != label;
                    self.api_spend_label = label;
                }
            }
        }

        changed
    }

    fn drain_command_events(&mut self) -> bool {
        let mut changed = false;

        while let Ok(event) = self.command_rx.try_recv() {
            self.command_line.running = false;

            if let Some(error) = event.error {
                self.command_line
                    .push_output_line(format!("error: {error}"));
                self.status = format!("command failed: {error}");
                changed = true;
                continue;
            }

            self.command_line.push_output_text(&event.stdout);
            if !event.stderr.is_empty() {
                self.command_line.push_output_text(&event.stderr);
            }

            match event.exit_code {
                Some(0) => {
                    self.status = format!("command done: {}", event.command);
                }
                Some(code) => {
                    self.command_line.push_output_line(format!("[exit {code}]"));
                    self.status = format!("command exited {code}: {}", event.command);
                }
                None => {
                    self.command_line.push_output_line("[terminated]");
                    self.status = format!("command terminated: {}", event.command);
                }
            }
            changed = true;
        }

        changed
    }

    fn drain_voice_events(&mut self) -> Result<bool> {
        let Some(outcome) = self.voice.poll() else {
            return Ok(false);
        };
        let destination = self.voice_destination.take();

        match outcome {
            VoiceOutcome::Transcript(transcript) => match destination {
                Some(VoiceDestination::CommandLine) => {
                    let chars = transcript.chars().count();
                    self.command_line.insert_text(&transcript);
                    self.status = format!("voice inserted {chars} chars into command line");
                }
                Some(VoiceDestination::Panes { tab, panes }) if tab == self.active_tab => {
                    let targets = panes
                        .iter()
                        .filter_map(|pane_id| {
                            self.panes.iter().position(|pane| pane.id() == *pane_id)
                        })
                        .collect::<Vec<_>>();
                    if targets.is_empty() {
                        self.status = "voice target is no longer available".into();
                    } else {
                        let pane_count = targets.len();
                        let status_changed =
                            self.route_input_to_targets(transcript.as_bytes(), targets)?;
                        if !status_changed {
                            self.status = format!(
                                "voice inserted into {pane_count} {}",
                                pane_word(pane_count)
                            );
                        }
                    }
                }
                Some(VoiceDestination::Panes { .. }) => {
                    self.status = "voice result discarded after tab changed".into();
                }
                None => {
                    self.status = "voice result had no input target".into();
                }
            },
            VoiceOutcome::NoSpeech => {
                self.status = "voice heard no speech; press Alt+Shift+V to try again".into();
            }
            VoiceOutcome::Error(error) => {
                self.status = format!("voice unavailable: {error}");
            }
        }

        Ok(true)
    }

    fn drain_control_events(&mut self) -> bool {
        let mut changed = false;

        loop {
            let envelope = self.control_rx.as_ref().and_then(|rx| rx.try_recv().ok());
            let Some(envelope) = envelope else {
                break;
            };

            let response = self.handle_control_command(envelope.command);
            changed = true;
            let _ = envelope.response_tx.send(response);
        }

        changed
    }

    fn handle_control_command(&mut self, command: ControlCommand) -> ControlResponse {
        match command {
            ControlCommand::SetStatus { message } => self.set_control_status(message),
            ControlCommand::SendCommand {
                panes,
                command,
                submit,
            } => self.send_control_command(&panes, &command, submit),
            ControlCommand::ShowImage { path, title } => self.show_control_image(path, title),
        }
    }

    fn set_control_status(&mut self, message: String) -> ControlResponse {
        let message = truncate_chars(message.trim(), 180);
        if message.is_empty() {
            return ControlResponse::error("status message cannot be empty");
        }

        self.status = message.clone();
        ControlResponse::ok(format!("status set: {message}"))
    }

    fn send_control_command(
        &mut self,
        pane_numbers: &[usize],
        command: &str,
        submit: bool,
    ) -> ControlResponse {
        let targets = match self.control_pane_indices(pane_numbers) {
            Ok(targets) => targets,
            Err(error) => return ControlResponse::error(format!("{error:#}")),
        };
        let command_bytes = command.as_bytes();

        for index in &targets {
            let Some(pane) = self.panes.get_mut(*index) else {
                return ControlResponse::error(format!("pane {} is unavailable", index + 1));
            };
            if !command_bytes.is_empty() {
                if let Err(error) = pane.write(command_bytes) {
                    return ControlResponse::error(format!(
                        "failed to send command to pane {}: {error:#}",
                        index + 1
                    ));
                }
                pane.record_input(command_bytes);
            }
            if submit {
                if let Err(error) = pane.write(b"\r") {
                    return ControlResponse::error(format!(
                        "failed to submit command in pane {}: {error:#}",
                        index + 1
                    ));
                }
                pane.record_input(b"\r");
            }
        }

        let panes = pane_number_list(&targets);
        self.status = if submit {
            format!("agent sent command to pane(s) {panes}")
        } else {
            format!("agent wrote text to pane(s) {panes}")
        };
        ControlResponse::ok(self.status.clone())
    }

    fn show_control_image(
        &mut self,
        path: std::path::PathBuf,
        title: Option<String>,
    ) -> ControlResponse {
        let preview = match image_preview::load_image_preview(&path, title, 72, 24) {
            Ok(preview) => preview,
            Err(error) => return ControlResponse::error(format!("{error:#}")),
        };
        let title = preview.title.clone();
        let data = serde_json::json!({
            "path": preview.path.display().to_string(),
            "source_width": preview.source_width,
            "source_height": preview.source_height,
            "preview_columns": preview.cell_width,
            "preview_rows": preview.cell_height
        });

        self.status = format!(
            "showing image {title} ({}x{})",
            preview.source_width, preview.source_height
        );
        self.image_overlay = Some(preview);
        ControlResponse::with_data(self.status.clone(), data)
    }

    fn control_pane_indices(&self, pane_numbers: &[usize]) -> Result<Vec<usize>> {
        if pane_numbers.is_empty() {
            return Err(anyhow!("at least one target pane is required"));
        }

        let mut targets = BTreeSet::new();
        for pane_number in pane_numbers {
            if *pane_number == 0 || *pane_number > self.panes.len() {
                return Err(anyhow!("pane {pane_number} is not available"));
            }
            let index = pane_number - 1;
            if self.sleeping.contains(&index) {
                return Err(anyhow!("pane {pane_number} is asleep"));
            }
            if self.panes.get(index).is_some_and(|pane| pane.exited) {
                return Err(anyhow!("pane {pane_number} has exited"));
            }
            targets.insert(index);
        }

        Ok(targets.into_iter().collect())
    }

    fn decay_activity(&mut self) -> bool {
        if self.last_activity_decay.elapsed() < ACTIVITY_DECAY_INTERVAL {
            return false;
        }

        let now = Instant::now();
        let mut changed = false;
        for (index, pane) in self.panes.iter_mut().enumerate() {
            let became_quiet = pane.refresh_output_activity(now, OUTPUT_QUIET_AFTER);
            if became_quiet {
                pane.active = false;
                changed |= !self.sleeping.contains(&index);
            }
        }
        for tab in self.tabs.iter_mut().filter_map(Option::as_mut) {
            for pane in &mut tab.panes {
                if pane.refresh_output_activity(now, OUTPUT_QUIET_AFTER) {
                    pane.active = false;
                    changed = true;
                }
            }
        }
        self.last_activity_decay = now;
        changed
    }

    fn handle_key(&mut self, terminal: &mut Tui, key: KeyEvent) -> Result<KeyOutcome> {
        if is_quit_shortcut(&key) {
            return Ok(self.request_quit());
        }

        if self.quit_confirmation_pending {
            self.quit_confirmation_pending = false;
            self.status = "quit canceled".into();
        }

        if self.help_open {
            return Ok(self.handle_help_key(key));
        }

        if is_help_shortcut(&key) {
            self.open_help();
            return Ok(KeyOutcome::Render);
        }

        if self.grid_resizer.is_some() {
            return self.handle_grid_resizer_key(key);
        }

        if self.image_overlay.is_some() {
            return Ok(self.handle_image_overlay_key(key));
        }

        if self.tab_rename.open {
            return self.handle_tab_rename_key(key);
        }

        if self.rename.open {
            return self.handle_rename_key(key);
        }

        let selection_cleared = self.clear_text_selection();

        if self.previous_panes.open {
            let outcome = self.handle_previous_panes_key(key);
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if self.pane_settings.open {
            let outcome = self.handle_pane_settings_key(key)?;
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if self.follow_up.is_some() {
            let outcome = self.handle_follow_up_key(key)?;
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if self.goal_editor.is_some() {
            let outcome = self.handle_goal_editor_key(key)?;
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if self.settings.open {
            let outcome = self.handle_settings_key(key)?;
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if key.modifiers.contains(KeyModifiers::ALT)
            && let Some(quit) = self.handle_app_key(terminal, key)?
        {
            return Ok(if quit {
                KeyOutcome::Quit
            } else {
                KeyOutcome::Render
            });
        }

        if self.command_line.focused {
            return Ok(if self.handle_command_key(key)? {
                KeyOutcome::Render
            } else {
                KeyOutcome::Continue
            });
        }

        if self.exited_recovery_view().is_some()
            && let Some(outcome) = self.handle_exited_recovery_key(key)
        {
            return Ok(outcome);
        }

        if let Some(bytes) = terminal_key_bytes(key) {
            let status_changed = self.route_input(&bytes)?;
            return Ok(if selection_cleared || status_changed {
                KeyOutcome::Render
            } else {
                KeyOutcome::Continue
            });
        }

        Ok(if selection_cleared {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn request_quit(&mut self) -> KeyOutcome {
        if !self.settings.confirm_quit || self.quit_confirmation_pending {
            return KeyOutcome::Quit;
        }

        self.quit_confirmation_pending = true;
        self.status = "press Alt+q again to quit; any other key cancels".into();
        KeyOutcome::Render
    }

    fn open_help(&mut self) {
        self.close_tab_modals();
        self.settings.open = false;
        self.image_overlay = None;
        self.help_open = true;
        self.status = "help open".into();
    }

    fn handle_help_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q'))
            || is_help_shortcut(&key)
        {
            self.help_open = false;
            self.status = "help closed".into();
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        }
    }

    fn handle_image_overlay_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return KeyOutcome::Quit;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.image_overlay = None;
                self.status = "image closed".into();
                KeyOutcome::Render
            }
            _ => KeyOutcome::Continue,
        }
    }

    fn handle_app_key(&mut self, terminal: &mut Tui, key: KeyEvent) -> Result<Option<bool>> {
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(terminal, ch, key.modifiers),
            KeyCode::Left => {
                self.focus_previous();
                self.status = self.focus_status();
                Ok(Some(false))
            }
            KeyCode::Right => {
                self.focus_next();
                self.status = self.focus_status();
                Ok(Some(false))
            }
            KeyCode::Up => {
                self.focus_in_grid(-1);
                self.status = self.focus_status();
                Ok(Some(false))
            }
            KeyCode::Down => {
                self.focus_in_grid(1);
                self.status = self.focus_status();
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn handle_alt_char(
        &mut self,
        terminal: &mut Tui,
        ch: char,
        modifiers: KeyModifiers,
    ) -> Result<Option<bool>> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            'q' => Ok(Some(true)),
            's' => {
                if self.command_line.focused {
                    self.status = "command line focused".into();
                } else {
                    self.toggle_pane_selection(self.focus);
                }
                Ok(Some(false))
            }
            'a' => {
                if self.selected.len() == self.panes.len() {
                    self.selected.clear();
                } else {
                    self.selected = (0..self.panes.len()).collect();
                }
                self.status = format!("selected {} panes", self.selected.len());
                Ok(Some(false))
            }
            'z' => {
                self.toggle_sleep_for_targets();
                Ok(Some(false))
            }
            't' if modifiers.contains(KeyModifiers::SHIFT) => {
                self.restart_exited_targets();
                Ok(Some(false))
            }
            't' => {
                self.next_tab();
                Ok(Some(false))
            }
            'n' => {
                self.open_new_tab(terminal)?;
                Ok(Some(false))
            }
            'l' => {
                self.open_grid_resizer();
                Ok(Some(false))
            }
            'x' => {
                self.swap_selected_tiles();
                Ok(Some(false))
            }
            'c' => {
                self.command_line.toggle_focus();
                self.status = self.focus_status();
                Ok(Some(false))
            }
            'v' if is_voice_shortcut(ch, modifiers) => {
                self.toggle_voice_input();
                Ok(Some(false))
            }
            'g' => {
                self.open_goal_editor_for(self.focus);
                Ok(Some(false))
            }
            'u' => {
                self.stop_pane_goal(self.focus);
                Ok(Some(false))
            }
            'o' => {
                self.settings.open = true;
                if self.settings.tab == SettingsTab::Auth {
                    self.start_auth_refresh();
                }
                self.status = "settings open".into();
                Ok(Some(false))
            }
            'p' if modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_previous_panes();
                Ok(Some(false))
            }
            'p' => {
                self.toggle_pane_settings();
                Ok(Some(false))
            }
            'r' if modifiers.contains(KeyModifiers::SHIFT) => {
                self.begin_tab_rename();
                Ok(Some(false))
            }
            'r' => {
                self.begin_rename();
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn handle_exited_recovery_key(&mut self, key: KeyEvent) -> Option<KeyOutcome> {
        match exited_recovery_action_for(key)? {
            ExitedRecoveryAction::Restart => {
                self.restart_focused_exited_pane();
                Some(KeyOutcome::Render)
            }
            ExitedRecoveryAction::Sleep => {
                self.toggle_sleep_for_focused_pane();
                Some(KeyOutcome::Render)
            }
            ExitedRecoveryAction::HoldAltPrefix => Some(KeyOutcome::Render),
        }
    }

    fn open_grid_resizer(&mut self) {
        self.close_tab_modals();
        self.grid_resizer = Some(GridPicker::new(self.layout.size()));
        self.status = "grid resizer open".into();
    }

    fn handle_grid_resizer_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }

        let action = self
            .grid_resizer
            .as_mut()
            .map(|picker| picker.handle_key(key))
            .unwrap_or(GridPickerAction::Cancel);
        match action {
            GridPickerAction::Continue => Ok(KeyOutcome::Render),
            GridPickerAction::Cancel => {
                self.grid_resizer = None;
                self.status = "grid resize canceled".into();
                Ok(KeyOutcome::Render)
            }
            GridPickerAction::Confirm(next) => {
                self.apply_grid_resize(next)?;
                self.grid_resizer = None;
                self.save_session_snapshot()?;
                Ok(KeyOutcome::Render)
            }
        }
    }

    fn open_new_tab(&mut self, terminal: &mut Tui) -> Result<()> {
        let current_dir = self.active_pane_cwd().unwrap_or(resolved_current_dir()?);
        let mut composer = Composer::new(current_dir, self.worktrees.clone());
        match composer.run(terminal, &self.config)? {
            Some(plan) => {
                self.add_tab_from_plan(plan)?;
                self.sync_initial_pane_sizes(terminal)?;
            }
            None => {
                self.status = "new tab canceled".into();
            }
        }
        Ok(())
    }

    fn next_tab(&mut self) {
        if self.tabs.len() <= 1 {
            self.status = "only one tab open".into();
            return;
        }

        let next = (self.active_tab + 1) % self.tabs.len();
        self.switch_to_tab(next);
    }

    fn switch_to_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }

        self.close_tab_modals();
        self.save_current_tab();
        let Some(snapshot) = self.tabs[index].take() else {
            self.status = format!("tab {} is not available", index + 1);
            return;
        };
        self.active_tab = index;
        self.restore_tab_snapshot(snapshot);
        self.start_usage_for_active_tab();
        self.status = format!("active tab {}", self.tab_title);
    }

    fn begin_tab_rename(&mut self) {
        self.rename.close();
        self.tab_rename.begin(&self.tab_title);
        self.status = format!("renaming tab {}", self.tab_title);
    }

    fn handle_tab_rename_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }

        let changed = match key.code {
            KeyCode::Esc => {
                self.tab_rename.close();
                self.status = "tab rename canceled".into();
                true
            }
            KeyCode::Enter => {
                self.save_tab_rename();
                true
            }
            KeyCode::Backspace => {
                self.tab_rename.backspace();
                true
            }
            KeyCode::Delete => {
                self.tab_rename.delete();
                true
            }
            KeyCode::Left => {
                self.tab_rename.move_cursor(-1);
                true
            }
            KeyCode::Right => {
                self.tab_rename.move_cursor(1);
                true
            }
            KeyCode::Home => {
                self.tab_rename.move_to_start();
                true
            }
            KeyCode::End => {
                self.tab_rename.move_to_end();
                true
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tab_rename.clear();
                true
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.tab_rename.insert_char(ch);
                true
            }
            _ => false,
        };

        Ok(if changed {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn save_tab_rename(&mut self) {
        let name = normalized_tab_title(&self.tab_rename.value);
        match name {
            Some(name) => {
                self.tab_title = name.clone();
                self.tab_rename.close();
                self.status = format!("renamed tab to {name}");
            }
            None => {
                self.status = "tab name cannot be empty".into();
            }
        }
    }

    fn open_previous_panes(&mut self) {
        if self.panes.is_empty() {
            self.status = "no panes to list".into();
            return;
        }

        self.pane_settings.close();
        self.previous_panes.begin(self.focus, self.panes.len());
        self.status = "previous panes open".into();
    }

    fn close_previous_panes(&mut self) {
        self.previous_panes.close();
        self.status = "previous panes closed".into();
    }

    fn handle_previous_panes_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return KeyOutcome::Quit;
        }
        if let Some(shortcut) = pane_overlay_shortcut(&key) {
            match shortcut {
                PaneOverlayShortcut::Summary => self.open_pane_settings(),
                PaneOverlayShortcut::Previous => self.close_previous_panes(),
            }
            return KeyOutcome::Render;
        }

        let changed = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.close_previous_panes();
                true
            }
            KeyCode::Up => {
                self.previous_panes.move_cursor(-1, self.panes.len());
                true
            }
            KeyCode::Down => {
                self.previous_panes.move_cursor(1, self.panes.len());
                true
            }
            KeyCode::Home => {
                self.previous_panes.move_to_start();
                true
            }
            KeyCode::End => {
                self.previous_panes.move_to_end(self.panes.len());
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.focus_previous_pane_entry(self.previous_panes.cursor);
                true
            }
            _ => false,
        };

        if changed {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        }
    }

    fn focus_previous_pane_entry(&mut self, index: usize) {
        if index >= self.panes.len() {
            self.previous_panes.close();
            self.status = format!("pane {} is no longer available", index + 1);
            return;
        }

        self.focus = index;
        let woke = self.sleeping.remove(&index);
        self.previous_panes.close();
        self.status = if woke {
            format!("woke pane {}", index + 1)
        } else {
            format!("focused pane {}", index + 1)
        };
    }

    fn begin_rename(&mut self) {
        if self.panes.is_empty() {
            self.status = "no panes to rename".into();
            return;
        }

        let pane_index = self.focus.min(self.panes.len() - 1);
        self.begin_rename_for(pane_index);
    }

    fn begin_rename_for(&mut self, pane_index: usize) {
        if pane_index >= self.panes.len() {
            self.status = format!("pane {} is no longer available", pane_index + 1);
            return;
        }

        let current_name = self
            .pane_names
            .get(pane_index)
            .and_then(|name| name.clone());
        self.rename.begin(pane_index, current_name.as_deref());
        self.status = format!("renaming pane {}", pane_index + 1);
    }

    fn toggle_pane_settings(&mut self) {
        if self.pane_settings.open && self.pane_settings.pane_index == self.focus {
            self.close_pane_settings();
            return;
        }

        self.open_pane_settings();
    }

    fn open_pane_settings(&mut self) {
        if self.panes.is_empty() {
            self.status = "no pane settings to show".into();
            return;
        }

        self.previous_panes.close();
        let pane_index = self.focus.min(self.panes.len() - 1);
        if self.auth_profiles.is_empty() {
            match auth::discover_profiles(&self.config.auth) {
                Ok(profiles) => self.auth_profiles = profiles,
                Err(error) => {
                    self.status = format!("failed to load auth profiles: {error:#}");
                }
            }
        }
        let history_summary = self.pane_history_summary(pane_index);
        let current_auth = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(pane_index))
            .and_then(|spec| spec.auth_name.as_deref());
        let auth_cursor = self
            .compatible_auth_profiles(pane_index)
            .iter()
            .position(|profile| Some(profile.name.as_str()) == current_auth)
            .unwrap_or(0);
        self.pane_settings
            .open(pane_index, history_summary, auth_cursor);
        self.status = format!("pane {} activity summary open", pane_index + 1);
    }

    fn close_pane_settings(&mut self) {
        let pane_number = self.pane_settings.pane_index + 1;
        self.pane_settings.close();
        self.status = format!("pane {pane_number} activity summary closed");
    }

    fn handle_pane_settings_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }
        if let Some(shortcut) = pane_overlay_shortcut(&key) {
            match shortcut {
                PaneOverlayShortcut::Summary => self.close_pane_settings(),
                PaneOverlayShortcut::Previous => self.open_previous_panes(),
            }
            return Ok(KeyOutcome::Render);
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O'))
        {
            self.pane_settings.close();
            self.settings.open = true;
            self.status = "settings open".into();
            return Ok(KeyOutcome::Render);
        }
        if pane_settings_rename_requested(&key) {
            self.begin_rename_for(self.pane_settings.pane_index);
            return Ok(KeyOutcome::Render);
        }
        if matches!(key.code, KeyCode::Char('z') | KeyCode::Char('Z')) {
            let pane_index = self.pane_settings.pane_index;
            self.toggle_sleep_for_panes(&[pane_index]);
            return Ok(KeyOutcome::Render);
        }
        if matches!(key.code, KeyCode::Char('g') | KeyCode::Char('G')) {
            let pane_index = self.pane_settings.pane_index;
            self.pane_settings.close();
            self.open_goal_editor_for(pane_index);
            return Ok(KeyOutcome::Render);
        }
        if matches!(key.code, KeyCode::Char('u') | KeyCode::Char('U')) {
            let pane_index = self.pane_settings.pane_index;
            self.stop_pane_goal(pane_index);
            return Ok(KeyOutcome::Render);
        }

        let changed = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.close_pane_settings();
                true
            }
            KeyCode::Left | KeyCode::Up => {
                self.move_pane_auth_cursor(-1);
                true
            }
            KeyCode::Right | KeyCode::Down => {
                self.move_pane_auth_cursor(1);
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('a') | KeyCode::Char('A') => {
                if self.selected_pane_auth_profile().is_some() {
                    self.apply_selected_pane_auth()?;
                } else {
                    self.reload_pane_history();
                }
                true
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.reload_pane_history();
                true
            }
            _ => false,
        };

        if changed {
            Ok(KeyOutcome::Render)
        } else {
            Ok(KeyOutcome::Continue)
        }
    }

    fn compatible_auth_profiles(&self, pane_index: usize) -> Vec<&AuthProfile> {
        let kind = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(pane_index))
            .and_then(|spec| spec.command.agent_kind);
        self.auth_profiles
            .iter()
            .filter(|profile| Some(profile.kind) == kind)
            .collect()
    }

    fn move_pane_auth_cursor(&mut self, delta: isize) {
        let len = self
            .compatible_auth_profiles(self.pane_settings.pane_index)
            .len();
        if len == 0 {
            self.pane_settings.auth_cursor = 0;
            return;
        }

        self.pane_settings.auth_cursor =
            (self.pane_settings.auth_cursor as isize + delta).rem_euclid(len as isize) as usize;
    }

    fn selected_pane_auth_profile(&self) -> Option<&AuthProfile> {
        self.compatible_auth_profiles(self.pane_settings.pane_index)
            .get(self.pane_settings.auth_cursor)
            .copied()
    }

    fn apply_selected_pane_auth(&mut self) -> Result<()> {
        let pane_index = self.pane_settings.pane_index;
        let Some(profile) = self.selected_pane_auth_profile().cloned() else {
            self.status = "no compatible auth profile selected".into();
            return Ok(());
        };
        let auth_env = auth::env_for_profile(&self.config.auth, profile.kind, &profile.name)?;
        let mut spec = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(pane_index))
            .cloned()
            .ok_or_else(|| anyhow!("pane {} has no launch settings", pane_index + 1))?;
        set_spec_auth(&mut spec, auth_env);
        let pane = self.spawn_pane_instance(&spec, pane_index)?;

        self.panes[pane_index] = pane;
        if let Some(plan) = &mut self.launch_plan {
            plan.panes[pane_index] = spec;
        }
        if let Some(idle) = self.pane_idle.get_mut(pane_index) {
            *idle = PaneIdleState::new(Instant::now());
        }
        self.sleeping.remove(&pane_index);
        self.pane_settings
            .refresh_history("waiting for output".into());
        self.start_usage_for_active_tab();
        self.save_session_snapshot()?;
        self.status = format!(
            "pane {} restarted with {} auth {}",
            pane_index + 1,
            profile.kind.display_name(),
            profile.name
        );
        Ok(())
    }

    fn reload_pane_history(&mut self) {
        let index = self.pane_settings.pane_index;
        if index >= self.panes.len() {
            self.pane_settings.close();
            self.status = format!("pane {} is no longer available", index + 1);
            return;
        }

        let history_summary = self.pane_history_summary(index);
        self.pane_settings.refresh_history(history_summary);
        self.status = format!("refreshed activity for pane {}", index + 1);
    }

    fn handle_rename_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }

        let changed = match key.code {
            KeyCode::Esc => {
                let pane_number = self.rename.pane_index + 1;
                self.rename.close();
                self.status = format!("rename canceled for pane {pane_number}");
                true
            }
            KeyCode::Enter => {
                self.save_rename();
                true
            }
            KeyCode::Backspace => {
                self.rename.backspace();
                true
            }
            KeyCode::Delete => {
                self.rename.delete();
                true
            }
            KeyCode::Left => {
                self.rename.move_cursor(-1);
                true
            }
            KeyCode::Right => {
                self.rename.move_cursor(1);
                true
            }
            KeyCode::Home => {
                self.rename.move_to_start();
                true
            }
            KeyCode::End => {
                self.rename.move_to_end();
                true
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.rename.clear();
                true
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.rename.insert_char(ch);
                true
            }
            _ => false,
        };

        Ok(if changed {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn save_rename(&mut self) {
        let pane_index = self.rename.pane_index;
        let name = normalized_pane_name(&self.rename.value);
        let pane_number = pane_index + 1;

        if let Some(slot) = self.pane_names.get_mut(pane_index) {
            match name {
                Some(name) => {
                    *slot = Some(name.clone());
                    self.status = format!("renamed pane {pane_number} to {name}");
                }
                None => {
                    *slot = None;
                    self.status = format!("cleared pane {pane_number} name");
                }
            }
        } else {
            self.status = format!("pane {pane_number} is no longer available");
        }

        self.rename.close();
    }

    fn swap_selected_tiles(&mut self) {
        let (first, second) = match selected_swap_pair(&self.selected) {
            SwapSelection::NeedsMore => {
                self.status = "select two panes to swap".into();
                return;
            }
            SwapSelection::TooMany => {
                self.status = "deselect panes until only two are selected".into();
                return;
            }
            SwapSelection::Pair(first, second) => (first, second),
        };

        if first >= self.panes.len() || second >= self.panes.len() {
            self.status = "select two visible panes to swap".into();
            return;
        }

        self.panes.swap(first, second);
        if first < self.pane_names.len() && second < self.pane_names.len() {
            self.pane_names.swap(first, second);
        }
        if first < self.pane_idle.len() && second < self.pane_idle.len() {
            self.pane_idle.swap(first, second);
        }
        if let Some(selection) = self.text_selection {
            self.text_selection = Some(MouseSelection {
                pane: swapped_index(selection.pane, first, second),
                ..selection
            });
        }
        if let Some(prompt) = self.follow_up.as_mut() {
            prompt.pane_index = swapped_index(prompt.pane_index, first, second);
        }
        if let Some(plan) = self.launch_plan.as_mut()
            && first < plan.panes.len()
            && second < plan.panes.len()
        {
            plan.panes.swap(first, second);
        }
        swap_set_indices(&mut self.sleeping, first, second);
        self.focus = swapped_index(self.focus, first, second);
        self.status = format!("swapped panes {} and {}", first + 1, second + 1);
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }

        if self.settings.editing_todo() {
            return self.handle_todo_edit_key(key);
        }

        if self.settings.editing_manager() {
            return self.handle_manager_edit_key(key);
        }

        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O'))
        {
            self.settings.open = false;
            self.status = "settings closed".into();
            return Ok(KeyOutcome::Render);
        }

        if self.settings.create_auth.is_some() {
            return Ok(if self.handle_auth_create_key(key)? {
                KeyOutcome::Render
            } else {
                KeyOutcome::Continue
            });
        }

        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            self.toggle_settings_tab();
            return Ok(KeyOutcome::Render);
        }

        if self.settings.tab == SettingsTab::Auth {
            return self.handle_auth_settings_key(key);
        }
        if self.settings.tab == SettingsTab::Manager {
            return self.handle_manager_settings_key(key);
        }

        let change = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings.open = false;
                self.status = "settings closed".into();
                SettingsChange::Render
            }
            KeyCode::Up => {
                self.settings.move_cursor(-1);
                SettingsChange::Render
            }
            KeyCode::Down => {
                self.settings.move_cursor(1);
                SettingsChange::Render
            }
            KeyCode::Left | KeyCode::Char('-') => self.settings.adjust(-1),
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => self.settings.adjust(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.settings.activate(),
            KeyCode::Delete => self.settings.delete_selected_todo(),
            _ => SettingsChange::None,
        };

        self.apply_settings_change(change);
        Ok(if change.render() {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn handle_auth_settings_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        let changed = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings.open = false;
                self.status = "settings closed".into();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_auth_cursor(-1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_auth_cursor(1);
                true
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.start_auth_refresh();
                true
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.set_selected_auth_default()?;
                true
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.toggle_auth_auto_cycle()?;
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.start_auth_create()?;
                true
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
                if let Some(profile) = self.selected_auth_profile().cloned() {
                    return Ok(KeyOutcome::AuthLogin(profile));
                }
                self.status = "no auth profile selected".into();
                true
            }
            _ => false,
        };

        Ok(if changed {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn open_goal_editor_for(&mut self, pane_index: usize) {
        if pane_index >= self.panes.len() {
            self.status = "no pane available for a manager goal".into();
            return;
        }
        let input = self
            .manager_goal
            .as_ref()
            .map(|goal| goal.objective.clone())
            .unwrap_or_default();
        self.goal_editor = Some(GoalEditorState { input });
        self.status = "editing grid manager goal".into();
    }

    fn handle_goal_editor_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }
        let Some(editor) = &mut self.goal_editor else {
            return Ok(KeyOutcome::Continue);
        };
        match key.code {
            KeyCode::Esc => {
                self.goal_editor = None;
                self.status = "manager goal edit cancelled".into();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Enter => self.save_goal_editor(),
            KeyCode::Backspace => {
                editor.input.pop();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.input.clear();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && editor.input.chars().count() < TODO_INPUT_LIMIT =>
            {
                editor.input.push(ch);
                Ok(KeyOutcome::Render)
            }
            _ => Ok(KeyOutcome::Continue),
        }
    }

    fn save_goal_editor(&mut self) -> Result<KeyOutcome> {
        let Some(editor) = self.goal_editor.as_ref() else {
            return Ok(KeyOutcome::Continue);
        };
        let objective = editor
            .input
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if objective.is_empty() {
            self.status = "manager goal cannot be empty".into();
            return Ok(KeyOutcome::Render);
        }
        if let Err(error) = self.config.manager.validate() {
            self.status = format!("manager unavailable: {error:#}");
            return Ok(KeyOutcome::Render);
        }
        if goal_targets(&self.panes, &self.sleeping).is_empty() {
            self.status = "wake or restart at least one pane before starting the goal".into();
            return Ok(KeyOutcome::Render);
        }

        self.goal_editor = None;
        let goal = ManagerGoal {
            id: self.next_goal_id,
            objective,
            active: true,
            output_buffer: "initial grid review requested".into(),
            last_output_at: Some(
                Instant::now()
                    .checked_sub(PANE_GOAL_REVIEW_IDLE)
                    .unwrap_or_else(Instant::now),
            ),
            in_flight: false,
            retry_after: None,
            review_notice: None,
            dispatch_retry: None,
            next_dispatch_sequence: 0,
            failure_count: 0,
            status: "preparing initial grid review".into(),
        };
        self.next_goal_id = self.next_goal_id.saturating_add(1);
        self.manager_goal = Some(goal);
        self.status = "grid manager goal started".into();
        Ok(KeyOutcome::Render)
    }

    fn stop_pane_goal(&mut self, _pane_index: usize) {
        let stopped = self.manager_goal.take().is_some();
        self.status = if stopped {
            "grid manager goal stopped".into()
        } else {
            "grid has no manager goal".into()
        };
    }

    fn handle_manager_settings_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        let changed = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings.open = false;
                self.status = "settings closed".into();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings.move_manager_cursor(-1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings.move_manager_cursor(1);
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings.begin_manager_edit(&self.config.manager);
                self.status = "editing manager API setting".into();
                true
            }
            _ => false,
        };
        Ok(if changed {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn handle_manager_edit_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        match key.code {
            KeyCode::Esc => {
                self.settings.manager_edit = None;
                self.status = "manager setting edit cancelled".into();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Enter => {
                let Some(edit) = self.settings.manager_edit.take() else {
                    return Ok(KeyOutcome::Continue);
                };
                let value = edit.buffer.trim().to_string();
                match edit.target {
                    ManagerSettingTarget::Endpoint => self.config.manager.endpoint = value,
                    ManagerSettingTarget::Model => self.config.manager.model = value,
                    ManagerSettingTarget::ApiKey => self.config.manager.api_key = value,
                }
                match self.config.save(self.config_path.as_deref()) {
                    Ok(_) => self.status = "manager API setting saved".into(),
                    Err(error) => {
                        self.status = format!("failed to save manager setting: {error:#}")
                    }
                }
                Ok(KeyOutcome::Render)
            }
            KeyCode::Backspace => Ok(if self.settings.backspace_manager_text() {
                KeyOutcome::Render
            } else {
                KeyOutcome::Continue
            }),
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                Ok(if self.settings.insert_manager_text(&ch.to_string()) {
                    KeyOutcome::Render
                } else {
                    KeyOutcome::Continue
                })
            }
            _ => Ok(KeyOutcome::Continue),
        }
    }

    fn handle_auth_create_key(&mut self, key: KeyEvent) -> Result<bool> {
        let changed = match key.code {
            KeyCode::Esc => {
                self.settings.create_auth = None;
                self.status = "auth profile creation cancelled".into();
                true
            }
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.toggle_create_auth_kind()?;
                true
            }
            KeyCode::Enter => {
                self.create_auth_profile()?;
                true
            }
            KeyCode::Backspace => {
                if let Some(create) = &mut self.settings.create_auth {
                    create.name.pop();
                }
                true
            }
            KeyCode::Char(ch) if valid_auth_name_char(ch) => {
                if let Some(create) = &mut self.settings.create_auth
                    && create.name.len() < 64
                {
                    create.name.push(ch);
                }
                true
            }
            _ => false,
        };

        Ok(changed)
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, terminal: &mut Tui) -> Result<bool> {
        if !matches!(
            mouse.kind,
            MouseEventKind::Moved
                | MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
                | MouseEventKind::ScrollUp
        ) {
            return Ok(false);
        }

        if self.mouse_enabled
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.previous_panes_button_at(mouse.column, mouse.row)
        {
            if self.previous_panes.open {
                self.close_previous_panes();
            } else {
                self.open_previous_panes();
            }
            return Ok(true);
        }

        if self.mouse_enabled
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.pane_settings_button_at(mouse.column, mouse.row)
        {
            self.toggle_pane_settings();
            return Ok(true);
        }

        if self.previous_panes.open {
            return Ok(if self.mouse_enabled {
                self.handle_previous_panes_mouse(mouse)
            } else {
                false
            });
        }

        if self.pane_settings.open {
            return Ok(if self.mouse_enabled {
                self.handle_pane_settings_mouse(mouse)
            } else {
                false
            });
        }

        if let Some(index) = pane_at(&self.rects, mouse.column, mouse.row)
            && self.sleeping.remove(&index)
        {
            self.focus = index;
            self.mark_pane_touched(index);
            self.status = format!("woke pane {}", index + 1);
            return Ok(true);
        }

        if !self.mouse_enabled {
            return Ok(false);
        }

        if is_mouse_scroll(mouse.kind) {
            return self.forward_mouse_scroll(mouse);
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(index) = pane_at(&self.rects, mouse.column, mouse.row) {
                    self.focus = index;
                    self.clear_text_selection();
                    self.toggle_pane_selection(index);
                    return Ok(true);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(cell) = self.pane_cell_at(mouse.column, mouse.row) {
                    self.focus = cell.pane;
                    self.text_selection = Some(MouseSelection {
                        pane: cell.pane,
                        anchor: cell.point,
                        cursor: cell.point,
                        active: true,
                        moved: false,
                    });
                    self.status = format!("selecting text in pane {}", cell.pane + 1);
                    return Ok(true);
                }

                if let Some(index) = pane_at(&self.rects, mouse.column, mouse.row) {
                    let changed = self.focus != index || self.clear_text_selection();
                    self.focus = index;
                    self.status = format!("focused pane {}", index + 1);
                    return Ok(changed);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.update_text_selection(mouse.column, mouse.row) {
                    return Ok(true);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                return self.finish_text_selection(mouse.column, mouse.row, terminal);
            }
            _ => {}
        }

        Ok(false)
    }

    fn forward_mouse_scroll(&mut self, mouse: MouseEvent) -> Result<bool> {
        let Some(index) = pane_at(&self.rects, mouse.column, mouse.row) else {
            return Ok(false);
        };
        let Some(point) = self.clamped_pane_cell(index, mouse.column, mouse.row) else {
            return Ok(false);
        };
        let Some(pane) = self.panes.get(index) else {
            return Ok(false);
        };

        // Selection is a GridBash control state, so wheel input must stay local to
        // the pane under the pointer instead of being consumed by the terminal app.
        let bytes =
            pane_mouse_scroll_bytes(mouse, point, pane.screen(), self.selected.contains(&index));
        let exited = pane.exited;
        let mut changed = self.focus != index || self.clear_text_selection();
        self.focus = index;

        if let Some(bytes) = bytes
            && !exited
            && let Some(pane) = self.panes.get_mut(index)
        {
            pane.write(&bytes)?;
            pane.record_input_activity(&bytes);
            if changed {
                self.status = format!("focused pane {}", index + 1);
            }
            return Ok(changed);
        }

        if let Some(rows) = pane_scroll_rows(mouse.kind)
            && let Some(pane) = self.panes.get_mut(index)
        {
            changed |= pane.scroll_view(rows);
            self.status = if pane.screen().scrollback() == 0 {
                format!("pane {} at live output", index + 1)
            } else {
                format!(
                    "pane {} scrollback: {} rows",
                    index + 1,
                    pane.screen().scrollback()
                )
            };
        } else if changed {
            self.status = format!("focused pane {}", index + 1);
        }

        Ok(changed)
    }

    fn handle_previous_panes_mouse(&mut self, mouse: MouseEvent) -> bool {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return false;
        }

        let Some(index) = self.previous_pane_row_at(mouse.column, mouse.row) else {
            return false;
        };

        self.previous_panes.cursor = index;
        self.focus_previous_pane_entry(index);
        true
    }

    fn handle_pane_settings_mouse(&mut self, mouse: MouseEvent) -> bool {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return false;
        }

        if self.pane_settings_reload_button_at(mouse.column, mouse.row) {
            self.reload_pane_history();
            return true;
        }
        if self.pane_settings_sleep_button_at(mouse.column, mouse.row) {
            let pane_index = self.pane_settings.pane_index;
            self.toggle_sleep_for_panes(&[pane_index]);
            return true;
        }
        if self.pane_settings_goal_button_at(mouse.column, mouse.row) {
            let pane_index = self.pane_settings.pane_index;
            self.pane_settings.close();
            self.open_goal_editor_for(pane_index);
            return true;
        }
        if self.pane_settings_stop_goal_button_at(mouse.column, mouse.row) {
            let pane_index = self.pane_settings.pane_index;
            self.stop_pane_goal(pane_index);
            return true;
        }
        if self.pane_settings_rename_button_at(mouse.column, mouse.row) {
            self.begin_rename_for(self.pane_settings.pane_index);
            return true;
        }

        false
    }

    fn previous_panes_button_at(&self, x: u16, y: u16) -> bool {
        self.previous_panes_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn previous_pane_row_at(&self, x: u16, y: u16) -> Option<usize> {
        self.previous_pane_rows
            .iter()
            .find_map(|(index, rect)| rect_contains(*rect, x, y).then_some(*index))
    }

    fn pane_settings_button_at(&self, x: u16, y: u16) -> bool {
        self.pane_settings_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn pane_settings_reload_button_at(&self, x: u16, y: u16) -> bool {
        self.pane_settings_reload_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn pane_settings_rename_button_at(&self, x: u16, y: u16) -> bool {
        self.pane_settings_rename_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn pane_settings_sleep_button_at(&self, x: u16, y: u16) -> bool {
        self.pane_settings_sleep_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn pane_settings_goal_button_at(&self, x: u16, y: u16) -> bool {
        self.pane_settings_goal_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn pane_settings_stop_goal_button_at(&self, x: u16, y: u16) -> bool {
        self.pane_settings_stop_goal_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn pane_cell_at(&self, x: u16, y: u16) -> Option<PaneCell> {
        let pane = pane_at(&self.rects, x, y)?;
        let rect = self.rects.get(pane).copied()?;
        let inner = pane_inner_rect(rect)?;

        if x < inner.x
            || x >= inner.x.saturating_add(inner.width)
            || y < inner.y
            || y >= inner.y.saturating_add(inner.height)
        {
            return None;
        }

        Some(PaneCell {
            pane,
            point: CellPoint {
                row: y.saturating_sub(inner.y),
                column: x.saturating_sub(inner.x),
            },
        })
    }

    fn clamped_pane_cell(&self, pane: usize, x: u16, y: u16) -> Option<CellPoint> {
        let rect = self.rects.get(pane).copied()?;
        let inner = pane_inner_rect(rect)?;
        let max_x = inner.x.saturating_add(inner.width.saturating_sub(1));
        let max_y = inner.y.saturating_add(inner.height.saturating_sub(1));

        Some(CellPoint {
            row: y.clamp(inner.y, max_y).saturating_sub(inner.y),
            column: x.clamp(inner.x, max_x).saturating_sub(inner.x),
        })
    }

    fn update_text_selection(&mut self, x: u16, y: u16) -> bool {
        let Some(selection) = self.text_selection else {
            return false;
        };
        if !selection.active {
            return false;
        }

        let Some(cursor) = self.clamped_pane_cell(selection.pane, x, y) else {
            return false;
        };

        self.text_selection = Some(MouseSelection {
            cursor,
            moved: true,
            ..selection
        });
        true
    }

    fn finish_text_selection(&mut self, x: u16, y: u16, terminal: &mut Tui) -> Result<bool> {
        let Some(selection) = self.text_selection else {
            return Ok(false);
        };
        if !selection.active {
            return Ok(false);
        }

        let cursor = self
            .clamped_pane_cell(selection.pane, x, y)
            .unwrap_or(selection.cursor);
        let selection = MouseSelection {
            cursor,
            active: false,
            ..selection
        };
        if !selection.moved {
            self.text_selection = None;
            self.status = format!("focused pane {}", selection.pane + 1);
            return Ok(true);
        }

        self.text_selection = Some(selection);

        let text = self.selected_text(selection);
        if text.is_empty() {
            self.status = format!("selection empty in pane {}", selection.pane + 1);
        } else {
            copy_to_clipboard(terminal, &text)?;
            self.status = format!(
                "copied {} chars from pane {}",
                text.chars().count(),
                selection.pane + 1
            );
        }

        Ok(true)
    }

    fn selected_text(&self, selection: MouseSelection) -> String {
        let Some(pane) = self.panes.get(selection.pane) else {
            return String::new();
        };
        let width = self
            .rects
            .get(selection.pane)
            .and_then(|rect| pane_inner_rect(*rect))
            .map(|inner| inner.width)
            .unwrap_or(0);

        extract_selection_text(pane.screen(), selection.range(), width)
    }

    fn clear_text_selection(&mut self) -> bool {
        self.text_selection.take().is_some()
    }

    fn toggle_pane_selection(&mut self, index: usize) {
        if index >= self.panes.len() {
            self.status = "no pane to select".into();
            return;
        }

        let selected = toggle_selection(&mut self.selected, index);
        let action = if selected { "selected" } else { "deselected" };
        self.status = format!(
            "{action} pane {}; {} selected",
            index + 1,
            self.selected.len()
        );
    }

    fn apply_grid_resize(&mut self, next: GridSize) -> Result<()> {
        let current = self.layout.size();
        if next == current && self.panes.len() == next.count() {
            self.status = format!("grid remains {}x{}", next.rows, next.columns);
            return Ok(());
        }

        let before = self.panes.len();
        let slots = grid_resize_slots(current, next, before);
        let old_to_new = slots
            .iter()
            .enumerate()
            .filter_map(|(new, old)| old.map(|old| (old, new)))
            .collect::<BTreeMap<_, _>>();
        let retained = old_to_new.len();
        let removed = before.saturating_sub(retained);
        let added = slots.iter().filter(|slot| slot.is_none()).count();

        let plan = self
            .launch_plan
            .as_ref()
            .ok_or_else(|| anyhow!("no launch plan selected"))?;
        let templates = plan.panes.clone();
        if templates.is_empty() {
            return Err(anyhow!("no pane template available"));
        }

        let mut next_plan = LaunchPlan {
            panes: slots
                .iter()
                .enumerate()
                .map(|(new_index, old_index)| {
                    let mut spec = old_index
                        .and_then(|old_index| plan.panes.get(old_index))
                        .cloned()
                        .unwrap_or_else(|| templates[new_index % templates.len()].clone());
                    if old_index.is_none() && self.config.auth.auto_cycle {
                        clear_spec_auth(&mut spec);
                    }
                    spec
                })
                .collect(),
            grid: next,
        };
        apply_auth_defaults(&mut next_plan, &self.config)?;

        // Spawn every new cell before touching the live vectors. If a launch fails,
        // the current grid remains intact.
        let mut spawned = BTreeMap::new();
        for (new_index, old_index) in slots.iter().enumerate() {
            if old_index.is_none() {
                let pane = self.spawn_pane_instance(&next_plan.panes[new_index], new_index)?;
                spawned.insert(new_index, pane);
            }
        }

        let mut old_panes = mem::take(&mut self.panes)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let mut old_idle = mem::take(&mut self.pane_idle)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let mut old_names = mem::take(&mut self.pane_names)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        self.panes = Vec::with_capacity(next.count());
        self.pane_idle = Vec::with_capacity(next.count());
        self.pane_names = Vec::with_capacity(next.count());
        for (new_index, old_index) in slots.iter().copied().enumerate() {
            if let Some(old_index) = old_index {
                self.panes.push(
                    old_panes
                        .get_mut(old_index)
                        .and_then(Option::take)
                        .expect("retained pane index"),
                );
                self.pane_idle.push(
                    old_idle
                        .get_mut(old_index)
                        .and_then(Option::take)
                        .unwrap_or_else(|| PaneIdleState::new(Instant::now())),
                );
                self.pane_names.push(
                    old_names
                        .get_mut(old_index)
                        .and_then(Option::take)
                        .unwrap_or(None),
                );
            } else {
                self.panes
                    .push(spawned.remove(&new_index).expect("spawned resize pane"));
                self.pane_idle.push(PaneIdleState::new(Instant::now()));
                self.pane_names.push(None);
            }
        }

        let old_focus = self.focus;
        self.focus = resized_focus_index(old_focus, current, next, &old_to_new);
        self.selected = remap_index_set(&self.selected, &old_to_new);
        self.sleeping = remap_index_set(&self.sleeping, &old_to_new);
        self.text_selection = self.text_selection.and_then(|selection| {
            old_to_new
                .get(&selection.pane)
                .copied()
                .map(|pane| MouseSelection { pane, ..selection })
        });
        self.follow_up = self.follow_up.and_then(|prompt| {
            old_to_new
                .get(&prompt.pane_index)
                .copied()
                .map(|pane_index| FollowUpPromptState {
                    pane_index,
                    ..prompt
                })
        });
        self.layout.set_size(next);
        self.launch_plan = Some(next_plan.clone());
        self.start_usage_monitor(&next_plan);

        self.status = if added > 0 && removed > 0 {
            format!(
                "grid resized to {}x{}; deactivated {removed} and spawned {added} pane(s)",
                next.rows, next.columns
            )
        } else if added > 0 {
            format!(
                "grid resized to {}x{}; spawned {added} pane(s)",
                next.rows, next.columns
            )
        } else if removed > 0 {
            format!(
                "grid resized to {}x{}; deactivated {removed} pane(s)",
                next.rows, next.columns
            )
        } else {
            format!("grid resized to {}x{}", next.rows, next.columns)
        };

        Ok(())
    }

    fn toggle_sleep_for_targets(&mut self) {
        let targets = self.target_panes();
        self.toggle_sleep_for_panes(&targets);
    }

    fn toggle_sleep_for_panes(&mut self, targets: &[usize]) {
        if targets.is_empty() {
            return;
        }

        let should_sleep = targets.iter().any(|index| !self.sleeping.contains(index));
        if should_sleep {
            for index in targets {
                self.sleeping.insert(*index);
                self.selected.remove(index);
            }
            if self
                .follow_up
                .is_some_and(|prompt| targets.contains(&prompt.pane_index))
            {
                self.follow_up = None;
            }

            if self.sleeping.contains(&self.focus)
                && let Some(index) = self.next_awake_pane(self.focus)
            {
                self.focus = index;
            }
        } else {
            for index in targets {
                self.sleeping.remove(index);
                self.mark_pane_touched(*index);
            }
            self.focus = targets[0];
        }

        let action = if should_sleep { "slept" } else { "woke" };
        self.status = format!("{} {} {}", action, targets.len(), pane_word(targets.len()));
    }

    fn handle_todo_edit_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O'))
        {
            self.settings.cancel_todo_edit();
            self.settings.open = false;
            self.status = "settings closed".into();
            return Ok(KeyOutcome::Render);
        }

        match key.code {
            KeyCode::Esc => {
                self.settings.cancel_todo_edit();
                self.status = "todo edit cancelled".into();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Enter => {
                let saved = self.settings.commit_todo_edit();
                if saved {
                    if self.save_todo_settings() {
                        self.status = "todo prompt saved".into();
                    }
                } else {
                    self.status = "empty todo ignored".into();
                }
                Ok(KeyOutcome::Render)
            }
            KeyCode::Backspace => Ok(if self.settings.backspace_todo_text() {
                KeyOutcome::Render
            } else {
                KeyOutcome::Continue
            }),
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let mut buffer = [0; 4];
                let changed = self.settings.insert_todo_text(ch.encode_utf8(&mut buffer));
                Ok(if changed {
                    KeyOutcome::Render
                } else {
                    KeyOutcome::Continue
                })
            }
            _ => Ok(KeyOutcome::Continue),
        }
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::ALT) {
            return Ok(false);
        }

        let changed = match key.code {
            KeyCode::Enter => {
                self.submit_command_line()?;
                true
            }
            KeyCode::Backspace => self.command_line.backspace(),
            KeyCode::Delete => self.command_line.delete(),
            KeyCode::Left => self.command_line.move_left(),
            KeyCode::Right => self.command_line.move_right(),
            KeyCode::Home => self.command_line.move_home(),
            KeyCode::End => self.command_line.move_end(),
            KeyCode::Esc => self.command_line.clear_input(),
            KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match ch.to_ascii_lowercase() {
                    'a' => self.command_line.move_home(),
                    'e' => self.command_line.move_end(),
                    'u' => self.command_line.clear_input(),
                    _ => false,
                }
            }
            KeyCode::Char(ch) => {
                self.command_line.insert_char(ch);
                true
            }
            _ => false,
        };

        Ok(changed)
    }

    fn submit_command_line(&mut self) -> Result<()> {
        if self.command_line.running {
            self.status = "command still running".into();
            return Ok(());
        }

        let Some(command) = self.command_line.take_submission() else {
            return Ok(());
        };

        self.command_line.push_output_line(format!("> {command}"));
        if self.handle_builtin_command(&command) {
            return Ok(());
        }

        self.command_line.running = true;
        self.status = format!("running: {command}");
        spawn_hidden_command(
            command,
            self.command_line.cwd.clone(),
            self.command_tx.clone(),
        );
        Ok(())
    }

    fn handle_builtin_command(&mut self, command: &str) -> bool {
        if let Some(target) = parse_cd_target(command) {
            match resolve_cd_target(&self.command_line.cwd, target.as_deref()) {
                Ok(Some(cwd)) => {
                    self.command_line.cwd = cwd;
                    self.status = format!("cwd: {}", self.command_line.cwd.display());
                }
                Ok(None) => {
                    self.command_line
                        .push_output_line(self.command_line.cwd.display().to_string());
                    self.status = format!("cwd: {}", self.command_line.cwd.display());
                }
                Err(error) => {
                    self.command_line.push_output_line(format!("cd: {error:#}"));
                    self.status = format!("cd failed: {error:#}");
                }
            }
            return true;
        }

        match command.trim().to_ascii_lowercase().as_str() {
            "pwd" => {
                self.command_line
                    .push_output_line(self.command_line.cwd.display().to_string());
                self.status = format!("cwd: {}", self.command_line.cwd.display());
                true
            }
            "clear" | "cls" => {
                self.command_line.output_lines.clear();
                self.status = "command output cleared".into();
                true
            }
            _ => false,
        }
    }

    fn apply_settings_change(&mut self, change: SettingsChange) {
        if change.save_todos() {
            self.save_todo_settings();
        }
        if change.save_ui() {
            self.save_ui_settings();
        }
        if change.save_workload() {
            self.save_workload_setting();
        }
    }

    fn save_todo_settings(&mut self) -> bool {
        self.config.todos = self.settings.todo_settings();
        match self.config.save(self.config_path.as_deref()) {
            Ok(_) => true,
            Err(error) => {
                self.status = format!("failed to save todos: {error:#}");
                false
            }
        }
    }

    fn save_ui_settings(&mut self) -> bool {
        let previous = self.config.ui.clone();
        self.config.ui = self.settings.ui_config();
        match self.config.save(self.config_path.as_deref()) {
            Ok(_) => true,
            Err(error) => {
                self.config.ui = previous.clone();
                self.settings.compact_titles = previous.compact_titles;
                self.settings.activity_badges = previous.activity_badges;
                self.settings.confirm_quit = previous.confirm_quit;
                self.settings.scrollback = previous.scrollback_rows.clamp(1_000, 50_000) as i32;
                self.settings.refresh_ms = previous.refresh_ms.clamp(8, 100) as i32;
                self.settings.palette = GridPalette::from(previous.palette);
                self.status = format!("failed to save UI settings: {error:#}");
                false
            }
        }
    }

    fn update_follow_up_prompt(&mut self) -> bool {
        if self.follow_up.is_some()
            || self.settings.open
            || !self.settings.idle_followups
            || self.settings.todo_prompts.is_empty()
        {
            return false;
        }

        let now = Instant::now();
        let idle_delay = self.settings.idle_delay();
        for (index, pane) in self.panes.iter().enumerate() {
            if pane.exited || self.sleeping.contains(&index) {
                continue;
            }

            let Some(idle) = self.pane_idle.get(index) else {
                continue;
            };
            if idle.snoozed_until.is_some_and(|until| now < until) {
                continue;
            }
            if now.duration_since(idle.last_output_at) < idle_delay {
                continue;
            }

            self.follow_up = Some(FollowUpPromptState {
                pane_index: index,
                todo_index: 0,
            });
            self.status = format!("pane {} is quiet; todo follow-up ready", index + 1);
            return true;
        }

        false
    }

    fn handle_follow_up_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }

        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.send_follow_up_prompt()?;
                Ok(KeyOutcome::Render)
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char(']') => {
                self.cycle_follow_up_prompt(1);
                Ok(KeyOutcome::Render)
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('[') => {
                self.cycle_follow_up_prompt(-1);
                Ok(KeyOutcome::Render)
            }
            KeyCode::Delete => {
                self.delete_follow_up_prompt();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.dismiss_follow_up_prompt();
                Ok(KeyOutcome::Render)
            }
            _ => Ok(KeyOutcome::Continue),
        }
    }

    fn send_follow_up_prompt(&mut self) -> Result<()> {
        let Some(dialog) = self.follow_up.take() else {
            return Ok(());
        };
        let Some(prompt) = self.settings.todo_prompts.get(dialog.todo_index).cloned() else {
            self.status = "todo prompt no longer exists".into();
            return Ok(());
        };

        let mut bytes = prompt.into_bytes();
        bytes.push(b'\r');
        let pane = self
            .panes
            .get_mut(dialog.pane_index)
            .ok_or_else(|| anyhow!("invalid pane index {}", dialog.pane_index))?;
        pane.write(&bytes)?;
        pane.record_input(&bytes);
        self.mark_pane_touched(dialog.pane_index);

        if dialog.todo_index < self.settings.todo_prompts.len() {
            self.settings.todo_prompts.remove(dialog.todo_index);
            self.settings.clamp_cursor();
            if self.save_todo_settings() {
                self.status = format!("sent todo follow-up to pane {}", dialog.pane_index + 1);
            }
        } else {
            self.status = format!("sent todo follow-up to pane {}", dialog.pane_index + 1);
        }

        Ok(())
    }

    fn dismiss_follow_up_prompt(&mut self) {
        let Some(dialog) = self.follow_up.take() else {
            return;
        };
        let delay = self.settings.idle_delay();
        if let Some(idle) = self.pane_idle.get_mut(dialog.pane_index) {
            idle.snoozed_until = Some(Instant::now() + delay);
        }
        self.status = format!("todo follow-up snoozed for pane {}", dialog.pane_index + 1);
    }

    fn delete_follow_up_prompt(&mut self) {
        let Some(dialog) = self.follow_up.take() else {
            return;
        };
        if dialog.todo_index < self.settings.todo_prompts.len() {
            self.settings.todo_prompts.remove(dialog.todo_index);
            self.settings.clamp_cursor();
            if self.save_todo_settings() {
                self.status = "todo prompt removed".into();
            }
        }
        if let Some(idle) = self.pane_idle.get_mut(dialog.pane_index) {
            idle.snoozed_until = Some(Instant::now() + self.settings.idle_delay());
        }
    }

    fn cycle_follow_up_prompt(&mut self, delta: isize) {
        let count = self.settings.todo_prompts.len();
        let Some(dialog) = self.follow_up.as_mut() else {
            return;
        };
        if count <= 1 {
            return;
        }

        dialog.todo_index =
            (dialog.todo_index as isize + delta).rem_euclid(count as isize) as usize;
        self.status = format!(
            "todo follow-up {}/{} for pane {}",
            dialog.todo_index + 1,
            count,
            dialog.pane_index + 1
        );
    }

    fn mark_pane_touched(&mut self, index: usize) {
        if let Some(idle) = self.pane_idle.get_mut(index) {
            idle.last_output_at = Instant::now();
            idle.snoozed_until = None;
        }
        if self
            .follow_up
            .is_some_and(|prompt| prompt.pane_index == index)
        {
            self.follow_up = None;
        }
    }

    fn toggle_voice_input(&mut self) {
        if self.voice.cancel() {
            self.voice_destination = None;
            self.status = "voice capture canceled".into();
            return;
        }

        let destination = if self.command_line.focused {
            VoiceDestination::CommandLine
        } else {
            let panes = self
                .input_targets()
                .into_iter()
                .filter_map(|index| self.panes.get(index).map(PtyPane::id))
                .collect::<Vec<_>>();
            if panes.is_empty() {
                self.status = "no awake panes available for voice input".into();
                return;
            }
            VoiceDestination::Panes {
                tab: self.active_tab,
                panes,
            }
        };

        self.voice_destination = Some(destination);
        self.status = match self.voice.start() {
            VoiceStart::Listening => format!(
                "voice listening for {} (Alt+Shift+V cancels; speech is not submitted)",
                self.input_scope_label()
            ),
            VoiceStart::DownloadingModel(display_size) => format!(
                "downloading the {} offline voice model, then listening (Alt+Shift+V cancels)",
                display_size
            ),
            VoiceStart::DownloadApprovalRequired(display_size) => {
                self.voice_destination = None;
                format!(
                    "Linux voice needs a one-time {} offline model download; press Alt+Shift+V again to approve",
                    display_size
                )
            }
        };
    }

    fn route_input(&mut self, bytes: &[u8]) -> Result<bool> {
        let targets = self.input_targets();
        self.route_input_to_targets(bytes, targets)
    }

    fn route_input_to_targets(&mut self, bytes: &[u8], targets: Vec<usize>) -> Result<bool> {
        let mut skipped_exited = 0;
        let mut changed = false;

        for index in targets {
            let pane = self
                .panes
                .get_mut(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?;
            if pane.exited {
                skipped_exited += 1;
                continue;
            }
            changed |= pane.reset_view();
            pane.write(bytes)
                .with_context(|| format!("failed to route input to pane {}", index + 1))?;
            pane.record_input(bytes);
            self.mark_pane_touched(index);
        }

        if skipped_exited > 0 {
            self.status = if skipped_exited == 1 {
                "pane exited; press R or Enter to restart, Z to sleep".into()
            } else {
                format!(
                    "skipped {skipped_exited} exited {}; press Alt+t to restart them",
                    pane_word(skipped_exited)
                )
            };
            return Ok(true);
        }

        Ok(changed)
    }

    fn input_targets(&self) -> Vec<usize> {
        awake_input_targets_for(self.focus, &self.selected, self.panes.len(), &self.sleeping)
    }

    fn target_panes(&self) -> Vec<usize> {
        input_targets_for(self.focus, &self.selected, self.panes.len())
    }

    fn toggle_sleep_for_focused_pane(&mut self) {
        if self.panes.is_empty() {
            self.status = "no panes to sleep".into();
            return;
        }

        let target = self.focus.min(self.panes.len() - 1);
        self.toggle_sleep_for_panes(&[target]);
    }

    fn restart_focused_exited_pane(&mut self) {
        if self.panes.is_empty() {
            self.status = "no panes to restart".into();
            return;
        }

        let target = self.focus.min(self.panes.len() - 1);
        self.restart_exited_panes(&[target]);
    }

    fn restart_exited_targets(&mut self) {
        let targets = self.target_panes();
        self.restart_exited_panes(&targets);
    }

    fn restart_exited_panes(&mut self, targets: &[usize]) {
        if targets.is_empty() {
            self.status = "no panes to restart".into();
            return;
        }

        let exited = self
            .panes
            .iter()
            .map(|pane| pane.exited)
            .collect::<Vec<_>>();
        let restart = restart_targets_for(targets, &exited);
        if restart.indices.is_empty() {
            self.status = "no exited target panes; Alt+Shift+t restarts exited panes".into();
            return;
        }

        let Some(plan) = self.launch_plan.as_ref() else {
            self.status = "cannot restart panes without a launch plan".into();
            return;
        };
        let specs = restart
            .indices
            .iter()
            .filter_map(|index| plan.panes.get(*index).cloned().map(|spec| (*index, spec)))
            .collect::<Vec<_>>();

        let mut restarted = 0;
        for (index, spec) in specs {
            let pane = match self.spawn_pane_instance(&spec, index) {
                Ok(pane) => pane,
                Err(error) => {
                    self.status = format!("restart failed for pane {}: {error:#}", index + 1);
                    return;
                }
            };

            self.panes[index] = pane;
            if let Some(idle) = self.pane_idle.get_mut(index) {
                *idle = PaneIdleState::new(Instant::now());
            }
            self.sleeping.remove(&index);
            restarted += 1;
        }

        self.sync_pane_sizes();
        self.status = if restart.running > 0 {
            format!(
                "restarted {restarted} {}; skipped {} running {}",
                pane_word(restarted),
                restart.running,
                pane_word(restart.running)
            )
        } else {
            format!("restarted {restarted} {}", pane_word(restarted))
        };
    }

    fn focus_next(&mut self) {
        if self.command_line.focused {
            if let Some(candidate) =
                (0..self.panes.len()).find(|index| !self.sleeping.contains(index))
            {
                self.command_line.focused = false;
                self.focus = candidate;
            }
            return;
        }

        if let Some(candidate) = wrapped_row_focus_target(
            self.focus,
            self.panes.len(),
            self.layout.size().columns,
            1,
            &self.sleeping,
        ) {
            self.focus = candidate;
        }
    }

    fn focus_previous(&mut self) {
        if self.command_line.focused {
            if let Some(candidate) = (0..self.panes.len())
                .rev()
                .find(|index| !self.sleeping.contains(index))
            {
                self.command_line.focused = false;
                self.focus = candidate;
            }
            return;
        }

        if let Some(candidate) = wrapped_row_focus_target(
            self.focus,
            self.panes.len(),
            self.layout.size().columns,
            -1,
            &self.sleeping,
        ) {
            self.focus = candidate;
        }
    }

    fn focus_in_grid(&mut self, row_delta: isize) {
        if self.command_line.focused {
            if row_delta.is_negative()
                && let Some(candidate) = (0..self.panes.len())
                    .rev()
                    .find(|index| !self.sleeping.contains(index))
            {
                self.command_line.focused = false;
                self.focus = candidate;
            }
            return;
        }

        if let Some(candidate) = wrapped_column_focus_target(
            self.focus,
            self.panes.len(),
            self.layout.size().columns,
            row_delta,
            &self.sleeping,
        ) {
            self.focus = candidate;
        }
    }

    fn focus_status(&self) -> String {
        if self.command_line.focused {
            "focused command line".into()
        } else {
            format!("focused pane {}", self.focus + 1)
        }
    }

    fn next_awake_pane(&self, start: usize) -> Option<usize> {
        if self.panes.is_empty() {
            return None;
        }

        (1..=self.panes.len())
            .map(|offset| (start + offset) % self.panes.len())
            .find(|index| !self.sleeping.contains(index))
    }

    pub fn pane_rects(&self, area: Rect) -> Vec<Rect> {
        self.layout.rects(area, self.panes.len())
    }

    pub fn panes(&self) -> &[PtyPane] {
        &self.panes
    }

    fn save_workload_setting(&mut self) -> bool {
        let previous = self.config.defaults.pane_workload;
        self.config.defaults.pane_workload = self.settings.pane_workload;
        match self.config.save(self.config_path.as_deref()) {
            Ok(_) => {
                self.applied_workloads.clear();
                true
            }
            Err(error) => {
                self.config.defaults.pane_workload = previous;
                self.settings.pane_workload = previous;
                self.status = format!("failed to save workload policy: {error:#}");
                false
            }
        }
    }

    pub fn pane_screen_lines(
        &self,
        index: usize,
        width: u16,
        height: u16,
        selection: Option<PaneSelection>,
    ) -> Vec<Line<'static>> {
        let Some(pane) = self.panes.get(index) else {
            return Vec::new();
        };
        let mut caches = self.pane_render_cache.borrow_mut();
        ui::cached_screen_lines(
            caches.entry(pane.id()).or_default(),
            pane.screen_revision(),
            pane.screen(),
            width,
            height,
            selection,
        )
    }

    pub fn focused_pane(&self) -> Option<usize> {
        (!self.command_line.focused).then_some(self.focus)
    }

    pub fn selected(&self) -> &BTreeSet<usize> {
        &self.selected
    }

    pub fn selection_for_pane(&self, index: usize) -> Option<PaneSelection> {
        self.text_selection
            .filter(|selection| selection.pane == index)
            .map(MouseSelection::range)
    }

    pub fn pane_sleeping(&self, index: usize) -> bool {
        self.sleeping.contains(&index)
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn grid_resizer(&self) -> Option<&GridPicker> {
        self.grid_resizer.as_ref()
    }

    pub fn settings_open(&self) -> bool {
        self.settings.open
    }

    pub fn pane_settings_open(&self) -> bool {
        self.pane_settings.open
    }

    pub fn previous_panes_open(&self) -> bool {
        self.previous_panes.open
    }

    pub fn image_overlay_view(&self) -> Option<&ImagePreview> {
        self.image_overlay.as_ref()
    }

    pub fn exited_recovery_view(&self) -> Option<ExitedPaneRecoveryView> {
        let pane = self.panes.get(self.focus)?;
        if !pane.exited || self.sleeping.contains(&self.focus) {
            return None;
        }

        Some(ExitedPaneRecoveryView {
            pane_index: self.focus,
            pane_label: self.pane_label(self.focus),
            target_count: 1,
        })
    }

    pub fn settings_rows(&self) -> Vec<SettingsRow> {
        self.settings.rows()
    }

    pub fn manager_settings_rows(&self) -> Vec<SettingsRow> {
        self.settings.manager_rows(&self.config.manager)
    }

    pub fn settings_tab(&self) -> SettingsTab {
        self.settings.tab
    }

    pub fn auth_profiles(&self) -> &[AuthProfile] {
        &self.auth_profiles
    }

    pub fn auth_cursor(&self) -> usize {
        self.settings.auth_cursor
    }

    pub fn auth_refreshing(&self) -> bool {
        self.settings.auth_refreshing
    }

    pub fn auth_create(&self) -> Option<&AuthCreateState> {
        self.settings.create_auth.as_ref()
    }

    pub fn auth_default(&self, kind: AgentKind) -> Option<&str> {
        self.config.auth.defaults.get(kind)
    }

    pub fn auth_home_label(&self) -> String {
        auth::resolve_home(&self.config.auth)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("unresolved: {error:#}"))
    }

    pub fn auth_auto_cycle(&self) -> bool {
        self.config.auth.auto_cycle
    }

    pub fn activity_badges_enabled(&self) -> bool {
        self.settings.activity_badges
    }

    pub fn compact_titles_enabled(&self) -> bool {
        self.settings.compact_titles
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    pub fn palette(&self) -> &GridPalette {
        &self.settings.palette
    }

    pub fn pane_label(&self, index: usize) -> String {
        self.pane_names
            .get(index)
            .and_then(|name| name.as_deref())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| (index + 1).to_string())
    }

    pub fn rename_pane_view(&self) -> Option<RenamePaneView> {
        self.rename.open.then(|| RenamePaneView {
            pane_index: self.rename.pane_index,
            pane_label: self.pane_label(self.rename.pane_index),
            value: self.rename.value.clone(),
            cursor: self.rename.cursor,
        })
    }

    pub fn rename_tab_view(&self) -> Option<RenameTabView> {
        self.tab_rename.open.then(|| RenameTabView {
            title: self.tab_title.clone(),
            value: self.tab_rename.value.clone(),
            cursor: self.tab_rename.cursor,
        })
    }

    pub fn tab_labels(&self) -> Vec<TabLabel> {
        (0..self.tabs.len())
            .map(|index| {
                if index == self.active_tab {
                    return TabLabel {
                        title: self.tab_title.clone(),
                        active: true,
                        activity: self.panes.iter().any(|pane| pane.active),
                        exited: !self.panes.is_empty() && self.panes.iter().all(|pane| pane.exited),
                    };
                }

                let Some(tab) = self.tabs.get(index).and_then(Option::as_ref) else {
                    return TabLabel {
                        title: format!("Grid {}", index + 1),
                        active: false,
                        activity: false,
                        exited: false,
                    };
                };

                TabLabel {
                    title: tab.title.clone(),
                    active: false,
                    activity: tab.panes.iter().any(|pane| pane.active),
                    exited: !tab.panes.is_empty() && tab.panes.iter().all(|pane| pane.exited),
                }
            })
            .collect()
    }

    pub fn pane_settings_view(&self) -> Option<PaneSettingsView> {
        if !self.pane_settings.open {
            return None;
        }

        let index = self.pane_settings.pane_index;
        let pane = self.panes.get(index)?;
        let spec = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index));
        let current_auth = spec.and_then(|spec| spec.auth_name.as_deref());
        let auth_options = self
            .compatible_auth_profiles(index)
            .into_iter()
            .map(|profile| PaneAuthOption {
                name: profile.name.clone(),
                account_label: profile.account_label.clone(),
                ready: profile.ready,
                current: Some(profile.name.as_str()) == current_auth,
            })
            .collect();
        Some(PaneSettingsView {
            index,
            label: self.pane_label(index),
            folder: self
                .pane_folder(index)
                .map(str::to_string)
                .unwrap_or_else(|| path_label(pane.cwd())),
            worktree: self.pane_worktree(index).map(str::to_string),
            history_summary: self
                .pane_settings
                .history_summary
                .clone()
                .unwrap_or_else(|| self.pane_history_summary(index)),
            focused: self.focus == index,
            selected: self.selected.contains(&index),
            sleeping: self.sleeping.contains(&index),
            exited: pane.exited,
            auth_kind: spec.and_then(|spec| spec.command.agent_kind),
            auth_options,
            auth_cursor: self.pane_settings.auth_cursor,
            goal: self.manager_goal.as_ref().map(|goal| PaneGoalView {
                objective: goal.objective.clone(),
                status: goal.status.clone(),
            }),
            manager_configured: self.config.manager.is_configured(),
        })
    }

    pub fn previous_panes_view(&self) -> Option<PreviousPanesView> {
        self.previous_panes.open.then(|| PreviousPanesView {
            cursor: self
                .previous_panes
                .cursor
                .min(self.panes.len().saturating_sub(1)),
            panes: self
                .panes
                .iter()
                .enumerate()
                .map(|(index, pane)| {
                    let agent_label = self
                        .launch_plan
                        .as_ref()
                        .and_then(|plan| plan.panes.get(index))
                        .and_then(|pane| pane.agent_label());
                    let summary =
                        pane_activity_summary(pane).unwrap_or_else(|| "waiting for output".into());
                    let summary = agent_label
                        .map(|label| format!("{label} | {summary}"))
                        .unwrap_or(summary);

                    PreviousPaneView {
                        index,
                        label: self.pane_label(index),
                        folder: self
                            .pane_folder(index)
                            .map(str::to_string)
                            .unwrap_or_else(|| path_label(pane.cwd())),
                        worktree: self.pane_worktree(index).map(str::to_string),
                        summary,
                        focused: self.focus == index,
                        selected: self.selected.contains(&index),
                        sleeping: self.sleeping.contains(&index),
                        exited: pane.exited,
                    }
                })
                .collect(),
        })
    }

    pub fn follow_up_dialog(&self) -> Option<FollowUpDialog> {
        let prompt = self.follow_up.as_ref()?;
        let todo = self.settings.todo_prompts.get(prompt.todo_index)?;
        let quiet_seconds = self
            .pane_idle
            .get(prompt.pane_index)
            .map(|idle| idle.last_output_at.elapsed().as_secs())
            .unwrap_or_default();

        Some(FollowUpDialog {
            pane_number: prompt.pane_index + 1,
            prompt: todo.clone(),
            todo_position: prompt.todo_index + 1,
            todo_count: self.settings.todo_prompts.len(),
            quiet_seconds,
        })
    }
    pub fn goal_editor_view(&self) -> Option<GoalEditorView> {
        self.goal_editor.as_ref().map(|editor| GoalEditorView {
            input: editor.input.clone(),
        })
    }

    pub fn command_focused(&self) -> bool {
        self.command_line.focused
    }

    pub fn command_cwd(&self) -> &Path {
        &self.command_line.cwd
    }

    pub fn command_input(&self) -> &str {
        &self.command_line.input
    }

    pub fn command_cursor_chars(&self) -> usize {
        self.command_line.cursor_chars()
    }

    pub fn command_output_expanded(&self) -> bool {
        self.command_line.output_expanded()
    }

    pub fn command_output_lines(&self) -> &[String] {
        &self.command_line.output_lines
    }

    pub fn command_running(&self) -> bool {
        self.command_line.running
    }

    pub fn voice_listening(&self) -> bool {
        self.voice.is_listening()
    }

    pub fn input_scope_label(&self) -> &'static str {
        if self.command_line.focused {
            "command line"
        } else if self.selected.len() > 1 {
            "selected panes"
        } else {
            "focused pane"
        }
    }

    pub fn pane_folder(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .map(|pane| pane.folder_name.as_str())
    }

    pub fn pane_worktree(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.worktree_name.as_deref())
    }

    pub fn pane_header_summary(&self, index: usize, max_chars: usize) -> String {
        if let Some(goal) = self.manager_goal.as_ref() {
            return pane_header_text(Some(&goal.objective), None, max_chars);
        }

        let Some(pane) = self.panes.get(index) else {
            return String::new();
        };
        let mut caches = self.conversation_cache.borrow_mut();
        let cache = caches.entry(pane.id()).or_default();
        if cache.revision != pane.screen_revision() {
            cache.revision = pane.screen_revision();
            cache.summary = pane_activity_summary(pane);
        }
        pane_header_text(None, cache.summary.as_deref(), max_chars)
    }

    fn pane_history_summary(&self, index: usize) -> String {
        let Some(pane) = self.panes.get(index) else {
            return "pane is no longer available".into();
        };

        let summary = pane_activity_summary(pane).unwrap_or_else(|| "waiting for output".into());
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.agent_label())
            .map(|label| format!("{label} | {summary}"))
            .unwrap_or(summary)
    }

    pub fn pane_usage_label(&self, index: usize) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(usage_key) = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .map(|pane| {
                pane.auth_dir
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| pane.profile_name.clone())
            })
            && let Some(label) = self.profile_usage.get(&usage_key)
        {
            parts.push(label.clone());
        }

        if let Some(label) = &self.api_spend_label {
            parts.push(label.clone());
        }

        (!parts.is_empty()).then(|| parts.join(" | "))
    }

    pub fn sync_pane_sizes(&mut self) {
        for (index, rect) in self.rects.iter().enumerate() {
            let Some(pane) = self.panes.get_mut(index) else {
                continue;
            };

            let rows = rect.height.saturating_sub(2).max(1);
            let cols = rect.width.saturating_sub(2).max(1);
            if let Err(error) = pane.resize(rows, cols) {
                self.status = format!("resize failed: {error:#}");
            }
        }
    }

    fn sync_initial_pane_sizes(&mut self, terminal: &Tui) -> Result<()> {
        let size = terminal.size().context("failed to read terminal size")?;
        self.grid_area = Rect::new(0, 1, size.width, size.height.saturating_sub(3));
        self.rects = self.pane_rects(self.grid_area);
        self.sync_pane_sizes();
        Ok(())
    }

    fn active_pane_cwd(&self) -> Option<PathBuf> {
        self.panes
            .get(self.focus)
            .map(|pane| pane.cwd().to_path_buf())
            .or_else(|| self.panes.first().map(|pane| pane.cwd().to_path_buf()))
            .or_else(|| {
                self.launch_plan
                    .as_ref()
                    .and_then(|plan| plan.panes.first())
                    .map(|pane| pane.cwd.clone())
            })
    }

    fn start_usage_for_active_tab(&mut self) {
        if let Some(plan) = self.launch_plan.clone() {
            self.start_usage_monitor(&plan);
        }
    }

    fn toggle_settings_tab(&mut self) {
        self.settings.tab = match self.settings.tab {
            SettingsTab::General => SettingsTab::Auth,
            SettingsTab::Auth => SettingsTab::Manager,
            SettingsTab::Manager => SettingsTab::General,
        };
        if self.settings.tab == SettingsTab::Auth && self.auth_profiles.is_empty() {
            self.start_auth_refresh();
        }
    }

    fn start_auth_refresh(&mut self) {
        if self.settings.auth_refreshing {
            self.status = "auth refresh already running".into();
            return;
        }

        let auth_config = self.config.auth.clone();
        let (tx, rx) = std_mpsc::channel();
        self.auth_refresh_rx = Some(rx);
        self.settings.auth_refreshing = true;
        self.status = "refreshing auth profiles".into();

        thread::spawn(move || {
            let result = auth::discover_profiles_with_usage(&auth_config)
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
        });
    }

    fn drain_auth_refresh(&mut self) -> bool {
        let Some(rx) = &self.auth_refresh_rx else {
            return false;
        };

        let result = match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std_mpsc::TryRecvError::Empty) => None,
            Err(std_mpsc::TryRecvError::Disconnected) => Some(Err("auth refresh stopped".into())),
        };

        let Some(result) = result else {
            return false;
        };

        self.auth_refresh_rx = None;
        self.settings.auth_refreshing = false;
        match result {
            Ok(profiles) => {
                self.auth_profiles = profiles;
                self.settings.auth_cursor = self
                    .settings
                    .auth_cursor
                    .min(self.auth_profiles.len().saturating_sub(1));
                if self.pane_settings.open {
                    let pane_index = self.pane_settings.pane_index;
                    let current_auth = self
                        .launch_plan
                        .as_ref()
                        .and_then(|plan| plan.panes.get(pane_index))
                        .and_then(|spec| spec.auth_name.as_deref());
                    self.pane_settings.auth_cursor = self
                        .compatible_auth_profiles(pane_index)
                        .iter()
                        .position(|profile| Some(profile.name.as_str()) == current_auth)
                        .unwrap_or(0);
                }
                self.status = format!("loaded {} auth profiles", self.auth_profiles.len());
            }
            Err(error) => self.status = format!("auth refresh failed: {error}"),
        }
        true
    }

    fn move_auth_cursor(&mut self, delta: isize) {
        if self.auth_profiles.is_empty() {
            return;
        }

        let len = self.auth_profiles.len() as isize;
        self.settings.auth_cursor =
            (self.settings.auth_cursor as isize + delta).clamp(0, len - 1) as usize;
    }

    fn selected_auth_profile(&self) -> Option<&AuthProfile> {
        self.auth_profiles.get(self.settings.auth_cursor)
    }

    fn set_selected_auth_default(&mut self) -> Result<()> {
        let Some(profile) = self.selected_auth_profile().cloned() else {
            self.status = "no auth profile selected".into();
            return Ok(());
        };

        self.config
            .auth
            .defaults
            .set(profile.kind, profile.name.clone());
        let path = self.config.save(self.config_path.as_deref())?;
        self.status = format!(
            "{} default auth: {} ({})",
            profile.kind.display_name(),
            profile.name,
            path.display()
        );
        Ok(())
    }

    fn toggle_auth_auto_cycle(&mut self) -> Result<()> {
        self.config.auth.auto_cycle = !self.config.auth.auto_cycle;
        let path = self.config.save(self.config_path.as_deref())?;
        let mode = if self.config.auth.auto_cycle {
            "auto-cycle ready profiles"
        } else {
            "manual profile selection"
        };
        self.status = format!("auth launch mode: {mode} ({})", path.display());
        Ok(())
    }

    fn start_auth_create(&mut self) -> Result<()> {
        let kind = self
            .selected_auth_profile()
            .map(|profile| profile.kind)
            .unwrap_or(AgentKind::Claude);
        let name = auth::next_profile_name(&self.config.auth, kind)?;
        self.settings.create_auth = Some(AuthCreateState { kind, name });
        self.status = "creating auth profile".into();
        Ok(())
    }

    fn toggle_create_auth_kind(&mut self) -> Result<()> {
        let Some(create) = &mut self.settings.create_auth else {
            return Ok(());
        };
        let kind = create.kind.toggle();
        let name = auth::next_profile_name(&self.config.auth, kind)?;
        *create = AuthCreateState { kind, name };
        Ok(())
    }

    fn create_auth_profile(&mut self) -> Result<()> {
        let Some(create) = self.settings.create_auth.clone() else {
            return Ok(());
        };
        let profile = auth::create_profile(&self.config.auth, create.kind, create.name.trim())?;
        self.settings.create_auth = None;
        self.auth_profiles = auth::discover_profiles(&self.config.auth)?;
        if let Some(index) = self
            .auth_profiles
            .iter()
            .position(|candidate| candidate.name == profile.name)
        {
            self.settings.auth_cursor = index;
        }
        self.status = format!(
            "created {} auth profile {}",
            create.kind.as_str(),
            profile.name
        );
        Ok(())
    }

    fn run_auth_login(&mut self, terminal: &mut Tui, profile: AuthProfile) -> Result<()> {
        let launch = auth::login_command(&profile);
        suspend_terminal(terminal)?;
        let run_result = Command::new(&launch.command)
            .args(&launch.args)
            .envs(&launch.env)
            .status()
            .with_context(|| format!("failed to run {}", launch.command.display()));
        let resume_result = resume_terminal(terminal);
        resume_result?;

        match run_result {
            Ok(status) if status.success() => {
                self.status = format!("{} login completed", profile.name);
                self.start_auth_refresh();
            }
            Ok(status) => {
                self.status = format!("{} login exited with {}", profile.name, status);
                self.start_auth_refresh();
            }
            Err(error) => self.status = format!("auth login failed: {error:#}"),
        }
        Ok(())
    }

    fn start_session_recorder(&mut self, plan: &LaunchPlan) -> Result<()> {
        if self.session_recorder.is_none() {
            self.session_recorder = Some(SessionRecorder::start_new(plan)?);
        }
        Ok(())
    }

    fn save_session_snapshot(&mut self) -> Result<()> {
        let Some(plan) = self.launch_plan.clone() else {
            return Ok(());
        };
        let Some(recorder) = self.session_recorder.as_mut() else {
            return Ok(());
        };

        recorder.update(&plan, &self.panes);
        recorder.save()
    }
}

fn adaptive_output_frame_interval(refresh_ms: i32, pane_count: usize) -> Duration {
    let configured = Duration::from_millis(refresh_ms.max(1) as u64);
    if pane_count > 8 {
        configured.max(LARGE_GRID_FRAME_INTERVAL)
    } else {
        configured
    }
}

fn resolve_grid(cli: &Cli) -> Result<GridSize> {
    if let Some(grid) = &cli.grid {
        return GridSize::parse(grid).with_context(|| format!("invalid grid '{grid}'"));
    }

    if cli.layout == GridMode::Auto {
        return Ok(GridSize::from_count(cli.count.unwrap_or(6)));
    }

    if let Some(count) = cli.count {
        return Ok(GridSize::from_count(count));
    }

    Ok(GridSize {
        rows: 2,
        columns: 3,
    })
}

fn resolve_direct_launch_plan(
    cli: &Cli,
    config: &Config,
    worktrees: Option<&ManagedWorktreeOptions>,
) -> Result<Option<LaunchPlan>> {
    if !uses_direct_launch(cli) {
        return Ok(None);
    }

    let grid = resolve_grid(cli)?;
    let profile_name = resolve_profile_name(cli, config);
    let profile = find_profile(config, &profile_name)?;
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or(env::current_dir().context("failed to resolve current directory")?);
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let pane_count = cli.count.unwrap_or_else(|| grid.count()).clamp(1, 100);

    Ok(Some(LaunchPlan::from_launch_options(
        profile_name,
        profile,
        cwd,
        pane_count,
        grid,
        worktrees,
    )?))
}

fn apply_auth_defaults(plan: &mut LaunchPlan, config: &Config) -> Result<()> {
    let cycle_profiles = if config.auth.auto_cycle {
        auth::discover_profiles(&config.auth)?
    } else {
        Vec::new()
    };
    let mut claude_index = plan
        .panes
        .iter()
        .filter(|spec| spec.auth_name.is_some() && spec.auth_kind == Some(AgentKind::Claude))
        .count();
    let mut codex_index = plan
        .panes
        .iter()
        .filter(|spec| spec.auth_name.is_some() && spec.auth_kind == Some(AgentKind::Codex))
        .count();

    for spec in &mut plan.panes {
        let Some(kind) = spec.command.agent_kind else {
            continue;
        };

        if let Some(name) = spec.auth_name.clone() {
            let auth_env = auth::env_for_profile(&config.auth, kind, &name)?;
            set_spec_auth(spec, auth_env);
            continue;
        }

        let auth_env = if config.auth.auto_cycle {
            let ready = cycle_profiles
                .iter()
                .filter(|profile| profile.kind == kind && profile.ready)
                .collect::<Vec<_>>();
            let index = match kind {
                AgentKind::Claude => &mut claude_index,
                AgentKind::Codex => &mut codex_index,
            };
            let selected = ready.get(*index % ready.len().max(1)).copied();
            *index += 1;
            selected
                .map(|profile| auth::env_for_profile(&config.auth, kind, &profile.name))
                .transpose()?
        } else {
            auth::env_for_default(&config.auth, kind)?
        };

        if let Some(auth_env) = auth_env {
            set_spec_auth(spec, auth_env);
        }
    }
    Ok(())
}

fn clear_spec_auth(spec: &mut PaneLaunchSpec) {
    spec.env.remove(AgentKind::Claude.env_var());
    spec.env.remove(AgentKind::Codex.env_var());
    spec.auth_name = None;
    spec.auth_kind = None;
    spec.auth_dir = None;
}

fn set_spec_auth(spec: &mut PaneLaunchSpec, auth_env: auth::AuthEnv) {
    clear_spec_auth(spec);
    spec.env.extend(auth_env.env_map());
    spec.auth_name = Some(auth_env.name);
    spec.auth_kind = Some(auth_env.kind);
    spec.auth_dir = Some(auth_env.dir);
}

fn uses_direct_launch(cli: &Cli) -> bool {
    cli.grid.is_some()
        || cli.count.is_some()
        || cli.profile.is_some()
        || cli.cwd.is_some()
        || cli.layout == GridMode::Auto
}

fn resolve_profile_name(cli: &Cli, config: &Config) -> String {
    resolve_profile_name_from(
        cli,
        config,
        env::var("GRIDBASH_PROFILE").ok(),
        env::var("GRIDBASH_INVOKING_PROFILE").ok(),
    )
}

fn resolve_profile_name_from(
    cli: &Cli,
    config: &Config,
    environment_profile: Option<String>,
    invoking_profile: Option<String>,
) -> String {
    cli.profile
        .clone()
        .or(environment_profile)
        .or(invoking_profile)
        .or_else(|| config.defaults.profile.clone())
        .unwrap_or_else(|| default_profile_name().into())
}
fn resolved_current_dir() -> Result<std::path::PathBuf> {
    let current = env::current_dir().context("failed to resolve current directory")?;
    Ok(current.canonicalize().unwrap_or(current))
}
fn capture_goal_text(
    manager_goal: &mut Option<ManagerGoal>,
    sleeping: &BTreeSet<usize>,
    pane_index: usize,
    output: &str,
) {
    if sleeping.contains(&pane_index) {
        return;
    }

    let Some(goal) = manager_goal.as_mut() else {
        return;
    };
    if !goal.active {
        return;
    }
    if output.trim().is_empty() {
        return;
    }
    goal.output_buffer
        .push_str(&format!("\n[PANE {} OUTPUT]\n{output}", pane_index + 1));
    trim_goal_buffer(&mut goal.output_buffer);
    goal.last_output_at = Some(Instant::now());
    if !goal.in_flight && goal.dispatch_retry.is_none() && goal.review_notice.is_none() {
        goal.status = "waiting for quiet grid output".into();
    }
}

fn goal_targets(panes: &[PtyPane], sleeping: &BTreeSet<usize>) -> Vec<GoalTarget> {
    panes
        .iter()
        .enumerate()
        .filter(|(index, pane)| !sleeping.contains(index) && !pane.exited)
        .map(|(index, pane)| GoalTarget {
            pane_number: index + 1,
            pane_id: pane.id(),
            pane_generation: pane.generation(),
            screen_revision: pane.screen_revision(),
            input_revision: pane.input_revision(),
        })
        .collect()
}

fn manager_goal_pane_metadata(
    panes: &[PtyPane],
    pane_names: &[Option<String>],
    launch_plan: Option<&LaunchPlan>,
) -> Vec<String> {
    panes
        .iter()
        .enumerate()
        .map(|(index, pane)| {
            let mut parts = Vec::new();
            if let Some(spec) = launch_plan.and_then(|plan| plan.panes.get(index)) {
                parts.push(format!(
                    "role={}",
                    spec.agent_label().unwrap_or_else(|| "shell".into())
                ));
            }
            if let Some(name) = pane_names
                .get(index)
                .and_then(Option::as_deref)
                .filter(|name| !name.trim().is_empty())
            {
                parts.push(format!("name={name}"));
            }
            if let Some(spec) = launch_plan.and_then(|plan| plan.panes.get(index)) {
                parts.push(format!("profile={}", spec.profile_name));
                parts.push(format!("folder={}", spec.folder_name));
            } else {
                parts.push(format!("folder={}", path_label(pane.cwd())));
            }
            bounded_prefix(&parts.join("; "), 64)
        })
        .collect()
}

fn manager_goal_context(
    panes: &[PtyPane],
    pane_metadata: &[String],
    sleeping: &BTreeSet<usize>,
) -> String {
    let metadata_bytes = pane_metadata
        .iter()
        .map(|metadata| metadata.len().saturating_add(56))
        .sum::<usize>();
    let output_budget = PANE_GOAL_OUTPUT_MAX_BYTES.saturating_sub(metadata_bytes);
    let per_pane_budget = output_budget.checked_div(panes.len().max(1)).unwrap_or(0);
    let mut contexts = Vec::with_capacity(panes.len());
    for (index, pane) in panes.iter().enumerate() {
        let state = if sleeping.contains(&index) {
            "sleeping; do not target"
        } else if pane.exited {
            "exited; do not target"
        } else {
            "available"
        };
        let metadata = pane_metadata
            .get(index)
            .filter(|metadata| !metadata.is_empty())
            .cloned()
            .unwrap_or_default();
        let output = if sleeping.contains(&index) || pane.exited {
            "(output omitted while unavailable)".into()
        } else {
            tail_text(pane.output_tail(), per_pane_budget)
                .filter(|output| !output.trim().is_empty())
                .unwrap_or_else(|| "(no recent output)".into())
        };
        contexts.push(GoalPaneContext {
            pane_number: index + 1,
            state,
            metadata,
            output,
        });
    }

    format_manager_goal_context(&contexts)
}

fn format_manager_goal_context(panes: &[GoalPaneContext]) -> String {
    let mut context = String::from(
        "Only panes marked available below are valid command targets for this review.\n",
    );
    for pane in panes {
        let metadata = if pane.metadata.is_empty() {
            String::new()
        } else {
            format!("; {}", pane.metadata)
        };
        context.push_str(&format!(
            "\n--- PANE {} [{}{}] ---\n{}\n",
            pane.pane_number, pane.state, metadata, pane.output
        ));
    }
    context
}

fn tail_text(value: &str, max_bytes: usize) -> Option<String> {
    if max_bytes == 0 {
        return None;
    }
    if value.len() <= max_bytes {
        return Some(value.to_string());
    }
    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    Some(value[start..].to_string())
}

fn bounded_prefix(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let content_bytes = max_bytes.saturating_sub(3);
    let mut end = content_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn goal_pane_states(panes: &[PtyPane], sleeping: &BTreeSet<usize>) -> Vec<GoalPaneState> {
    panes
        .iter()
        .enumerate()
        .map(|(index, pane)| GoalPaneState {
            pane_id: pane.id(),
            pane_generation: pane.generation(),
            screen_revision: pane.screen_revision(),
            input_revision: pane.input_revision(),
            unavailable: sleeping.contains(&index) || pane.exited,
        })
        .collect()
}

fn goal_snapshot_is_stale(targets: &[GoalTarget], current: &[GoalPaneState]) -> bool {
    if current.iter().filter(|pane| !pane.unavailable).count() != targets.len() {
        return true;
    }
    targets.iter().any(|target| {
        current
            .iter()
            .find(|pane| {
                pane.pane_id == target.pane_id && pane.pane_generation == target.pane_generation
            })
            .is_none_or(|pane| {
                pane.unavailable
                    || pane.screen_revision != target.screen_revision
                    || pane.input_revision != target.input_revision
            })
    })
}

fn goal_target_index(
    targets: &[GoalTarget],
    pane_number: usize,
    current: &[GoalPaneState],
) -> Result<usize, String> {
    let target = targets
        .iter()
        .find(|target| target.pane_number == pane_number)
        .ok_or_else(|| format!("pane {pane_number} was not an available target"))?;
    let index = current
        .iter()
        .position(|pane| {
            pane.pane_id == target.pane_id && pane.pane_generation == target.pane_generation
        })
        .ok_or_else(|| format!("pane {pane_number} changed before dispatch"))?;
    if current[index].unavailable {
        return Err(format!("pane {pane_number} became unavailable"));
    }
    if current[index].screen_revision != target.screen_revision
        || current[index].input_revision != target.input_revision
    {
        return Err(format!("pane {pane_number} changed before dispatch"));
    }
    Ok(index)
}

#[derive(Debug)]
struct PlannedGoalCommand<'a> {
    command: &'a ManagerCommand,
    pane_index: usize,
    key: GoalCommandKey,
}

#[derive(Debug)]
struct GoalCommandPlan<'a> {
    commands: Vec<PlannedGoalCommand<'a>>,
    skipped_successful: usize,
}

fn goal_command_plan<'a>(
    commands: &'a [ManagerCommand],
    targets: &[GoalTarget],
    current: &[GoalPaneState],
    successful: &BTreeSet<GoalCommandKey>,
) -> Result<GoalCommandPlan<'a>, Vec<String>> {
    let mut planned = Vec::with_capacity(commands.len());
    let mut failures = Vec::new();
    let mut skipped_successful = 0;
    for command in commands {
        match goal_target_index(targets, command.pane, current) {
            Ok(index) => {
                let target = &targets[targets
                    .iter()
                    .position(|target| target.pane_number == command.pane)
                    .expect("validated goal target")];
                let key = GoalCommandKey {
                    pane_id: target.pane_id,
                    pane_generation: target.pane_generation,
                    command: command.command.clone(),
                };
                if successful.contains(&key) {
                    skipped_successful += 1;
                } else {
                    planned.push(PlannedGoalCommand {
                        command,
                        pane_index: index,
                        key,
                    });
                }
            }
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        Ok(GoalCommandPlan {
            commands: planned,
            skipped_successful,
        })
    } else {
        Err(failures)
    }
}

fn next_goal_dispatch_token(goal: &mut ManagerGoal) -> PtyWriteToken {
    goal.next_dispatch_sequence = goal.next_dispatch_sequence.wrapping_add(1);
    if goal.next_dispatch_sequence == 0 {
        goal.next_dispatch_sequence = 1;
    }
    PtyWriteToken(((goal.id as u128) << 64) | goal.next_dispatch_sequence as u128)
}

fn goal_command_label(key: &GoalCommandKey, current: &[GoalPaneState]) -> String {
    current
        .iter()
        .position(|pane| pane.pane_id == key.pane_id && pane.pane_generation == key.pane_generation)
        .map(|index| format!("PANE {}", index + 1))
        .unwrap_or_else(|| {
            format!(
                "unavailable PTY {} generation {}",
                key.pane_id.0, key.pane_generation
            )
        })
}

fn format_goal_dispatch_record(dispatch: &GoalDispatchRetry, current: &[GoalPaneState]) -> String {
    let mut lines = Vec::new();
    for key in &dispatch.successful {
        lines.push(format!(
            "- {} command {:?}: sent successfully; do not repeat this exact command during this retry.",
            goal_command_label(key, current),
            key.command
        ));
    }
    for key in dispatch.pending.values() {
        lines.push(format!(
            "- {} command {:?}: awaiting PTY writer acknowledgement.",
            goal_command_label(key, current),
            key.command
        ));
    }
    for (key, error) in &dispatch.failed {
        lines.push(format!(
            "- {} command {:?}: failed and still needs attention ({error}).",
            goal_command_label(key, current),
            key.command
        ));
    }
    if lines.is_empty() {
        "No pane command remains recorded for this retry.".into()
    } else {
        lines.join("\n")
    }
}

fn dispatch_failure_summary(dispatch: &GoalDispatchRetry, current: &[GoalPaneState]) -> String {
    dispatch
        .failed
        .iter()
        .map(|(key, error)| format!("{} write failed: {error}", goal_command_label(key, current)))
        .collect::<Vec<_>>()
        .join("; ")
}

fn schedule_goal_retry(goal: &mut ManagerGoal, notice: Option<String>, status: String) -> bool {
    goal.failure_count = goal.failure_count.saturating_add(1);
    goal.review_notice = notice.map(|notice| bounded_prefix(&notice, 2_048));
    if goal.failure_count >= PANE_GOAL_MAX_FAILURES {
        goal.active = false;
        goal.output_buffer.clear();
        goal.last_output_at = None;
        goal.retry_after = None;
        goal.dispatch_retry = None;
        goal.status = format!("stopped after repeated failures: {status}");
        return false;
    }

    goal.output_buffer
        .push_str("\nGrid manager retry requested");
    trim_goal_buffer(&mut goal.output_buffer);
    goal.last_output_at = Some(Instant::now());
    goal.retry_after = Some(Instant::now() + PANE_GOAL_RETRY_DELAY);
    goal.status = status;
    true
}

fn finish_goal_dispatch(goal: &mut ManagerGoal, current: &[GoalPaneState]) -> Option<String> {
    let dispatch = goal.dispatch_retry.as_ref()?;
    if !dispatch.pending.is_empty() {
        return None;
    }

    if dispatch.failed.is_empty() {
        let sent = dispatch.successful.len();
        let summary = dispatch.summary.clone();
        goal.dispatch_retry = None;
        goal.retry_after = None;
        goal.failure_count = 0;
        goal.review_notice = None;
        goal.status = if sent == 0 {
            if summary.is_empty() {
                "monitoring grid".into()
            } else {
                format!("monitoring: {summary}")
            }
        } else if summary.is_empty() {
            format!("sent {sent} pane command(s)")
        } else {
            format!("sent {sent} command(s): {summary}")
        };
        return Some(if sent == 0 {
            "grid manager is monitoring pane output".into()
        } else {
            format!("grid manager sent {sent} pane command(s)")
        });
    }

    let sent = dispatch.successful.len();
    let error = dispatch_failure_summary(dispatch, current);
    let retrying = schedule_goal_retry(goal, None, format!("sent {sent}; dispatch issue: {error}"));
    Some(if retrying {
        format!("grid manager sent {sent}; dispatch issue: {error}")
    } else {
        "grid manager stopped after repeated dispatch failures".into()
    })
}

fn apply_goal_dispatch_result(
    manager_goal: &mut Option<ManagerGoal>,
    current: &[GoalPaneState],
    pane: PaneId,
    generation: u64,
    token: PtyWriteToken,
    result: Result<(), String>,
) -> Option<String> {
    let goal = manager_goal.as_mut()?;
    let dispatch = goal.dispatch_retry.as_mut()?;
    let key = dispatch.pending.remove(&token)?;
    if key.pane_id != pane || key.pane_generation != generation {
        dispatch.pending.insert(token, key);
        return None;
    }

    match result {
        Ok(()) => {
            dispatch.successful.insert(key);
        }
        Err(error) => {
            let label = goal_command_label(&key, current);
            dispatch.failed.insert(key, error.clone());
            goal.status = format!("dispatch issue for {label}: {error}");
        }
    }

    if dispatch.pending.is_empty() {
        return finish_goal_dispatch(goal, current);
    }

    let pending = dispatch.pending.len();
    if dispatch.failed.is_empty() {
        goal.status = format!("dispatching grid commands; awaiting {pending} acknowledgement(s)");
        Some(format!(
            "grid manager is awaiting {pending} PTY write acknowledgement(s)"
        ))
    } else {
        let error = dispatch_failure_summary(dispatch, current);
        goal.status = format!("dispatch issue: {error}; awaiting {pending} acknowledgement(s)");
        Some(format!("grid manager dispatch issue: {error}"))
    }
}

fn apply_goal_review(
    panes: &mut [PtyPane],
    manager_goal: &mut Option<ManagerGoal>,
    sleeping: &BTreeSet<usize>,
    event: &GoalReviewEvent,
) -> Option<String> {
    let goal = manager_goal.as_mut()?;
    if goal.id != event.goal_id {
        return None;
    }
    goal.in_flight = false;
    let current = goal_pane_states(panes, sleeping);
    if event.result.is_ok() && goal_snapshot_is_stale(&event.targets, &current) {
        if goal.output_buffer.trim().is_empty() {
            goal.output_buffer
                .push_str("Grid activity changed during manager review");
        }
        goal.last_output_at.get_or_insert_with(Instant::now);
        goal.status = "grid changed during review; refreshing snapshot".into();
        return Some("grid manager discarded a stale review".into());
    }

    match &event.result {
        Ok(ManagerDecision::Continue { commands, summary }) => {
            let successful = goal
                .dispatch_retry
                .as_ref()
                .map(|dispatch| &dispatch.successful)
                .cloned()
                .unwrap_or_default();
            let plan = match goal_command_plan(commands, &event.targets, &current, &successful) {
                Ok(plan) => plan,
                Err(failures) => {
                    let error = failures.join("; ");
                    let retrying = schedule_goal_retry(
                        goal,
                        Some(format!("Manager dispatch validation failed: {error}")),
                        format!("dispatch issue: {error}"),
                    );
                    return Some(if retrying {
                        format!("grid manager dispatch issue: {error}")
                    } else {
                        "grid manager stopped after repeated dispatch failures".into()
                    });
                }
            };

            let mut dispatch = goal.dispatch_retry.take().unwrap_or_default();
            dispatch.pending.clear();
            dispatch.failed.clear();
            dispatch.summary = summary.clone();
            let dispatching = plan.commands.len();
            let skipped_successful = plan.skipped_successful;
            for planned in plan.commands {
                let token = next_goal_dispatch_token(goal);
                let bytes = paste_and_enter_bytes(&planned.command.command);
                match panes[planned.pane_index].write_tracked(&bytes, token) {
                    Ok(()) => {
                        panes[planned.pane_index].record_input(&bytes);
                        dispatch.pending.insert(token, planned.key);
                    }
                    Err(error) => {
                        dispatch.failed.insert(planned.key, format!("{error:#}"));
                    }
                }
            }
            goal.dispatch_retry = Some(dispatch);
            goal.retry_after = None;
            goal.review_notice = None;
            if goal
                .dispatch_retry
                .as_ref()
                .is_some_and(|dispatch| !dispatch.pending.is_empty())
            {
                goal.status = if skipped_successful == 0 {
                    format!(
                        "dispatching {dispatching} grid command(s); awaiting PTY acknowledgement"
                    )
                } else {
                    format!(
                        "dispatching {dispatching} grid command(s); skipped {skipped_successful} already sent"
                    )
                };
                Some(format!(
                    "grid manager queued {dispatching} pane command(s) for acknowledged delivery"
                ))
            } else {
                finish_goal_dispatch(goal, &current)
            }
        }
        Ok(ManagerDecision::Done(summary)) => {
            goal.active = false;
            goal.output_buffer.clear();
            goal.last_output_at = None;
            goal.retry_after = None;
            goal.review_notice = None;
            goal.dispatch_retry = None;
            goal.failure_count = 0;
            goal.status = if summary.is_empty() {
                "goal complete".into()
            } else {
                format!("complete: {summary}")
            };
            Some("grid manager goal complete".into())
        }
        Err(error) => {
            let prior = goal.review_notice.as_deref().unwrap_or_default();
            let notice = if prior.is_empty() {
                format!("Manager API error: {error}")
            } else {
                format!("{prior}\nManager API error while reviewing that record: {error}")
            };
            let retrying = schedule_goal_retry(goal, Some(notice), format!("API error: {error}"));
            Some(if retrying {
                format!("grid manager error: {error}")
            } else {
                "grid manager stopped after repeated API failures".into()
            })
        }
    }
}

fn trim_goal_buffer(buffer: &mut String) {
    if buffer.len() <= PANE_GOAL_OUTPUT_MAX_BYTES {
        return;
    }
    let mut keep_from = buffer.len().saturating_sub(PANE_GOAL_OUTPUT_MAX_BYTES);
    while keep_from < buffer.len() && !buffer.is_char_boundary(keep_from) {
        keep_from += 1;
    }
    buffer.drain(..keep_from);
}
fn paste_and_enter_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 16);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~\r");
    bytes
}

fn toggle_selection(selected: &mut BTreeSet<usize>, index: usize) -> bool {
    if selected.remove(&index) {
        false
    } else {
        selected.insert(index);
        true
    }
}

fn spawn_hidden_command(
    command: String,
    cwd: PathBuf,
    event_tx: mpsc::UnboundedSender<CommandRunEvent>,
) {
    thread::spawn(move || {
        let event = match run_shell_command(&command, &cwd) {
            Ok(output) => CommandRunEvent {
                command,
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code(),
                error: None,
            },
            Err(error) => CommandRunEvent {
                command,
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: Some(format!("{error:#}")),
            },
        };
        let _ = event_tx.send(event);
    });
}

fn run_shell_command(command: &str, cwd: &Path) -> io::Result<std::process::Output> {
    let mut shell = if cfg!(windows) {
        let mut shell =
            Command::new(env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")));
        shell.arg("/C").arg(command);
        shell
    } else {
        let mut shell = Command::new(env::var_os("SHELL").unwrap_or_else(|| OsString::from("sh")));
        shell.arg("-c").arg(command);
        shell
    };

    shell.current_dir(cwd).output()
}

fn parse_cd_target(command: &str) -> Option<Option<String>> {
    let trimmed = command.trim();
    let lower = trimmed.to_ascii_lowercase();

    if matches!(lower.as_str(), "cd" | "chdir") {
        return Some(None);
    }
    if lower == "cd.." {
        return Some(Some("..".into()));
    }
    if lower.starts_with("cd ") {
        return Some(Some(normalize_cd_target(&trimmed[2..])));
    }
    if lower.starts_with("chdir ") {
        return Some(Some(normalize_cd_target(&trimmed[5..])));
    }

    None
}

fn normalize_cd_target(raw: &str) -> String {
    let mut value = raw.trim();
    if value
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("/d "))
    {
        value = value[3..].trim();
    }
    trim_matching_quotes(value).to_string()
}

fn trim_matching_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if matches!(
            (bytes[0], bytes[value.len() - 1]),
            (b'"', b'"') | (b'\'', b'\'')
        ) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn resolve_cd_target(current: &Path, target: Option<&str>) -> Result<Option<PathBuf>> {
    let Some(target) = target.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let path = if target == "~" {
        home_dir().ok_or_else(|| anyhow!("home directory is not available"))?
    } else if let Some(rest) = target
        .strip_prefix("~/")
        .or_else(|| target.strip_prefix("~\\"))
    {
        home_dir()
            .ok_or_else(|| anyhow!("home directory is not available"))?
            .join(rest)
    } else {
        let path = PathBuf::from(target);
        if path.is_absolute() {
            path
        } else {
            current.join(path)
        }
    };

    let canonical = path
        .canonicalize()
        .with_context(|| format!("directory not found: {}", path.display()))?;
    if !canonical.is_dir() {
        return Err(anyhow!("not a directory: {}", canonical.display()));
    }
    Ok(Some(canonical))
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let drive = env::var_os("HOMEDRIVE")?;
            let path = env::var_os("HOMEPATH")?;
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home)
        })
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
}

fn previous_char_boundary(value: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    value[..cursor]
        .char_indices()
        .last()
        .map(|(index, _)| index)
}

fn next_char_boundary(value: &str, cursor: usize) -> usize {
    value[cursor..]
        .char_indices()
        .nth(1)
        .map(|(offset, _)| cursor + offset)
        .unwrap_or(value.len())
}

fn selected_swap_pair(selected: &BTreeSet<usize>) -> SwapSelection {
    match selected.len() {
        0 | 1 => SwapSelection::NeedsMore,
        2 => {
            let mut selected = selected.iter().copied();
            let first = selected.next().expect("pair has a first index");
            let second = selected.next().expect("pair has a second index");
            SwapSelection::Pair(first, second)
        }
        _ => SwapSelection::TooMany,
    }
}

fn swap_set_indices(indices: &mut BTreeSet<usize>, first: usize, second: usize) {
    let had_first = indices.remove(&first);
    let had_second = indices.remove(&second);

    if had_first {
        indices.insert(second);
    }
    if had_second {
        indices.insert(first);
    }
}

fn swapped_index(index: usize, first: usize, second: usize) -> usize {
    if index == first {
        second
    } else if index == second {
        first
    } else {
        index
    }
}

fn wrapped_row_focus_target(
    focus: usize,
    pane_count: usize,
    columns: usize,
    column_delta: isize,
    sleeping: &BTreeSet<usize>,
) -> Option<usize> {
    if pane_count == 0 || columns == 0 {
        return None;
    }

    let focus = focus.min(pane_count - 1);
    let row_start = (focus / columns) * columns;
    let row_end = row_start.saturating_add(columns).min(pane_count);
    let row_len = row_end.saturating_sub(row_start);
    if row_len == 0 {
        return None;
    }

    let position = focus - row_start;
    let moving_right = !column_delta.is_negative();
    for offset in 1..=row_len {
        let next_position = if moving_right {
            (position + offset) % row_len
        } else {
            (position + row_len - offset) % row_len
        };
        let candidate = row_start + next_position;
        if !sleeping.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn wrapped_column_focus_target(
    focus: usize,
    pane_count: usize,
    columns: usize,
    row_delta: isize,
    sleeping: &BTreeSet<usize>,
) -> Option<usize> {
    if pane_count == 0 || columns == 0 {
        return None;
    }

    let focus = focus.min(pane_count - 1);
    let column = focus % columns;
    let column_indices = (column..pane_count).step_by(columns).collect::<Vec<_>>();
    let position = column_indices.iter().position(|index| *index == focus)?;
    let column_len = column_indices.len();
    let moving_down = !row_delta.is_negative();

    for offset in 1..=column_len {
        let next_position = if moving_down {
            (position + offset) % column_len
        } else {
            (position + column_len - offset) % column_len
        };
        let candidate = column_indices[next_position];
        if !sleeping.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn input_targets_for(focus: usize, selected: &BTreeSet<usize>, pane_count: usize) -> Vec<usize> {
    if pane_count == 0 {
        return Vec::new();
    }

    if selected.len() > 1 {
        selected.iter().copied().collect()
    } else {
        vec![focus.min(pane_count - 1)]
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RestartTargets {
    indices: Vec<usize>,
    running: usize,
}

fn restart_targets_for(targets: &[usize], exited: &[bool]) -> RestartTargets {
    let mut indices = Vec::new();
    let mut running = 0;

    for index in targets {
        match exited.get(*index) {
            Some(true) => indices.push(*index),
            Some(false) => running += 1,
            None => {}
        }
    }

    RestartTargets { indices, running }
}

fn exited_recovery_action_for(key: KeyEvent) -> Option<ExitedRecoveryAction> {
    if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }

    match key.code {
        KeyCode::Enter => Some(ExitedRecoveryAction::Restart),
        KeyCode::Esc => Some(ExitedRecoveryAction::HoldAltPrefix),
        KeyCode::Char(ch) => match ch.to_ascii_lowercase() {
            'r' | 't' => Some(ExitedRecoveryAction::Restart),
            's' | 'z' => Some(ExitedRecoveryAction::Sleep),
            _ => None,
        },
        _ => None,
    }
}

fn normalized_pane_name(value: &str) -> Option<String> {
    let name = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_PANE_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_string();

    (!name.is_empty()).then_some(name)
}

fn normalized_tab_title(value: &str) -> Option<String> {
    let title = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_TAB_TITLE_CHARS)
        .collect::<String>();

    (!title.is_empty()).then_some(title)
}

fn char_to_byte_index(value: &str, cursor: usize) -> usize {
    value
        .char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

fn render_if_selection_cleared(outcome: KeyOutcome, selection_cleared: bool) -> KeyOutcome {
    match outcome {
        KeyOutcome::Continue if selection_cleared => KeyOutcome::Render,
        _ => outcome,
    }
}

fn pane_settings_rename_requested(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N'))
        || (key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R')))
}

fn pane_inner_rect(rect: Rect) -> Option<Rect> {
    let width = rect.width.checked_sub(2)?;
    let height = rect.height.checked_sub(2)?;
    if width == 0 || height == 0 {
        return None;
    }

    Some(Rect {
        x: rect.x.saturating_add(1),
        y: rect.y.saturating_add(1),
        width,
        height,
    })
}

fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn path_label(path: &std::path::Path) -> String {
    let mut label = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string());

    if !matches!(label.chars().last(), Some('/') | Some('\\')) {
        label.push('/');
    }

    label
}

fn extract_selection_text(screen: &vt100::Screen, selection: PaneSelection, width: u16) -> String {
    if width == 0 {
        return String::new();
    }

    let last_column = width.saturating_sub(1);
    let mut lines = Vec::new();
    for row in selection.start_row..=selection.end_row {
        let start_column = if row == selection.start_row {
            selection.start_column.min(last_column)
        } else {
            0
        };
        let end_column = if row == selection.end_row {
            selection.end_column.min(last_column)
        } else {
            last_column
        };

        let mut line = String::new();
        for column in start_column..=end_column {
            let Some(cell) = screen.cell(row, column) else {
                line.push(' ');
                continue;
            };

            if cell.is_wide_continuation() {
                continue;
            }

            if cell.has_contents() {
                line.push_str(cell.contents());
            } else {
                line.push(' ');
            }
        }
        lines.push(line.trim_end().to_string());
    }

    lines.join("\n").trim_end().to_string()
}

fn copy_to_clipboard(terminal: &mut Tui, text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    if copy_with_pbcopy(text) {
        return Ok(());
    }

    write!(
        terminal.backend_mut(),
        "\x1b]52;c;{}\x07",
        base64_encode(text.as_bytes())
    )
    .context("failed to send terminal clipboard sequence")?;
    terminal
        .backend_mut()
        .flush()
        .context("failed to flush terminal clipboard sequence")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn copy_with_pbcopy(text: &str) -> bool {
    let Ok(mut child) = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    wrote && child.wait().is_ok_and(|status| status.success())
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        let value = ((first as u32) << 16) | ((second as u32) << 8) | third as u32;

        output.push(TABLE[((value >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[((value >> 6) & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(value & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }

    output
}

fn grid_resize_slots(current: GridSize, next: GridSize, pane_count: usize) -> Vec<Option<usize>> {
    let mut slots = Vec::with_capacity(next.count());
    for row in 0..next.rows {
        for column in 0..next.columns {
            let old_index = (row < current.rows && column < current.columns)
                .then_some(row * current.columns + column)
                .filter(|index| *index < pane_count);
            slots.push(old_index);
        }
    }
    slots
}

fn remap_index_set(
    indices: &BTreeSet<usize>,
    old_to_new: &BTreeMap<usize, usize>,
) -> BTreeSet<usize> {
    indices
        .iter()
        .filter_map(|old| old_to_new.get(old).copied())
        .collect()
}

fn resized_focus_index(
    old_focus: usize,
    current: GridSize,
    next: GridSize,
    old_to_new: &BTreeMap<usize, usize>,
) -> usize {
    if let Some(new_focus) = old_to_new.get(&old_focus) {
        return *new_focus;
    }

    let old_row = old_focus / current.columns;
    let old_column = old_focus % current.columns;
    let row = old_row.min(next.rows.saturating_sub(1));
    let column = old_column.min(next.columns.saturating_sub(1));
    row * next.columns + column
}

fn awake_input_targets_for(
    focus: usize,
    selected: &BTreeSet<usize>,
    pane_count: usize,
    sleeping: &BTreeSet<usize>,
) -> Vec<usize> {
    input_targets_for(focus, selected, pane_count)
        .into_iter()
        .filter(|index| !sleeping.contains(index))
        .collect()
}

fn pane_word(count: usize) -> &'static str {
    if count == 1 { "pane" } else { "panes" }
}

fn valid_auth_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')
}

fn pane_number_list(indices: &[usize]) -> String {
    indices
        .iter()
        .map(|index| (index + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_voice_shortcut(ch: char, modifiers: KeyModifiers) -> bool {
    ch.eq_ignore_ascii_case(&'v')
        && modifiers.contains(KeyModifiers::ALT)
        && modifiers.contains(KeyModifiers::SHIFT)
        && !modifiers.contains(KeyModifiers::CONTROL)
}

fn is_quit_shortcut(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
}

fn is_help_shortcut(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::F(1))
        || (key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('h') | KeyCode::Char('H')))
}

fn control_byte(ch: char) -> Option<u8> {
    let lower = ch.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        Some((lower as u8) - b'a' + 1)
    } else {
        match ch {
            '[' => Some(0x1b),
            '\\' => Some(0x1c),
            ']' => Some(0x1d),
            '^' => Some(0x1e),
            '_' => Some(0x1f),
            _ => None,
        }
    }
}

fn terminal_key_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }

    match key.code {
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::F(number) => bytes.extend_from_slice(function_key_sequence(number)?),
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            bytes.push(control_byte(ch)?);
        }
        KeyCode::Char(ch) => {
            let mut buffer = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
        }
        _ => return None,
    }

    Some(bytes)
}

fn function_key_sequence(number: u8) -> Option<&'static [u8]> {
    match number {
        1 => Some(b"\x1bOP"),
        2 => Some(b"\x1bOQ"),
        3 => Some(b"\x1bOR"),
        4 => Some(b"\x1bOS"),
        5 => Some(b"\x1b[15~"),
        6 => Some(b"\x1b[17~"),
        7 => Some(b"\x1b[18~"),
        8 => Some(b"\x1b[19~"),
        9 => Some(b"\x1b[20~"),
        10 => Some(b"\x1b[21~"),
        11 => Some(b"\x1b[23~"),
        12 => Some(b"\x1b[24~"),
        _ => None,
    }
}

fn is_mouse_scroll(kind: MouseEventKind) -> bool {
    matches!(
        kind,
        MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
            | MouseEventKind::ScrollUp
    )
}

fn pane_scroll_rows(kind: MouseEventKind) -> Option<isize> {
    match kind {
        MouseEventKind::ScrollUp => Some(PANE_SCROLL_ROWS),
        MouseEventKind::ScrollDown => Some(-PANE_SCROLL_ROWS),
        _ => None,
    }
}

fn mouse_scroll_bytes(mouse: MouseEvent, point: CellPoint, screen: &Screen) -> Option<Vec<u8>> {
    if screen.mouse_protocol_mode() == MouseProtocolMode::None {
        return None;
    }

    let button = mouse_scroll_button(mouse.kind)?;
    let button = button.saturating_add(mouse_modifier_bits(mouse.modifiers));
    let column = point.column.saturating_add(1);
    let row = point.row.saturating_add(1);

    match screen.mouse_protocol_encoding() {
        MouseProtocolEncoding::Sgr => Some(format!("\x1b[<{button};{column};{row}M").into_bytes()),
        MouseProtocolEncoding::Default => default_mouse_bytes(button, column, row),
        MouseProtocolEncoding::Utf8 => utf8_mouse_bytes(button, column, row),
    }
}

fn pane_mouse_scroll_bytes(
    mouse: MouseEvent,
    point: CellPoint,
    screen: &Screen,
    selected: bool,
) -> Option<Vec<u8>> {
    if selected {
        None
    } else {
        mouse_scroll_bytes(mouse, point, screen)
    }
}

fn mouse_scroll_button(kind: MouseEventKind) -> Option<u8> {
    match kind {
        MouseEventKind::ScrollUp => Some(64),
        MouseEventKind::ScrollDown => Some(65),
        MouseEventKind::ScrollLeft => Some(66),
        MouseEventKind::ScrollRight => Some(67),
        _ => None,
    }
}

fn mouse_modifier_bits(modifiers: KeyModifiers) -> u8 {
    let mut bits = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits += 16;
    }
    bits
}

fn default_mouse_bytes(button: u8, column: u16, row: u16) -> Option<Vec<u8>> {
    let values = [
        u16::from(button).saturating_add(32),
        column.saturating_add(32),
        row.saturating_add(32),
    ];
    if values.iter().any(|value| *value > u8::MAX as u16) {
        return None;
    }

    let mut bytes = b"\x1b[M".to_vec();
    bytes.extend(values.map(|value| value as u8));
    Some(bytes)
}

fn utf8_mouse_bytes(button: u8, column: u16, row: u16) -> Option<Vec<u8>> {
    let mut bytes = b"\x1b[M".to_vec();
    for value in [
        u16::from(button).saturating_add(32),
        column.saturating_add(32),
        row.saturating_add(32),
    ] {
        let ch = char::from_u32(u32::from(value))?;
        let mut buffer = [0; 4];
        bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use clap::Parser as ClapParser;

    use super::*;
    use crate::profiles::Profile;
    use vt100::Parser;

    struct TempHome {
        path: PathBuf,
    }

    static NEXT_TEMP_HOME_ID: AtomicU64 = AtomicU64::new(0);

    impl TempHome {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let sequence = NEXT_TEMP_HOME_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "gridbash-app-auth-test-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp home");
            Self { path }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn selected(indices: &[usize]) -> BTreeSet<usize> {
        indices.iter().copied().collect()
    }

    #[test]
    fn invoking_profile_precedes_saved_fallback() {
        let cli = Cli::parse_from(["gridbash"]);
        let mut config = Config::default();
        config.set_default_profile("git-bash");

        assert_eq!(
            resolve_profile_name_from(&cli, &config, None, Some("powershell".into())),
            "powershell"
        );
    }

    #[test]
    fn explicit_profiles_precede_invoking_profile() {
        let cli = Cli::parse_from(["gridbash", "--profile", "cmd"]);
        let config = Config::default();
        assert_eq!(
            resolve_profile_name_from(
                &cli,
                &config,
                Some("codex".into()),
                Some("powershell".into()),
            ),
            "cmd"
        );

        let cli = Cli::parse_from(["gridbash"]);
        assert_eq!(
            resolve_profile_name_from(
                &cli,
                &config,
                Some("codex".into()),
                Some("powershell".into()),
            ),
            "codex"
        );
    }

    fn mouse_event(kind: MouseEventKind, modifiers: KeyModifiers) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers,
        }
    }

    #[test]
    fn shrinking_columns_deactivates_the_rightmost_column() {
        let slots = grid_resize_slots(
            GridSize {
                rows: 3,
                columns: 3,
            },
            GridSize {
                rows: 3,
                columns: 2,
            },
            9,
        );

        assert_eq!(
            slots,
            vec![Some(0), Some(1), Some(3), Some(4), Some(6), Some(7)]
        );
    }

    #[test]
    fn expanding_columns_preserves_rows_and_inserts_new_cells() {
        let slots = grid_resize_slots(
            GridSize {
                rows: 3,
                columns: 2,
            },
            GridSize {
                rows: 3,
                columns: 3,
            },
            6,
        );

        assert_eq!(
            slots,
            vec![
                Some(0),
                Some(1),
                None,
                Some(2),
                Some(3),
                None,
                Some(4),
                Some(5),
                None,
            ]
        );
    }

    #[test]
    fn equal_count_reshape_removes_and_adds_by_coordinate() {
        let slots = grid_resize_slots(
            GridSize {
                rows: 2,
                columns: 3,
            },
            GridSize {
                rows: 3,
                columns: 2,
            },
            6,
        );

        assert_eq!(slots, vec![Some(0), Some(1), Some(3), Some(4), None, None]);
    }

    #[test]
    fn focus_moves_to_nearest_retained_cell_when_its_column_is_removed() {
        let current = GridSize {
            rows: 3,
            columns: 3,
        };
        let next = GridSize {
            rows: 3,
            columns: 2,
        };
        let old_to_new = grid_resize_slots(current, next, 9)
            .into_iter()
            .enumerate()
            .filter_map(|(new, old)| old.map(|old| (old, new)))
            .collect();

        assert_eq!(resized_focus_index(5, current, next, &old_to_new), 3);
    }

    #[test]
    fn mouse_scroll_bytes_skip_plain_shells() {
        let parser = Parser::new(24, 80, 100);

        assert_eq!(
            mouse_scroll_bytes(
                mouse_event(MouseEventKind::ScrollUp, KeyModifiers::NONE),
                CellPoint { row: 0, column: 0 },
                parser.screen()
            ),
            None
        );
    }

    #[test]
    fn plain_pane_scroll_uses_vertical_wheel_steps() {
        assert_eq!(
            pane_scroll_rows(MouseEventKind::ScrollUp),
            Some(PANE_SCROLL_ROWS)
        );
        assert_eq!(
            pane_scroll_rows(MouseEventKind::ScrollDown),
            Some(-PANE_SCROLL_ROWS)
        );
        assert_eq!(pane_scroll_rows(MouseEventKind::ScrollLeft), None);
    }

    #[test]
    fn mouse_scroll_bytes_use_sgr_mouse_encoding() {
        let mut parser = Parser::new(24, 80, 100);
        parser.process(b"\x1b[?1000h\x1b[?1006h");

        assert_eq!(
            mouse_scroll_bytes(
                mouse_event(
                    MouseEventKind::ScrollDown,
                    KeyModifiers::SHIFT | KeyModifiers::CONTROL
                ),
                CellPoint { row: 2, column: 3 },
                parser.screen()
            ),
            Some(b"\x1b[<85;4;3M".to_vec())
        );
    }

    #[test]
    fn mouse_scroll_bytes_use_default_mouse_encoding() {
        let mut parser = Parser::new(24, 80, 100);
        parser.process(b"\x1b[?1000h");

        assert_eq!(
            mouse_scroll_bytes(
                mouse_event(MouseEventKind::ScrollUp, KeyModifiers::NONE),
                CellPoint { row: 0, column: 0 },
                parser.screen()
            ),
            Some(vec![0x1b, b'[', b'M', 96, 33, 33])
        );
    }

    #[test]
    fn selected_pane_scroll_stays_in_gridbash_scrollback() {
        let mut parser = Parser::new(24, 80, 100);
        parser.process(b"\x1b[?1000h\x1b[?1006h");
        let wheel = mouse_event(MouseEventKind::ScrollUp, KeyModifiers::NONE);
        let point = CellPoint { row: 2, column: 3 };

        assert!(pane_mouse_scroll_bytes(wheel, point, parser.screen(), false).is_some());
        assert_eq!(
            pane_mouse_scroll_bytes(wheel, point, parser.screen(), true),
            None
        );
    }

    #[test]
    fn selected_swap_pair_requires_exactly_two_panes() {
        assert_eq!(selected_swap_pair(&selected(&[])), SwapSelection::NeedsMore);
        assert_eq!(
            selected_swap_pair(&selected(&[1])),
            SwapSelection::NeedsMore
        );
        assert_eq!(
            selected_swap_pair(&selected(&[0, 1, 2])),
            SwapSelection::TooMany
        );
    }

    #[test]
    fn selected_swap_pair_returns_selected_pair_in_order() {
        assert_eq!(
            selected_swap_pair(&selected(&[3, 1])),
            SwapSelection::Pair(1, 3)
        );
    }

    #[test]
    fn swap_set_indices_moves_membership_between_swapped_panes() {
        let mut indices = selected(&[0, 4]);
        swap_set_indices(&mut indices, 0, 2);
        assert_eq!(indices, selected(&[2, 4]));

        swap_set_indices(&mut indices, 0, 2);
        assert_eq!(indices, selected(&[0, 4]));
    }

    #[test]
    fn swapped_index_follows_the_swapped_pair() {
        assert_eq!(swapped_index(0, 0, 2), 2);
        assert_eq!(swapped_index(2, 0, 2), 0);
        assert_eq!(swapped_index(4, 0, 2), 4);
    }

    #[test]
    fn horizontal_focus_wraps_within_the_current_row() {
        let sleeping = selected(&[]);

        assert_eq!(wrapped_row_focus_target(2, 6, 3, 1, &sleeping), Some(0));
        assert_eq!(wrapped_row_focus_target(3, 6, 3, -1, &sleeping), Some(5));
        assert_eq!(wrapped_row_focus_target(1, 6, 3, 1, &sleeping), Some(2));
        assert_eq!(wrapped_row_focus_target(1, 6, 3, -1, &sleeping), Some(0));
    }

    #[test]
    fn vertical_focus_wraps_within_the_current_column() {
        let sleeping = selected(&[]);

        assert_eq!(wrapped_column_focus_target(3, 6, 3, 1, &sleeping), Some(0));
        assert_eq!(wrapped_column_focus_target(0, 6, 3, -1, &sleeping), Some(3));
        assert_eq!(wrapped_column_focus_target(1, 6, 3, 1, &sleeping), Some(4));
        assert_eq!(wrapped_column_focus_target(4, 6, 3, -1, &sleeping), Some(1));
    }

    #[test]
    fn wrapped_focus_skips_sleeping_panes() {
        assert_eq!(
            wrapped_row_focus_target(0, 4, 4, 1, &selected(&[1, 2])),
            Some(3)
        );
        assert_eq!(
            wrapped_column_focus_target(0, 6, 2, 1, &selected(&[2])),
            Some(4)
        );
        assert_eq!(
            wrapped_row_focus_target(0, 3, 3, 1, &selected(&[0, 1, 2])),
            None
        );
    }

    #[test]
    fn toggle_selection_deselects_one_pane_without_clearing_others() {
        let mut panes = selected(&[0, 1, 2]);

        assert!(!toggle_selection(&mut panes, 1));
        assert_eq!(panes, selected(&[0, 2]));
    }

    #[test]
    fn toggle_selection_adds_unselected_pane() {
        let mut panes = selected(&[0, 2]);

        assert!(toggle_selection(&mut panes, 1));
        assert_eq!(panes, selected(&[0, 1, 2]));
    }

    #[test]
    fn input_targets_focused_pane_without_multiple_selected_panes() {
        assert_eq!(input_targets_for(2, &selected(&[]), 4), vec![2]);
        assert_eq!(input_targets_for(2, &selected(&[0]), 4), vec![2]);
    }

    #[test]
    fn input_targets_selected_panes_when_multiple_panes_are_selected() {
        assert_eq!(input_targets_for(2, &selected(&[0, 3]), 4), vec![0, 3]);
    }

    #[test]
    fn input_targets_clamps_focus_to_available_panes() {
        assert_eq!(input_targets_for(8, &selected(&[]), 4), vec![3]);
        assert!(input_targets_for(0, &selected(&[]), 0).is_empty());
    }

    #[test]
    fn restart_targets_include_only_exited_panes() {
        assert_eq!(
            restart_targets_for(&[0, 1, 2], &[true, false, true]),
            RestartTargets {
                indices: vec![0, 2],
                running: 1,
            }
        );
    }

    #[test]
    fn restart_targets_ignore_invalid_indices() {
        assert_eq!(
            restart_targets_for(&[1, 4], &[false, true]),
            RestartTargets {
                indices: vec![1],
                running: 0,
            }
        );
    }

    #[test]
    fn exited_recovery_keys_map_to_dialog_actions() {
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(ExitedRecoveryAction::Restart)
        );
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            Some(ExitedRecoveryAction::Restart)
        );
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
            Some(ExitedRecoveryAction::Restart)
        );
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
            Some(ExitedRecoveryAction::Sleep)
        );
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
            Some(ExitedRecoveryAction::Sleep)
        );
    }

    #[test]
    fn exited_recovery_keeps_escape_for_alt_prefixes() {
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(ExitedRecoveryAction::HoldAltPrefix)
        );
        assert_eq!(
            exited_recovery_action_for(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT)),
            None
        );
    }

    #[test]
    fn normalized_pane_name_trims_and_clears_empty_names() {
        assert_eq!(
            normalized_pane_name("  api server  "),
            Some("api server".into())
        );
        assert_eq!(normalized_pane_name("   "), None);
    }

    #[test]
    fn rename_state_edits_at_the_cursor() {
        let mut rename = RenamePaneState::default();
        rename.begin(0, Some("abc"));
        rename.move_cursor(-1);
        rename.insert_char('X');
        assert_eq!(rename.value, "abXc");

        rename.backspace();
        assert_eq!(rename.value, "abc");

        rename.delete();
        assert_eq!(rename.value, "ab");
    }

    #[test]
    fn pane_settings_rename_keys_do_not_replace_plain_reload_key() {
        assert!(pane_settings_rename_requested(&KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert!(pane_settings_rename_requested(&KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::ALT,
        )));
        assert!(!pane_settings_rename_requested(&KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn sleeping_panes_do_not_receive_input() {
        assert_eq!(
            awake_input_targets_for(2, &selected(&[]), 4, &selected(&[2])),
            Vec::<usize>::new()
        );
        assert_eq!(
            awake_input_targets_for(2, &selected(&[0, 3]), 4, &selected(&[0])),
            vec![3]
        );
    }

    #[test]
    fn applies_auth_defaults_to_agent_launch_specs() {
        let temp = TempHome::new();
        let codex_dir = temp.path.join("codex-2");
        fs::create_dir(&codex_dir).expect("codex dir");
        fs::write(codex_dir.join(".profile-kind"), "codex").expect("kind");
        let mut config = Config::default();
        config.auth.home = Some(temp.path.clone());
        config.auth.defaults.set(AgentKind::Codex, "codex-2");
        let cwd = env::current_dir().expect("cwd");
        let mut plan = LaunchPlan::legacy(
            "codex".into(),
            Profile {
                command: "codex".into(),
                args: vec![],
                title: Some("codex".into()),
                agent_kind: Some(AgentKind::Codex),
            },
            cwd,
            1,
            GridSize {
                rows: 1,
                columns: 1,
            },
        );

        apply_auth_defaults(&mut plan, &config).expect("apply auth");

        assert_eq!(
            plan.panes[0].env.get("CODEX_HOME"),
            Some(&codex_dir.display().to_string())
        );
        assert_eq!(plan.panes[0].auth_name.as_deref(), Some("codex-2"));
        assert_eq!(plan.panes[0].auth_kind, Some(AgentKind::Codex));
        assert_eq!(plan.panes[0].auth_dir.as_deref(), Some(codex_dir.as_path()));
    }

    #[test]
    fn auto_cycles_ready_auth_profiles_across_matching_panes() {
        let temp = TempHome::new();
        for name in ["codex-1", "codex-2"] {
            let dir = temp.path.join(name);
            fs::create_dir(&dir).expect("codex dir");
            fs::write(dir.join(".profile-kind"), "codex").expect("kind");
            fs::write(dir.join("auth.json"), r#"{"tokens":{}}"#).expect("auth");
        }
        let mut config = Config::default();
        config.auth.home = Some(temp.path.clone());
        config.auth.auto_cycle = true;
        let mut plan = LaunchPlan::legacy(
            "codex".into(),
            Profile {
                command: "codex".into(),
                args: vec![],
                title: Some("codex".into()),
                agent_kind: Some(AgentKind::Codex),
            },
            env::current_dir().expect("cwd"),
            3,
            GridSize {
                rows: 1,
                columns: 3,
            },
        );

        apply_auth_defaults(&mut plan, &config).expect("apply auth");

        assert_eq!(plan.panes[0].auth_name.as_deref(), Some("codex-1"));
        assert_eq!(plan.panes[1].auth_name.as_deref(), Some("codex-2"));
        assert_eq!(plan.panes[2].auth_name.as_deref(), Some("codex-1"));
    }

    #[test]
    fn command_line_edits_at_cursor() {
        let mut command = CommandLineState::new(PathBuf::from("C:\\repo"));
        command.insert_text("abc");
        assert!(command.move_left());
        command.insert_char('X');

        assert_eq!(command.input, "abXc");
        assert_eq!(command.cursor_chars(), 3);
        assert!(command.backspace());
        assert_eq!(command.input, "abc");
    }

    #[test]
    fn command_line_focus_controls_output_visibility() {
        let mut command = CommandLineState::new(PathBuf::from("C:\\repo"));
        assert!(!command.focused);
        assert!(!command.output_expanded());

        command.toggle_focus();
        assert!(command.focused);
        assert!(command.output_expanded());

        command.toggle_focus();
        assert!(!command.focused);
        assert!(!command.output_expanded());
    }

    #[test]
    fn voice_shortcut_preserves_plain_alt_v_for_agent_image_paste() {
        let image_paste = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT);
        assert!(!is_voice_shortcut('v', KeyModifiers::ALT));
        assert_eq!(terminal_key_bytes(image_paste), Some(b"\x1bv".to_vec()));

        let voice_modifiers = KeyModifiers::ALT | KeyModifiers::SHIFT;
        assert!(is_voice_shortcut('V', voice_modifiers));
    }

    #[test]
    fn help_shortcuts_are_modeless_and_plain_h_passes_through() {
        assert!(is_help_shortcut(&KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::ALT,
        )));
        assert!(is_help_shortcut(&KeyEvent::new(
            KeyCode::F(1),
            KeyModifiers::NONE,
        )));
        assert!(!is_help_shortcut(&KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn parses_cd_commands_without_treating_other_commands_as_cd() {
        assert_eq!(parse_cd_target("cd"), Some(None));
        assert_eq!(parse_cd_target("cd.."), Some(Some("..".into())));
        assert_eq!(
            parse_cd_target("cd /d \"C:\\Users\\Jason\""),
            Some(Some("C:\\Users\\Jason".into()))
        );
        assert_eq!(parse_cd_target("cargo test"), None);
    }

    #[test]
    fn palette_color_cycles_in_both_directions() {
        assert_eq!(PaletteColor::Cyan.adjust(1), PaletteColor::Sky);
        assert_eq!(PaletteColor::Cyan.adjust(-1), PaletteColor::White);
    }

    #[test]
    fn settings_rows_include_live_grid_palette_roles() {
        let settings = SettingsState::default();
        let rows = settings.rows();
        let palette_start = rows
            .iter()
            .position(|row| row.label == "Accent color")
            .expect("accent palette row");

        assert_eq!(rows.len(), settings.row_targets().len());
        assert_eq!(rows[1].label, "Activity badges");
        assert_eq!(rows[1].value, "on");
        assert_eq!(rows[palette_start].label, "Accent color");
        assert_eq!(rows[palette_start + 3].label, "Quiet border");
        assert_eq!(rows[palette_start + 3].value, "dark gray");
    }

    #[test]
    fn pane_settings_tracks_active_pane_history() {
        let mut settings = PaneSettingsState::default();
        settings.open(2, "Assistant: ready".into(), 1);
        assert!(settings.open);
        assert_eq!(settings.pane_index, 2);
        assert_eq!(
            settings.history_summary.as_deref(),
            Some("Assistant: ready")
        );
        assert_eq!(settings.auth_cursor, 1);

        settings.refresh_history("User: rerun tests".into());
        assert_eq!(
            settings.history_summary.as_deref(),
            Some("User: rerun tests")
        );

        settings.close();
        assert!(!settings.open);
        assert!(settings.history_summary.is_none());
    }

    #[test]
    fn pane_overlay_shortcuts_keep_summary_and_previous_panes_distinct() {
        assert_eq!(
            pane_overlay_shortcut(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT)),
            Some(PaneOverlayShortcut::Summary)
        );
        assert_eq!(
            pane_overlay_shortcut(&KeyEvent::new(
                KeyCode::Char('P'),
                KeyModifiers::ALT | KeyModifiers::SHIFT
            )),
            Some(PaneOverlayShortcut::Previous)
        );
        assert_eq!(
            pane_overlay_shortcut(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
            None
        );
    }
}

fn conversation_summary(screen: &Screen) -> Option<String> {
    let (_, cols) = screen.size();
    let mut lines = screen.rows(0, cols).collect::<Vec<_>>();
    lines.reverse();

    lines
        .into_iter()
        .filter_map(|line| normalize_conversation_line(&line))
        .next()
}

fn pane_overlay_shortcut(key: &KeyEvent) -> Option<PaneOverlayShortcut> {
    if !key.modifiers.contains(KeyModifiers::ALT)
        || !matches!(key.code, KeyCode::Char('p') | KeyCode::Char('P'))
    {
        return None;
    }

    Some(if key.modifiers.contains(KeyModifiers::SHIFT) {
        PaneOverlayShortcut::Previous
    } else {
        PaneOverlayShortcut::Summary
    })
}

fn pane_activity_summary(pane: &PtyPane) -> Option<String> {
    output_tail_summary(pane.output_tail()).or_else(|| conversation_summary(pane.screen()))
}

fn output_tail_summary(output_tail: &str) -> Option<String> {
    output_tail
        .lines()
        .rev()
        .filter_map(normalize_conversation_line)
        .next()
}

fn pane_header_text(goal: Option<&str>, activity: Option<&str>, max_chars: usize) -> String {
    let text = goal
        .filter(|goal| !goal.trim().is_empty())
        .map(|goal| format!("goal: {}", goal.trim()))
        .unwrap_or_else(|| activity.unwrap_or("waiting for output").to_string());
    truncate_chars(&text, max_chars)
}

fn normalize_conversation_line(line: &str) -> Option<String> {
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty()
        || !trimmed.chars().any(char::is_alphanumeric)
        || is_low_signal_terminal_line(trimmed)
    {
        return None;
    }

    Some(truncate_chars(trimmed, CONVERSATION_SUMMARY_MAX_CHARS))
}

fn is_low_signal_terminal_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower == "esc"
        || lower == "escape"
        || lower == "last output"
        || lower == "previous commands"
        || lower.starts_with("gridbash resumed pane history")
        || lower.contains("alt+q quit")
        || lower.contains("ctrl+c")
        || lower.contains("press enter")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() && max_chars > 3 {
        format!(
            "{}...",
            truncated.chars().take(max_chars - 3).collect::<String>()
        )
    } else {
        truncated
    }
}

fn setup_terminal(enable_mouse: bool) -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    if enable_mouse {
        execute!(stdout, EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to create terminal")
}

fn teardown_terminal(terminal: &mut Tui, enable_mouse: bool) -> Result<()> {
    disable_raw_mode()?;
    if enable_mouse {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn suspend_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn resume_terminal(terminal: &mut Tui) -> Result<()> {
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    Ok(())
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use vt100::Parser;

    #[test]
    fn pty_drain_budget_bounds_event_and_byte_work() {
        let mut event_budget = PtyDrainBudget::new();
        for _ in 0..PTY_DRAIN_MAX_EVENTS {
            assert!(event_budget.within_size_limits());
            event_budget.record(0);
        }
        assert!(!event_budget.within_size_limits());

        let mut byte_budget = PtyDrainBudget::new();
        while byte_budget.bytes < PTY_DRAIN_MAX_BYTES {
            assert!(byte_budget.within_size_limits());
            byte_budget.record(32 * 1024);
        }
        assert!(!byte_budget.within_size_limits());
    }

    #[test]
    fn large_grids_use_a_thirty_fps_output_floor() {
        assert_eq!(
            adaptive_output_frame_interval(8, 20),
            LARGE_GRID_FRAME_INTERVAL
        );
        assert_eq!(
            adaptive_output_frame_interval(16, 4),
            Duration::from_millis(16)
        );
        assert_eq!(
            adaptive_output_frame_interval(48, 20),
            Duration::from_millis(48)
        );
    }

    #[test]
    fn pane_selection_contains_single_and_multi_line_ranges() {
        let single = PaneSelection {
            start_row: 1,
            start_column: 2,
            end_row: 1,
            end_column: 4,
        };
        assert!(!single.contains(1, 1));
        assert!(single.contains(1, 2));
        assert!(single.contains(1, 4));
        assert!(!single.contains(1, 5));

        let multi = PaneSelection {
            start_row: 1,
            start_column: 3,
            end_row: 3,
            end_column: 2,
        };
        assert!(!multi.contains(1, 2));
        assert!(multi.contains(1, 3));
        assert!(multi.contains(2, 0));
        assert!(multi.contains(3, 2));
        assert!(!multi.contains(3, 3));
    }

    #[test]
    fn mouse_selection_normalizes_anchor_and_cursor() {
        let selection = MouseSelection {
            pane: 0,
            anchor: CellPoint { row: 3, column: 4 },
            cursor: CellPoint { row: 1, column: 2 },
            active: false,
            moved: true,
        };

        assert_eq!(
            selection.range(),
            PaneSelection {
                start_row: 1,
                start_column: 2,
                end_row: 3,
                end_column: 4
            }
        );
    }

    #[test]
    fn summarizes_last_meaningful_visible_line() {
        let mut parser = Parser::new(4, 80, 100);
        parser.process(b"User: add tests\r\n\r\nAssistant: tests are passing\r\n");

        assert_eq!(
            conversation_summary(parser.screen()).as_deref(),
            Some("Assistant: tests are passing")
        );
    }

    #[test]
    fn summarizes_latest_meaningful_output_tail_line() {
        assert_eq!(
            output_tail_summary(
                "older work\nGridBash resumed pane history. Commands were not replayed.\nlatest result\nAlt+q quit\n"
            )
            .as_deref(),
            Some("latest result")
        );
        assert_eq!(output_tail_summary("\nAlt+q quit\n"), None);
    }

    #[test]
    fn pane_header_prefers_a_goal_over_the_activity_summary() {
        assert_eq!(
            pane_header_text(Some("finish the API"), Some("tests are passing"), 80),
            "goal: finish the API"
        );
        assert_eq!(
            pane_header_text(None, Some("tests are passing"), 80),
            "tests are passing"
        );
        assert_eq!(pane_header_text(None, None, 80), "waiting for output");
    }

    #[test]
    fn selected_text_uses_only_the_selection_width() {
        let mut parser = Parser::new(3, 10, 100);
        parser.process(b"hello\r\nworld");

        let text = extract_selection_text(
            parser.screen(),
            PaneSelection {
                start_row: 0,
                start_column: 1,
                end_row: 1,
                end_column: 3,
            },
            10,
        );

        assert_eq!(text, "ello\nworl");
    }

    #[test]
    fn skips_empty_and_control_hint_lines() {
        let mut parser = Parser::new(4, 80, 100);
        parser.process(b"Assistant: ready\r\n\r\nAlt+q quit\r\n");

        assert_eq!(
            conversation_summary(parser.screen()).as_deref(),
            Some("Assistant: ready")
        );
    }

    #[test]
    fn base64_encoder_handles_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"pane text"), "cGFuZSB0ZXh0");
    }

    #[test]
    fn truncates_long_footer_text() {
        assert_eq!(truncate_chars("abcdef", 4), "a...");
        assert_eq!(truncate_chars("abc", 4), "abc");
    }

    #[test]
    fn manager_goal_observes_output_from_any_awake_pane() {
        let mut goal = Some(ManagerGoal {
            id: 1,
            objective: "ship the grid".into(),
            active: true,
            output_buffer: String::new(),
            last_output_at: None,
            in_flight: false,
            retry_after: None,
            review_notice: None,
            dispatch_retry: None,
            next_dispatch_sequence: 0,
            failure_count: 0,
            status: String::new(),
        });

        capture_goal_text(&mut goal, &BTreeSet::new(), 1, "pane two ready\n");

        let goal = goal.as_ref().unwrap();
        assert!(goal.output_buffer.contains("[PANE 2 OUTPUT]"));
        assert!(goal.output_buffer.contains("pane two ready"));
        assert!(goal.last_output_at.is_some());
    }

    #[test]
    fn manager_goal_ignores_sleeping_pane_output() {
        let mut goal = Some(ManagerGoal {
            id: 1,
            objective: "ship the grid".into(),
            active: true,
            output_buffer: String::new(),
            last_output_at: None,
            in_flight: false,
            retry_after: None,
            review_notice: None,
            dispatch_retry: None,
            next_dispatch_sequence: 0,
            failure_count: 0,
            status: String::new(),
        });

        capture_goal_text(&mut goal, &BTreeSet::from([1]), 1, "hidden output\n");

        assert!(goal.as_ref().unwrap().output_buffer.is_empty());
    }

    #[test]
    fn tail_text_keeps_utf8_boundary() {
        assert_eq!(tail_text("one ☃ two", 5).as_deref(), Some(" two"));
    }

    #[test]
    fn manager_goal_context_numbers_multiple_panes_with_roles_and_output() {
        let context = format_manager_goal_context(&[
            GoalPaneContext {
                pane_number: 1,
                state: "available",
                metadata: "role=codex; name=implementer".into(),
                output: "working on the feature".into(),
            },
            GoalPaneContext {
                pane_number: 2,
                state: "sleeping; do not target",
                metadata: "role=claude; name=reviewer".into(),
                output: "(output omitted while unavailable)".into(),
            },
        ]);

        assert!(context.contains("PANE 1 [available; role=codex; name=implementer]"));
        assert!(context.contains("working on the feature"));
        assert!(context.contains("PANE 2 [sleeping; do not target; role=claude"));
        assert!(context.contains("output omitted while unavailable"));
    }

    #[test]
    fn manager_goal_targets_follow_stable_pane_identity_after_reorder() {
        let targets = vec![
            GoalTarget {
                pane_number: 1,
                pane_id: PaneId(10),
                pane_generation: 2,
                screen_revision: 7,
                input_revision: 3,
            },
            GoalTarget {
                pane_number: 2,
                pane_id: PaneId(20),
                pane_generation: 4,
                screen_revision: 9,
                input_revision: 5,
            },
        ];
        let reordered = vec![
            GoalPaneState {
                pane_id: PaneId(20),
                pane_generation: 4,
                screen_revision: 9,
                input_revision: 5,
                unavailable: false,
            },
            GoalPaneState {
                pane_id: PaneId(10),
                pane_generation: 2,
                screen_revision: 7,
                input_revision: 3,
                unavailable: false,
            },
        ];

        assert_eq!(goal_target_index(&targets, 1, &reordered), Ok(1));
        assert_eq!(goal_target_index(&targets, 2, &reordered), Ok(0));
        assert!(!goal_snapshot_is_stale(&targets, &reordered));

        let commands = vec![
            ManagerCommand {
                pane: 1,
                command: "implement the change".into(),
            },
            ManagerCommand {
                pane: 2,
                command: "review the implementation".into(),
            },
        ];
        let plan = goal_command_plan(&commands, &targets, &reordered, &BTreeSet::new()).unwrap();
        assert_eq!(plan.commands[0].pane_index, 1);
        assert_eq!(plan.commands[1].pane_index, 0);
        assert_eq!(plan.commands[0].command.command, "implement the change");
        assert_eq!(
            plan.commands[1].command.command,
            "review the implementation"
        );
    }

    #[test]
    fn manager_goal_rejects_invalid_changed_and_unavailable_targets() {
        let targets = vec![GoalTarget {
            pane_number: 2,
            pane_id: PaneId(20),
            pane_generation: 4,
            screen_revision: 9,
            input_revision: 5,
        }];
        let pane = |generation, screen_revision, unavailable| GoalPaneState {
            pane_id: PaneId(20),
            pane_generation: generation,
            screen_revision,
            input_revision: 5,
            unavailable,
        };

        assert!(
            goal_target_index(&targets, 1, &[pane(4, 9, false)])
                .unwrap_err()
                .contains("not an available target")
        );
        assert!(
            goal_target_index(&targets, 2, &[pane(5, 9, false)])
                .unwrap_err()
                .contains("changed before dispatch")
        );
        assert!(
            goal_target_index(&targets, 2, &[pane(4, 9, true)])
                .unwrap_err()
                .contains("became unavailable")
        );
        assert!(goal_snapshot_is_stale(&targets, &[pane(4, 10, false)]));
        assert!(goal_snapshot_is_stale(&targets, &[pane(4, 9, true)]));
        let mut input_changed = pane(4, 9, false);
        input_changed.input_revision += 1;
        assert!(goal_snapshot_is_stale(&targets, &[input_changed]));
        assert!(
            goal_target_index(&targets, 2, &[input_changed])
                .unwrap_err()
                .contains("changed before dispatch")
        );
    }

    #[test]
    fn manager_goal_partial_retry_uses_current_labels_after_reorder() {
        let sent = GoalCommandKey {
            pane_id: PaneId(10),
            pane_generation: 2,
            command: "implement the change".into(),
        };
        let failed = GoalCommandKey {
            pane_id: PaneId(20),
            pane_generation: 4,
            command: "review the implementation".into(),
        };
        let dispatch = GoalDispatchRetry {
            successful: BTreeSet::from([sent]),
            failed: BTreeMap::from([(failed, "broken pipe".into())]),
            ..Default::default()
        };
        let reordered = vec![
            GoalPaneState {
                pane_id: PaneId(20),
                pane_generation: 4,
                screen_revision: 10,
                input_revision: 5,
                unavailable: false,
            },
            GoalPaneState {
                pane_id: PaneId(10),
                pane_generation: 2,
                screen_revision: 8,
                input_revision: 4,
                unavailable: false,
            },
        ];

        let record = format_goal_dispatch_record(&dispatch, &reordered);
        assert!(record.contains("PANE 2 command \"implement the change\": sent successfully"));
        assert!(record.contains("PANE 1 command \"review the implementation\": failed"));
        assert!(!record.contains("PANE 1 command \"implement the change\""));
    }

    #[test]
    fn manager_goal_retry_skips_successful_command_without_duplicates_after_reorder() {
        let successful = BTreeSet::from([GoalCommandKey {
            pane_id: PaneId(10),
            pane_generation: 2,
            command: "implement the change".into(),
        }]);
        let targets = vec![
            GoalTarget {
                pane_number: 1,
                pane_id: PaneId(20),
                pane_generation: 4,
                screen_revision: 10,
                input_revision: 5,
            },
            GoalTarget {
                pane_number: 2,
                pane_id: PaneId(10),
                pane_generation: 2,
                screen_revision: 8,
                input_revision: 4,
            },
        ];
        let current = targets
            .iter()
            .map(|target| GoalPaneState {
                pane_id: target.pane_id,
                pane_generation: target.pane_generation,
                screen_revision: target.screen_revision,
                input_revision: target.input_revision,
                unavailable: false,
            })
            .collect::<Vec<_>>();
        let commands = vec![
            ManagerCommand {
                pane: 2,
                command: "implement the change".into(),
            },
            ManagerCommand {
                pane: 1,
                command: "review the implementation".into(),
            },
        ];

        let plan = goal_command_plan(&commands, &targets, &current, &successful).unwrap();
        assert_eq!(plan.skipped_successful, 1);
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].pane_index, 0);
        assert_eq!(plan.commands[0].key.pane_id, PaneId(20));
        assert_eq!(
            plan.commands[0].command.command,
            "review the implementation"
        );
    }

    #[test]
    fn manager_goal_async_write_failure_restores_visible_bounded_retry() {
        let token = PtyWriteToken(17);
        let key = GoalCommandKey {
            pane_id: PaneId(20),
            pane_generation: 4,
            command: "review the implementation".into(),
        };
        let mut goal = Some(ManagerGoal {
            id: 1,
            objective: "ship the grid".into(),
            active: true,
            output_buffer: String::new(),
            last_output_at: None,
            in_flight: false,
            retry_after: None,
            review_notice: None,
            dispatch_retry: Some(GoalDispatchRetry {
                pending: BTreeMap::from([(token, key.clone())]),
                summary: "delegated review".into(),
                ..Default::default()
            }),
            next_dispatch_sequence: 1,
            failure_count: 0,
            status: "dispatching".into(),
        });
        let current = [GoalPaneState {
            pane_id: PaneId(20),
            pane_generation: 4,
            screen_revision: 10,
            input_revision: 5,
            unavailable: false,
        }];

        let status = apply_goal_dispatch_result(
            &mut goal,
            &current,
            PaneId(20),
            4,
            token,
            Err("broken pipe".into()),
        )
        .expect("handled tracked manager failure");
        let goal = goal.as_ref().unwrap();
        assert!(status.contains("dispatch issue"));
        assert!(goal.status.contains("PANE 1 write failed: broken pipe"));
        assert!(goal.active);
        assert_eq!(goal.failure_count, 1);
        assert!(goal.retry_after.is_some());
        assert_eq!(
            goal.dispatch_retry
                .as_ref()
                .and_then(|dispatch| dispatch.failed.get(&key))
                .map(String::as_str),
            Some("broken pipe")
        );
    }

    #[test]
    fn manager_goal_waits_for_writer_ack_before_marking_dispatch_successful() {
        let token = PtyWriteToken(23);
        let key = GoalCommandKey {
            pane_id: PaneId(40),
            pane_generation: 8,
            command: "run the targeted tests".into(),
        };
        let mut goal = Some(ManagerGoal {
            id: 2,
            objective: "verify the grid".into(),
            active: true,
            output_buffer: String::new(),
            last_output_at: None,
            in_flight: false,
            retry_after: None,
            review_notice: None,
            dispatch_retry: Some(GoalDispatchRetry {
                pending: BTreeMap::from([(token, key)]),
                summary: "tests delegated".into(),
                ..Default::default()
            }),
            next_dispatch_sequence: 1,
            failure_count: 0,
            status: "dispatching".into(),
        });
        let current = [GoalPaneState {
            pane_id: PaneId(40),
            pane_generation: 8,
            screen_revision: 3,
            input_revision: 2,
            unavailable: false,
        }];

        assert!(finish_goal_dispatch(goal.as_mut().unwrap(), &current).is_none());
        assert_eq!(goal.as_ref().unwrap().status, "dispatching");

        let status = apply_goal_dispatch_result(&mut goal, &current, PaneId(40), 8, token, Ok(()))
            .expect("handled tracked acknowledgement");
        let goal = goal.as_ref().unwrap();
        assert_eq!(status, "grid manager sent 1 pane command(s)");
        assert_eq!(goal.status, "sent 1 command(s): tests delegated");
        assert!(goal.dispatch_retry.is_none());
    }

    #[test]
    fn manager_goal_stale_review_detects_history_free_mouse_input_activity() {
        let targets = [GoalTarget {
            pane_number: 1,
            pane_id: PaneId(30),
            pane_generation: 6,
            screen_revision: 12,
            input_revision: 7,
        }];
        let after_forwarded_mouse_input = [GoalPaneState {
            pane_id: PaneId(30),
            pane_generation: 6,
            screen_revision: 12,
            input_revision: 8,
            unavailable: false,
        }];

        assert!(goal_snapshot_is_stale(
            &targets,
            &after_forwarded_mouse_input
        ));
    }

    #[test]
    fn manager_goal_stops_after_bounded_retries() {
        let mut goal = ManagerGoal {
            id: 1,
            objective: "ship the grid".into(),
            active: true,
            output_buffer: String::new(),
            last_output_at: None,
            in_flight: false,
            retry_after: None,
            review_notice: None,
            dispatch_retry: None,
            next_dispatch_sequence: 0,
            failure_count: 0,
            status: String::new(),
        };

        for attempt in 1..PANE_GOAL_MAX_FAILURES {
            assert!(schedule_goal_retry(
                &mut goal,
                Some(format!("attempt {attempt}")),
                "temporary failure".into()
            ));
            assert!(goal.active);
        }
        assert!(!schedule_goal_retry(
            &mut goal,
            Some("final attempt".into()),
            "permanent failure".into()
        ));
        assert!(!goal.active);
        assert!(goal.retry_after.is_none());
        assert!(goal.status.contains("stopped after repeated failures"));
    }

    #[test]
    fn manager_api_key_is_masked_in_settings_rows() {
        let settings = SettingsState::default();
        let config = crate::config::ManagerConfig {
            api_key: "top-secret-key".into(),
            ..Default::default()
        };

        let rows = settings.manager_rows(&config);
        let key = rows
            .iter()
            .find(|row| row.label == "API key")
            .expect("API key row");
        assert_eq!(key.value, "********");
        assert!(!format!("{rows:?}").contains("top-secret-key"));
    }
}
