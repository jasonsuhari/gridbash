use std::{
    env,
    io::Stdout,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    auth::{self, AuthProfile},
    config::Config,
    layout::GridSize,
    profiles::{Profile, find_profile, is_agent_profile, is_terminal_profile, startup_profiles},
    setup::LaunchPlan,
    worktrees::ManagedWorktreeOptions,
};

type ComposerTerminal = Terminal<CrosstermBackend<Stdout>>;

const DEFAULT_ROWS: usize = 2;
const DEFAULT_COLUMNS: usize = 3;
const MAX_DIMENSION: usize = 10;
const STARTUP_PREVIEW: PreviewPalette = PreviewPalette {
    border: Color::Rgb(95, 214, 150),
    light: Color::Rgb(43, 128, 84),
    mid: Color::Rgb(28, 96, 65),
    dark: Color::Rgb(15, 65, 49),
};
const RESIZE_PREVIEW: PreviewPalette = PreviewPalette {
    border: Color::Rgb(88, 166, 255),
    light: Color::Rgb(47, 129, 247),
    mid: Color::Rgb(31, 91, 191),
    dark: Color::Rgb(15, 50, 110),
};

pub struct Composer {
    current_dir: PathBuf,
    project_input: String,
    project_error: Option<String>,
    editing_project: bool,
    worktree_options: ManagedWorktreeOptions,
    worktrees_enabled: bool,
    picker: GridPicker,
    profiles: Vec<StartupProfile>,
    profile_cursor: usize,
    auth_profiles: Vec<AuthProfile>,
    auth_cursor: usize,
    active_field: StartupField,
}

