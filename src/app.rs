use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    io::{self, Stdout, Write},
    path::{Path, PathBuf},
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
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio::sync::mpsc;
use vt100::Screen;

use crate::{
    cli::{Cli, GridMode},
    composer::Composer,
    config::Config,
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
const COMMAND_OUTPUT_MAX_LINES: usize = 2000;

pub struct App {
    config: Config,
    worktrees: Option<ManagedWorktreeOptions>,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    pane_names: Vec<Option<String>>,
    text_selection: Option<MouseSelection>,
    sleeping: BTreeSet<usize>,
    rects: Vec<Rect>,
    mouse_enabled: bool,
    command_line: CommandLineState,
    command_tx: mpsc::UnboundedSender<CommandRunEvent>,
    command_rx: mpsc::UnboundedReceiver<CommandRunEvent>,
    settings: SettingsState,
    rename: RenamePaneState,
    status: String,
    next_pane_id: usize,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    usage_tx: std_mpsc::Sender<UsageEvent>,
    usage_rx: std_mpsc::Receiver<UsageEvent>,
    profile_usage: BTreeMap<String, String>,
    api_spend_label: Option<String>,
    last_activity_decay: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyOutcome {
    Continue,
    Render,
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

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub selected: bool,
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
}

#[derive(Debug, Clone)]
pub struct RenamePaneView {
    pub pane_index: usize,
    pub pane_label: String,
    pub value: String,
    pub cursor: usize,
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
    cursor: usize,
    compact_titles: bool,
    activity_badges: bool,
    confirm_quit: bool,
    pane_density: i32,
    scrollback: i32,
    refresh_ms: i32,
    accent_index: usize,
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
            cursor: 0,
            compact_titles: false,
            activity_badges: true,
            confirm_quit: false,
            pane_density: 2,
            scrollback: 10_000,
            refresh_ms: 16,
            accent_index: 0,
        }
    }
}

impl SettingsState {
    const ROW_COUNT: usize = 7;
    const ACCENTS: [&'static str; 4] = ["cyan", "yellow", "green", "magenta"];

    fn move_cursor(&mut self, delta: isize) {
        let current = self.cursor as isize;
        self.cursor = (current + delta).clamp(0, Self::ROW_COUNT as isize - 1) as usize;
    }

    fn activate(&mut self) {
        match self.cursor {
            0 => self.compact_titles = !self.compact_titles,
            1 => self.activity_badges = !self.activity_badges,
            2 => self.confirm_quit = !self.confirm_quit,
            6 => self.adjust(1),
            _ => self.adjust(1),
        }
    }

    fn adjust(&mut self, delta: i32) {
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
            6 => {
                let count = Self::ACCENTS.len() as isize;
                self.accent_index =
                    (self.accent_index as isize + delta as isize).rem_euclid(count) as usize;
            }
            _ => {}
        }
    }

    fn rows(&self) -> Vec<SettingsRow> {
        vec![
            self.row(
                0,
                "Compact pane titles",
                switch_value(self.compact_titles),
                "shorter labels in pane chrome",
            ),
            self.row(
                1,
                "Activity badges",
                switch_value(self.activity_badges),
                "show live and selected state",
            ),
            self.row(
                2,
                "Confirm before quit",
                switch_value(self.confirm_quit),
                "extra guard for Alt+q",
            ),
            self.row(
                3,
                "Pane density",
                self.pane_density.to_string(),
                "spacing scale from 1 to 5",
            ),
            self.row(
                4,
                "Scrollback rows",
                self.scrollback.to_string(),
                "history budget per pane",
            ),
            self.row(
                5,
                "Refresh delay",
                format!("{} ms", self.refresh_ms),
                "render loop throttle",
            ),
            self.row(
                6,
                "Accent color",
                Self::ACCENTS[self.accent_index].to_string(),
                "cycle the UI highlight",
            ),
        ]
    }

    fn row(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        hint: &'static str,
    ) -> SettingsRow {
        SettingsRow {
            selected: self.cursor == index,
            label,
            value,
            hint,
        }
    }
}

