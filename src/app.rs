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
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio::sync::mpsc;

use crate::{
    cli::{Cli, GridMode},
    config::Config,
    layout::{GridSize, PaneId, pane_at},
    profiles::{available_profiles, find_profile},
    pty::{PtyEvent, PtyPane},
    ui,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Command,
}

pub struct App {
    cli: Cli,
    config: Config,
    grid: GridSize,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    rects: Vec<Rect>,
    mode: Mode,
    broadcast: bool,
    command_filter: String,
    status: String,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    last_activity_decay: Instant,
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let grid = resolve_grid(&cli)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            cli,
            config,
            grid,
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            rects: Vec::new(),
            mode: Mode::Normal,
            broadcast: false,
            command_filter: String::new(),
            status: "Esc command mode | Ctrl-click select | Ctrl-b broadcast selected".into(),
            event_tx,
            event_rx,
            last_activity_decay: Instant::now(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        self.spawn_initial_panes()?;

        let mut terminal = setup_terminal(!self.cli.no_mouse)?;
        let result = self.run_loop(&mut terminal);
        teardown_terminal(&mut terminal, !self.cli.no_mouse)?;
        result
    }

    fn spawn_initial_panes(&mut self) -> Result<()> {
        let pane_count = self
            .cli
            .count
            .unwrap_or_else(|| self.grid.count())
            .clamp(1, 100);
        let cwd = self
            .cli
            .cwd
            .clone()
            .unwrap_or(env::current_dir().context("failed to resolve current directory")?);

        for index in 0..pane_count {
            self.spawn_pane(index, &self.cli.profile.clone(), cwd.clone())?;
        }

        Ok(())
    }

    fn spawn_pane(&mut self, index: usize, profile_name: &str, cwd: PathBuf) -> Result<()> {
        let profile = find_profile(&self.config, profile_name)?;
        let (command, args) = profile.resolved_command()?;
        let title = format!("{} {}", profile.display_name(profile_name), index + 1);
        let pane = PtyPane::spawn(
            PaneId(index),
            profile_name,
            title,
            &command,
            &args,
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
                self.rects = ui::draw(frame, self);
            })?;
            self.sync_pane_sizes();

            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if self.handle_key(key)? {
                            break;
                        }
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse(mouse.kind, mouse.column, mouse.row, mouse.modifiers)
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
                PtyEvent::Output { pane, bytes } => {
                    if let Some(target) = self.panes.iter_mut().find(|p| p.id() == pane) {
                        target.process_output(&bytes);
                    }
                }
                PtyEvent::Exited { pane } => {
                    if let Some(target) = self.panes.iter_mut().find(|p| p.id() == pane) {
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
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('b') => {
                    self.broadcast = !self.broadcast;
                    self.status = if self.broadcast {
                        "broadcast selected: on".into()
                    } else {
                        "broadcast selected: off".into()
                    };
                    return Ok(false);
                }
                KeyCode::Char('a') => {
                    self.selected = (0..self.panes.len()).collect();
                    self.status = format!("selected {} panes", self.selected.len());
                    return Ok(false);
                }
                _ => {}
            }
        }

        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Command => self.handle_command_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Command;
                self.command_filter.clear();
                self.status =
                    "command mode: launch, profiles, select, broadcast, help, quit".into();
            }
            KeyCode::Tab => self.focus_next(),
            KeyCode::BackTab => self.focus_previous(),
            KeyCode::Enter => self.route_input(b"\r")?,
            KeyCode::Backspace => self.route_input(&[0x7f])?,
            KeyCode::Char(ch) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    if let Some(byte) = control_byte(ch) {
                        self.route_input(&[byte])?;
                    }
                } else {
                    let mut buf = [0; 4];
                    self.route_input(ch.encode_utf8(&mut buf).as_bytes())?;
                }
            }
            KeyCode::Left => self.route_input(b"\x1b[D")?,
            KeyCode::Right => self.route_input(b"\x1b[C")?,
            KeyCode::Up => self.route_input(b"\x1b[A")?,
            KeyCode::Down => self.route_input(b"\x1b[B")?,
            _ => {}
        }
        Ok(false)
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command_filter.clear();
                self.status = "normal mode".into();
            }
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('b') => {
                self.broadcast = !self.broadcast;
                self.status = if self.broadcast {
                    "broadcast selected: on".into()
                } else {
                    "broadcast selected: off".into()
                };
            }
            KeyCode::Char('c') => {
                self.selected.clear();
                self.status = "selection cleared".into();
            }
            KeyCode::Char('a') => {
                self.selected = (0..self.panes.len()).collect();
                self.status = format!("selected {} panes", self.selected.len());
            }
            KeyCode::Char('p') => {
                let profiles = available_profiles(&self.config);
                let summary = profiles
                    .iter()
                    .take(8)
                    .map(|(name, ok)| format!("{}{}", if *ok { "" } else { "!" }, name))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.status = format!("profiles: {summary}");
            }
            KeyCode::Char(ch) => {
                self.command_filter.push(ch);
                self.status = format!("command filter: {}", self.command_filter);
            }
            KeyCode::Backspace => {
                self.command_filter.pop();
                self.status = format!("command filter: {}", self.command_filter);
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Right => self.focus_next(),
            KeyCode::BackTab | KeyCode::Up | KeyCode::Left => self.focus_previous(),
            _ => {}
        }
        Ok(false)
    }

    fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16, modifiers: KeyModifiers) {
        let Some(index) = pane_at(&self.rects, x, y) else {
            return;
        };

        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    toggle_selection(&mut self.selected, index);
                    self.focus = index;
                    self.status = format!("selected {} panes", self.selected.len());
                } else if modifiers.contains(KeyModifiers::SHIFT) {
                    self.select_range(self.focus, index);
                    self.focus = index;
                    self.status = format!("selected {} panes", self.selected.len());
                } else {
                    self.focus = index;
                    self.selected.clear();
                    self.status = format!("focused pane {}", index + 1);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                toggle_selection(&mut self.selected, index);
                self.focus = index;
                self.status = format!("selected {} panes", self.selected.len());
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                self.broadcast = !self.broadcast;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.selected.insert(index);
                self.focus = index;
            }
            _ => {}
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

    fn select_range(&mut self, start: usize, end: usize) {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };

        for index in start..=end {
            if index < self.panes.len() {
                self.selected.insert(index);
            }
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

    pub fn grid(&self) -> GridSize {
        self.grid
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

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn broadcast(&self) -> bool {
        self.broadcast
    }

    pub fn status(&self) -> &str {
        &self.status
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
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