#[derive(Debug, Clone)]
struct StartupProfile {
    name: String,
    title: String,
    profile: Profile,
    agent: bool,
    terminal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridPickerMode {
    Startup,
    Resize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridPickerAction {
    Continue,
    Confirm(GridSize),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct GridPicker {
    initial: GridSize,
    rows: usize,
    columns: usize,
    active_field: DimensionField,
}

#[derive(Debug, Clone, Copy)]
struct PreviewPalette {
    border: Color,
    light: Color,
    mid: Color,
    dark: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DimensionField {
    Rows,
    Columns,
}

enum ComposerEvent {
    Continue,
    Launch(LaunchPlan),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupField {
    Profile,
    Auth,
    Rows,
    Columns,
    Worktrees,
    Project,
}

impl StartupField {
    const ALL: [Self; 6] = [
        Self::Profile,
        Self::Auth,
        Self::Rows,
        Self::Columns,
        Self::Worktrees,
        Self::Project,
    ];
}

impl Composer {
    pub fn new(
        current_dir: PathBuf,
        worktrees: Option<ManagedWorktreeOptions>,
        config: &Config,
    ) -> Result<Self> {
        let grid = GridSize {
            rows: DEFAULT_ROWS,
            columns: DEFAULT_COLUMNS,
        };
        let profiles = startup_profiles(config)
            .into_iter()
            .map(|(name, profile)| {
                let agent = is_agent_profile(&name, &profile);
                StartupProfile {
                    title: profile.display_name(&name),
                    agent,
                    terminal: is_terminal_profile(&name) && !agent,
                    name,
                    profile,
                }
            })
            .collect::<Vec<_>>();
        if profiles.is_empty() {
            return Err(anyhow!(
                "no agent or terminal profiles are available; install Codex, Claude, or a supported shell"
            ));
        }

        let has_agent = profiles.iter().any(|profile| profile.agent);
        let preferred = env::var("GRIDBASH_PROFILE").ok().or_else(|| {
            config.defaults.profile.clone().filter(|name| {
                !profiles
                    .iter()
                    .find(|profile| profile.name == *name)
                    .is_some_and(|profile| profile.terminal)
                    || !has_agent
            })
        });
        let profile_cursor = preferred
            .as_deref()
            .and_then(|preferred| {
                profiles
                    .iter()
                    .position(|profile| profile.name == preferred)
            })
            .unwrap_or(0);
        let current_dir = current_dir.canonicalize().unwrap_or(current_dir);
        let project_input = display_path(&current_dir);
        let worktrees_enabled = worktrees.is_some();
        let worktree_options = worktrees.unwrap_or(ManagedWorktreeOptions::new("gridbash".into())?);

        Ok(Self {
            current_dir,
            project_input,
            project_error: None,
            editing_project: false,
            worktree_options,
            worktrees_enabled,
            picker: GridPicker::new(grid),
            profiles,
            profile_cursor,
            auth_profiles: auth::discover_profiles(&config.auth)?,
            auth_cursor: 0,
            active_field: StartupField::Profile,
        })
    }

    pub fn run(
        &mut self,
        terminal: &mut ComposerTerminal,
        config: &Config,
    ) -> Result<Option<LaunchPlan>> {
        loop {
            terminal.draw(|frame| self.draw(frame, config))?;

            if !event::poll(Duration::from_millis(50))? {
                continue;
            }

            let result = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key, config),
                Event::Paste(text) if self.editing_project => {
                    self.project_input.push_str(&text);
                    ComposerEvent::Continue
                }
                _ => ComposerEvent::Continue,
            };

            match result {
                ComposerEvent::Continue => {}
                ComposerEvent::Launch(plan) => return Ok(Some(plan)),
                ComposerEvent::Quit => return Ok(None),
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent, config: &Config) -> ComposerEvent {
        if self.editing_project {
            match key.code {
                KeyCode::Esc => {
                    self.editing_project = false;
                    self.project_input = display_path(&self.current_dir);
                    self.project_error = None;
                }
                KeyCode::Enter => self.commit_project_input(),
                KeyCode::Backspace => {
                    self.project_input.pop();
                    self.project_error = None;
                }
                KeyCode::Char(ch) => {
                    self.project_input.push(ch);
                    self.project_error = None;
                }
                _ => {}
            }
            return ComposerEvent::Continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ComposerEvent::Quit,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.move_field(-1);
                ComposerEvent::Continue
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.move_field(1);
                ComposerEvent::Continue
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.adjust_active(-1);
                ComposerEvent::Continue
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.adjust_active(1);
                ComposerEvent::Continue
            }
            KeyCode::Char('w') => {
                self.worktrees_enabled = !self.worktrees_enabled;
                ComposerEvent::Continue
            }
            KeyCode::Char('e') if self.active_field == StartupField::Project => {
                self.editing_project = true;
                self.project_error = None;
                ComposerEvent::Continue
            }
            KeyCode::Enter => match self.launch_plan(config) {
                Ok(plan) => ComposerEvent::Launch(plan),
                Err(error) => {
                    self.project_error = Some(format!("{error:#}"));
                    ComposerEvent::Continue
                }
            },
            _ => ComposerEvent::Continue,
        }
    }

    fn launch_plan(&self, config: &Config) -> Result<LaunchPlan> {
        let selected = self.selected_profile();
        let profile = find_profile(config, &selected.name)?;
        let grid = self.picker.grid();
        let worktrees = self.worktrees_enabled.then_some(&self.worktree_options);
        let mut plan = LaunchPlan::from_launch_options(
            selected.name.clone(),
            profile,
            self.current_dir.clone(),
            grid.count(),
            grid,
            worktrees,
        )?;

        if let Some(profile) = self.selected_auth_profile() {
            for pane in &mut plan.panes {
                pane.auth_name = Some(profile.name.clone());
            }
        }
        Ok(plan)
    }

    fn selected_profile(&self) -> &StartupProfile {
        &self.profiles[self.profile_cursor.min(self.profiles.len() - 1)]
    }

    fn compatible_auth_profiles(&self) -> Vec<&AuthProfile> {
        let kind = self.selected_profile().profile.agent_kind;
        self.auth_profiles
            .iter()
            .filter(|profile| Some(profile.kind) == kind)
            .collect()
    }

    fn selected_auth_profile(&self) -> Option<&AuthProfile> {
        self.auth_cursor
            .checked_sub(1)
            .and_then(|index| self.compatible_auth_profiles().get(index).copied())
    }

    fn move_field(&mut self, delta: isize) {
        let current = StartupField::ALL
            .iter()
            .position(|field| *field == self.active_field)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(StartupField::ALL.len() as isize);
        self.active_field = StartupField::ALL[next as usize];
    }

    fn adjust_active(&mut self, delta: isize) {
        match self.active_field {
            StartupField::Profile => {
                self.profile_cursor = (self.profile_cursor as isize + delta)
                    .rem_euclid(self.profiles.len() as isize)
                    as usize;
                self.auth_cursor = 0;
            }
            StartupField::Auth => {
                let choices = self.compatible_auth_profiles().len() + 1;
                self.auth_cursor =
                    (self.auth_cursor as isize + delta).rem_euclid(choices as isize) as usize;
            }
            StartupField::Rows => {
                self.picker.active_field = DimensionField::Rows;
                self.picker.adjust_active(delta);
            }
            StartupField::Columns => {
                self.picker.active_field = DimensionField::Columns;
                self.picker.adjust_active(delta);
            }
            StartupField::Worktrees => self.worktrees_enabled = !self.worktrees_enabled,
            StartupField::Project => {}
        }
    }

    fn commit_project_input(&mut self) {
        match resolve_project_path(&self.project_input, &self.current_dir) {
            Ok(path) => {
                self.current_dir = path;
                self.project_input = display_path(&self.current_dir);
                self.project_error = None;
                self.editing_project = false;
            }
            Err(error) => self.project_error = Some(error.to_string()),
        }
    }

    fn draw(&self, frame: &mut Frame<'_>, config: &Config) {
        let area = frame.area();
        frame.render_widget(background(), area);
        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Create Agent Workspace ")
            .border_style(Style::default().fg(Color::Cyan))
            .style(panel_style());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(8),
                Constraint::Min(5),
                Constraint::Length(4),
            ])
            .split(inner);
        self.draw_workspace_header(frame, chunks[0]);
        self.draw_workspace_fields(frame, chunks[1], config);
        self.picker
            .draw_preview(frame, chunks[2], GridPickerMode::Startup);
        self.draw_workspace_controls(frame, chunks[3]);
    }

    fn draw_workspace_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let selected = self.selected_profile();
        let mode = if selected.terminal {
            "Raw terminal grid"
        } else if selected.agent {
            "Managed agent workspace"
        } else {
            "Custom command workspace"
        };
        let detail = if selected.terminal {
            "Shells stay unmodified; managed auth applies only to agents GridBash launches."
        } else if selected.agent {
            "GridBash launches, isolates, monitors, and coordinates every pane."
        } else {
            "GridBash arranges this command in real PTYs; the command owns its login behavior."
        };
        let lines = vec![
            Line::from(Span::styled(
                mode,
                Style::default()
                    .fg(if selected.terminal {
                        Color::Gray
                    } else {
                        Color::LightCyan
                    })
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(detail, Style::default().fg(Color::Gray))),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(panel_style()),
            area,
        );
    }

    fn draw_workspace_fields(&self, frame: &mut Frame<'_>, area: Rect, config: &Config) {
        let selected = self.selected_profile();
        let profile_kind = if selected.terminal {
            "Raw terminal"
        } else if selected.agent {
            "Agent"
        } else {
            "Command"
        };
        let (auth_value, auth_hint) = self.auth_display(config);
        let project_value = if self.editing_project {
            format!("{}▌", self.project_input)
        } else {
            self.project_input.clone()
        };
        let project_hint = self
            .project_error
            .clone()
            .unwrap_or_else(|| "e edits the project folder".into());
        let lines = vec![
            workspace_row(
                self.active_field == StartupField::Profile,
                "Profile",
                format!("{profile_kind} · {}", selected.title),
                format!("{} ({})", selected.profile.command, selected.name),
            ),
            workspace_row(
                self.active_field == StartupField::Auth,
                "Auth",
                auth_value,
                auth_hint,
            ),
            workspace_row(
                self.active_field == StartupField::Rows,
                "Rows",
                self.picker.rows.to_string(),
                "1–10",
            ),
            workspace_row(
                self.active_field == StartupField::Columns,
                "Columns",
                self.picker.columns.to_string(),
                format!("{} panes total", self.picker.grid().count()),
            ),
            workspace_row(
                self.active_field == StartupField::Worktrees,
                "Worktrees",
                if self.worktrees_enabled { "on" } else { "off" },
                if self.worktrees_enabled {
                    "one isolated git worktree per pane"
                } else {
                    "all panes use the project folder"
                },
            ),
            workspace_row(
                self.active_field == StartupField::Project,
                "Project",
                project_value,
                project_hint,
            ),
        ];
        frame.render_widget(Paragraph::new(lines).style(panel_style()), area);
    }

    fn draw_workspace_controls(&self, frame: &mut Frame<'_>, area: Rect) {
        let lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
                Span::raw(" field  "),
                Span::styled("Left/Right", Style::default().fg(Color::Yellow)),
                Span::raw(" change  "),
                Span::styled("e", Style::default().fg(Color::Yellow)),
                Span::raw(" edit project  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(" launch  "),
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::raw(" quit"),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(panel_style()),
            area,
        );
    }

    fn auth_display(&self, config: &Config) -> (String, String) {
        let selected = self.selected_profile();
        if selected.terminal {
            return (
                "unmanaged".into(),
                "normal shell environment; no command interception".into(),
            );
        }
        let Some(kind) = selected.profile.agent_kind else {
            return (
                "tool login".into(),
                "this agent manages its own authentication".into(),
            );
        };
        if let Some(profile) = self.selected_auth_profile() {
            let account = profile
                .account_label
                .as_deref()
                .map(|label| format!(" · {label}"))
                .unwrap_or_default();
            return (
                profile.name.clone(),
                format!(
                    "{} profile · {}{account}",
                    kind.display_name(),
                    profile.status_label()
                ),
            );
        }
        if config.auth.auto_cycle {
            return (
                "launch policy".into(),
                "round-robin across ready profiles for this agent".into(),
            );
        }
        match config.auth.defaults.get(kind) {
            Some(name) => (
                "launch policy".into(),
                format!("configured {} default: {name}", kind.display_name()),
            ),
            None => (
                "normal login".into(),
                format!("use the default {} account", kind.display_name()),
            ),
        }
    }
}

fn workspace_row(
    selected: bool,
    label: impl Into<String>,
    value: impl Into<String>,
    hint: impl Into<String>,
) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let value_style = if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::LightCyan)
    };
    Line::from(vec![
        Span::styled(
            format!("{marker} {:<10}", label.into()),
            Style::default().fg(if selected { Color::Yellow } else { Color::Gray }),
        ),
        Span::styled(format!(" {} ", value.into()), value_style),
        Span::raw("  "),
        Span::styled(hint.into(), Style::default().fg(Color::DarkGray)),
    ])
}

