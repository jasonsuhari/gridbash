use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::{self, Stdout, Write},
    path::PathBuf,
    process::Command,
    sync::mpsc as std_mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect, style::Color};
use tokio::sync::mpsc;
use vt100::Screen;

use crate::{
    auth::{self, AgentKind, AuthProfile},
    cli::{Cli, GridMode},
    composer::Composer,
    config::Config,
    control::{self, ControlCommand, ControlEnvelope, ControlHandle, ControlResponse},
    image_preview::{self, ImagePreview},
    layout::{GridLayout, GridSize, PaneId, pane_at},
    profiles::find_profile,
    pty::{PtyEvent, PtyPane},
    setup::{LaunchPlan, PaneLaunchSpec},
    ui,
    usage::{self, UsageEvent, UsageTarget},
    worktrees::ManagedWorktreeOptions,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const MAX_PANE_NAME_CHARS: usize = 32;
const CONVERSATION_SUMMARY_MAX_CHARS: usize = 120;
const TODO_INPUT_LIMIT: usize = 240;
const MIN_TODO_IDLE_SECONDS: u64 = 15;
const MAX_TODO_IDLE_SECONDS: u64 = 600;
const TODO_IDLE_STEP_SECONDS: u64 = 15;
const ACTIVITY_DECAY_INTERVAL: Duration = Duration::from_millis(250);
const OUTPUT_QUIET_AFTER: Duration = Duration::from_secs(3);

pub struct App {
    config: Config,
    config_path: Option<PathBuf>,
    worktrees: Option<ManagedWorktreeOptions>,
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
    rects: Vec<Rect>,
    mouse_enabled: bool,
    control_handle: Option<ControlHandle>,
    control_rx: Option<std_mpsc::Receiver<ControlEnvelope>>,
    image_overlay: Option<ImagePreview>,
    settings: SettingsState,
    rename: RenamePaneState,
    previous_panes: PreviousPanesState,
    follow_up: Option<FollowUpPromptState>,
    auth_profiles: Vec<AuthProfile>,
    auth_refresh_rx: Option<std_mpsc::Receiver<Result<Vec<AuthProfile>, String>>>,
    status: String,
    next_pane_id: usize,
    previous_panes_button: Option<Rect>,
    previous_pane_rows: Vec<(usize, Rect)>,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    usage_tx: std_mpsc::Sender<UsageEvent>,
    usage_rx: std_mpsc::Receiver<UsageEvent>,
    profile_usage: BTreeMap<String, String>,
    api_spend_label: Option<String>,
    last_activity_decay: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyOutcome {
    Continue,
    Render,
    AuthLogin(AuthProfile),
    Quit,
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
enum GridAxis {
    Rows,
    Columns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwapSelection {
    NeedsMore,
    TooMany,
    Pair(usize, usize),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteColor {
    Cyan,
    Sky,
    Blue,
    Teal,
    Green,
    Yellow,
    Amber,
    Orange,
    Red,
    Magenta,
    Gray,
    White,
}

impl PaletteColor {
    const ALL: [Self; 12] = [
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
        Self {
            accent: PaletteColor::Cyan,
            focus: PaletteColor::Yellow,
            selected: PaletteColor::Cyan,
            quiet: PaletteColor::Magenta,
            exited: PaletteColor::Red,
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
    PaneDensity,
    Scrollback,
    RefreshMs,
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
}

#[derive(Debug, Clone)]
pub struct RenamePaneView {
    pub pane_index: usize,
    pub pane_label: String,
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
pub struct ExitedPaneRecoveryView {
    pub pane_index: usize,
    pub pane_label: String,
    pub target_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Auth,
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
    pane_density: i32,
    scrollback: i32,
    refresh_ms: i32,
    palette: GridPalette,
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
            pane_density: 2,
            scrollback: 10_000,
            refresh_ms: 16,
            palette: GridPalette::default(),
        }
    }
}

impl SettingsState {
    fn from_config(config: &Config) -> Self {
        Self {
            idle_followups: config.todos.enabled,
            idle_seconds: config
                .todos
                .idle_seconds
                .clamp(MIN_TODO_IDLE_SECONDS, MAX_TODO_IDLE_SECONDS),
            todo_prompts: config.todos.normalized_prompts(),
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
                SettingsChange::Render
            }
            Some(SettingsTarget::ActivityBadges) => {
                self.activity_badges = !self.activity_badges;
                SettingsChange::Render
            }
            Some(SettingsTarget::ConfirmQuit) => {
                self.confirm_quit = !self.confirm_quit;
                SettingsChange::Render
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
                SettingsChange::Render
            }
            Some(SettingsTarget::ActivityBadges) => {
                if delta != 0 {
                    self.activity_badges = !self.activity_badges;
                }
                SettingsChange::Render
            }
            Some(SettingsTarget::ConfirmQuit) => {
                if delta != 0 {
                    self.confirm_quit = !self.confirm_quit;
                }
                SettingsChange::Render
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
            Some(SettingsTarget::PaneDensity) => {
                self.pane_density = (self.pane_density + delta).clamp(1, 5);
                SettingsChange::Render
            }
            Some(SettingsTarget::Scrollback) => {
                self.scrollback = (self.scrollback + delta * 1000).clamp(1_000, 50_000);
                SettingsChange::Render
            }
            Some(SettingsTarget::RefreshMs) => {
                self.refresh_ms = (self.refresh_ms + delta * 4).clamp(8, 100);
                SettingsChange::Render
            }
            Some(SettingsTarget::Palette(role)) => {
                self.palette.adjust(role, delta as isize);
                SettingsChange::Render
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

    fn idle_delay(&self) -> Duration {
        Duration::from_secs(
            self.idle_seconds
                .clamp(MIN_TODO_IDLE_SECONDS, MAX_TODO_IDLE_SECONDS),
        )
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
            SettingsTarget::PaneDensity,
            SettingsTarget::Scrollback,
            SettingsTarget::RefreshMs,
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
            SettingsTarget::PaneDensity,
            SettingsGroup::Performance,
            SettingsValueKind::Stepper,
            "Pane density",
            self.pane_density.to_string(),
            "spacing scale from 1 to 5",
        ));
        rows.push(self.row(
            SettingsTarget::Scrollback,
            SettingsGroup::Performance,
            SettingsValueKind::Stepper,
            "Scrollback rows",
            self.scrollback.to_string(),
            "history budget per pane",
        ));
        rows.push(self.row(
            SettingsTarget::RefreshMs,
            SettingsGroup::Performance,
            SettingsValueKind::Stepper,
            "Refresh delay",
            format!("{} ms", self.refresh_ms),
            "render loop throttle",
        ));
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

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
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
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (usage_tx, usage_rx) = std_mpsc::channel();
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
        let base_status = if mouse_enabled {
            "Drag copies | Right-click toggles pane | Alt+arrows move | Alt+Shift+arrows resize | Alt+r rename | Alt+t restart | Alt+x swap | Alt+z sleep"
        } else {
            "Alt+arrows move | Alt+Shift+arrows resize | Alt+s select | Alt+r rename | Alt+t restart | Alt+x swap | Alt+z sleep"
        };
        let status = control_handle
            .as_ref()
            .map(|control| format!("agent API {} | {base_status}", control.endpoint()))
            .unwrap_or_else(|| base_status.into());
        let settings = SettingsState::from_config(&config);

        Ok(Self {
            config,
            config_path,
            worktrees,
            launch_plan,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            pane_idle: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            pane_names: Vec::new(),
            text_selection: None,
            sleeping: BTreeSet::new(),
            rects: Vec::new(),
            mouse_enabled,
            control_handle,
            control_rx,
            image_overlay: None,
            settings,
            rename: RenamePaneState::default(),
            previous_panes: PreviousPanesState::default(),
            follow_up: None,
            auth_profiles: Vec::new(),
            auth_refresh_rx: None,
            status,
            next_pane_id: 0,
            previous_panes_button: None,
            previous_pane_rows: Vec::new(),
            event_tx,
            event_rx,
            usage_tx,
            usage_rx,
            profile_usage: BTreeMap::new(),
            api_spend_label: None,
            last_activity_decay: Instant::now(),
        })
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
        self.run_loop(terminal)
    }

    fn set_launch_plan(&mut self, mut plan: LaunchPlan) -> Result<()> {
        apply_auth_defaults(&mut plan, &self.config)?;
        self.layout = GridLayout::new(plan.grid);
        self.launch_plan = Some(plan);
        Ok(())
    }

    fn spawn_initial_panes(&mut self) -> Result<()> {
        let plan = self
            .launch_plan
            .clone()
            .ok_or_else(|| anyhow!("no launch plan selected"))?;
        self.layout = GridLayout::new(plan.grid);
        self.panes.clear();
        self.pane_names = vec![None; plan.panes.len()];
        self.text_selection = None;
        self.sleeping.clear();
        self.next_pane_id = 0;
        self.pane_idle.clear();
        self.follow_up = None;

        for spec in &plan.panes {
            self.spawn_pane_spec(spec)?;
        }
        self.start_usage_monitor(&plan);

        Ok(())
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

    fn spawn_pane_spec(&mut self, spec: &PaneLaunchSpec) -> Result<()> {
        let pane_index = self.panes.len();
        let pane = self.spawn_pane_instance(spec, pane_index)?;
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
            id,
            0,
            &launch.command,
            &launch.args,
            &spec.env,
            &spec.cwd,
            &extra_env,
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
        let mut needs_render = true;
        let mut mouse_capture_enabled = self.mouse_enabled;

        loop {
            needs_render |= self.drain_pty_events();
            needs_render |= self.drain_usage_events();
            needs_render |= self.drain_auth_refresh();
            needs_render |= self.drain_control_events();
            needs_render |= self.decay_activity();
            needs_render |= self.update_follow_up_prompt();

            if needs_render {
                terminal.draw(|frame| {
                    let draw_state = ui::draw(frame, self);
                    self.grid_area = draw_state.grid_area;
                    self.rects = draw_state.pane_rects;
                    self.previous_panes_button = draw_state.previous_panes_button;
                    self.previous_pane_rows = draw_state.previous_pane_rows;
                })?;
                self.sync_pane_sizes();
                needs_render = false;
            }
            self.sync_mouse_capture(terminal, &mut mouse_capture_enabled)?;

            if event::poll(INPUT_POLL_INTERVAL)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match self.handle_key(key)? {
                            KeyOutcome::Continue => {}
                            KeyOutcome::Render => needs_render = true,
                            KeyOutcome::AuthLogin(profile) => {
                                self.run_auth_login(terminal, profile)?;
                                mouse_capture_enabled = false;
                                needs_render = true;
                            }
                            KeyOutcome::Quit => break,
                        }
                    }
                    Event::Resize(_, _) => needs_render = true,
                    Event::Paste(text) if self.rename.open => {
                        self.rename.insert_text(&text);
                        needs_render = true;
                    }
                    Event::Paste(text) if self.settings.editing_todo() => {
                        if self.settings.insert_todo_text(&text) {
                            needs_render = true;
                        }
                    }
                    Event::Paste(text)
                        if !self.settings.open
                            && !self.rename.open
                            && !self.previous_panes.open
                            && self.image_overlay.is_none()
                            && self.follow_up.is_none() =>
                    {
                        self.route_input(text.as_bytes())?;
                    }
                    Event::Mouse(mouse)
                        if (self.mouse_enabled || !self.sleeping.is_empty())
                            && !self.settings.open
                            && self.follow_up.is_none() =>
                    {
                        needs_render |= self.handle_mouse(mouse, terminal)?;
                    }
                    _ => {}
                }
            }
        }

        if mouse_capture_enabled {
            execute!(terminal.backend_mut(), DisableMouseCapture)?;
        }

        Ok(())
    }

    fn sync_mouse_capture(&self, terminal: &mut Tui, enabled: &mut bool) -> Result<()> {
        let should_enable = self.mouse_enabled || !self.sleeping.is_empty();
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

        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                PtyEvent::Output {
                    pane,
                    generation,
                    bytes,
                } => {
                    if let Some((index, target)) = self
                        .panes
                        .iter_mut()
                        .enumerate()
                        .find(|(_, p)| p.id() == pane && p.generation() == generation)
                    {
                        target.process_output(&bytes);
                        self.mark_pane_touched(index);
                        changed = true;
                    }
                }
                PtyEvent::Exited { pane, generation } => {
                    if let Some((index, target)) = self
                        .panes
                        .iter_mut()
                        .enumerate()
                        .find(|(_, p)| p.id() == pane && p.generation() == generation)
                        && !target.exited
                    {
                        target.exited = true;
                        if self
                            .follow_up
                            .is_some_and(|prompt| prompt.pane_index == index)
                        {
                            self.follow_up = None;
                        }
                        changed = true;
                    }
                }
            }
        }

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

        changed
    }

    fn drain_usage_events(&mut self) -> bool {
        let mut changed = false;

        while let Ok(event) = self.usage_rx.try_recv() {
            match event {
                UsageEvent::Profile {
                    profile_name,
                    label,
                } => match label {
                    Some(label) => {
                        changed |= self.profile_usage.get(&profile_name) != Some(&label);
                        self.profile_usage.insert(profile_name, label);
                    }
                    None => {
                        changed |= self.profile_usage.remove(&profile_name).is_some();
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
            let Some(pane) = self.panes.get(*index) else {
                return ControlResponse::error(format!("pane {} is unavailable", index + 1));
            };
            if !command_bytes.is_empty()
                && let Err(error) = pane.write(command_bytes)
            {
                return ControlResponse::error(format!(
                    "failed to send command to pane {}: {error:#}",
                    index + 1
                ));
            }
            if submit && let Err(error) = pane.write(b"\r") {
                return ControlResponse::error(format!(
                    "failed to submit command in pane {}: {error:#}",
                    index + 1
                ));
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
        for pane in &mut self.panes {
            if pane.active {
                pane.active = false;
                changed = true;
            }
            changed |= pane.refresh_output_activity(now, OUTPUT_QUIET_AFTER);
        }
        self.last_activity_decay = now;
        changed
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if self.image_overlay.is_some() {
            return Ok(self.handle_image_overlay_key(key));
        }

        if self.rename.open {
            return self.handle_rename_key(key);
        }

        let selection_cleared = self.clear_text_selection();

        if self.previous_panes.open {
            let outcome = self.handle_previous_panes_key(key);
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if self.follow_up.is_some() {
            let outcome = self.handle_follow_up_key(key)?;
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if self.settings.open {
            let outcome = self.handle_settings_key(key)?;
            return Ok(render_if_selection_cleared(outcome, selection_cleared));
        }

        if key.modifiers.contains(KeyModifiers::ALT)
            && let Some(quit) = self.handle_app_key(key)?
        {
            return Ok(if quit {
                KeyOutcome::Quit
            } else {
                KeyOutcome::Render
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

    fn handle_app_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(ch, key.modifiers),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.adjust_grid(GridAxis::Columns, -1)?;
                Ok(Some(false))
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.adjust_grid(GridAxis::Columns, 1)?;
                Ok(Some(false))
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.adjust_grid(GridAxis::Rows, -1)?;
                Ok(Some(false))
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.adjust_grid(GridAxis::Rows, 1)?;
                Ok(Some(false))
            }
            KeyCode::Left => {
                self.focus_previous();
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            KeyCode::Right => {
                self.focus_next();
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            KeyCode::Up => {
                self.focus_in_grid(-1);
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            KeyCode::Down => {
                self.focus_in_grid(1);
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn handle_alt_char(&mut self, ch: char, _modifiers: KeyModifiers) -> Result<Option<bool>> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            'q' => Ok(Some(true)),
            's' => {
                self.toggle_pane_selection(self.focus);
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
            't' => {
                self.restart_exited_targets();
                Ok(Some(false))
            }
            'x' => {
                self.swap_selected_tiles();
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
            'p' => {
                self.open_previous_panes();
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

    fn begin_rename(&mut self) {
        if self.panes.is_empty() {
            self.status = "no panes to rename".into();
            return;
        }

        let pane_index = self.focus.min(self.panes.len() - 1);
        let current_name = self
            .pane_names
            .get(pane_index)
            .and_then(|name| name.clone());
        self.rename.begin(pane_index, current_name.as_deref());
        self.status = format!("renaming pane {}", pane_index + 1);
    }

    fn open_previous_panes(&mut self) {
        if self.panes.is_empty() {
            self.status = "no panes to list".into();
            return;
        }

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
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('p') | KeyCode::Char('P'))
        {
            self.close_previous_panes();
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

        if self.previous_panes.open {
            return Ok(if self.mouse_enabled {
                self.handle_previous_panes_mouse(mouse)
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

    fn previous_panes_button_at(&self, x: u16, y: u16) -> bool {
        self.previous_panes_button
            .is_some_and(|rect| rect_contains(rect, x, y))
    }

    fn previous_pane_row_at(&self, x: u16, y: u16) -> Option<usize> {
        self.previous_pane_rows
            .iter()
            .find_map(|(index, rect)| rect_contains(*rect, x, y).then_some(*index))
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

    fn adjust_grid(&mut self, axis: GridAxis, delta: isize) -> Result<()> {
        let current = self.layout.size();
        let Some(rows) =
            adjust_dimension(current.rows, if axis == GridAxis::Rows { delta } else { 0 })
        else {
            self.status = "grid is capped at 100 cells".into();
            return Ok(());
        };
        let Some(columns) = adjust_dimension(
            current.columns,
            if axis == GridAxis::Columns { delta } else { 0 },
        ) else {
            self.status = "grid is capped at 100 cells".into();
            return Ok(());
        };
        let Some(next) = GridSize::new(rows, columns) else {
            self.status = invalid_grid_status(rows, columns);
            return Ok(());
        };

        if next == current {
            return Ok(());
        }

        let before = self.panes.len();
        if next.count() > self.panes.len() {
            self.spawn_panes_to_fill(next.count())?;
        } else if next.count() < self.panes.len() && !self.remove_overflow_panes(next.count(), next)
        {
            return Ok(());
        }

        self.layout.set_size(next);
        if let Some(plan) = &mut self.launch_plan {
            plan.grid = next;
        }

        let added = self.panes.len().saturating_sub(before);
        let removed = before.saturating_sub(self.panes.len());
        self.status = if added > 0 {
            format!(
                "grid resized to {}x{}; spawned {added} pane(s)",
                next.rows, next.columns
            )
        } else if removed > 0 {
            format!(
                "grid resized to {}x{}; removed {removed} exited pane(s)",
                next.rows, next.columns
            )
        } else {
            format!(
                "grid resized to {}x{}; {} pane(s)",
                next.rows,
                next.columns,
                self.panes.len()
            )
        };

        Ok(())
    }

    fn spawn_panes_to_fill(&mut self, target_count: usize) -> Result<()> {
        let specs = self.pane_specs_to_fill(target_count)?;
        for spec in specs {
            self.spawn_pane_spec(&spec)?;
        }
        self.pane_names.resize(self.panes.len(), None);
        Ok(())
    }

    fn pane_specs_to_fill(&mut self, target_count: usize) -> Result<Vec<PaneLaunchSpec>> {
        let plan = self
            .launch_plan
            .as_mut()
            .ok_or_else(|| anyhow!("no launch plan selected"))?;
        if plan.panes.is_empty() {
            return Err(anyhow!("no pane template available"));
        }

        let templates = plan.panes.clone();
        while plan.panes.len() < target_count {
            let spec = templates[plan.panes.len() % templates.len()].clone();
            plan.panes.push(spec);
        }

        Ok(plan.panes[self.panes.len()..target_count].to_vec())
    }

    fn remove_overflow_panes(&mut self, target_count: usize, next: GridSize) -> bool {
        let running = self
            .panes
            .iter()
            .skip(target_count)
            .filter(|pane| !pane.exited)
            .count();
        if running > 0 {
            self.status = format!(
                "cannot shrink to {}x{}; {running} running pane(s) would be removed",
                next.rows, next.columns
            );
            return false;
        }

        self.panes.truncate(target_count);
        if let Some(plan) = &mut self.launch_plan {
            plan.panes.truncate(target_count);
        }
        self.selected = self
            .selected
            .iter()
            .copied()
            .filter(|index| *index < target_count)
            .collect();
        if self.focus >= target_count {
            self.focus = target_count.saturating_sub(1);
        }
        self.pane_names.truncate(target_count);
        self.pane_idle.truncate(target_count);
        if self
            .follow_up
            .is_some_and(|prompt| prompt.pane_index >= target_count)
        {
            self.follow_up = None;
        }
        self.sleeping = self
            .sleeping
            .iter()
            .copied()
            .filter(|index| *index < target_count)
            .collect();
        if self
            .text_selection
            .is_some_and(|selection| selection.pane >= target_count)
        {
            self.text_selection = None;
        }
        true
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

    fn apply_settings_change(&mut self, change: SettingsChange) {
        if change.save_todos() {
            self.save_todo_settings();
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
        self.panes
            .get(dialog.pane_index)
            .ok_or_else(|| anyhow!("invalid pane index {}", dialog.pane_index))?
            .write(&bytes)?;
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

    fn route_input(&mut self, bytes: &[u8]) -> Result<bool> {
        let targets = self.input_targets();
        let mut skipped_exited = 0;

        for index in targets {
            let pane = self
                .panes
                .get(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?;
            if pane.exited {
                skipped_exited += 1;
                continue;
            }
            pane.write(bytes)?;
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

        Ok(false)
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
            self.status = "no exited target panes; Alt+t restarts exited panes".into();
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
        if self.panes.is_empty() {
            return;
        }

        for offset in 1..=self.panes.len() {
            let candidate = (self.focus + offset) % self.panes.len();
            if !self.sleeping.contains(&candidate) {
                self.focus = candidate;
                return;
            }
        }
    }

    fn focus_previous(&mut self) {
        if self.panes.is_empty() {
            return;
        }

        for offset in 1..=self.panes.len() {
            let candidate = (self.focus + self.panes.len() - offset) % self.panes.len();
            if !self.sleeping.contains(&candidate) {
                self.focus = candidate;
                return;
            }
        }
    }

    fn focus_in_grid(&mut self, row_delta: isize) {
        if self.panes.is_empty() {
            return;
        }

        let columns = self.layout.size().columns;
        let candidate = if row_delta.is_negative() {
            self.focus.saturating_sub(columns)
        } else {
            self.focus.saturating_add(columns)
        };
        if candidate < self.panes.len() && !self.sleeping.contains(&candidate) {
            self.focus = candidate;
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

    pub fn focus(&self) -> usize {
        self.focus
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

    pub fn settings_open(&self) -> bool {
        self.settings.open
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

    pub fn activity_badges_enabled(&self) -> bool {
        self.settings.activity_badges
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
                    let summary = conversation_summary(pane.screen())
                        .unwrap_or_else(|| "waiting for output".into());
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

    pub fn pane_folder(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .map(|pane| pane.folder_name.as_str())
    }

    pub fn pane_profile(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .map(|pane| pane.profile_name.as_str())
    }

    pub fn pane_worktree(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.worktree_name.as_deref())
    }

    pub fn pane_auth(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.auth_name.as_deref())
    }

    pub fn pane_conversation_footer(&self, index: usize, max_chars: usize) -> Option<String> {
        let label = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.agent_label())?;
        let pane = self.panes.get(index)?;
        let summary = conversation_summary(pane.screen())
            .unwrap_or_else(|| "waiting for conversation".into());
        Some(truncate_chars(&format!("{label} | {summary}"), max_chars))
    }

    pub fn pane_usage_label(&self, index: usize) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(profile_name) = self
            .launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .map(|pane| pane.profile_name.as_str())
            && let Some(label) = self.profile_usage.get(profile_name)
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
        self.grid_area = Rect::new(0, 0, size.width, size.height.saturating_sub(1));
        self.rects = self.pane_rects(self.grid_area);
        self.sync_pane_sizes();
        Ok(())
    }

    fn toggle_settings_tab(&mut self) {
        self.settings.tab = match self.settings.tab {
            SettingsTab::General => SettingsTab::Auth,
            SettingsTab::Auth => SettingsTab::General,
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
    for spec in &mut plan.panes {
        let Some(kind) = spec.command.agent_kind else {
            continue;
        };
        let Some(auth_env) = auth::env_for_default(&config.auth, kind)? else {
            continue;
        };
        spec.env.extend(auth_env.env_map());
        spec.auth_name = Some(auth_env.name);
        spec.auth_kind = Some(auth_env.kind);
        spec.auth_dir = Some(auth_env.dir);
    }
    Ok(())
}

fn uses_direct_launch(cli: &Cli) -> bool {
    cli.grid.is_some()
        || cli.count.is_some()
        || cli.profile.is_some()
        || cli.cwd.is_some()
        || cli.layout == GridMode::Auto
}

fn resolve_profile_name(cli: &Cli, config: &Config) -> String {
    cli.profile
        .clone()
        .or_else(|| env::var("GRIDBASH_PROFILE").ok())
        .or_else(|| config.defaults.profile.clone())
        .unwrap_or_else(|| "git-bash".into())
}

fn resolved_current_dir() -> Result<std::path::PathBuf> {
    let current = env::current_dir().context("failed to resolve current directory")?;
    Ok(current.canonicalize().unwrap_or(current))
}

fn toggle_selection(selected: &mut BTreeSet<usize>, index: usize) -> bool {
    if selected.remove(&index) {
        false
    } else {
        selected.insert(index);
        true
    }
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

fn adjust_dimension(value: usize, delta: isize) -> Option<usize> {
    if delta < 0 {
        value.checked_sub((-delta) as usize)
    } else {
        value.checked_add(delta as usize)
    }
}

fn invalid_grid_status(rows: usize, columns: usize) -> String {
    if rows == 0 || columns == 0 {
        "grid needs at least 1 row and 1 column".into()
    } else {
        "grid is capped at 100 cells".into()
    }
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::profiles::Profile;

    struct TempHome {
        path: PathBuf,
    }

    impl TempHome {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = env::temp_dir().join(format!("gridbash-app-auth-test-{nonce}"));
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
        assert_eq!(rows[palette_start + 3].value, "magenta");
    }

    #[test]
    fn previous_panes_cursor_clamps_to_available_panes() {
        let mut previous = PreviousPanesState::default();
        previous.begin(8, 3);
        assert_eq!(previous.cursor, 2);

        previous.move_cursor(-5, 3);
        assert_eq!(previous.cursor, 0);

        previous.move_cursor(10, 3);
        assert_eq!(previous.cursor, 2);

        previous.move_cursor(1, 0);
        assert_eq!(previous.cursor, 0);
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
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
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
        EnableBracketedPaste
    )?;
    Ok(())
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use vt100::Parser;

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
}
