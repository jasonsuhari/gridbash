use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    io::{self, Stdout, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{OnceLock, mpsc as std_mpsc},
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
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect, style::Color, text::Line};
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
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_PANE_NAME_CHARS: usize = 32;
const MAX_TAB_TITLE_CHARS: usize = 40;
const CONVERSATION_SUMMARY_MAX_CHARS: usize = 120;
const COMMAND_OUTPUT_MAX_LINES: usize = 2000;
const ACTIVITY_DECAY_INTERVAL: Duration = Duration::from_millis(250);
const OUTPUT_QUIET_AFTER: Duration = Duration::from_secs(3);

pub struct App {
    config: Config,
    config_path: Option<PathBuf>,
    worktrees: Option<ManagedWorktreeOptions>,
    initial_launch_plan: Option<LaunchPlan>,
    tabs: Vec<GridTab>,
    active_tab: usize,
    grid_area: Rect,
    mouse_enabled: bool,
    command_line: CommandLineState,
    command_tx: mpsc::UnboundedSender<CommandRunEvent>,
    command_rx: mpsc::UnboundedReceiver<CommandRunEvent>,
    control_handle: Option<ControlHandle>,
    control_rx: Option<std_mpsc::Receiver<ControlEnvelope>>,
    image_overlay: Option<ImagePreview>,
    settings: SettingsState,
    rename: RenamePaneState,
    tab_rename: RenameTabState,
    auth_profiles: Vec<AuthProfile>,
    auth_refresh_rx: Option<std_mpsc::Receiver<Result<Vec<AuthProfile>, String>>>,
    status: String,
    next_tab_number: usize,
    next_pane_id: usize,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    usage_tx: std_mpsc::Sender<UsageEvent>,
    usage_rx: std_mpsc::Receiver<UsageEvent>,
    profile_usage: BTreeMap<String, String>,
    api_spend_label: Option<String>,
    last_activity_decay: Instant,
    last_exit_poll: Instant,
}

struct GridTab {
    title: String,
    launch_plan: LaunchPlan,
    layout: GridLayout,
    panes: Vec<PtyPane>,
    pane_index_by_id: BTreeMap<PaneId, usize>,
    pane_render_caches: Vec<ui::PaneRenderCache>,
    pane_screen_dirty: Vec<bool>,
    pane_conversation_summaries: Vec<Option<String>>,
    focus: usize,
    selected: BTreeSet<usize>,
    pane_names: Vec<Option<String>>,
    text_selection: Option<MouseSelection>,
    sleeping: BTreeSet<usize>,
    rects: Vec<Rect>,
}

impl GridTab {
    fn new(title: String, launch_plan: LaunchPlan) -> Self {
        Self {
            title,
            layout: GridLayout::new(launch_plan.grid),
            launch_plan,
            panes: Vec::new(),
            pane_index_by_id: BTreeMap::new(),
            pane_render_caches: Vec::new(),
            pane_screen_dirty: Vec::new(),
            pane_conversation_summaries: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            pane_names: Vec::new(),
            text_selection: None,
            sleeping: BTreeSet::new(),
            rects: Vec::new(),
        }
    }

    fn push_pane(&mut self, pane: PtyPane) {
        let pane_index = self.panes.len();
        let pane_id = pane.id();
        self.panes.push(pane);
        self.pane_index_by_id.insert(pane_id, pane_index);
        self.pane_render_caches.push(ui::PaneRenderCache::default());
        self.pane_screen_dirty.push(true);
        self.pane_conversation_summaries.push(None);
    }

    fn replace_pane(&mut self, index: usize, pane: PtyPane) {
        self.panes[index] = pane;
        self.sleeping.remove(&index);
        if let Some(cache) = self.pane_render_caches.get_mut(index) {
            *cache = ui::PaneRenderCache::default();
        }
        self.mark_pane_screen_dirty(index);
        if let Some(summary) = self.pane_conversation_summaries.get_mut(index) {
            *summary = None;
        }
        self.rebuild_pane_index();
    }

    fn truncate_panes(&mut self, target_count: usize) {
        self.panes.truncate(target_count);
        self.pane_names.truncate(target_count);
        self.pane_render_caches.truncate(target_count);
        self.pane_screen_dirty.truncate(target_count);
        self.pane_conversation_summaries.truncate(target_count);
        self.rebuild_pane_index();
    }

    fn pane_index_for_event(&self, id: PaneId, generation: u64) -> Option<usize> {
        let index = *self.pane_index_by_id.get(&id)?;
        let pane = self.panes.get(index)?;
        (pane.id() == id && pane.generation() == generation).then_some(index)
    }

    fn rebuild_pane_index(&mut self) {
        self.pane_index_by_id.clear();
        for (index, pane) in self.panes.iter().enumerate() {
            self.pane_index_by_id.insert(pane.id(), index);
        }
    }

    fn mark_pane_screen_dirty(&mut self, index: usize) {
        if let Some(dirty) = self.pane_screen_dirty.get_mut(index) {
            *dirty = true;
        }
    }

    fn refresh_conversation_summary(&mut self, index: usize) {
        let Some(summary) = self.pane_conversation_summaries.get_mut(index) else {
            return;
        };
        *summary = self
            .panes
            .get(index)
            .and_then(|pane| conversation_summary(pane.screen()));
    }

    fn pane_screen_lines(
        &mut self,
        index: usize,
        width: u16,
        height: u16,
        selection: Option<PaneSelection>,
    ) -> Vec<Line<'static>> {
        let Some(pane) = self.panes.get(index) else {
            return Vec::new();
        };
        let Some(cache) = self.pane_render_caches.get_mut(index) else {
            return ui::screen_lines(pane.screen(), width, height, selection);
        };
        let Some(dirty) = self.pane_screen_dirty.get_mut(index) else {
            return ui::screen_lines(pane.screen(), width, height, selection);
        };