fn resolve_project_path(input: &str, base: &Path) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("project folder cannot be empty"));
    }
    let path = PathBuf::from(input);
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    if !path.is_dir() {
        return Err(anyhow!(
            "project folder does not exist: {}",
            display_path(&path)
        ));
    }
    path.canonicalize()
        .with_context(|| format!("failed to resolve project folder {}", display_path(&path)))
}

impl GridPicker {
    pub fn new(grid: GridSize) -> Self {
        Self {
            initial: grid,
            rows: grid.rows,
            columns: grid.columns,
            active_field: DimensionField::Rows,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> GridPickerAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => GridPickerAction::Cancel,
            KeyCode::Enter => GridPickerAction::Confirm(self.grid()),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                self.active_field = DimensionField::Rows;
                GridPickerAction::Continue
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.active_field = DimensionField::Columns;
                GridPickerAction::Continue
            }
            KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('k') => {
                self.adjust_active(1);
                GridPickerAction::Continue
            }
            KeyCode::Down | KeyCode::Char('-') | KeyCode::Char('j') => {
                self.adjust_active(-1);
                GridPickerAction::Continue
            }
            KeyCode::Char('r') => {
                self.active_field = DimensionField::Rows;
                GridPickerAction::Continue
            }
            KeyCode::Char('c') => {
                self.active_field = DimensionField::Columns;
                GridPickerAction::Continue
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                self.set_active_from_digit(ch);
                GridPickerAction::Continue
            }
            _ => GridPickerAction::Continue,
        }
    }

    pub fn grid(&self) -> GridSize {
        GridSize {
            rows: self.rows,
            columns: self.columns,
        }
    }

    fn adjust_active(&mut self, delta: isize) {
        let value = match self.active_field {
            DimensionField::Rows => &mut self.rows,
            DimensionField::Columns => &mut self.columns,
        };
        *value = (*value as isize + delta).clamp(1, MAX_DIMENSION as isize) as usize;
    }

    fn set_active_from_digit(&mut self, ch: char) {
        let Some(mut value) = ch.to_digit(10).map(|digit| digit as usize) else {
            return;
        };
        if value == 0 {
            value = MAX_DIMENSION;
        }

        match self.active_field {
            DimensionField::Rows => self.rows = value.min(MAX_DIMENSION),
            DimensionField::Columns => self.columns = value.min(MAX_DIMENSION),
        }
    }

    pub fn draw(&self, frame: &mut Frame<'_>, mode: GridPickerMode, cwd: Option<&Path>) {
        let area = frame.area();
        frame.render_widget(background(), area);

        let panel = area;
        frame.render_widget(Clear, panel);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(match mode {
                GridPickerMode::Startup => " GridBash Startup ",
                GridPickerMode::Resize => " GridBash Resize ",
            })
            .border_style(Style::default().fg(Color::Cyan))
            .style(panel_style());
        let inner = block.inner(panel);
        frame.render_widget(block, panel);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(5),
            ])
            .split(inner);

        self.draw_header(frame, chunks[0], mode, cwd);
        self.draw_preview(frame, chunks[1], mode);
        self.draw_controls(frame, chunks[2], mode);
    }

    fn draw_header(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        mode: GridPickerMode,
        cwd: Option<&Path>,
    ) {
        let context = match mode {
            GridPickerMode::Startup => cwd
                .map(display_path)
                .map(|cwd| ("cwd ", cwd))
                .unwrap_or_else(|| ("", String::new())),
            GridPickerMode::Resize => (
                "current ",
                format!("{}x{}", self.initial.rows, self.initial.columns),
            ),
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    match mode {
                        GridPickerMode::Startup => "Choose grid dimensions",
                        GridPickerMode::Resize => "Resize active grid",
                    },
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{} panes", self.rows * self.columns),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(context.0, Style::default().fg(Color::DarkGray)),
                Span::styled(context.1, Style::default().fg(Color::Gray)),
            ]),
        ];

        frame.render_widget(Paragraph::new(lines).style(panel_style()), area);
    }

    fn draw_preview(&self, frame: &mut Frame<'_>, area: Rect, mode: GridPickerMode) {
        let preview_area = inset(area, 1, 0);
        let palette = match mode {
            GridPickerMode::Startup => STARTUP_PREVIEW,
            GridPickerMode::Resize => RESIZE_PREVIEW,
        };
        let rects = square_preview_rects(preview_area, self.grid());

        for (index, rect) in rects.into_iter().enumerate() {
            if rect.width == 0 || rect.height == 0 {
                continue;
            }

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .style(Style::default().bg(palette.dark));
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            frame.render_widget(dithered_fill(index, inner, palette), inner);
        }
    }

    fn draw_controls(&self, frame: &mut Frame<'_>, area: Rect, mode: GridPickerMode) {
        let lines = vec![
            Line::from(""),
            Line::from(vec![
                control_box(self.active_field == DimensionField::Rows, self.rows),
                Span::raw(" "),
                Span::styled("rows", Style::default().fg(Color::Gray)),
                Span::styled("  x  ", Style::default().fg(Color::DarkGray)),
                control_box(self.active_field == DimensionField::Columns, self.columns),
                Span::raw(" "),
                Span::styled("cols", Style::default().fg(Color::Gray)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
                Span::raw(" change  "),
                Span::styled("Left/Right", Style::default().fg(Color::Yellow)),
                Span::raw(" switch  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(match mode {
                    GridPickerMode::Startup => " launch",
                    GridPickerMode::Resize => " apply  ",
                }),
                if mode == GridPickerMode::Resize {
                    Span::styled("Esc", Style::default().fg(Color::Yellow))
                } else {
                    Span::raw("")
                },
                if mode == GridPickerMode::Resize {
                    Span::raw(" cancel")
                } else {
                    Span::raw("")
                },
            ]),
        ];

        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(panel_style()),
            area,
        );
    }
}

fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn control_box(active: bool, value: usize) -> Span<'static> {
    let style = if active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightCyan)
            .bg(Color::Rgb(20, 35, 44))
            .add_modifier(Modifier::BOLD)
    };

    Span::styled(format!(" {value:>2} "), style)
}

fn background() -> Paragraph<'static> {
    Paragraph::new("").style(Style::default().bg(Color::Rgb(7, 11, 15)))
}

fn panel_style() -> Style {
    Style::default()
        .fg(Color::Rgb(230, 237, 243))
        .bg(Color::Rgb(11, 15, 20))
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(x),
        y: area.y.saturating_add(y),
        width: area.width.saturating_sub(x.saturating_mul(2)),
        height: area.height.saturating_sub(y.saturating_mul(2)),
    }
}

fn square_preview_rects(area: Rect, grid: GridSize) -> Vec<Rect> {
    let rows = grid.rows as u16;
    let columns = grid.columns as u16;
    if rows == 0 || columns == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let gap_y = if area.height >= rows.saturating_mul(2).saturating_sub(1) {
        1
    } else {
        0
    };
    let gap_x = if area.width >= columns.saturating_mul(4).saturating_sub(2) {
        2
    } else if area.width >= columns.saturating_mul(3).saturating_sub(1) {
        1
    } else {
        0
    };

    let row_gaps = rows.saturating_sub(1).saturating_mul(gap_y);
    let column_gaps = columns.saturating_sub(1).saturating_mul(gap_x);
    let height_fit = area.height.saturating_sub(row_gaps) / rows;
    let width_fit = area.width.saturating_sub(column_gaps) / columns / 2;
    let side_height = height_fit.min(width_fit).max(1);
    let side_width = side_height.saturating_mul(2).max(1);
    let total_height = rows
        .saturating_mul(side_height)
        .saturating_add(row_gaps)
        .min(area.height);
    let total_width = columns
        .saturating_mul(side_width)
        .saturating_add(column_gaps)
        .min(area.width);
    let start_y = area.y + area.height.saturating_sub(total_height) / 2;
    let start_x = area.x + area.width.saturating_sub(total_width) / 2;

    let mut rects = Vec::with_capacity(grid.count());
    for row in 0..rows {
        for column in 0..columns {
            rects.push(Rect {
                x: start_x + column.saturating_mul(side_width.saturating_add(gap_x)),
                y: start_y + row.saturating_mul(side_height.saturating_add(gap_y)),
                width: side_width,
                height: side_height,
            });
        }
    }
    rects
}

