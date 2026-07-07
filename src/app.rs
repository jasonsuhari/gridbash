use std::{
    collections::BTreeSet,
    env,
    io::{self, Stdout},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio::sync::mpsc;

use crate::{
    cli::{Cli, GridMode},
    composer::Composer,
    config::Config,
    layout::{GridLayout, GridSize, PaneId},
    profiles::find_profile,
    pty::{PtyEvent, PtyPane},
    setup::{LaunchPlan, PaneLaunchSpec},
    ui,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const TODO_INPUT_LIMIT: usize = 240;
const MIN_TODO_IDLE_SECONDS: u64 = 15;
const MAX_TODO_IDLE_SECONDS: u64 = 600;
const TODO_IDLE_STEP_SECONDS: u64 = 15;

pub struct App {
    config: Config,
    config_path: Option<PathBuf>,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    pane_idle: Vec<PaneIdleState>,
    focus: usize,
    selected: BTreeSet<usize>,
    rects: Vec<Rect>,
    broadcast: bool,
    settings: SettingsState,
    follow_up: Option<FollowUpPromptState>,
    status: String,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    last_activity_decay: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyOutcome {
    Continue,
    Render,
    Quit,
}

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub selected: bool,
    pub group: SettingsGroup,
    pub value_kind: SettingsValueKind,
    pub editing: bool,
    pub label: String,
    pub value: String,
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
    AccentColor,
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
struct SettingsState {
    open: bool,
    cursor: usize,
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
    accent_index: usize,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            cursor: 0,
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
            accent_index: 0,
        }
    }
}

impl SettingsState {
    const ACCENTS: [&'static str; 4] = ["cyan", "yellow", "green", "magenta"];

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
            Some(SettingsTarget::AccentColor) => {
                let count = Self::ACCENTS.len() as isize;
                self.accent_index =
                    (self.accent_index as isize + delta as isize).rem_euclid(count) as usize;
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
            SettingsTarget::AccentColor,
        ]);
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
        rows.push(self.row(
            SettingsTarget::AccentColor,
            SettingsGroup::Theme,
            SettingsValueKind::Choice,
            "Accent color",
            Self::ACCENTS[self.accent_index].to_string(),
            "cycle the UI highlight",
        ));

        rows
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
        let launch_plan = resolve_direct_launch_plan(&cli, &config)?;
        let config_path = cli.config.clone();
        let grid = launch_plan
            .as_ref()
            .map(|plan| plan.grid)
            .unwrap_or(GridSize {
                rows: 2,
                columns: 3,
            });
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            settings: SettingsState::from_config(&config),
            config,
            config_path,
            launch_plan,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            pane_idle: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            rects: Vec::new(),
            broadcast: false,
            follow_up: None,
            status:
                "Alt+arrows move | Alt+s select | Alt+a all/none | Alt+b broadcast | Alt+o settings"
                    .into(),
            event_tx,
            event_rx,
            last_activity_decay: Instant::now(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.run_in_terminal(&mut terminal);
        teardown_terminal(&mut terminal)?;
        result
    }

    fn run_in_terminal(&mut self, terminal: &mut Tui) -> Result<()> {
        if self.launch_plan.is_none() {
            let current_dir = resolved_current_dir()?;
            let mut composer = Composer::new(current_dir);
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
        self.pane_idle.clear();
        self.follow_up = None;

        for (index, spec) in plan.panes.iter().enumerate() {
            self.spawn_pane_spec(index, spec, 0)?;
        }

        Ok(())
    }

    fn spawn_pane_spec(
        &mut self,
        index: usize,
        spec: &PaneLaunchSpec,
        generation: u64,
    ) -> Result<()> {
        let launch = spec.resolved_command()?;
        let pane = PtyPane::spawn(
            PaneId(index),
            generation,
            &launch.command,
            &launch.args,
            &spec.cwd,
            self.event_tx.clone(),
        )?;
        self.panes.push(pane);
        self.pane_idle.push(PaneIdleState::new(Instant::now()));
        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut needs_render = true;

        loop {
            needs_render |= self.drain_pty_events();
            needs_render |= self.decay_activity();
            needs_render |= self.update_follow_up_prompt();

            if needs_render {
                terminal.draw(|frame| {
                    let draw_state = ui::draw(frame, self);
                    self.grid_area = draw_state.grid_area;
                    self.rects = draw_state.pane_rects;
                })?;
                self.sync_pane_sizes();
                needs_render = false;
            }

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
                    Event::Paste(text) if self.settings.editing_todo() => {
                        if self.settings.insert_todo_text(&text) {
                            needs_render = true;
                        }
                    }
                    Event::Paste(text) if !self.settings.open && self.follow_up.is_none() => {
                        self.route_input(text.as_bytes())?;
                    }
                    _ => {}
                }
            }
        }

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
        if self.follow_up.is_some() {
            return self.handle_follow_up_key(key);
        }

        if self.settings.open {
            return self.handle_settings_key(key);
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

        if let Some(bytes) = terminal_key_bytes(key) {
            self.route_input(&bytes)?;
        }
        Ok(KeyOutcome::Continue)
    }

    fn handle_app_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(ch),
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

    fn handle_alt_char(&mut self, ch: char) -> Result<Option<bool>> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            'q' => Ok(Some(true)),
            'b' => {
                self.broadcast = !self.broadcast;
                self.status = if self.broadcast {
                    "broadcast selected: on".into()
                } else {
                    "broadcast selected: off".into()
                };
                Ok(Some(false))
            }
            's' => {
                toggle_selection(&mut self.selected, self.focus);
                self.status = format!("selected {} panes", self.selected.len());
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
            'o' => {
                self.settings.open = true;
                self.status = "settings open".into();
                Ok(Some(false))
            }
            _ => Ok(None),
        }
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
            if pane.exited {
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

    fn route_input(&mut self, bytes: &[u8]) -> Result<()> {
        let targets = self.input_targets();
        for index in targets {
            self.panes
                .get(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?
                .write(bytes)?;
            self.mark_pane_touched(index);
        }
        Ok(())
    }

    fn input_targets(&self) -> Vec<usize> {
        if self.broadcast && !self.selected.is_empty() {
            self.selected.iter().copied().collect()
        } else {
            vec![self.focus.min(self.panes.len().saturating_sub(1))]
        }
    }

    fn focus_next(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.focus = (self.focus + 1) % self.panes.len();
    }

    fn focus_previous(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.focus = if self.focus == 0 {
            self.panes.len() - 1
        } else {
            self.focus - 1
        };
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
        if candidate < self.panes.len() {
            self.focus = candidate;
        }
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

    pub fn broadcast(&self) -> bool {
        self.broadcast
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

    pub fn pane_worktree(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.worktree_name.as_deref())
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

fn resolve_direct_launch_plan(cli: &Cli, config: &Config) -> Result<Option<LaunchPlan>> {
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

    Ok(Some(LaunchPlan::legacy(
        profile_name,
        profile,
        cwd,
        pane_count,
        grid,
    )))
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

fn toggle_selection(selected: &mut BTreeSet<usize>, index: usize) {
    if !selected.insert(index) {
        selected.remove(&index);
    }
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

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to create terminal")
}

fn teardown_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