        ui::cached_screen_lines(cache, dirty, pane.screen(), width, height, selection)
    }
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
    pub label: &'static str,
    pub value: String,
    pub value_color: Option<Color>,
    pub hint: &'static str,
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
    pane_density: i32,
    scrollback: i32,
    refresh_ms: i32,
    palette: GridPalette,
}

#[derive(Debug, Clone)]
struct CommandLineState {
    focused: bool,
    cwd: PathBuf,
    input: String,
    cursor: usize,
    output_lines: Vec<String>,
    output_expanded: bool,
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
            output_expanded: false,
            running: false,
        }
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
            pane_density: 2,
            scrollback: 10_000,
            refresh_ms: 16,
            palette: GridPalette::default(),
        }
    }
}

impl SettingsState {
    const BASE_ROW_COUNT: usize = 6;
    const ROW_COUNT: usize = Self::BASE_ROW_COUNT + PaletteRole::ALL.len();

    fn move_cursor(&mut self, delta: isize) {
        let current = self.cursor as isize;
        self.cursor = (current + delta).clamp(0, Self::ROW_COUNT as isize - 1) as usize;
    }

    fn activate(&mut self) {
        if self.palette_role().is_some() {
            self.adjust(1);
            return;
        }

        match self.cursor {
            0 => self.compact_titles = !self.compact_titles,
            1 => self.activity_badges = !self.activity_badges,
            2 => self.confirm_quit = !self.confirm_quit,
            _ => self.adjust(1),
        }
    }

    fn adjust(&mut self, delta: i32) {
        if let Some(role) = self.palette_role() {
            self.palette.adjust(role, delta as isize);
            return;
        }

        match self.cursor {
            0 => {
                if delta != 0 {
                    self.compact_titles = !self.compact_titles;
                }
            }
            1 => {
                if delta != 0 {
                    self.activity_badges = !self.activity_badges;
                }
            }
            2 => {
                if delta != 0 {
                    self.confirm_quit = !self.confirm_quit;
                }
            }
            3 => self.pane_density = (self.pane_density + delta).clamp(1, 5),
            4 => self.scrollback = (self.scrollback + delta * 1000).clamp(1_000, 50_000),
            5 => self.refresh_ms = (self.refresh_ms + delta * 4).clamp(8, 100),
            _ => {}
        }
    }

    fn rows(&self) -> Vec<SettingsRow> {
        let mut rows = vec![
            self.row(
                0,
                "Compact pane titles",
                switch_value(self.compact_titles),
                None,
                "shorter labels in pane chrome",
            ),
            self.row(
                1,
                "Activity badges",
                switch_value(self.activity_badges),
                None,
                "quiet markers",
            ),
            self.row(
                2,
                "Confirm before quit",
                switch_value(self.confirm_quit),
                None,
                "extra guard for Alt+q",
            ),
            self.row(
                3,
                "Pane density",
                self.pane_density.to_string(),
                None,
                "spacing scale from 1 to 5",
            ),
            self.row(
                4,
                "Scrollback rows",
                self.scrollback.to_string(),
                None,
                "history budget per pane",
            ),
            self.row(
                5,
                "Refresh delay",
                format!("{} ms", self.refresh_ms),
                None,
                "render loop throttle",
            ),
        ];

        rows.extend(
            PaletteRole::ALL
                .iter()
                .enumerate()
                .map(|(offset, role)| self.palette_row(Self::BASE_ROW_COUNT + offset, *role)),
        );
        rows
    }

    fn palette_role(&self) -> Option<PaletteRole> {
        self.cursor
            .checked_sub(Self::BASE_ROW_COUNT)
            .and_then(|index| PaletteRole::ALL.get(index).copied())
    }

    fn palette_row(&self, index: usize, role: PaletteRole) -> SettingsRow {
        let color = self.palette.color_for(role);
        self.row(
            index,
            role.label(),
            color.name().to_string(),
            Some(color.color()),
            "-/+ color",
        )
    }

    fn row(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        value_color: Option<Color>,
        hint: &'static str,
    ) -> SettingsRow {
        SettingsRow {
            selected: self.cursor == index,
            label,
            value,
            value_color,
            hint,
        }
    }
}