fn switch_value(enabled: bool) -> String {
    if enabled { "on".into() } else { "off".into() }
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let startup_cwd = resolved_current_dir()?;
        let worktrees = cli
            .worktrees
            .then(|| ManagedWorktreeOptions::new(cli.worktree_prefix.clone()))
            .transpose()?;
        let launch_plan = resolve_direct_launch_plan(&cli, &config, worktrees.as_ref())?;
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
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        Ok(Self {
            config,
            worktrees,
            launch_plan,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            pane_names: Vec::new(),
            text_selection: None,
            sleeping: BTreeSet::new(),
            rects: Vec::new(),
            mouse_enabled,
            command_line: CommandLineState::new(startup_cwd),
            command_tx,
            command_rx,
            settings: SettingsState::default(),
            rename: RenamePaneState::default(),
            status: if mouse_enabled {
                "Drag copies within pane | Alt+arrows move | Alt+Shift+arrows resize | Alt+r rename | Alt+e output | Alt+x swap | Alt+z sleep | Alt+o settings"
                    .into()
            } else {
                "Alt+arrows move | Alt+Shift+arrows resize | Alt+s select | Alt+r rename | Alt+e output | Alt+x swap | Alt+z sleep | Alt+o settings"
                    .into()
            },
            next_pane_id: 0,
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
            self.set_launch_plan(plan);
        }

        self.spawn_initial_panes()?;
        self.sync_initial_pane_sizes(terminal)?;
        self.run_loop(terminal)
    }

    fn set_launch_plan(&mut self, plan: LaunchPlan) {
        self.layout = GridLayout::new(plan.grid);
        self.launch_plan = Some(plan);
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
            })
            .collect::<Vec<_>>();
        usage::spawn_usage_monitor(targets, self.usage_tx.clone());
    }

    fn spawn_pane_spec(&mut self, spec: &PaneLaunchSpec) -> Result<()> {
        let launch = spec.resolved_command()?;
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        let pane = PtyPane::spawn(
            id,
            0,
            &launch.command,
            &launch.args,
            &spec.cwd,
            self.event_tx.clone(),
        )?;
        self.panes.push(pane);
        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut needs_render = true;
        let mut mouse_capture_enabled = self.mouse_enabled;

        loop {
            needs_render |= self.drain_pty_events();
            needs_render |= self.drain_usage_events();
            needs_render |= self.drain_command_events();
            needs_render |= self.decay_activity();

            if needs_render {
                terminal.draw(|frame| {
                    let draw_state = ui::draw(frame, self);
                    self.grid_area = draw_state.grid_area;
                    self.rects = draw_state.pane_rects;
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
                            KeyOutcome::Quit => break,
                        }
                    }
                    Event::Resize(_, _) => needs_render = true,
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
                        if (self.mouse_enabled || !self.sleeping.is_empty())
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
                    if let Some(target) = self
                        .panes
                        .iter_mut()
                        .find(|p| p.id() == pane && p.generation() == generation)
                    {
                        target.process_output(&bytes);
                        changed = true;
                    }
                }
                PtyEvent::Exited { pane, generation } => {
                    if let Some(target) = self
                        .panes
                        .iter_mut()
                        .find(|p| p.id() == pane && p.generation() == generation)
                        && !target.exited
                    {
                        target.exited = true;
                        changed = true;
                    }
                }
            }
        }

        for pane in &mut self.panes {
            changed |= pane.poll_exit();
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

    fn decay_activity(&mut self) -> bool {
        if self.last_activity_decay.elapsed() < Duration::from_millis(250) {
            return false;
        }

        let changed = self.panes.iter().any(|pane| pane.active);
        for pane in &mut self.panes {
            pane.active = false;
        }
        self.last_activity_decay = Instant::now();
        changed
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if self.rename.open {
            return self.handle_rename_key(key);
        }

        let selection_cleared = self.clear_text_selection();

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

        if self.command_line.focused {
            return Ok(if self.handle_command_key(key)? {
                KeyOutcome::Render
            } else {
                KeyOutcome::Continue
            });
        }

        if let Some(bytes) = terminal_key_bytes(key) {
            self.route_input(&bytes)?;
        }
        Ok(if selection_cleared {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
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

    fn handle_alt_char(&mut self, ch: char, _modifiers: KeyModifiers) -> Result<Option<bool>> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            'q' => Ok(Some(true)),
            's' => {
                if self.command_line.focused {
                    self.status = "command line focused".into();
                } else {
                    toggle_selection(&mut self.selected, self.focus);
                    self.status = format!("selected {} panes", self.selected.len());
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
                self.status = "settings open".into();
                Ok(Some(false))
            }
            'r' => {
                self.begin_rename();
                Ok(Some(false))
            }
            _ => Ok(None),
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
        if let Some(selection) = self.text_selection {
            self.text_selection = Some(MouseSelection {
                pane: swapped_index(selection.pane, first, second),
                ..selection
            });
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
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O'))
        {
            self.settings.open = false;
            self.status = "settings closed".into();
            return Ok(KeyOutcome::Render);
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

        if let Some(index) = pane_at(&self.rects, mouse.column, mouse.row)
            && self.sleeping.remove(&index)
        {
            self.focus = index;
            self.status = format!("woke pane {}", index + 1);
            return Ok(true);
        }

        if !self.mouse_enabled {
            return Ok(false);
        }

        match mouse.kind {
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
        if targets.is_empty() {
            return;
        }

        let should_sleep = targets.iter().any(|index| !self.sleeping.contains(index));
        if should_sleep {
            for index in &targets {
                self.sleeping.insert(*index);
                self.selected.remove(index);
            }

            if self.sleeping.contains(&self.focus)
                && let Some(index) = self.next_awake_pane(self.focus)
            {
                self.focus = index;
            }
        } else {
            for index in &targets {
                self.sleeping.remove(index);
            }
            self.focus = targets[0];
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

    fn route_input(&mut self, bytes: &[u8]) -> Result<()> {
        let targets = self.input_targets();
        for index in targets {
            self.panes
                .get(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?
                .write(bytes)?;
        }
        Ok(())
    }

    fn input_targets(&self) -> Vec<usize> {
        awake_input_targets_for(self.focus, &self.selected, self.panes.len(), &self.sleeping)
    }

    fn target_panes(&self) -> Vec<usize> {
        input_targets_for(self.focus, &self.selected, self.panes.len())
    }

    fn focus_next(&mut self) {
        if self.panes.is_empty() {
            return;
        }

        if self.command_line.focused {
            if let Some(candidate) =
                (0..self.panes.len()).find(|index| !self.sleeping.contains(index))
            {
                self.command_line.focused = false;
                self.focus = candidate;
            }
            return;
        }

        if let Some(candidate) =
            ((self.focus + 1)..self.panes.len()).find(|index| !self.sleeping.contains(index))
        {
            self.focus = candidate;
            return;
        }

        self.command_line.focused = true;
    }

    fn focus_previous(&mut self) {
        if self.panes.is_empty() {
            return;
        }

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

        if let Some(candidate) = (0..self.focus)
            .rev()
            .find(|index| !self.sleeping.contains(index))
        {
            self.focus = candidate;
            return;
        }

        self.command_line.focused = true;
    }

    fn focus_in_grid(&mut self, row_delta: isize) {
        if self.panes.is_empty() {
            return;
        }

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

        let columns = self.layout.size().columns;
        let candidate = if row_delta.is_negative() {
            self.focus.saturating_sub(columns)
        } else {
            self.focus.saturating_add(columns)
        };
        if candidate < self.panes.len() && !self.sleeping.contains(&candidate) {
            self.focus = candidate;
        } else if row_delta.is_positive() {
            self.command_line.focused = true;
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

    pub fn settings_open(&self) -> bool {
        self.settings.open
    }

    pub fn settings_rows(&self) -> Vec<SettingsRow> {
        self.settings.rows()
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
        self.grid_area = Rect::new(0, 0, size.width, size.height.saturating_sub(2));
        self.rects = self.pane_rects(self.grid_area);
        self.sync_pane_sizes();
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
    use super::*;

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
