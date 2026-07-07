use std::{
    collections::BTreeSet,
    env,
    io::{self, Stdout},
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
const ACTIVITY_DECAY_INTERVAL: Duration = Duration::from_millis(250);
const OUTPUT_QUIET_AFTER: Duration = Duration::from_secs(3);

pub struct App {
    config: Config,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    rects: Vec<Rect>,
    broadcast: bool,
    settings: SettingsState,
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
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
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
        let launch_plan = resolve_direct_launch_plan(&cli, &config)?;
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
            launch_plan,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            rects: Vec::new(),
            broadcast: false,
            settings: SettingsState::default(),
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
        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut needs_render = true;

        loop {
            needs_render |= self.drain_pty_events();
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

    pub fn quiet_pane_count(&self) -> usize {
        self.panes.iter().filter(|pane| pane.output_quiet()).count()
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