fn switch_value(enabled: bool) -> String {
    if enabled { "on".into() } else { "off".into() }
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let config_path = cli.config.clone();
        let startup_cwd = resolved_current_dir()?;
        let worktrees = cli
            .worktrees
            .then(|| ManagedWorktreeOptions::new(cli.worktree_prefix.clone()))
            .transpose()?;
        let mut launch_plan = resolve_direct_launch_plan(&cli, &config, worktrees.as_ref())?;
        if let Some(plan) = launch_plan.as_mut() {
            apply_auth_defaults(plan, &config)?;
        }
        let mouse_enabled = !cli.no_mouse;
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (usage_tx, usage_rx) = std_mpsc::channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
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
            "Drag copies within pane | Alt+t tab | Alt+n new tab | Alt+r pane rename | Alt+Shift+r tab rename | Alt+e output | Alt+arrows move | Alt+Shift+arrows resize | Alt+x swap | Alt+z sleep | Alt+o settings"
        } else {
            "Alt+t tab | Alt+n new tab | Alt+arrows move | Alt+Shift+arrows resize | Alt+s select | Alt+r pane rename | Alt+Shift+r tab rename | Alt+e output | Alt+x swap | Alt+z sleep | Alt+o settings"
        };
        let status = control_handle
            .as_ref()
            .map(|control| format!("agent API {} | {base_status}", control.endpoint()))
            .unwrap_or_else(|| base_status.into());

        Ok(Self {
            config,
            config_path,
            worktrees,
            initial_launch_plan: launch_plan,
            tabs: Vec::new(),
            active_tab: 0,
            grid_area: Rect::default(),
            mouse_enabled,
            command_line: CommandLineState::new(startup_cwd),
            command_tx,
            command_rx,
            control_handle,
            control_rx,
            image_overlay: None,
            settings: SettingsState::default(),
            rename: RenamePaneState::default(),
            tab_rename: RenameTabState::default(),
            auth_profiles: Vec::new(),
            auth_refresh_rx: None,
            status,
            next_tab_number: 1,
            next_pane_id: 0,
            event_tx,
            event_rx,
            usage_tx,
            usage_rx,
            profile_usage: BTreeMap::new(),
            api_spend_label: None,
            last_activity_decay: Instant::now(),
            last_exit_poll: Instant::now(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal(self.mouse_enabled)?;
        let result = self.run_in_terminal(&mut terminal);
        teardown_terminal(&mut terminal, self.mouse_enabled)?;
        result
    }

    fn run_in_terminal(&mut self, terminal: &mut Tui) -> Result<()> {
        let initial_plan = if let Some(plan) = self.initial_launch_plan.take() {
            plan
        } else {
            let current_dir = resolved_current_dir()?;
            let mut composer = Composer::new(current_dir, self.worktrees.clone());
            let Some(plan) = composer.run(terminal, &self.config)? else {
                return Ok(());
            };
            let mut plan = plan;
            apply_auth_defaults(&mut plan, &self.config)?;
            plan
        };

        self.add_tab_from_plan(initial_plan)?;
        self.sync_initial_pane_sizes(terminal)?;
        self.run_loop(terminal)
    }

    fn add_tab_from_plan(&mut self, plan: LaunchPlan) -> Result<()> {
        let title = self.next_tab_title();
        let mut tab = GridTab::new(title.clone(), plan);
        let pane_specs = tab.launch_plan.panes.clone();

        for (pane_index, spec) in pane_specs.iter().enumerate() {
            tab.push_pane(self.spawn_pane_spec(spec, pane_index)?);
        }
        tab.pane_names = vec![None; tab.panes.len()];
        self.start_usage_monitor(&tab.launch_plan);

        self.tabs.push(tab);
        self.active_tab = self.tabs.len().saturating_sub(1);
        self.rename.close();
        self.tab_rename.close();
        self.status = format!("opened tab {title}");

        Ok(())
    }

    fn next_tab_title(&mut self) -> String {
        let title = format!("Grid {}", self.next_tab_number);
        self.next_tab_number += 1;
        title
    }

    fn start_usage_monitor(&self, plan: &LaunchPlan) {
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

    fn spawn_pane_spec(&mut self, spec: &PaneLaunchSpec, pane_index: usize) -> Result<PtyPane> {
        self.spawn_pane_instance(spec, pane_index)
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
            needs_render |= self.drain_command_events();
            needs_render |= self.drain_control_events();
            needs_render |= self.decay_activity();

            if needs_render {
                terminal.draw(|frame| {
                    let draw_state = ui::draw(frame, self);
                    self.grid_area = draw_state.grid_area;
                    if let Some(tab) = self.active_tab_mut() {
                        tab.rects = draw_state.pane_rects;
                    }
                })?;
                needs_render = self.sync_pane_sizes();
            }
            self.sync_mouse_capture(terminal, &mut mouse_capture_enabled)?;

            if event::poll(INPUT_POLL_INTERVAL)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match self.handle_key(terminal, key)? {
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
                    Event::Paste(text) if self.tab_rename.open => {
                        self.tab_rename.insert_text(&text);
                        needs_render = true;
                    }
                    Event::Paste(text) if self.rename.open => {
                        self.rename.insert_text(&text);
                        needs_render = true;
                    }
                    Event::Paste(text) if !self.settings.open => {
                        if self.command_line.focused {
                            self.command_line.insert_text(&text);
                            needs_render = true;
                        } else {
                            self.route_input(text.as_bytes())?;
                        }
                    }
                    Event::Mouse(mouse)
                        if (self.mouse_enabled || self.active_tab_has_sleeping_panes())
                            && !self.settings.open =>
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
        let should_enable = self.mouse_enabled || self.active_tab_has_sleeping_panes();
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
        let mut pending_output = BTreeMap::<(PaneId, u64), Vec<u8>>::new();
        let mut exited = Vec::new();

        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                PtyEvent::Output {
                    pane,
                    generation,
                    bytes,
                } => {
                    pending_output
                        .entry((pane, generation))
                        .or_default()
                        .extend(bytes);
                }
                PtyEvent::Exited { pane, generation } => exited.push((pane, generation)),
            }
        }

        for ((pane_id, generation), bytes) in pending_output {
            let Some((tab_index, pane_index)) = self.pane_location_for_event(pane_id, generation)
            else {
                continue;
            };

            let tab = &mut self.tabs[tab_index];
            tab.panes[pane_index].process_output(&bytes);
            tab.mark_pane_screen_dirty(pane_index);
            tab.refresh_conversation_summary(pane_index);
            changed = true;
        }

        for (pane_id, generation) in exited {
            let Some((tab_index, pane_index)) = self.pane_location_for_event(pane_id, generation)
            else {
                continue;
            };

            let pane = &mut self.tabs[tab_index].panes[pane_index];
            if !pane.exited {
                pane.exited = true;
                changed = true;
            }
        }

        if self.last_exit_poll.elapsed() >= EXIT_POLL_INTERVAL {
            for tab in &mut self.tabs {
                for index in 0..tab.panes.len() {
                    if tab.panes[index].poll_exit() {
                        tab.mark_pane_screen_dirty(index);
                        changed = true;
                    }
                }
            }
            self.last_exit_poll = Instant::now();
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
            let Some(pane) = self.panes().get(*index) else {
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
            if *pane_number == 0 || *pane_number > self.panes().len() {
                return Err(anyhow!("pane {pane_number} is not available"));
            }
            let index = pane_number - 1;
            if self.pane_sleeping(index) {
                return Err(anyhow!("pane {pane_number} is asleep"));
            }
            if self.panes().get(index).is_some_and(|pane| pane.exited) {
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
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.active {
                    pane.active = false;
                    changed = true;
                }
                changed |= pane.refresh_output_activity(now, OUTPUT_QUIET_AFTER);
            }
        }
        self.last_activity_decay = now;
        changed
    }

    fn handle_key(&mut self, terminal: &mut Tui, key: KeyEvent) -> Result<KeyOutcome> {
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

    fn handle_app_key(&mut self, terminal: &mut Tui, key: KeyEvent) -> Result<Option<bool>> {
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(terminal, ch, key.modifiers),
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
            't' => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    self.restart_exited_targets();
                } else {
                    self.next_tab();
                }
                Ok(Some(false))
            }
            'n' => {
                self.open_new_tab(terminal)?;
                Ok(Some(false))
            }
            's' => {
                if self.command_line.focused {
                    self.status = "command line focused".into();
                } else {
                    let selected_count = {
                        let Some(tab) = self.active_tab_mut() else {
                            return Ok(Some(false));
                        };
                        toggle_selection(&mut tab.selected, tab.focus);
                        tab.selected.len()
                    };
                    self.status = format!("selected {} panes", selected_count);
                }
                Ok(Some(false))
            }
            'a' => {
                let selected_count = {
                    let Some(tab) = self.active_tab_mut() else {
                        return Ok(Some(false));
                    };
                    if tab.selected.len() == tab.panes.len() {
                        tab.selected.clear();
                    } else {
                        tab.selected = (0..tab.panes.len()).collect();
                    }
                    tab.selected.len()
                };
                self.status = format!("selected {} panes", selected_count);
                Ok(Some(false))
            }
            'z' => {
                self.toggle_sleep_for_targets();
                Ok(Some(false))
            }
            'x' => {
                self.swap_selected_tiles();
                Ok(Some(false))
            }
            'e' => {
                self.command_line.output_expanded = !self.command_line.output_expanded;
                self.status = if self.command_line.output_expanded {
                    "command output expanded".into()
                } else {
                    "command output hidden".into()
                };
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
            'r' => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    self.begin_tab_rename();
                } else {
                    self.begin_rename();
                }
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn open_new_tab(&mut self, terminal: &mut Tui) -> Result<()> {
        self.drain_pty_events();
        let current_dir = match self.active_pane_cwd() {
            Some(path) => path,
            None => resolved_current_dir()?,
        };
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

        self.clear_text_selection();
        self.rename.close();
        self.tab_rename.close();
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
        let title = self.active_tab_title().to_string();
        self.status = format!("active tab {title}");
    }

    fn begin_tab_rename(&mut self) {
        let Some(title) = self.active_tab().map(|tab| tab.title.clone()) else {
            self.status = "no tab to rename".into();
            return;
        };

        self.rename.close();
        self.tab_rename.begin(&title);
        self.status = format!("renaming tab {title}");
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
                if let Some(tab) = self.active_tab_mut() {
                    tab.title = name.clone();
                }
                self.status = format!("renamed tab to {name}");
                self.tab_rename.close();
            }
            None => {
                self.status = "tab name cannot be empty".into();
            }
        }
    }

    fn begin_rename(&mut self) {
        if self.panes().is_empty() {
            self.status = "no panes to rename".into();
            return;
        }

        let Some(tab) = self.active_tab() else {
            self.status = "no panes to rename".into();
            return;
        };
        let pane_index = tab.focus.min(tab.panes.len() - 1);
        let current_name = tab.pane_names.get(pane_index).and_then(|name| name.clone());
        self.tab_rename.close();
        self.rename.begin(pane_index, current_name.as_deref());
        self.status = format!("renaming pane {}", pane_index + 1);
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

        if let Some(slot) = self
            .active_tab_mut()
            .and_then(|tab| tab.pane_names.get_mut(pane_index))
        {
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
        let Some(tab) = self.active_tab_mut() else {
            self.status = "no panes to swap".into();
            return;
        };

        let (first, second) = match selected_swap_pair(&tab.selected) {
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

        if first >= tab.panes.len() || second >= tab.panes.len() {
            self.status = "select two visible panes to swap".into();
            return;
        }

        tab.panes.swap(first, second);
        if first < tab.pane_names.len() && second < tab.pane_names.len() {
            tab.pane_names.swap(first, second);
        }
        if let Some(selection) = tab.text_selection {
            tab.text_selection = Some(MouseSelection {
                pane: swapped_index(selection.pane, first, second),
                ..selection
            });
        }
        if first < tab.launch_plan.panes.len() && second < tab.launch_plan.panes.len() {
            tab.launch_plan.panes.swap(first, second);
        }
        tab.pane_render_caches.swap(first, second);
        tab.pane_screen_dirty.swap(first, second);
        tab.pane_conversation_summaries.swap(first, second);
        tab.rebuild_pane_index();
        swap_set_indices(&mut tab.sleeping, first, second);
        tab.focus = swapped_index(tab.focus, first, second);
        self.status = format!("swapped panes {} and {}", first + 1, second + 1);
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
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

        let changed = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings.open = false;
                self.status = "settings closed".into();
                true
            }
            KeyCode::Up => {
                self.settings.move_cursor(-1);
                true
            }
            KeyCode::Down => {
                self.settings.move_cursor(1);
                true
            }
            KeyCode::Left | KeyCode::Char('-') => {
                self.settings.adjust(-1);
                true
            }
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => {
                self.settings.adjust(1);
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings.activate();
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

        let sleeping_pane = self
            .active_tab()
            .and_then(|tab| pane_at(&tab.rects, mouse.column, mouse.row));
        let sleeping_pane = sleeping_pane.and_then(|index| {
            let tab = self.active_tab_mut()?;
            if tab.sleeping.remove(&index) {
                tab.focus = index;
                Some(index)
            } else {
                None
            }
        });
        if let Some(index) = sleeping_pane {
            self.status = format!("woke pane {}", index + 1);
            return Ok(true);
        }

        if !self.mouse_enabled {
            return Ok(false);
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(cell) = self.pane_cell_at(mouse.column, mouse.row) {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.focus = cell.pane;
                        tab.text_selection = Some(MouseSelection {
                            pane: cell.pane,
                            anchor: cell.point,
                            cursor: cell.point,
                            active: true,
                            moved: false,
                        });
                    }
                    self.status = format!("selecting text in pane {}", cell.pane + 1);
                    return Ok(true);
                }

                if let Some(index) = self
                    .active_tab()
                    .and_then(|tab| pane_at(&tab.rects, mouse.column, mouse.row))
                {
                    let selection_cleared = self.clear_text_selection();
                    let changed = self.focus() != index || selection_cleared;
                    if let Some(tab) = self.active_tab_mut() {
                        tab.focus = index;
                    }
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

    fn pane_cell_at(&self, x: u16, y: u16) -> Option<PaneCell> {
        let tab = self.active_tab()?;
        let pane = pane_at(&tab.rects, x, y)?;
        let rect = tab.rects.get(pane).copied()?;
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
        let tab = self.active_tab()?;
        let rect = tab.rects.get(pane).copied()?;
        let inner = pane_inner_rect(rect)?;
        let max_x = inner.x.saturating_add(inner.width.saturating_sub(1));
        let max_y = inner.y.saturating_add(inner.height.saturating_sub(1));

        Some(CellPoint {
            row: y.clamp(inner.y, max_y).saturating_sub(inner.y),
            column: x.clamp(inner.x, max_x).saturating_sub(inner.x),
        })
    }

    fn update_text_selection(&mut self, x: u16, y: u16) -> bool {
        let Some(selection) = self.active_tab().and_then(|tab| tab.text_selection) else {
            return false;
        };
        if !selection.active {
            return false;
        }

        let Some(cursor) = self.clamped_pane_cell(selection.pane, x, y) else {
            return false;
        };

        if let Some(tab) = self.active_tab_mut() {
            tab.text_selection = Some(MouseSelection {
                cursor,
                moved: true,
                ..selection
            });
        }
        true
    }

    fn finish_text_selection(&mut self, x: u16, y: u16, terminal: &mut Tui) -> Result<bool> {
        let Some(selection) = self.active_tab().and_then(|tab| tab.text_selection) else {
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
            if let Some(tab) = self.active_tab_mut() {
                tab.text_selection = None;
            }
            self.status = format!("focused pane {}", selection.pane + 1);
            return Ok(true);
        }

        if let Some(tab) = self.active_tab_mut() {
            tab.text_selection = Some(selection);
        }

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
        let Some(tab) = self.active_tab() else {
            return String::new();
        };
        let Some(pane) = tab.panes.get(selection.pane) else {
            return String::new();
        };
        let width = tab
            .rects
            .get(selection.pane)
            .and_then(|rect| pane_inner_rect(*rect))
            .map(|inner| inner.width)
            .unwrap_or(0);

        extract_selection_text(pane.screen(), selection.range(), width)
    }

    fn clear_text_selection(&mut self) -> bool {
        self.active_tab_mut()
            .and_then(|tab| tab.text_selection.take())
            .is_some()
    }

    fn adjust_grid(&mut self, axis: GridAxis, delta: isize) -> Result<()> {
        let Some(current) = self.active_tab().map(|tab| tab.layout.size()) else {
            self.status = "no active tab".into();
            return Ok(());
        };
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

        let before = self.panes().len();
        if next.count() > self.panes().len() {
            self.spawn_panes_to_fill(next.count())?;
        } else if next.count() < self.panes().len()
            && !self.remove_overflow_panes(next.count(), next)
        {
            return Ok(());
        }

        if let Some(tab) = self.active_tab_mut() {
            tab.layout.set_size(next);
            tab.launch_plan.grid = next;
        }

        let after = self.panes().len();
        let added = after.saturating_sub(before);
        let removed = before.saturating_sub(after);
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
                next.rows, next.columns, after
            )
        };

        Ok(())
    }

    fn spawn_panes_to_fill(&mut self, target_count: usize) -> Result<()> {
        let start_index = self.panes().len();
        let specs = self.pane_specs_to_fill(target_count)?;
        for (offset, spec) in specs.into_iter().enumerate() {
            let pane = self.spawn_pane_spec(&spec, start_index + offset)?;
            let Some(tab) = self.active_tab_mut() else {
                return Err(anyhow!("no active tab"));
            };
            tab.panes.push(pane);
        }
        if let Some(tab) = self.active_tab_mut() {
            tab.pane_names.resize(tab.panes.len(), None);
        }
        Ok(())
    }

    fn pane_specs_to_fill(&mut self, target_count: usize) -> Result<Vec<PaneLaunchSpec>> {
        let tab = self
            .active_tab_mut()
            .ok_or_else(|| anyhow!("no active tab selected"))?;
        let current_count = tab.panes.len();
        let plan = &mut tab.launch_plan;
        if plan.panes.is_empty() {
            return Err(anyhow!("no pane template available"));
        }

        let templates = plan.panes.clone();
        while plan.panes.len() < target_count {
            let spec = templates[plan.panes.len() % templates.len()].clone();
            plan.panes.push(spec);
        }

        Ok(plan.panes[current_count..target_count].to_vec())
    }

    fn remove_overflow_panes(&mut self, target_count: usize, next: GridSize) -> bool {
        let Some(tab) = self.active_tab_mut() else {
            self.status = "no active tab".into();
            return false;
        };

        let running = tab
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

        tab.panes.truncate(target_count);
        tab.launch_plan.panes.truncate(target_count);
        tab.truncate_panes(target_count);
        tab.selected = tab
            .selected
            .iter()
            .copied()
            .filter(|index| *index < target_count)
            .collect();
        if tab.focus >= target_count {
            tab.focus = target_count.saturating_sub(1);
        }
        tab.sleeping = tab
            .sleeping
            .iter()
            .copied()
            .filter(|index| *index < target_count)
            .collect();
        if tab
            .text_selection
            .is_some_and(|selection| selection.pane >= target_count)
        {
            tab.text_selection = None;
        }
        true
    }

    fn toggle_sleep_for_targets(&mut self) {
        let targets = self.target_panes();
        if targets.is_empty() {
            return;
        }

        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let should_sleep = targets.iter().any(|index| !tab.sleeping.contains(index));
        if should_sleep {
            for index in &targets {
                tab.sleeping.insert(*index);
                tab.selected.remove(index);
            }

            if tab.sleeping.contains(&tab.focus)
                && let Some(index) = next_awake_pane_for(tab.focus, tab.panes.len(), &tab.sleeping)
            {
                tab.focus = index;
            }
        } else {
            for index in &targets {
                tab.sleeping.remove(index);
            }
            tab.focus = targets[0];
        }

        let action = if should_sleep { "slept" } else { "woke" };
        self.status = format!("{} {} {}", action, targets.len(), pane_word(targets.len()));
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

    fn route_input(&mut self, bytes: &[u8]) -> Result<bool> {
        let targets = self.input_targets();
        let Some(tab) = self.active_tab_mut() else {
            return Ok(false);
        };
        let mut skipped_exited = 0;

        for index in targets {
            let pane = tab
                .panes
                .get_mut(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?;
            if pane.exited {
                skipped_exited += 1;
                continue;
            }
            pane.write(bytes)?;
        }

        if skipped_exited > 0 {
            self.status = if skipped_exited == 1 {
                "pane exited; press Alt+Shift+t to restart it".into()
            } else {
                format!(
                    "skipped {skipped_exited} exited {}; press Alt+Shift+t to restart them",
                    pane_word(skipped_exited)
                )
            };
            return Ok(true);
        }

        Ok(false)
    }

    fn input_targets(&self) -> Vec<usize> {
        self.active_tab()
            .map(|tab| {
                awake_input_targets_for(tab.focus, &tab.selected, tab.panes.len(), &tab.sleeping)
            })
            .unwrap_or_default()
    }

    fn target_panes(&self) -> Vec<usize> {
        self.active_tab()
            .map(|tab| input_targets_for(tab.focus, &tab.selected, tab.panes.len()))
            .unwrap_or_default()
    }

    fn restart_exited_targets(&mut self) {
        let targets = self.target_panes();
        if targets.is_empty() {
            self.status = "no panes to restart".into();
            return;
        }

        let Some(tab) = self.active_tab() else {
            self.status = "no active tab".into();
            return;
        };

        let exited = tab.panes.iter().map(|pane| pane.exited).collect::<Vec<_>>();
        let restart = restart_targets_for(&targets, &exited);
        if restart.indices.is_empty() {
            self.status = "no exited target panes; Alt+Shift+t restarts exited panes".into();
            return;
        }

        let specs = restart
            .indices
            .iter()
            .filter_map(|index| {
                tab.launch_plan
                    .panes
                    .get(*index)
                    .cloned()
                    .map(|spec| (*index, spec))
            })
            .collect::<Vec<_>>();
        if specs.is_empty() {
            self.status = "cannot restart panes without a launch plan".into();
            return;
        }

        let mut restarted = 0;
        for (index, spec) in specs {
            let pane = match self.spawn_pane_instance(&spec, index) {
                Ok(pane) => pane,
                Err(error) => {
                    self.status = format!("restart failed for pane {}: {error:#}", index + 1);
                    return;
                }
            };

            if let Some(tab) = self.active_tab_mut() {
                tab.replace_pane(index, pane);
            }
            restarted += 1;
        }

        let _ = self.sync_pane_sizes();
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
            self.command_line.focused = false;
            let Some(tab) = self.active_tab_mut() else {
                return;
            };
            if let Some(candidate) = (0..tab.panes.len()).find(|index| !tab.sleeping.contains(index))
            {
                tab.focus = candidate;
            }
            return;
        }

        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.panes.is_empty() {
            return;
        }

        if let Some(candidate) =
            ((tab.focus + 1)..tab.panes.len()).find(|index| !tab.sleeping.contains(index))
        {
            tab.focus = candidate;
            return;
        }

        self.command_line.focused = true;
    }

    fn focus_previous(&mut self) {
        if self.command_line.focused {
            self.command_line.focused = false;
            let Some(tab) = self.active_tab_mut() else {
                return;
            };
            if let Some(candidate) = (0..tab.panes.len())
                .rev()
                .find(|index| !tab.sleeping.contains(index))
            {
                tab.focus = candidate;
            }
            return;
        }

        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.panes.is_empty() {
            return;
        }

        if let Some(candidate) = (0..tab.focus)
            .rev()
            .find(|index| !tab.sleeping.contains(index))
        {
            tab.focus = candidate;
            return;
        }

        self.command_line.focused = true;
    }

    fn focus_in_grid(&mut self, row_delta: isize) {
        if self.command_line.focused {
            if row_delta.is_negative() {
                self.command_line.focused = false;
                let Some(tab) = self.active_tab_mut() else {
                    return;
                };
                if let Some(candidate) = (0..tab.panes.len())
                    .rev()
                    .find(|index| !tab.sleeping.contains(index))
                {
                    tab.focus = candidate;
                }
            }
            return;
        }

        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.panes.is_empty() {
            return;
        }

        let columns = tab.layout.size().columns;
        let candidate = if row_delta.is_negative() {
            tab.focus.saturating_sub(columns)
        } else {
            tab.focus.saturating_add(columns)
        };
        if candidate < tab.panes.len() && !tab.sleeping.contains(&candidate) {
            tab.focus = candidate;
        } else if row_delta.is_positive() {
            self.command_line.focused = true;
        }
    }

    fn focus_status(&self) -> String {
        if self.command_line.focused {
            "focused command line".into()
        } else {
            format!("focused pane {}", self.focus() + 1)
        }
    }

    pub fn pane_rects(&self, area: Rect) -> Vec<Rect> {
        self.active_tab()
            .map(|tab| tab.layout.rects(area, tab.panes.len()))
            .unwrap_or_default()
    }

    pub fn panes(&self) -> &[PtyPane] {
        self.active_tab()
            .map(|tab| tab.panes.as_slice())
            .unwrap_or(&[])
    }

    pub fn focus(&self) -> usize {
        self.active_tab().map(|tab| tab.focus).unwrap_or(0)
    }

    pub fn focused_pane(&self) -> Option<usize> {
        (!self.command_line.focused).then_some(self.focus())
    }

    pub fn selected(&self) -> &BTreeSet<usize> {
        match self.active_tab() {
            Some(tab) => &tab.selected,
            None => empty_selection(),
        }
    }

    pub fn selection_for_pane(&self, index: usize) -> Option<PaneSelection> {
        self.active_tab()
            .and_then(|tab| tab.text_selection)
            .filter(|selection| selection.pane == index)
            .map(MouseSelection::range)
    }

    pub fn pane_sleeping(&self, index: usize) -> bool {
        self.active_tab()
            .is_some_and(|tab| tab.sleeping.contains(&index))
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn settings_open(&self) -> bool {
        self.settings.open
    }

    pub fn image_overlay_view(&self) -> Option<&ImagePreview> {
        self.image_overlay.as_ref()
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
        self.active_tab()
            .and_then(|tab| tab.pane_names.get(index))
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
            title: self.active_tab_title().to_string(),
            value: self.tab_rename.value.clone(),
            cursor: self.tab_rename.cursor,
        })
    }

    pub fn tab_labels(&self) -> Vec<TabLabel> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabLabel {
                title: tab.title.clone(),
                active: index == self.active_tab,
                activity: tab.panes.iter().any(|pane| pane.active),
                exited: !tab.panes.is_empty() && tab.panes.iter().all(|pane| pane.exited),
            })
            .collect()
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
        self.command_line.output_expanded
    }

    pub fn command_output_lines(&self) -> &[String] {
        &self.command_line.output_lines
    }

    pub fn command_running(&self) -> bool {
        self.command_line.running
    }

    pub fn input_scope_label(&self) -> &'static str {
        if self.command_line.focused {
            "command line"
        } else if self.selected().len() > 1 {
            "selected panes"
        } else {
            "focused pane"
        }
    }

    pub fn pane_folder(&self, index: usize) -> Option<&str> {
        self.active_tab()
            .and_then(|tab| tab.launch_plan.panes.get(index))
            .map(|pane| pane.folder_name.as_str())
    }

    pub fn pane_profile(&self, index: usize) -> Option<&str> {
        self.active_tab()
            .and_then(|tab| tab.launch_plan.panes.get(index))
            .map(|pane| pane.profile_name.as_str())
    }

    pub fn pane_worktree(&self, index: usize) -> Option<&str> {
        self.active_tab()
            .and_then(|tab| tab.launch_plan.panes.get(index))
            .and_then(|pane| pane.worktree_name.as_deref())
    }

    pub fn pane_auth(&self, index: usize) -> Option<&str> {
        self.active_tab()
            .and_then(|tab| tab.launch_plan.panes.get(index))
            .and_then(|pane| pane.auth_name.as_deref())
    }

    pub fn pane_conversation_footer(&self, index: usize, max_chars: usize) -> Option<String> {
        let tab = self.active_tab()?;
        let label = tab
            .launch_plan
            .panes
            .get(index)
            .and_then(|pane| pane.agent_label())?;
        let summary = tab
            .pane_conversation_summaries
            .get(index)
            .and_then(|summary| summary.as_deref())
            .unwrap_or("waiting for conversation");
        Some(truncate_chars(&format!("{label} | {summary}"), max_chars))
    }

    pub fn pane_usage_label(&self, index: usize) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(profile_name) = self
            .active_tab()
            .and_then(|tab| tab.launch_plan.panes.get(index))
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

    pub fn pane_count(&self) -> usize {
        self.panes().len()
    }

    pub fn pane_active(&self, index: usize) -> bool {
        self.panes().get(index).is_some_and(|pane| pane.active)
    }

    pub fn pane_exited(&self, index: usize) -> bool {
        self.panes().get(index).is_some_and(|pane| pane.exited)
    }

    pub fn pane_output_quiet(&self, index: usize) -> bool {
        self.panes()
            .get(index)
            .is_some_and(|pane| pane.output_quiet())
    }

    pub fn pane_cwd(&self, index: usize) -> Option<&std::path::Path> {
        self.panes().get(index).map(|pane| pane.cwd())
    }

    pub fn pane_screen(&self, index: usize) -> Option<&Screen> {
        self.panes().get(index).map(|pane| pane.screen())
    }

    pub fn pane_screen_lines(
        &mut self,
        index: usize,
        width: u16,
        height: u16,
        selection: Option<PaneSelection>,
    ) -> Vec<Line<'static>> {
        self.active_tab_mut()
            .map(|tab| tab.pane_screen_lines(index, width, height, selection))
            .unwrap_or_default()
    }

    pub fn sync_pane_sizes(&mut self) -> bool {
        let mut changed = false;
        let mut resize_error = None;
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };

        for index in 0..tab.rects.len() {
            let rect = tab.rects[index];
            let Some(pane) = tab.panes.get_mut(index) else {
                continue;
            };

            let rows = rect.height.saturating_sub(2).max(1);
            let cols = rect.width.saturating_sub(2).max(1);
            match pane.resize(rows, cols) {
                Ok(true) => {
                    tab.mark_pane_screen_dirty(index);
                    tab.refresh_conversation_summary(index);
                    changed = true;
                }
                Ok(false) => {}
                Err(error) => {
                    resize_error = Some(format!("resize failed: {error:#}"));
                }
            }
        }

        if let Some(status) = resize_error {
            self.status = status;
        }

        changed
    }

    fn sync_initial_pane_sizes(&mut self, terminal: &Tui) -> Result<()> {
        let size = terminal.size().context("failed to read terminal size")?;
        self.grid_area = Rect::new(0, 1, size.width, size.height.saturating_sub(3));
        let rects = self.pane_rects(self.grid_area);
        if let Some(tab) = self.active_tab_mut() {
            tab.rects = rects;
        }
        self.sync_pane_sizes();
        Ok(())
    }

    fn active_tab(&self) -> Option<&GridTab> {
        self.tabs.get(self.active_tab)
    }

    fn active_tab_mut(&mut self) -> Option<&mut GridTab> {
        self.tabs.get_mut(self.active_tab)
    }

    fn active_tab_title(&self) -> &str {
        self.active_tab()
            .map(|tab| tab.title.as_str())
            .unwrap_or("Grid")
    }

    fn active_tab_has_sleeping_panes(&self) -> bool {
        self.active_tab()
            .is_some_and(|tab| !tab.sleeping.is_empty())
    }

    fn active_pane_cwd(&self) -> Option<std::path::PathBuf> {
        let tab = self.active_tab()?;
        tab.panes
            .get(tab.focus)
            .map(|pane| pane.cwd().to_path_buf())
            .or_else(|| tab.panes.first().map(|pane| pane.cwd().to_path_buf()))
    }

    fn pane_location_for_event(&self, id: PaneId, generation: u64) -> Option<(usize, usize)> {
        self.tabs
            .iter()
            .enumerate()
            .find_map(|(tab_index, tab)| {
                tab.pane_index_for_event(id, generation)
                    .map(|pane_index| (tab_index, pane_index))
            })
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

fn toggle_selection(selected: &mut BTreeSet<usize>, index: usize) {
    if !selected.insert(index) {
        selected.remove(&index);
    }
}

fn empty_selection() -> &'static BTreeSet<usize> {
    static EMPTY: OnceLock<BTreeSet<usize>> = OnceLock::new();
    EMPTY.get_or_init(BTreeSet::new)
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

fn next_awake_pane_for(
    start: usize,
    pane_count: usize,
    sleeping: &BTreeSet<usize>,
) -> Option<usize> {
    if pane_count == 0 {
        return None;
    }

    (1..=pane_count)
        .map(|offset| (start + offset) % pane_count)
        .find(|index| !sleeping.contains(index))
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
    fn normalized_pane_name_trims_and_clears_empty_names() {
        assert_eq!(
            normalized_pane_name("  api server  "),
            Some("api server".into())
        );
        assert_eq!(normalized_pane_name("   "), None);
    }

    #[test]
    fn normalized_tab_title_trims_and_limits_titles() {
        assert_eq!(
            normalized_tab_title("  review   grid  "),
            Some("review grid".into())
        );
        assert_eq!(normalized_tab_title("   "), None);
        assert_eq!(
            normalized_tab_title("abcdefghijklmnopqrstuvwxyz1234567890extra"),
            Some("abcdefghijklmnopqrstuvwxyz1234567890extr".into())
        );
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

        assert_eq!(rows.len(), SettingsState::ROW_COUNT);
        assert_eq!(rows[1].label, "Activity badges");
        assert_eq!(rows[1].value, "on");
        assert_eq!(rows[SettingsState::BASE_ROW_COUNT].label, "Accent color");
        assert_eq!(
            rows[SettingsState::BASE_ROW_COUNT + 3].label,
            "Quiet border"
        );
        assert_eq!(rows[SettingsState::BASE_ROW_COUNT + 3].value, "magenta");
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