fn dithered_fill(index: usize, rect: Rect, palette: PreviewPalette) -> Paragraph<'static> {
    let lines = (0..rect.height)
        .map(|y| {
            let mut spans = Vec::with_capacity(rect.width as usize);
            for x in 0..rect.width {
                let bg = match (x.saturating_mul(3) + y.saturating_mul(5) + index as u16) % 6 {
                    0 | 3 => palette.light,
                    1 | 4 => palette.mid,
                    _ => palette.dark,
                };
                spans.push(Span::styled(" ", Style::default().bg(bg)));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    Paragraph::new(lines).style(Style::default().bg(palette.dark))
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;

    use crate::auth::AgentKind;

    use super::*;

    #[test]
    fn launch_plan_uses_managed_agent_workspace_defaults() {
        if env::var_os("GRIDBASH_PROFILE").is_some() {
            return;
        }

        let mut config = Config::default();
        let profile = "managed-test";
        config.profiles.insert(
            profile.into(),
            Profile {
                command: env::current_exe()
                    .expect("test executable")
                    .display()
                    .to_string(),
                args: Vec::new(),
                title: Some("Managed test agent".into()),
                agent_kind: Some(AgentKind::Codex),
            },
        );
        config.set_default_profile(profile);
        config.auth.home = Some(env::temp_dir().join("gridbash-composer-no-auth-profiles"));
        let current_dir = env::current_dir().expect("current dir");

        let composer = Composer::new(current_dir.clone(), None, &config).expect("composer");
        let plan = composer.launch_plan(&config).expect("launch plan");

        assert_eq!(plan.grid.rows, DEFAULT_ROWS);
        assert_eq!(plan.grid.columns, DEFAULT_COLUMNS);
        assert_eq!(plan.panes.len(), DEFAULT_ROWS * DEFAULT_COLUMNS);
        assert!(
            plan.panes
                .iter()
                .all(|pane| { pane.profile_name == profile && pane.cwd == current_dir })
        );
    }

    #[test]
    fn raw_terminals_are_explicit_secondary_profiles() {
        if env::var_os("GRIDBASH_PROFILE").is_some() {
            return;
        }

        let mut config = Config::default();
        let profile = "managed-test";
        config.profiles.insert(
            profile.into(),
            Profile {
                command: env::current_exe()
                    .expect("test executable")
                    .display()
                    .to_string(),
                args: Vec::new(),
                title: Some("Managed test agent".into()),
                agent_kind: Some(AgentKind::Codex),
            },
        );
        config.set_default_profile(profile);
        config.auth.home = Some(env::temp_dir().join("gridbash-composer-no-auth-profiles"));
        let composer =
            Composer::new(env::current_dir().expect("cwd"), None, &config).expect("composer");

        assert!(!composer.selected_profile().terminal);
        let first_terminal = composer
            .profiles
            .iter()
            .position(|choice| choice.terminal)
            .expect("raw terminal choice");
        assert!(
            composer.profiles[..first_terminal]
                .iter()
                .all(|choice| !choice.terminal)
        );
    }

    #[test]
    fn explicit_workspace_auth_is_applied_to_every_agent_pane() {
        if env::var_os("GRIDBASH_PROFILE").is_some() {
            return;
        }

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let auth_home = env::temp_dir().join(format!("gridbash-composer-auth-{suffix}"));
        let auth_dir = auth_home.join("codex-work");
        fs::create_dir_all(&auth_dir).expect("auth profile dir");
        fs::write(auth_dir.join(".profile-kind"), "codex").expect("profile kind");
        fs::write(auth_dir.join("auth.json"), "{}").expect("auth marker");

        let mut config = Config::default();
        let profile = "managed-test";
        config.profiles.insert(
            profile.into(),
            Profile {
                command: env::current_exe()
                    .expect("test executable")
                    .display()
                    .to_string(),
                args: Vec::new(),
                title: Some("Managed test agent".into()),
                agent_kind: Some(AgentKind::Codex),
            },
        );
        config.set_default_profile(profile);
        config.auth.home = Some(auth_home.clone());
        let mut composer =
            Composer::new(env::current_dir().expect("cwd"), None, &config).expect("composer");
        composer.auth_cursor = 1;

        let plan = composer.launch_plan(&config).expect("launch plan");
        assert!(
            plan.panes
                .iter()
                .all(|pane| pane.auth_name.as_deref() == Some("codex-work"))
        );

        fs::remove_dir_all(auth_home).expect("remove auth fixture");
    }

    #[test]
    fn project_paths_must_resolve_to_existing_directories() {
        let cwd = env::current_dir().expect("cwd");
        assert_eq!(
            resolve_project_path(".", &cwd).expect("resolve project"),
            cwd.canonicalize().expect("canonical cwd")
        );
        assert!(resolve_project_path("definitely-not-a-gridbash-project", &cwd).is_err());
    }

    #[test]
    fn resize_picker_starts_from_the_live_grid_and_confirms_changes() {
        let mut picker = GridPicker::new(GridSize {
            rows: 3,
            columns: 3,
        });

        picker.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

        assert_eq!(
            picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            GridPickerAction::Confirm(GridSize {
                rows: 3,
                columns: 2,
            })
        );
    }

    #[test]
    fn resize_picker_renders_active_cells_in_blue() {
        let picker = GridPicker::new(GridSize {
            rows: 2,
            columns: 3,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        terminal
            .draw(|frame| picker.draw(frame, GridPickerMode::Resize, None))
            .expect("draw picker");

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| {
            matches!(
                cell.bg,
                Color::Rgb(47, 129, 247) | Color::Rgb(31, 91, 191) | Color::Rgb(15, 50, 110)
            )
        }));
        assert!(
            !buffer
                .content()
                .iter()
                .any(|cell| cell.bg == STARTUP_PREVIEW.light)
        );
    }
}
