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
    config::Config,
    layout::{GridLayout, GridSize, PaneId},
    profiles::{available_profiles, find_profile},
    pty::{PtyEvent, PtyPane},
    ui,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub struct App {
    cli: Cli,
    config: Config,
    cwd: PathBuf,
    profile_names: Vec<String>,
    target_profile_index: usize,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    rects: Vec<Rect>,
    broadcast: bool,
    status: String,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    last_activity_decay: Instant,
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let grid = resolve_grid(&cli)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let profile_name = resolve_profile_name(&cli, &config);
        find_profile(&config, &profile_name)?;
        let cwd = cli
            .cwd
            .clone()
            .unwrap_or(env::current_dir().context("failed to resolve current directory")?);
        let mut profile_names = available_profile_names(&config);
        if !profile_names.iter().any(|name| name == &profile_name) {
            profile_names.push(profile_name.clone());
        }
        let target_profile_index = profile_names
            .iter()
            .position(|name| name == &profile_name)
            .unwrap_or(0);

        Ok(Self {
            cli,
            config,
            cwd,
            profile_names,
            target_profile_index,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            rects: Vec::new(),
            broadcast: false,
            status: "mouse selects text | Alt+t terminal | Alt+Enter apply | Alt+b broadcast"
                .into(),
            event_tx,
            event_rx,
            last_activity_decay: Instant::now(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        self.spawn_initial_panes()?;

        let mut terminal = setup_terminal()?;
        self.sync_initial_pane_sizes(&terminal)?;
        let result = self.run_loop(&mut terminal);
        teardown_terminal(&mut terminal)?;
        result
    }

    fn spawn_initial_panes(&mut self) -> Result<()> {
        let pane_count = self
            .cli
            .count
            .unwrap_or_else(|| self.layout.size().count())
            .clamp(1, 100);
        let cwd = self.cwd.clone();
        let profile_name = self.target_profile_name().to_string();

        for index in 0..pane_count {
            self.spawn_pane(index, &profile_name, cwd.clone())?;
        }

        Ok(())
    }

    fn spawn_pane(&mut self, index: usize, profile_name: &str, cwd: PathBuf) -> Result<()> {
        let profile = find_profile(&self.config, profile_name)?;
        let launch = profile.resolved_command()?;
        let title = format!("{} {}", profile.display_name(profile_name), index + 1);
        let pane = PtyPane::spawn(
            PaneId(index),
            0,
            profile_name,
            title,
            &launch.command,
            &launch.args,
            &cwd,
            self.event_tx.clone(),
        )?;
        self.panes.push(pane);
        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        loop {
            self.drain_pty_events();
            self.decay_activity();

            terminal.draw(|frame| {
                let draw_state = ui::draw(frame, self);
                self.grid_area = draw_state.grid_area;
                self.rects = draw_state.pane_rects;
            })?;
            self.sync_pane_sizes();

            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if self.handle_key(key)? {
                            break;
                        }
                    }
                    Event::Resize(_, _) => {}
                    Event::Paste(text) => self.route_input(text.as_bytes())?,
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
                    if let Some(target) = self
                        .panes
                        .iter_mut()
                        .find(|p| p.id() == pane && p.generation() == generation)
                    {
                        target.process_output(&bytes);
                    }
                }
                PtyEvent::Exited { pane, generation } => {
                    if let Some(target) = self
                        .panes
                        .iter_mut()
                        .find(|p| p.id() == pane && p.generation() == generation)
                    {
                        target.exited = true;
                    }
                }
            }
        }

        for pane in &mut self.panes {
            pane.poll_exit();
        }
    }

    fn decay_activity(&mut self) {
        if self.last_activity_decay.elapsed() < Duration::from_millis(250) {
            return;
        }

        for pane in &mut self.panes {
            pane.active = false;
        }
        self.last_activity_decay = Instant::now();
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::ALT) {
            if let Some(quit) = self.handle_app_key(key)? {
                return Ok(quit);
            }
        }

        if let Some(bytes) = terminal_key_bytes(key) {
            self.route_input(&bytes)?;
        }
        Ok(false)
    }

    fn handle_app_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(ch, shifted),
            KeyCode::Enter => {
                self.apply_target_profile();
                Ok(Some(false))
            }
            KeyCode::Left if shifted => {
                self.layout.adjust_focused(self.focus, -1, 0);
                self.status = "focused column narrowed".into();
                Ok(Some(false))
            }
            KeyCode::Right if shifted => {
                self.layout.adjust_focused(self.focus, 1, 0);
                self.status = "focused column widened".into();
                Ok(Some(false))
            }
            KeyCode::Up if shifted => {
                self.layout.adjust_focused(self.focus, 0, -1);
                self.status = "focused row shortened".into();
                Ok(Some(false))
            }
            KeyCode::Down if shifted => {
                self.layout.adjust_focused(self.focus, 0, 1);
                self.status = "focused row heightened".into();
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

    fn handle_alt_char(&mut self, ch: char, shifted: bool) -> Result<Option<bool>> {
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
            's' | ' ' => {
                toggle_selection(&mut self.selected, self.focus);
                self.status = format!("selected {} panes", self.selected.len());
                Ok(Some(false))
            }
            'a' => {
                self.selected = (0..self.panes.len()).collect();
                self.status = format!("selected {} panes", self.selected.len());
                Ok(Some(false))
            }
            'c' => {
                self.selected.clear();
                self.status = "selection cleared".into();
                Ok(Some(false))
            }
            'p' => {
                let profiles = available_profiles(&self.config);
                let summary = profiles
                    .iter()
                    .take(8)
                    .map(|(name, ok)| format!("{}{}", if *ok { "" } else { "!" }, name))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.status = format!("profiles: {summary}");
                Ok(Some(false))
            }
            't' => {
                self.cycle_target_profile(if shifted { -1 } else { 1 });
                Ok(Some(false))
            }
            'd' => {
                self.save_target_as_default();
                Ok(Some(false))
            }
            'r' => {
                self.layout.reset_equal();
                self.status = "grid reset to equal rows/columns".into();
                Ok(Some(false))
            }
            '1'..='9' | '0' => {
                let Some(index) = pane_digit_index(lower) else {
                    return Ok(Some(false));
                };
                if index < self.panes.len() {
                    self.focus = index;
                    self.status = format!("focused pane {}", index + 1);
                }
                Ok(Some(false))
            }
            _ => Ok(None),
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
        if self.broadcast && !self.selected.is_empty() {
            self.selected.iter().copied().collect()
        } else {
            vec![self.focus.min(self.panes.len().saturating_sub(1))]
        }
    }

    fn target_profile_name(&self) -> &str {
        self.profile_names
            .get(self.target_profile_index)
            .map(String::as_str)
            .unwrap_or("git-bash")
    }

    fn cycle_target_profile(&mut self, delta: isize) {
        if self.profile_names.is_empty() {
            self.status = "no available profiles".into();
            return;
        }

        let count = self.profile_names.len() as isize;
        let current = self.target_profile_index as isize;
        self.target_profile_index = (current + delta).rem_euclid(count) as usize;
        self.status = format!(
            "target terminal: {} | Alt+Enter apply | Alt+d default",
            self.target_profile_name()
        );
    }

    fn apply_target_profile(&mut self) {
        let profile_name = self.target_profile_name().to_string();
        let targets = self.restart_targets();
        for index in &targets {
            if let Err(error) = self.replace_pane(*index, &profile_name) {
                self.status = format!("failed to launch {profile_name}: {error:#}");
                return;
            }
        }

        self.status = format!("restarted {} pane(s) with {profile_name}", targets.len());
    }

    fn save_target_as_default(&mut self) {
        let profile_name = self.target_profile_name().to_string();
        self.config.set_default_profile(profile_name.clone());
        match self.config.save(self.cli.config.as_deref()) {
            Ok(path) => {
                self.status = format!("default terminal: {profile_name} ({})", path.display());
            }
            Err(error) => {
                self.status = format!("failed to save default: {error:#}");
            }
        }
    }

    fn restart_targets(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            vec![self.focus.min(self.panes.len().saturating_sub(1))]
        } else {
            self.selected
                .iter()
                .copied()
                .filter(|index| *index < self.panes.len())
                .collect()
        }
    }

    fn replace_pane(&mut self, index: usize, profile_name: &str) -> Result<()> {
        let profile = find_profile(&self.config, profile_name)?;
        let launch = profile.resolved_command()?;
        let title = format!("{} {}", profile.display_name(profile_name), index + 1);
        let mut pane = PtyPane::spawn(
            PaneId(index),
            self.panes
                .get(index)
                .map(|pane| pane.generation().saturating_add(1))
                .unwrap_or(0),
            profile_name,
            title,
            &launch.command,
            &launch.args,
            &self.cwd,
            self.event_tx.clone(),
        )?;

        if let Some(rect) = self.rects.get(index) {
            let rows = rect.height.saturating_sub(2).max(1);
            let cols = rect.width.saturating_sub(2).max(1);
            pane.resize(rows, cols)?;
        }

        let _old = std::mem::replace(
            self.panes
                .get_mut(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?,
            pane,
        );
        Ok(())
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

    pub fn target_profile(&self) -> &str {
        self.target_profile_name()
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

fn resolve_profile_name(cli: &Cli, config: &Config) -> String {
    cli.profile
        .clone()
        .or_else(|| env::var("GRIDBASH_PROFILE").ok())
        .or_else(|| config.defaults.profile.clone())
        .unwrap_or_else(|| "git-bash".into())
}

fn available_profile_names(config: &Config) -> Vec<String> {
    available_profiles(config)
        .into_iter()
        .filter_map(|(name, available)| available.then_some(name))
        .collect()
}

fn toggle_selection(selected: &mut BTreeSet<usize>, index: usize) {
    if !selected.insert(index) {
        selected.remove(&index);
    }
}

fn pane_digit_index(ch: char) -> Option<usize> {
    match ch {
        '1'..='9' => Some(ch as usize - '1' as usize),
        '0' => Some(9),
        _ => None,
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
