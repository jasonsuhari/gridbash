use std::{
    collections::BTreeSet,
    env,
    io::{self, Stdout},
    path::PathBuf,
    process::Command,
    sync::mpsc as std_mpsc,
    thread,
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
    auth::{self, AgentKind, AuthProfile},
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

pub struct App {
    config: Config,
    config_path: Option<PathBuf>,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    rects: Vec<Rect>,
    broadcast: bool,
    settings: SettingsState,
    auth_profiles: Vec<AuthProfile>,
    auth_refresh_rx: Option<std_mpsc::Receiver<Result<Vec<AuthProfile>, String>>>,
    status: String,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    last_activity_decay: Instant,
}

#[derive(Debug, Clone)]
enum KeyOutcome {
    Continue,
    Render,
    AuthLogin(AuthProfile),
    Quit,
}

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub selected: bool,
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
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
    accent_index: usize,
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
                "sample switch",
            ),
            self.row(
                1,
                "Activity badges",
                switch_value(self.activity_badges),
                "sample switch",
            ),
            self.row(
                2,
                "Confirm before quit",
                switch_value(self.confirm_quit),
                "sample switch",
            ),
            self.row(
                3,
                "Pane density",
                self.pane_density.to_string(),
                "-/+ sample stepper",
            ),
            self.row(
                4,
                "Scrollback rows",
                self.scrollback.to_string(),
                "-/+ sample stepper",
            ),
            self.row(
                5,
                "Refresh delay",
                format!("{} ms", self.refresh_ms),
                "-/+ sample stepper",
            ),
            self.row(
                6,
                "Accent color",
                Self::ACCENTS[self.accent_index].to_string(),
                "sample choice",
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
        let config_path = cli.config.clone();
        let mut launch_plan = resolve_direct_launch_plan(&cli, &config)?;
        if let Some(plan) = launch_plan.as_mut() {
            apply_auth_defaults(plan, &config)?;
        }
        let grid = launch_plan
            .as_ref()
            .map(|plan| plan.grid)
            .unwrap_or(GridSize {
                rows: 2,
                columns: 3,
            });
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            config,
            config_path,
            launch_plan,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            rects: Vec::new(),
            broadcast: false,
            settings: SettingsState::default(),
            auth_profiles: Vec::new(),
            auth_refresh_rx: None,
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
            &spec.env,
            &spec.cwd,
            self.event_tx.clone(),
        )?;
        self.panes.push(pane);
        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut needs_render = true;

        loop {
            needs_render |= self.drain_pty_events();
            needs_render |= self.drain_auth_refresh();
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

            if event::poll(INPUT_POLL_INTERVAL)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match self.handle_key(key)? {
                            KeyOutcome::Continue => {}
                            KeyOutcome::Render => needs_render = true,
                            KeyOutcome::AuthLogin(profile) => {
                                self.run_auth_login(terminal, profile)?;
                                needs_render = true;
                            }
                            KeyOutcome::Quit => break,
                        }
                    }
                    Event::Resize(_, _) => needs_render = true,
                    Event::Paste(text) if !self.settings.open => {
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
                    {
                        if !target.exited {
                            target.exited = true;
                            changed = true;
                        }
                    }
                }
            }
        }

        for pane in &mut self.panes {
            changed |= pane.poll_exit();
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
                if self.settings.tab == SettingsTab::Auth {
                    self.start_auth_refresh();
                }
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

    pub fn settings_tab(&self) -> SettingsTab {
        self.settings.tab
    }

    pub fn settings_rows(&self) -> Vec<SettingsRow> {
        self.settings.rows()
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

    pub fn pane_auth(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.auth_name.as_deref())
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

fn valid_auth_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')
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

fn suspend_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
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
    }
}
