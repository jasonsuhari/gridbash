use std::{
    collections::BTreeSet,
    env,
    io::{self, Stdout},
    sync::OnceLock,
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

pub struct App {
    cli: Cli,
    config: Config,
    initial_launch_plan: Option<LaunchPlan>,
    tabs: Vec<GridTab>,
    active_tab: usize,
    grid_area: Rect,
    settings: SettingsState,
    rename: RenameState,
    status: String,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    last_activity_decay: Instant,
    next_tab_number: usize,
    next_pane_id: usize,
}

struct GridTab {
    title: String,
    launch_plan: LaunchPlan,
    layout: GridLayout,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    rects: Vec<Rect>,
    broadcast: bool,
}

#[derive(Debug, Clone)]
pub struct TabLabel {
    pub title: String,
    pub active: bool,
    pub activity: bool,
    pub exited: bool,
}

#[derive(Debug, Clone, Default)]
struct RenameState {
    open: bool,
    input: String,
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

impl GridTab {
    fn new(title: String, launch_plan: LaunchPlan) -> Self {
        Self {
            title,
            layout: GridLayout::new(launch_plan.grid),
            launch_plan,
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            rects: Vec::new(),
            broadcast: false,
        }
    }
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let initial_launch_plan = resolve_direct_launch_plan(&cli, &config)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            cli,
            config,
            initial_launch_plan,
            tabs: Vec::new(),
            active_tab: 0,
            grid_area: Rect::default(),
            settings: SettingsState::default(),
            rename: RenameState::default(),
            status: "Alt+t tabs | Alt+n new tab | Alt+r rename | Alt+arrows move | Alt+b broadcast"
                .into(),
            event_tx,
            event_rx,
            last_activity_decay: Instant::now(),
            next_tab_number: 1,
            next_pane_id: 0,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.run_in_terminal(&mut terminal);
        teardown_terminal(&mut terminal)?;
        result
    }

    fn run_in_terminal(&mut self, terminal: &mut Tui) -> Result<()> {
        let initial_plan = if let Some(plan) = self.initial_launch_plan.take() {
            plan
        } else {
            let current_dir = resolved_current_dir()?;
            let mut composer = Composer::new(&self.config, current_dir);
            let Some(plan) =
                composer.run(terminal, &mut self.config, self.cli.config.as_deref())?
            else {
                return Ok(());
            };
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

        for spec in &pane_specs {
            tab.panes.push(self.spawn_pane_spec(spec, 0)?);
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len().saturating_sub(1);
        self.status = format!("opened tab {}", title);
        Ok(())
    }

    fn next_tab_title(&mut self) -> String {
        let title = format!("Grid {}", self.next_tab_number);
        self.next_tab_number += 1;
        title
    }

    fn spawn_pane_spec(&mut self, spec: &PaneLaunchSpec, generation: u64) -> Result<PtyPane> {
        let launch = spec.resolved_command()?;
        let pane_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;

        let pane = PtyPane::spawn(
            &spec.profile_name,
            pane_id,
            generation,
            &launch.command,
            &launch.args,
            &spec.cwd,
            self.event_tx.clone(),
        )?;
        Ok(pane)
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        loop {
            self.drain_pty_events();
            self.decay_activity();

            terminal.draw(|frame| {
                let draw_state = ui::draw(frame, self);
                self.grid_area = draw_state.grid_area;
                if let Some(tab) = self.active_tab_mut() {
                    tab.rects = draw_state.pane_rects;
                }
            })?;
            self.sync_pane_sizes();

            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if self.handle_key(terminal, key)? {
                            break;
                        }
                    }
                    Event::Resize(_, _) => {}
                    Event::Paste(text) => {
                        if self.rename.open {
                            self.append_rename_input(&text);
                        } else if !self.settings.open {
                            self.route_input(text.as_bytes())?;
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn drain_pty_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                PtyEvent::Output {
                    pane,
                    generation,
                    bytes,
                } => {
                    if let Some(target) = self.find_pane_mut(pane, generation) {
                        target.process_output(&bytes);
                    }
                }
                PtyEvent::Exited { pane, generation } => {
                    if let Some(target) = self.find_pane_mut(pane, generation) {
                        target.exited = true;
                    }
                }
            }
        }

        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.poll_exit();
            }
        }
    }

    fn decay_activity(&mut self) {
        if self.last_activity_decay.elapsed() < Duration::from_millis(250) {
            return;
        }

        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.active = false;
            }
        }
        self.last_activity_decay = Instant::now();
    }

    fn find_pane_mut(&mut self, id: PaneId, generation: u64) -> Option<&mut PtyPane> {
        self.tabs
            .iter_mut()
            .flat_map(|tab| tab.panes.iter_mut())
            .find(|pane| pane.id() == id && pane.generation() == generation)
    }

    fn handle_key(&mut self, terminal: &mut Tui, key: KeyEvent) -> Result<bool> {
        if self.rename.open {
            return self.handle_rename_key(key);
        }

        if self.settings.open {
            return self.handle_settings_key(key);
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            if let Some(quit) = self.handle_app_key(terminal, key)? {
                return Ok(quit);
            }
        }

        if let Some(bytes) = terminal_key_bytes(key) {
            self.route_input(&bytes)?;
        }
        Ok(false)
    }

    fn handle_app_key(&mut self, terminal: &mut Tui, key: KeyEvent) -> Result<Option<bool>> {
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(terminal, ch),
            KeyCode::Left => {
                self.focus_previous();
                self.status = format!("focused pane {}", self.focus() + 1);
                Ok(Some(false))
            }
            KeyCode::Right => {
                self.focus_next();
                self.status = format!("focused pane {}", self.focus() + 1);
                Ok(Some(false))
            }
            KeyCode::Up => {
                self.focus_in_grid(-1);
                self.status = format!("focused pane {}", self.focus() + 1);
                Ok(Some(false))
            }
            KeyCode::Down => {
                self.focus_in_grid(1);
                self.status = format!("focused pane {}", self.focus() + 1);
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn handle_alt_char(&mut self, terminal: &mut Tui, ch: char) -> Result<Option<bool>> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            'q' => Ok(Some(true)),
            't' => {
                self.next_tab();
                Ok(Some(false))
            }
            'n' => {
                self.open_new_tab(terminal)?;
                Ok(Some(false))
            }
            'r' => {
                self.open_rename();
                Ok(Some(false))
            }
            'b' => {
                let enabled = {
                    let Some(tab) = self.active_tab_mut() else {
                        return Ok(Some(false));
                    };
                    tab.broadcast = !tab.broadcast;
                    tab.broadcast
                };
                self.status = if enabled {
                    "broadcast selected: on".into()
                } else {
                    "broadcast selected: off".into()
                };
                Ok(Some(false))
            }
            's' => {
                let selected_count = {
                    let Some(tab) = self.active_tab_mut() else {
                        return Ok(Some(false));
                    };
                    toggle_selection(&mut tab.selected, tab.focus);
                    tab.selected.len()
                };
                self.status = format!("selected {} panes", selected_count);
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
            'o' => {
                self.settings.open = true;
                self.status = "settings open".into();
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
        let mut composer = Composer::new(&self.config, current_dir);

        match composer.run(terminal, &mut self.config, self.cli.config.as_deref())? {
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

        self.active_tab = (self.active_tab + 1) % self.tabs.len();
        let title = self.active_tab_title().to_string();
        self.status = format!("active tab {title}");
    }

    fn open_rename(&mut self) {
        let Some(title) = self.active_tab().map(|tab| tab.title.clone()) else {
            return;
        };

        self.rename.input = title;
        self.rename.open = true;
        self.status = "rename tab".into();
    }

    fn handle_rename_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        {
            return Ok(true);
        }

        match key.code {
            KeyCode::Enter => {
                if let Some(title) = clean_tab_title(&self.rename.input) {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.title = title.clone();
                    }
                    self.rename.open = false;
                    self.status = format!("renamed tab to {title}");
                } else {
                    self.status = "tab name cannot be empty".into();
                }
            }
            KeyCode::Esc => {
                self.rename.open = false;
                self.status = "rename canceled".into();
            }
            KeyCode::Backspace => {
                self.rename.input.pop();
            }
            KeyCode::Delete => {
                self.rename.input.clear();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.append_rename_char(ch);
            }
            _ => {}
        }

        Ok(false)
    }

    fn append_rename_input(&mut self, text: &str) {
        for ch in text.chars() {
            self.append_rename_char(ch);
        }
    }

    fn append_rename_char(&mut self, ch: char) {
        if ch.is_control() || self.rename.input.chars().count() >= 60 {
            return;
        }

        self.rename.input.push(ch);
    }

    fn active_pane_cwd(&self) -> Option<std::path::PathBuf> {
        let tab = self.active_tab()?;
        tab.panes
            .get(tab.focus)
            .map(|pane| pane.cwd().to_path_buf())
            .or_else(|| tab.panes.first().map(|pane| pane.cwd().to_path_buf()))
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(true);
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O'))
        {
            self.settings.open = false;
            self.status = "settings closed".into();
            return Ok(false);
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings.open = false;
                self.status = "settings closed".into();
            }
            KeyCode::Up => self.settings.move_cursor(-1),
            KeyCode::Down => self.settings.move_cursor(1),
            KeyCode::Left | KeyCode::Char('-') => self.settings.adjust(-1),
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => self.settings.adjust(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.settings.activate(),
            _ => {}
        }

        Ok(false)
    }

    fn route_input(&mut self, bytes: &[u8]) -> Result<()> {
        let targets = self.input_targets();
        let Some(tab) = self.active_tab_mut() else {
            return Ok(());
        };

        for index in targets {
            tab.panes
                .get(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?
                .write(bytes)?;
        }
        Ok(())
    }

    fn input_targets(&self) -> Vec<usize> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        if tab.panes.is_empty() {
            return Vec::new();
        }

        if tab.broadcast && !tab.selected.is_empty() {
            tab.selected.iter().copied().collect()
        } else {
            vec![tab.focus.min(tab.panes.len().saturating_sub(1))]
        }
    }

    fn focus_next(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.panes.is_empty() {
            return;
        }
        tab.focus = (tab.focus + 1) % tab.panes.len();
    }

    fn focus_previous(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.panes.is_empty() {
            return;
        }
        tab.focus = if tab.focus == 0 {
            tab.panes.len() - 1
        } else {
            tab.focus - 1
        };
    }

    fn focus_in_grid(&mut self, row_delta: isize) {
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
        if candidate < tab.panes.len() {
            tab.focus = candidate;
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

    pub fn selected(&self) -> &BTreeSet<usize> {
        if let Some(tab) = self.active_tab() {
            &tab.selected
        } else {
            empty_selection()
        }
    }

    pub fn broadcast(&self) -> bool {
        self.active_tab().is_some_and(|tab| tab.broadcast)
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

    pub fn rename_open(&self) -> bool {
        self.rename.open
    }

    pub fn rename_input(&self) -> &str {
        &self.rename.input
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

    pub fn pane_worktree(&self, index: usize) -> Option<&str> {
        self.active_tab()
            .and_then(|tab| tab.launch_plan.panes.get(index))
            .and_then(|pane| pane.worktree_name.as_deref())
    }

    pub fn sync_pane_sizes(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let mut resize_error = None;

        for (index, rect) in tab.rects.iter().enumerate() {
            if let Some(pane) = tab.panes.get_mut(index) {
                let rows = rect.height.saturating_sub(2).max(1);
                let cols = rect.width.saturating_sub(2).max(1);
                if let Err(error) = pane.resize(rows, cols) {
                    resize_error = Some(format!("resize failed: {error:#}"));
                }
            }
        }

        if let Some(status) = resize_error {
            self.status = status;
        }
    }

    fn sync_initial_pane_sizes(&mut self, terminal: &Tui) -> Result<()> {
        let size = terminal.size().context("failed to read terminal size")?;
        self.grid_area = Rect::new(0, 1, size.width, size.height.saturating_sub(2));
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

fn empty_selection() -> &'static BTreeSet<usize> {
    static EMPTY: OnceLock<BTreeSet<usize>> = OnceLock::new();
    EMPTY.get_or_init(BTreeSet::new)
}

fn clean_tab_title(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = normalized.chars().take(40).collect::<String>();
    (!title.is_empty()).then_some(title)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_tab_titles() {
        assert_eq!(
            clean_tab_title("  review   grid  "),
            Some("review grid".into())
        );
        assert_eq!(clean_tab_title("   "), None);
        assert_eq!(
            clean_tab_title("abcdefghijklmnopqrstuvwxyz1234567890extra"),
            Some("abcdefghijklmnopqrstuvwxyz1234567890extr".into())
        );
    }
}
