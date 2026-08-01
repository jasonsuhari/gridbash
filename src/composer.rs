use std::{
    env, fs,
    io::Stdout,
    path::{Path, PathBuf},
    sync::mpsc::{self as std_mpsc, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    config::Config,
    diagnostics,
    layout::{GridSize, MAX_PANES},
    profiles::{Profile, find_profile, is_agent_profile, is_terminal_profile, startup_profiles},
    setup::LaunchPlan,
    worktrees::{
        ManagedWorktreeOptions, WorktreeReadiness, managed_branch_name, probe_worktree_readiness,
    },
};

type ComposerTerminal = Terminal<CrosstermBackend<Stdout>>;

const DEFAULT_ROWS: usize = 2;
const DEFAULT_COLUMNS: usize = 3;
const MAX_DIMENSION: usize = 10;
const MAX_NAME_CHARS: usize = 40;
const MAX_SUGGESTIONS: usize = 64;
/// Length of one animation step. The event loop polls faster than this so
/// motion stays smooth without the composer ever busy-spinning.
const FRAME_MS: u64 = 40;
/// How long the project path must hold still before the git probe runs.
const PROBE_DEBOUNCE: Duration = Duration::from_millis(160);

#[cfg(windows)]
const SHELL_PREFERENCE: &[&str] = &["git-bash", "bash", "pwsh", "powershell", "cmd"];
#[cfg(target_os = "macos")]
const SHELL_PREFERENCE: &[&str] = &["zsh", "bash", "fish", "sh", "pwsh"];
#[cfg(all(not(windows), not(target_os = "macos")))]
const SHELL_PREFERENCE: &[&str] = &["bash", "zsh", "fish", "sh", "pwsh"];

const CANVAS_BG: Color = Color::Rgb(0, 5, 2);
const PANEL_BG: Color = Color::Rgb(1, 10, 5);
const RAISED_BG: Color = Color::Rgb(2, 16, 8);
const SUNKEN_BG: Color = Color::Rgb(1, 8, 4);
const HAIRLINE: Color = Color::Rgb(18, 76, 40);
const HAIRLINE_HI: Color = Color::Rgb(34, 120, 64);
const MUTED: Color = Color::Rgb(72, 128, 88);
const TEXT: Color = Color::Rgb(214, 255, 224);
const TERMINAL_GREEN: Color = Color::Rgb(91, 255, 139);
const DIM_GREEN: Color = Color::Rgb(50, 176, 92);
const SOFT_GREEN: Color = Color::Rgb(159, 255, 183);
const AMBER: Color = Color::Rgb(255, 196, 92);
const ALERT_RED: Color = Color::Rgb(255, 118, 118);
const RESIZE_ACCENT: Color = Color::Rgb(91, 255, 139);
const RESIZE_FILL: Color = Color::Rgb(5, 35, 18);

/// Three-row wordmark. Every glyph is a box-drawing character, so the banner
/// occupies exactly 29 single-width columns wherever GridBash can draw borders.
const WORDMARK: [&str; 3] = [
    "╔═╗ ╦═╗ ╦ ╔╦╗ ╔╗  ╔═╗ ╔═╗ ╦ ╦",
    "║ ╦ ╠╦╝ ║  ║║ ╠╩╗ ╠═╣ ╚═╗ ╠═╣",
    "╚═╝ ╩╚═ ╩ ═╩╝ ╚═╝ ╩ ╩ ╚═╝ ╩ ╩",
];
const WORDMARK_WIDTH: u16 = 29;

/// Everything the composer hands back when the user launches a grid.
#[derive(Debug, Clone)]
pub struct ComposerOutcome {
    pub title: String,
    pub plan: LaunchPlan,
}

/// The new-grid screen: four inputs (rows, columns, name, project), a live
/// preview of the grid that is about to exist, and nothing else to decide.
/// Managed worktrees are on whenever the repository can host them, and every
/// pane starts in the platform shell.
pub struct Composer {
    /// Folder that relative paths resolve against. Fixed for the composer's
    /// lifetime so completion never drifts while the user types.
    base_dir: PathBuf,
    project: TextField,
    project_dir: Option<PathBuf>,
    project_touched: bool,
    name: TextField,
    default_name: String,
    rows: usize,
    columns: usize,
    active: Field,
    suggestions: Suggestions,
    profile_name: String,
    profile_title: String,
    worktree_options: ManagedWorktreeOptions,
    worktrees_enabled: bool,
    readiness: Option<WorktreeReadiness>,
    probe_tx: Sender<(PathBuf, WorktreeReadiness)>,
    probe_rx: Receiver<(PathBuf, WorktreeReadiness)>,
    probing: Option<PathBuf>,
    probe_due: Option<(PathBuf, Instant)>,
    notice: Option<Notice>,
    started: Instant,
    shape_changed: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Rows,
    Columns,
    Name,
    Project,
}

impl Field {
    pub const ALL: [Self; 4] = [Self::Rows, Self::Columns, Self::Name, Self::Project];

    fn is_text(self) -> bool {
        matches!(self, Self::Name | Self::Project)
    }
}

#[derive(Debug, Clone)]
struct Notice {
    text: String,
    level: NoticeLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeLevel {
    Error,
    Warn,
    Info,
}

/// A single-line editor: value plus a character-indexed cursor.
#[derive(Debug, Clone, Default)]
struct TextField {
    value: String,
    cursor: usize,
    limit: Option<usize>,
}

/// Directory completions for the project field. `stem` records the text the
/// list was generated from so repeated Tab presses cycle instead of rebuilding.
/// `siblings` is everything in the parent folder, so the browser can show where
/// else the grid could land; `matches` is the subset Tab walks through.
#[derive(Debug, Clone, Default)]
struct Suggestions {
    stem: String,
    siblings: Vec<String>,
    matches: Vec<String>,
    index: Option<usize>,
}

enum ComposerEvent {
    Continue,
    Launch(Box<ComposerOutcome>),
    Quit,
}

enum TextTarget {
    Name,
    Project,
}

impl Composer {
    pub fn new(
        current_dir: PathBuf,
        worktrees: Option<ManagedWorktreeOptions>,
        config: &Config,
        default_name: &str,
    ) -> Result<Self> {
        let (profile_name, profile) = resolve_shell_profile(config)?;
        Self::with_profile(current_dir, worktrees, default_name, profile_name, profile)
    }

    fn with_profile(
        current_dir: PathBuf,
        worktrees: Option<ManagedWorktreeOptions>,
        default_name: &str,
        profile_name: String,
        profile: Profile,
    ) -> Result<Self> {
        let base_dir = current_dir.canonicalize().unwrap_or(current_dir);
        let worktree_options = worktrees.unwrap_or(ManagedWorktreeOptions::new("gridbash".into())?);
        let (probe_tx, probe_rx) = std_mpsc::channel();

        let mut composer = Self {
            project: TextField::new(display_path(&base_dir), None),
            project_dir: Some(base_dir.clone()),
            project_touched: false,
            name: TextField::new(default_name, Some(MAX_NAME_CHARS)),
            default_name: default_name.to_string(),
            base_dir,
            rows: DEFAULT_ROWS,
            columns: DEFAULT_COLUMNS,
            active: Field::Rows,
            suggestions: Suggestions::default(),
            profile_title: profile.display_name(&profile_name),
            profile_name,
            worktree_options,
            worktrees_enabled: true,
            readiness: None,
            probe_tx,
            probe_rx,
            probing: None,
            probe_due: None,
            notice: None,
            started: Instant::now(),
            shape_changed: Instant::now(),
        };
        composer.refresh_suggestions();
        composer.request_probe(true);
        Ok(composer)
    }

    pub fn run(
        &mut self,
        terminal: &mut ComposerTerminal,
        config: &Config,
    ) -> Result<Option<ComposerOutcome>> {
        loop {
            terminal.draw(|frame| self.draw(frame, config))?;
            self.drain_probes();
            self.settle_probe();

            if !event::poll(Duration::from_millis(FRAME_MS / 2))? {
                continue;
            }

            let result = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key, config),
                Event::Paste(text) => {
                    self.paste(&text);
                    ComposerEvent::Continue
                }
                _ => ComposerEvent::Continue,
            };

            match result {
                ComposerEvent::Continue => {}
                ComposerEvent::Launch(outcome) => return Ok(Some(*outcome)),
                ComposerEvent::Quit => return Ok(None),
            }
        }
    }

    // ----------------------------------------------------------------- input

    fn handle_key(&mut self, key: KeyEvent, config: &Config) -> ComposerEvent {
        if is_enter_key(key) {
            return match self.build_outcome(config) {
                Ok(outcome) => ComposerEvent::Launch(Box::new(outcome)),
                Err(error) => {
                    self.notice = Some(Notice {
                        text: format!("{error:#}"),
                        level: NoticeLevel::Error,
                    });
                    ComposerEvent::Continue
                }
            };
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc => return ComposerEvent::Quit,
            KeyCode::Char('c') if ctrl => return ComposerEvent::Quit,
            KeyCode::Char('w') if alt => {
                self.worktrees_enabled = !self.worktrees_enabled;
                self.notice = None;
                return ComposerEvent::Continue;
            }
            KeyCode::Tab if self.active == Field::Project => {
                self.complete_project();
                return ComposerEvent::Continue;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.move_field(1);
                return ComposerEvent::Continue;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.move_field(-1);
                return ComposerEvent::Continue;
            }
            _ => {}
        }

        match self.active {
            Field::Rows | Field::Columns => self.handle_shape_key(key),
            Field::Name => self.handle_text_key(key, ctrl, TextTarget::Name),
            Field::Project => self.handle_text_key(key, ctrl, TextTarget::Project),
        }
        ComposerEvent::Continue
    }

    fn handle_shape_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Char('-') | KeyCode::Char('_') => self.adjust_shape(-1),
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_shape(1),
            KeyCode::Home => self.set_shape(1),
            KeyCode::End => self.set_shape(MAX_DIMENSION),
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let digit = ch.to_digit(10).unwrap_or(1) as usize;
                // 0 reads as "all the way up", the way the resize picker works.
                self.set_shape(if digit == 0 { MAX_DIMENSION } else { digit });
            }
            _ => {}
        }
    }

    fn handle_text_key(&mut self, key: KeyEvent, ctrl: bool, target: TextTarget) {
        let changed = {
            let field = match target {
                TextTarget::Name => &mut self.name,
                TextTarget::Project => &mut self.project,
            };
            match key.code {
                KeyCode::Char('u') if ctrl => field.clear(),
                KeyCode::Char('w') if ctrl => field.delete_word(),
                KeyCode::Char('a') if ctrl => {
                    field.home();
                    false
                }
                KeyCode::Char('e') if ctrl => {
                    field.end();
                    false
                }
                KeyCode::Char(ch) if !ctrl => field.insert(ch),
                KeyCode::Backspace => field.backspace(),
                KeyCode::Delete => field.delete(),
                KeyCode::Left => {
                    field.move_cursor(-1);
                    false
                }
                KeyCode::Right => {
                    field.move_cursor(1);
                    false
                }
                KeyCode::Home => {
                    field.home();
                    false
                }
                KeyCode::End => {
                    field.end();
                    false
                }
                _ => false,
            }
        };

        if !changed {
            return;
        }

        self.notice = None;
        if matches!(target, TextTarget::Project) {
            self.project_touched = true;
            self.refresh_suggestions();
            self.refresh_project();
        }
    }

    fn paste(&mut self, text: &str) {
        let clean = text
            .chars()
            .filter(|ch| !ch.is_control())
            .collect::<String>();
        if clean.is_empty() {
            return;
        }

        match self.active {
            Field::Name => {
                self.name.insert_str(&clean);
            }
            Field::Project => {
                self.project.insert_str(&clean);
                self.project_touched = true;
                self.refresh_suggestions();
                self.refresh_project();
            }
            Field::Rows | Field::Columns => return,
        }
        self.notice = None;
    }

    fn move_field(&mut self, delta: isize) {
        let current = Field::ALL
            .iter()
            .position(|field| *field == self.active)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(Field::ALL.len() as isize) as usize;
        self.active = Field::ALL[next];
        if self.active == Field::Project {
            self.refresh_suggestions();
        }
    }

    fn adjust_shape(&mut self, delta: isize) {
        let value = match self.active {
            Field::Columns => self.columns,
            _ => self.rows,
        };
        self.set_shape((value as isize + delta).clamp(1, MAX_DIMENSION as isize) as usize);
    }

    fn set_shape(&mut self, value: usize) {
        let value = value.clamp(1, MAX_DIMENSION);
        let slot = match self.active {
            Field::Columns => &mut self.columns,
            _ => &mut self.rows,
        };
        if *slot == value {
            return;
        }
        *slot = value;
        self.shape_changed = Instant::now();
        self.notice = None;
    }

    // ------------------------------------------------------------ completion

    fn refresh_suggestions(&mut self) {
        let value = self.project.value.clone();
        let (siblings, matches) = scan_folders(&value, &self.base_dir);
        self.suggestions = Suggestions {
            stem: value,
            siblings,
            matches,
            index: None,
        };
    }

    fn complete_project(&mut self) {
        let stale = self.suggestions.stem != self.project.value
            && self.suggestions.applied() != Some(&self.project.value);
        if stale {
            self.refresh_suggestions();
        }
        if self.suggestions.matches.is_empty() {
            self.notice = Some(Notice {
                text: "no folders match that path".into(),
                level: NoticeLevel::Warn,
            });
            return;
        }

        // The first Tab extends to the prefix every candidate shares, the way a
        // shell would. Once there is nothing left to share, Tab cycles.
        if self.suggestions.index.is_none() && self.suggestions.matches.len() > 1 {
            let shared = longest_common_prefix(&self.suggestions.matches);
            if shared.chars().count() > self.project.value.chars().count() {
                self.apply_completion(shared);
                return;
            }
        }

        let next = match self.suggestions.index {
            Some(index) => (index + 1) % self.suggestions.matches.len(),
            None => 0,
        };
        self.suggestions.index = Some(next);
        let mut candidate = self.suggestions.matches[next].clone();
        if self.suggestions.matches.len() == 1 && has_subdirectories(&candidate, &self.base_dir) {
            candidate.push(separator_for(&candidate));
        }
        self.apply_completion(candidate);
    }

    fn apply_completion(&mut self, value: String) {
        self.project.set(value);
        self.project_touched = true;
        self.notice = None;
        self.refresh_project();
    }

    fn refresh_project(&mut self) {
        match resolve_project_path(&self.project.value, &self.base_dir) {
            Ok(path) => {
                let changed = self.project_dir.as_deref() != Some(path.as_path());
                self.project_dir = Some(path);
                if changed {
                    self.request_probe(false);
                }
            }
            Err(_) => self.project_dir = None,
        }
    }

    // -------------------------------------------------------- worktree probe

    /// Ask a worker thread whether the selected folder can host one managed
    /// worktree per pane. `git status` is slow on big repositories, so it never
    /// runs on the draw thread.
    fn request_probe(&mut self, immediate: bool) {
        let Some(target) = self.project_dir.clone() else {
            return;
        };
        if self.probing.as_deref() == Some(target.as_path()) {
            return;
        }
        self.readiness = None;
        if immediate {
            self.spawn_probe(target);
        } else {
            self.probe_due = Some((target, Instant::now()));
        }
    }

    fn settle_probe(&mut self) {
        let Some((target, requested)) = self.probe_due.clone() else {
            return;
        };
        if requested.elapsed() < PROBE_DEBOUNCE {
            return;
        }
        self.probe_due = None;
        if self.project_dir.as_deref() == Some(target.as_path()) {
            self.spawn_probe(target);
        }
    }

    fn spawn_probe(&mut self, target: PathBuf) {
        self.probing = Some(target.clone());
        let tx = self.probe_tx.clone();
        thread::spawn(move || {
            // A panicking probe would leave the composer waiting on a result
            // that never arrives.
            let readiness = diagnostics::recovering("the worktree probe", || {
                Ok(probe_worktree_readiness(&target))
            })
            .unwrap_or_else(|reason| WorktreeReadiness::Blocked { reason });
            let _ = tx.send((target, readiness));
        });
    }

    fn drain_probes(&mut self) {
        while let Ok((path, readiness)) = self.probe_rx.try_recv() {
            if self.project_dir.as_deref() == Some(path.as_path()) {
                self.readiness = Some(readiness);
                self.probing = None;
            }
        }
    }

    fn worktrees_active(&self) -> bool {
        self.worktrees_enabled
            && self
                .readiness
                .as_ref()
                .is_some_and(WorktreeReadiness::is_ready)
    }

    fn branch_label(&self, index: usize) -> Option<String> {
        let base_slug = self.readiness.as_ref()?.base_slug()?;
        self.worktrees_active()
            .then(|| managed_branch_name(&self.worktree_options.prefix, base_slug, index))
    }

    // ---------------------------------------------------------------- launch

    fn grid(&self) -> GridSize {
        GridSize {
            rows: self.rows,
            columns: self.columns,
        }
    }

    fn title(&self) -> String {
        let name = self
            .name
            .value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if name.is_empty() {
            self.default_name.clone()
        } else {
            name
        }
    }

    fn build_outcome(&self, config: &Config) -> Result<ComposerOutcome> {
        let Some(cwd) = self.project_dir.clone() else {
            return Err(resolve_project_path(&self.project.value, &self.base_dir)
                .err()
                .unwrap_or_else(|| anyhow!("project folder is not usable")));
        };
        let profile = find_profile(config, &self.profile_name)?;
        let grid = self.grid();
        let worktrees = self.worktrees_active().then_some(&self.worktree_options);
        let plan = LaunchPlan::from_launch_options(
            self.profile_name.clone(),
            profile,
            cwd,
            grid.count(),
            grid,
            worktrees,
        )?;

        Ok(ComposerOutcome {
            title: self.title(),
            plan,
        })
    }

    // --------------------------------------------------------------- drawing

    fn frame_tick(&self) -> u64 {
        self.started.elapsed().as_millis() as u64 / FRAME_MS
    }

    fn draw(&self, frame: &mut Frame<'_>, _config: &Config) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(CANVAS_BG)),
            area,
        );
        let tick = self.frame_tick();

        let panel = if area.width >= 90 && area.height >= 26 {
            inset(area, 2, 1)
        } else {
            area
        };
        if panel.width < 10 || panel.height < 6 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(HAIRLINE_HI))
            .title(chrome_title())
            .title_bottom(chrome_footer_title(&self.profile_title))
            .style(panel_style());
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        paint_border_runner(frame, panel, tick);

        let content = inset(inner, if inner.width >= 8 { 2 } else { 0 }, 0);
        if content.width == 0 || content.height == 0 {
            return;
        }

        let header_height = if content.height >= 22 {
            5
        } else if content.height >= 18 {
            4
        } else {
            1
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(8),
                Constraint::Length(3),
            ])
            .split(content);

        self.draw_header(frame, chunks[0], tick);

        let body = chunks[1];
        if body.width >= 80 {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(48),
                    Constraint::Length(1),
                    Constraint::Percentage(52),
                ])
                .split(body);
            self.draw_form(frame, split[0], tick);
            self.draw_preview(frame, split[2], tick);
        } else {
            self.draw_form(frame, body, tick);
        }

        self.draw_footer(frame, chunks[2], tick);
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        if area.height == 0 {
            return;
        }
        if area.height < 3 || area.width < WORDMARK_WIDTH {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "GRIDBASH",
                        Style::default()
                            .fg(TERMINAL_GREEN)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ·  ", Style::default().fg(HAIRLINE_HI)),
                    Span::styled(self.telemetry_line(), Style::default().fg(MUTED)),
                ]))
                .style(panel_style()),
                area,
            );
            return;
        }

        paint_starfield(frame, area, tick, 0x51D3);

        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(WORDMARK_WIDTH), Constraint::Min(0)])
            .split(area);
        draw_wordmark(frame, split[0], tick);

        let stats = split[1];
        if stats.width < 14 {
            return;
        }
        let stats = inset(stats, 2, 0);
        let panes = self.grid().count();
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{panes:02}"),
                    Style::default().fg(SOFT_GREEN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" PANES   ", Style::default().fg(MUTED)),
                Span::styled(
                    format!("{}×{}", self.rows, self.columns),
                    Style::default()
                        .fg(TERMINAL_GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(self.worktree_status_spans()),
        ];
        if stats.height >= 3 {
            lines.push(Line::from(vec![
                Span::styled("▸ ", Style::default().fg(DIM_GREEN)),
                Span::styled(self.profile_title.to_uppercase(), Style::default().fg(TEXT)),
                Span::styled(" in every pane", Style::default().fg(MUTED)),
            ]));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Right)
                .style(panel_style()),
            stats,
        );
    }

    fn telemetry_line(&self) -> String {
        format!(
            "{} panes · {} · {}",
            self.grid().count(),
            self.profile_title,
            if self.worktrees_active() {
                "worktrees"
            } else {
                "shared folder"
            }
        )
    }

    fn worktree_status_spans(&self) -> Vec<Span<'static>> {
        let (dot, label, color) = match (&self.readiness, self.worktrees_enabled) {
            (_, false) => ("○", "WORKTREES OFF", MUTED),
            (None, _) => ("◌", "CHECKING REPO", MUTED),
            (Some(WorktreeReadiness::Ready { .. }), _) => ("●", "WORKTREES ON", SOFT_GREEN),
            (Some(WorktreeReadiness::Blocked { .. }), _) => ("▲", "SHARED FOLDER", AMBER),
        };
        vec![
            Span::styled(format!("{dot} "), Style::default().fg(color)),
            Span::styled(label, Style::default().fg(color)),
        ]
    }

    // ------------------------------------------------------------------ form

    fn draw_form(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        if area.width < 22 || area.height < 6 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(4)])
            .split(area);

        self.draw_shape(frame, chunks[0], tick);
        self.draw_identity(frame, chunks[1], tick);
    }

    fn draw_shape(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        if area.height < 4 {
            return;
        }
        let block = section_block(
            "01",
            "SHAPE",
            self.active_in(&[Field::Rows, Field::Columns]),
        );
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let width = inner.width as usize;
        let lines = vec![
            shape_row("ROWS", self.rows, self.active == Field::Rows, tick, width),
            shape_row(
                "COLUMNS",
                self.columns,
                self.active == Field::Columns,
                tick,
                width,
            ),
        ];
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(RAISED_BG)),
            inner,
        );
    }

    fn draw_identity(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        if area.height < 4 {
            return;
        }
        let block = section_block(
            "02",
            "IDENTITY",
            self.active_in(&[Field::Name, Field::Project]),
        );
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let value_width = (inner.width as usize).saturating_sub(13).max(6);
        let hint_width = (inner.width as usize).saturating_sub(11).max(6);
        let mut lines = vec![
            text_row(
                "NAME",
                &self.name,
                self.active == Field::Name,
                tick,
                value_width,
                None,
                DIM_GREEN,
            ),
            hint_row(Span::styled(
                truncate(
                    &if self.name.value.trim().is_empty() {
                        format!("defaults to {}", self.default_name)
                    } else {
                        "tab name for this grid".into()
                    },
                    hint_width,
                ),
                Style::default().fg(MUTED),
            )),
            text_row(
                "PROJECT",
                &self.project,
                self.active == Field::Project,
                tick,
                value_width,
                self.project_ghost().as_deref(),
                TERMINAL_GREEN,
            ),
            hint_row(self.project_state_span(hint_width)),
        ];
        lines.truncate(inner.height as usize);

        let used = lines.len() as u16;
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(RAISED_BG)),
            Rect {
                height: used.min(inner.height),
                ..inner
            },
        );

        let remaining = inner.height.saturating_sub(used);
        if remaining >= 2 {
            self.draw_suggestions(
                frame,
                Rect {
                    y: inner.y.saturating_add(used),
                    height: remaining,
                    ..inner
                },
            );
        }
    }

    /// Live folder browser for the project field. It stays on screen even when
    /// the field is not focused, so the panel always shows where else the grid
    /// could land instead of a block of dead space.
    fn draw_suggestions(&self, frame: &mut Frame<'_>, area: Rect) {
        let focused = self.active == Field::Project;
        let folders = &self.suggestions.siblings;
        let matches = &self.suggestions.matches;
        let selected = self
            .suggestions
            .applied()
            .filter(|_| focused)
            .and_then(|applied| folders.iter().position(|folder| folder == applied));

        let mut lines = vec![Line::from(vec![
            Span::styled("╌╌ ", Style::default().fg(HAIRLINE)),
            Span::styled(
                match (folders.len(), focused) {
                    (0, _) => "NOTHING TO BROWSE HERE".to_string(),
                    (total, true) => {
                        format!("{} OF {total} MATCH · TAB CYCLES", matches.len())
                    }
                    (total, false) => format!("{total} NEARBY {}", plural("FOLDER", total)),
                },
                Style::default().fg(HAIRLINE_HI),
            ),
        ])];

        let capacity = area.height.saturating_sub(1) as usize;
        // Keep the Tab-selected row on screen, otherwise lead with the first
        // candidate Tab would land on.
        let anchor = selected
            .or_else(|| folders.iter().position(|folder| matches.contains(folder)))
            .unwrap_or(0);
        let start = if folders.len() > capacity && anchor + 1 > capacity {
            anchor + 1 - capacity
        } else {
            0
        };
        let width = area.width.saturating_sub(6) as usize;
        // When the list overflows, the last row becomes the "+N more" counter,
        // so the room for entries shrinks by one and N has to account for it.
        let mut room = capacity;
        let mut hidden = folders.len().saturating_sub(start + room);
        if hidden > 0 {
            room = capacity.saturating_sub(1);
            hidden = folders.len().saturating_sub(start + room);
        }

        for (offset, candidate) in folders.iter().enumerate().skip(start).take(room) {
            let is_selected = selected == Some(offset);
            let is_match = matches.contains(candidate);
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(TERMINAL_GREEN)
                    .add_modifier(Modifier::BOLD)
            } else if is_match && focused {
                Style::default().fg(TEXT)
            } else {
                Style::default().fg(if is_match { MUTED } else { HAIRLINE_HI })
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if is_selected { " ▸ " } else { "   " },
                    Style::default().fg(if is_selected {
                        TERMINAL_GREEN
                    } else {
                        HAIRLINE
                    }),
                ),
                Span::styled(
                    format!(" {} ", truncate(&leaf_name(candidate), width)),
                    style,
                ),
            ]));
        }

        if hidden > 0 {
            lines.push(Line::from(Span::styled(
                format!("   +{hidden} more"),
                Style::default().fg(HAIRLINE_HI),
            )));
        }

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(RAISED_BG)),
            area,
        );
    }

    /// Dim text after the cursor showing what Tab would fill in.
    fn project_ghost(&self) -> Option<String> {
        if self.active != Field::Project || self.project.cursor != self.project.len() {
            return None;
        }
        if self.suggestions.index.is_some() {
            return None;
        }
        self.suggestions
            .matches
            .first()?
            .strip_prefix(self.project.value.as_str())
            .filter(|rest| !rest.is_empty())
            .map(str::to_string)
    }

    fn project_state_span(&self, width: usize) -> Span<'static> {
        match (&self.project_dir, self.project_touched) {
            (Some(path), _) => Span::styled(
                format!("✓ {}", short_path(path, width.saturating_sub(2))),
                Style::default().fg(SOFT_GREEN),
            ),
            (None, true) => Span::styled(
                truncate("folder not found yet · keep typing or press Tab", width),
                Style::default().fg(AMBER),
            ),
            (None, false) => Span::styled(
                truncate("choose a project folder", width),
                Style::default().fg(MUTED),
            ),
        }
    }

    fn active_in(&self, fields: &[Field]) -> bool {
        fields.contains(&self.active)
    }

    // --------------------------------------------------------------- preview

    fn draw_preview(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        if area.width < 20 || area.height < 6 {
            return;
        }
        let block = section_block("03", "LIVE GRID", false);
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let header_height = if inner.height >= 10 { 3 } else { 1 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(1)])
            .split(inner);

        let mut lines = vec![Line::from(vec![
            Span::styled(
                format!("{:02}", self.rows),
                Style::default()
                    .fg(TERMINAL_GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ROWS  ×  ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{:02}", self.columns),
                Style::default()
                    .fg(TERMINAL_GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" COLS  =  ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{:02}", self.grid().count()),
                Style::default().fg(SOFT_GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" PANES", Style::default().fg(MUTED)),
        ])];
        if header_height >= 3 {
            lines.push(Line::from(self.isolation_spans()));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(RAISED_BG)),
            chunks[0],
        );

        let stage = chunks[1];
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(SUNKEN_BG)),
            stage,
        );
        paint_starfield(frame, stage, tick, 0x9F17);

        let since_change = self.shape_changed.elapsed().as_millis() as u64;
        let folder = self
            .project_dir
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::to_string);
        let total = self.grid().count();

        for (index, rect) in square_preview_rects(inset(stage, 1, 0), self.grid())
            .into_iter()
            .enumerate()
        {
            let row = index / self.columns.max(1);
            let column = index % self.columns.max(1);
            draw_preview_cell(
                frame,
                rect,
                &PreviewCell {
                    accent: cell_accent(index, total),
                    // Cells wake up in a diagonal wave whenever the shape changes.
                    revealed: since_change >= (row as u64 + column as u64) * 35 + 20,
                    shimmer: shimmer_hits(tick, row, column, self.rows + self.columns),
                    title: format!("{:02}", index + 1),
                    subtitle: self
                        .branch_label(index)
                        .map(|branch| leaf_after(&branch))
                        .or_else(|| folder.clone()),
                    fill: SUNKEN_BG,
                },
            );
        }
    }

    fn isolation_spans(&self) -> Vec<Span<'static>> {
        if let Some(branch) = self.branch_label(0) {
            let last = self
                .branch_label(self.grid().count().saturating_sub(1))
                .map(|value| leaf_after(&value))
                .unwrap_or_default();
            return vec![
                Span::styled("⎇ ", Style::default().fg(SOFT_GREEN)),
                Span::styled(branch, Style::default().fg(TEXT)),
                Span::styled(
                    if self.grid().count() > 1 {
                        format!(" … {last}")
                    } else {
                        String::new()
                    },
                    Style::default().fg(MUTED),
                ),
            ];
        }

        let reason = match (&self.readiness, self.worktrees_enabled) {
            (_, false) => "worktrees off · alt+w re-enables".to_string(),
            (None, _) => "checking git worktree readiness…".to_string(),
            (Some(readiness), _) => readiness
                .reason()
                .map(|reason| format!("shared folder · {reason}"))
                .unwrap_or_else(|| "shared folder".into()),
        };
        vec![
            Span::styled("⌂ ", Style::default().fg(AMBER)),
            Span::styled(reason, Style::default().fg(MUTED)),
        ]
    }

    // ---------------------------------------------------------------- footer

    fn draw_footer(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        if area.height == 0 {
            return;
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(area);

        let notice = self.notice.clone().unwrap_or_else(|| self.hint_notice());
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    notice.level.glyph(),
                    Style::default().fg(notice.level.color()),
                ),
                Span::styled(notice.text, Style::default().fg(notice.level.color())),
            ]))
            .alignment(Alignment::Center)
            .style(panel_style()),
            rows[0],
        );

        frame.render_widget(
            Paragraph::new(launch_bar(self.grid().count(), tick, rows[1].width))
                .alignment(Alignment::Center)
                .style(panel_style()),
            rows[1],
        );

        if rows[2].height > 0 {
            frame.render_widget(
                Paragraph::new(self.controls_line(rows[2].width))
                    .alignment(Alignment::Center)
                    .style(panel_style()),
                rows[2],
            );
        }
    }

    fn hint_notice(&self) -> Notice {
        let text = match self.active {
            Field::Rows | Field::Columns => "type a digit, or ←/→ to resize the grid",
            Field::Name => "name this grid so its tab is easy to find",
            Field::Project => "Tab completes folders · Tab again cycles matches",
        };
        Notice {
            text: text.into(),
            level: NoticeLevel::Info,
        }
    }

    fn controls_line(&self, width: u16) -> Line<'static> {
        let mut spans = vec![
            keycap("↑↓"),
            label("FIELD"),
            keycap("←→"),
            label(if self.active.is_text() {
                "CURSOR"
            } else {
                "SIZE"
            }),
            keycap("TAB"),
            label(if self.active == Field::Project {
                "COMPLETE"
            } else {
                "NEXT"
            }),
        ];
        if width >= 78 {
            spans.push(keycap("ALT+W"));
            spans.push(label("WORKTREES"));
        }
        spans.push(keycap("ESC"));
        spans.push(label("CANCEL"));
        Line::from(spans)
    }
}

impl NoticeLevel {
    fn color(self) -> Color {
        match self {
            Self::Error => ALERT_RED,
            Self::Warn => AMBER,
            Self::Info => MUTED,
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Error => "✕ ",
            Self::Warn => "▲ ",
            Self::Info => "› ",
        }
    }
}

impl Suggestions {
    fn applied(&self) -> Option<&String> {
        self.index.and_then(|index| self.matches.get(index))
    }
}

impl TextField {
    fn new(value: impl Into<String>, limit: Option<usize>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self {
            value,
            cursor,
            limit,
        }
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    fn byte_index(&self, cursor: usize) -> usize {
        self.value
            .char_indices()
            .nth(cursor)
            .map(|(index, _)| index)
            .unwrap_or(self.value.len())
    }

    fn insert(&mut self, ch: char) -> bool {
        if ch.is_control() || self.limit.is_some_and(|limit| self.len() >= limit) {
            return false;
        }
        let at = self.byte_index(self.cursor);
        self.value.insert(at, ch);
        self.cursor += 1;
        true
    }

    fn insert_str(&mut self, text: &str) -> bool {
        let mut inserted = false;
        for ch in text.chars() {
            inserted |= self.insert(ch);
        }
        inserted
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.remove_char_at(self.cursor)
    }

    fn delete(&mut self) -> bool {
        if self.cursor >= self.len() {
            return false;
        }
        self.remove_char_at(self.cursor)
    }

    /// `String::remove` panics at the end of the string, so only call it for a
    /// character index the value actually holds.
    fn remove_char_at(&mut self, cursor: usize) -> bool {
        let at = self.byte_index(cursor);
        if at >= self.value.len() {
            return false;
        }
        self.value.remove(at);
        true
    }

    /// Backspace over one path segment: trailing separators first, then the
    /// name in front of them.
    fn delete_word(&mut self) -> bool {
        let mut removed = false;
        while self.cursor > 0 && self.char_before().is_some_and(is_path_boundary) {
            removed |= self.backspace();
        }
        while self.cursor > 0 && self.char_before().is_some_and(|ch| !is_path_boundary(ch)) {
            removed |= self.backspace();
        }
        removed
    }

    fn char_before(&self) -> Option<char> {
        self.value.chars().nth(self.cursor.checked_sub(1)?)
    }

    fn clear(&mut self) -> bool {
        if self.value.is_empty() {
            return false;
        }
        self.value.clear();
        self.cursor = 0;
        true
    }

    fn set(&mut self, value: String) {
        self.value = value;
        self.cursor = self.len();
    }

    fn move_cursor(&mut self, delta: isize) {
        self.cursor = (self.cursor as isize + delta).clamp(0, self.len() as isize) as usize;
    }

    fn home(&mut self) {
        self.cursor = 0;
    }

    fn end(&mut self) {
        self.cursor = self.len();
    }
}

fn is_path_boundary(ch: char) -> bool {
    matches!(ch, '/' | '\\' | ' ')
}

// -------------------------------------------------------------------- widgets

fn chrome_title() -> Line<'static> {
    Line::from(vec![
        Span::styled("─┤ ", Style::default().fg(HAIRLINE_HI)),
        Span::styled("◆", Style::default().fg(SOFT_GREEN)),
        Span::styled(
            " NEW GRID ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("├─", Style::default().fg(HAIRLINE_HI)),
    ])
}

fn chrome_footer_title(profile: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("─┤ ", Style::default().fg(HAIRLINE)),
        Span::styled(profile.to_uppercase(), Style::default().fg(HAIRLINE_HI)),
        Span::styled(" ├─", Style::default().fg(HAIRLINE)),
    ])
    .alignment(Alignment::Right)
}

fn section_block(index: &'static str, label: &'static str, active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(if active { HAIRLINE_HI } else { HAIRLINE }))
        .title(Line::from(vec![
            Span::styled(
                format!(" {index} "),
                Style::default().fg(if active { DIM_GREEN } else { HAIRLINE_HI }),
            ),
            Span::styled(
                format!("{label} "),
                Style::default()
                    .fg(if active { TEXT } else { MUTED })
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(Style::default().bg(RAISED_BG))
}

fn shape_row(label: &str, value: usize, focused: bool, tick: u64, width: usize) -> Line<'static> {
    let mut spans = vec![
        focus_marker(focused, TERMINAL_GREEN, tick),
        Span::styled(format!("{label:<9}"), field_label_style(focused)),
        Span::styled(
            format!(" {value:02} "),
            if focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(TERMINAL_GREEN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(TERMINAL_GREEN)
                    .bg(SUNKEN_BG)
                    .add_modifier(Modifier::BOLD)
            },
        ),
    ];

    if width >= 28 {
        spans.push(Span::raw("  "));
        for slot in 0..MAX_DIMENSION {
            let (glyph, color) = if slot < value {
                let ramp = lerp_color(
                    TERMINAL_GREEN,
                    DIM_GREEN,
                    slot as f32 / (MAX_DIMENSION - 1) as f32,
                );
                let leading = focused && slot + 1 == value;
                ("█", if leading { pulse(ramp, tick) } else { ramp })
            } else {
                ("▒", HAIRLINE)
            };
            spans.push(Span::styled(glyph, Style::default().fg(color)));
        }
    }

    Line::from(spans)
}

fn text_row(
    label: &str,
    field: &TextField,
    focused: bool,
    tick: u64,
    width: usize,
    ghost: Option<&str>,
    accent: Color,
) -> Line<'static> {
    let mut spans = vec![
        focus_marker(focused, accent, tick),
        Span::styled(format!("{label:<9}"), field_label_style(focused)),
    ];

    let chars = field.value.chars().collect::<Vec<_>>();
    let visible = width.max(6);
    // Clamp before scrolling: a cursor left past the end of a replaced value
    // would put `start` beyond `end`, and `clamp` panics when its bounds cross.
    let caret = field.cursor.min(chars.len());
    // Scroll the window so the cursor is always on screen.
    let start = if chars.len() < visible {
        0
    } else {
        caret.saturating_sub(visible.saturating_sub(1))
    };
    let end = (start + visible).min(chars.len()).max(start);
    let value_style = Style::default().fg(if focused {
        TEXT
    } else {
        Color::Rgb(150, 210, 170)
    });

    if start > 0 {
        spans.push(Span::styled("…", Style::default().fg(HAIRLINE_HI)));
    }
    let cursor = caret.clamp(start, end);
    spans.push(Span::styled(
        chars
            .get(start..cursor)
            .unwrap_or_default()
            .iter()
            .collect::<String>(),
        value_style,
    ));

    if focused {
        let blink = tick % 16 < 11;
        match chars.get(cursor) {
            Some(ch) if cursor < end => spans.push(Span::styled(
                ch.to_string(),
                if blink {
                    Style::default().fg(Color::Black).bg(accent)
                } else {
                    value_style
                },
            )),
            _ => spans.push(Span::styled(
                if blink { "▌" } else { " " },
                Style::default().fg(accent),
            )),
        }
    }

    let tail = if focused {
        (cursor + 1).min(end)
    } else {
        cursor
    };
    if tail < end {
        spans.push(Span::styled(
            chars[tail..end].iter().collect::<String>(),
            value_style,
        ));
    }

    if let Some(ghost) = ghost {
        let room = visible.saturating_sub(end.saturating_sub(start));
        if room > 1 {
            spans.push(Span::styled(
                truncate(ghost, room),
                Style::default().fg(HAIRLINE_HI),
            ));
        }
    }

    Line::from(spans)
}

fn hint_row(hint: Span<'static>) -> Line<'static> {
    Line::from(vec![Span::raw("           "), hint])
}

fn focus_marker(focused: bool, accent: Color, tick: u64) -> Span<'static> {
    Span::styled(
        if focused { "▌ " } else { "  " },
        Style::default().fg(if focused {
            pulse(accent, tick)
        } else {
            HAIRLINE
        }),
    )
}

fn field_label_style(focused: bool) -> Style {
    Style::default()
        .fg(if focused { TEXT } else { MUTED })
        .add_modifier(if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

struct PreviewCell {
    accent: Color,
    revealed: bool,
    shimmer: bool,
    title: String,
    subtitle: Option<String>,
    fill: Color,
}

fn draw_preview_cell(frame: &mut Frame<'_>, rect: Rect, cell: &PreviewCell) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }

    let border = if !cell.revealed {
        HAIRLINE
    } else if cell.shimmer {
        Color::Rgb(232, 255, 240)
    } else {
        cell.accent
    };
    // Clear first: the stage behind this cell is dusted with starfield glyphs,
    // and a Block only restyles cells, it does not overwrite their symbols.
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(if cell.revealed { cell.fill } else { CANVAS_BG }));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if !cell.revealed || inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines = vec![Line::from(Span::styled(
        cell.title.clone(),
        Style::default()
            .fg(if cell.shimmer { Color::White } else { TEXT })
            .add_modifier(Modifier::BOLD),
    ))];
    // Below this width a branch name is all ellipsis, so the number stands alone.
    if inner.height >= 3
        && inner.width >= 12
        && let Some(subtitle) = cell.subtitle.as_deref()
    {
        lines.push(Line::from(Span::styled(
            truncate(subtitle, inner.width as usize),
            Style::default().fg(dim_color(cell.accent)),
        )));
    }

    let text_height = (lines.len() as u16).min(inner.height);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        Rect {
            y: inner.y + inner.height.saturating_sub(text_height) / 2,
            height: text_height,
            ..inner
        },
    );
}

/// The launch affordance: a solid bar with a highlight sweeping across it.
fn launch_bar(panes: usize, tick: u64, width: u16) -> Line<'static> {
    let text = if width >= 56 {
        format!("  ▶  LAUNCH {panes} PANES   ·   ENTER  ")
    } else {
        format!("  ▶  LAUNCH {panes}  ·  ENTER  ")
    };
    let chars = text.chars().collect::<Vec<_>>();
    let len = chars.len().max(1);
    let sweep = (tick % (len as u64 + 14)) as f32 - 7.0;

    let spans = chars
        .into_iter()
        .enumerate()
        .map(|(index, ch)| {
            let base = lerp_color(SOFT_GREEN, TERMINAL_GREEN, index as f32 / len as f32);
            let distance = (index as f32 - sweep).abs();
            let background = if distance < 3.0 {
                lerp_color(Color::White, base, distance / 3.0)
            } else {
                base
            };
            Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(Color::Rgb(3, 12, 10))
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn draw_wordmark(frame: &mut Frame<'_>, area: Rect, tick: u64) {
    let sweep = (tick % (WORDMARK_WIDTH as u64 + 26)) as f32 - 13.0;
    for (row, text) in WORDMARK.iter().enumerate() {
        let y = area.y.saturating_add(row as u16);
        if y >= area.bottom() {
            break;
        }
        let spans = text
            .chars()
            .enumerate()
            .map(|(column, ch)| {
                let base = lerp_color(
                    TERMINAL_GREEN,
                    DIM_GREEN,
                    column as f32 / WORDMARK_WIDTH as f32,
                );
                let distance = (column as f32 - sweep).abs() + row as f32 * 0.6;
                let color = if distance < 4.0 {
                    lerp_color(Color::White, base, distance / 4.0)
                } else {
                    base
                };
                Span::styled(
                    ch.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
    }
}

/// Sparse drifting dust. Written straight into the buffer so it costs one cell
/// write per star instead of a span per column.
fn paint_starfield(frame: &mut Frame<'_>, area: Rect, tick: u64, seed: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let drift = tick / 14;
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let noise = hash3(x as u64, (y as u64).wrapping_add(drift), seed);
            if !noise.is_multiple_of(29) {
                continue;
            }
            let (glyph, color) = match (noise >> 8).wrapping_add(tick / 6) % 14 {
                0 => ("*", Color::Rgb(74, 150, 96)),
                1 | 2 => ("·", Color::Rgb(44, 104, 62)),
                _ => ("·", Color::Rgb(16, 52, 30)),
            };
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_symbol(glyph).set_fg(color);
            }
        }
    }
}

/// A short bright comet running clockwise around the dialog border.
fn paint_border_runner(frame: &mut Frame<'_>, rect: Rect, tick: u64) {
    if rect.width < 4 || rect.height < 4 {
        return;
    }
    let perimeter = border_perimeter(rect);
    if perimeter == 0 {
        return;
    }

    let head = (tick % u64::from(perimeter)) as u16;
    let buffer = frame.buffer_mut();
    for trail in 0..6u16 {
        // Walk backwards in `u32` so a wide dialog cannot overflow the sum.
        let index = ((u32::from(head) + u32::from(perimeter) - u32::from(trail))
            % u32::from(perimeter)) as u16;
        let (x, y) = perimeter_point(rect, index);
        let color = lerp_color(TERMINAL_GREEN, HAIRLINE_HI, trail as f32 / 6.0);
        if let Some(cell) = buffer.cell_mut((x, y)) {
            cell.set_fg(color);
        }
    }
}

/// Number of border cells `perimeter_point` walks, saturating rather than
/// overflowing `u16` for an implausibly large rect.
fn border_perimeter(rect: Rect) -> u16 {
    rect.width
        .saturating_sub(1)
        .saturating_add(rect.height.saturating_sub(1))
        .saturating_mul(2)
}

fn perimeter_point(rect: Rect, index: u16) -> (u16, u16) {
    let across = rect.width.saturating_sub(1);
    let down = rect.height.saturating_sub(1);
    let perimeter = border_perimeter(rect);
    let index = if perimeter == 0 { 0 } else { index % perimeter };

    // Each branch subtracts only within the side it has already matched, so the
    // differences stay non-negative; the adds saturate because `Rect` fields are
    // caller-supplied.
    if index < across {
        (rect.x.saturating_add(index), rect.y)
    } else if index < across.saturating_add(down) {
        (
            rect.x.saturating_add(across),
            rect.y.saturating_add(index - across),
        )
    } else if index < across.saturating_add(down).saturating_add(across) {
        (
            rect.x
                .saturating_add(across)
                .saturating_sub(index - across - down),
            rect.y.saturating_add(down),
        )
    } else {
        (
            rect.x,
            rect.y
                .saturating_add(down)
                .saturating_sub(index.saturating_sub(across + down + across)),
        )
    }
}

fn shimmer_hits(tick: u64, row: usize, column: usize, span: usize) -> bool {
    let period = (span + 10) as u64;
    tick / 2 % period == (row + column) as u64
}

fn cell_accent(index: usize, total: usize) -> Color {
    let position = if total <= 1 {
        0.0
    } else {
        index as f32 / (total - 1) as f32
    };
    if position < 0.5 {
        lerp_color(TERMINAL_GREEN, DIM_GREEN, position * 2.0)
    } else {
        lerp_color(DIM_GREEN, SOFT_GREEN, (position - 0.5) * 2.0)
    }
}

// ------------------------------------------------------------------ plumbing

fn resolve_shell_profile(config: &Config) -> Result<(String, Profile)> {
    resolve_shell_profile_from(
        config,
        env::var("GRIDBASH_PROFILE").ok(),
        env::var("GRIDBASH_INVOKING_PROFILE").ok(),
    )
}

/// The composer always builds a shell grid, so a configured agent default never
/// wins here; agents are chosen per pane once the grid is up.
fn resolve_shell_profile_from(
    config: &Config,
    environment_profile: Option<String>,
    invoking_profile: Option<String>,
) -> Result<(String, Profile)> {
    let available = startup_profiles(config);
    let pick = |name: &str| available.iter().find(|(key, _)| key == name).cloned();

    if let Some(found) = environment_profile.and_then(|name| pick(&name)) {
        return Ok(found);
    }
    if let Some(found) = invoking_profile.and_then(|name| pick(&name)) {
        return Ok(found);
    }
    if let Some(found) = config
        .defaults
        .profile
        .clone()
        .and_then(|name| pick(&name))
        .filter(|(name, profile)| is_terminal_profile(name) || !is_agent_profile(name, profile))
    {
        return Ok(found);
    }
    if let Some(found) = SHELL_PREFERENCE.iter().find_map(|name| pick(name)) {
        return Ok(found);
    }
    if let Some(found) = config.defaults.profile.clone().and_then(|name| pick(&name)) {
        return Ok(found);
    }

    available
        .iter()
        .find(|(name, profile)| !is_agent_profile(name, profile))
        .or_else(|| available.first())
        .cloned()
        .ok_or_else(|| {
            anyhow!("no launchable shell was found; install Git Bash or another supported shell")
        })
}

fn is_enter_key(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Enter | KeyCode::Char('\r') | KeyCode::Char('\n')
    ) || (key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('m') | KeyCode::Char('M')))
}

fn resolve_project_path(input: &str, base: &Path) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("project folder cannot be empty"));
    }
    let path = PathBuf::from(expand_home(input));
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

/// Read the folder `input` points into once and return `(siblings, matches)`,
/// both as full replacement text so the user's separator style and any `~`
/// prefix survive completion.
fn scan_folders(input: &str, base: &Path) -> (Vec<String>, Vec<String>) {
    let (parent_text, prefix) = split_completion_input(input);
    let parent = absolute_from(parent_text, base).unwrap_or_else(|| base.to_path_buf());

    let Ok(entries) = fs::read_dir(&parent) else {
        return (Vec::new(), Vec::new());
    };
    let mut names = entries
        .flatten()
        .filter(is_directory_entry)
        .filter_map(|entry| entry.file_name().into_string().ok())
        // Hidden folders only show up once the user asks for them.
        .filter(|name| !name.starts_with('.') || prefix.starts_with('.'))
        .collect::<Vec<_>>();
    names.sort_by_key(|name| (name.to_lowercase(), name.clone()));
    names.truncate(MAX_SUGGESTIONS);

    let needle = prefix.to_lowercase();
    let siblings = names
        .into_iter()
        .map(|name| format!("{parent_text}{name}"))
        .collect::<Vec<_>>();
    let matches = siblings
        .iter()
        .filter(|candidate| leaf_name(candidate).to_lowercase().starts_with(&needle))
        .cloned()
        .collect();
    (siblings, matches)
}

fn split_completion_input(input: &str) -> (&str, &str) {
    match input.rfind(['/', '\\']) {
        Some(index) => input.split_at(index + 1),
        None => ("", input),
    }
}

fn absolute_from(text: &str, base: &Path) -> Option<PathBuf> {
    if text.is_empty() {
        return None;
    }
    let path = PathBuf::from(expand_home(text));
    Some(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

fn has_subdirectories(candidate: &str, base: &Path) -> bool {
    let Some(path) = absolute_from(candidate, base) else {
        return false;
    };
    fs::read_dir(path)
        .map(|entries| entries.flatten().any(|entry| is_directory_entry(&entry)))
        .unwrap_or(false)
}

/// This runs once per entry on every keystroke in the project field. `file_type`
/// comes from the directory enumeration itself, so only symlinks pay for a stat.
fn is_directory_entry(entry: &fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_dir() => true,
        Ok(file_type) if file_type.is_symlink() => entry.path().is_dir(),
        _ => false,
    }
}

fn separator_for(value: &str) -> char {
    if value.contains('/') {
        '/'
    } else if value.contains('\\') {
        '\\'
    } else {
        std::path::MAIN_SEPARATOR
    }
}

fn expand_home(value: &str) -> String {
    let Some(rest) = value.strip_prefix('~') else {
        return value.to_string();
    };
    if !(rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')) {
        return value.to_string();
    }
    let Some(home) = home_dir() else {
        return value.to_string();
    };
    format!("{}{rest}", home.display())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn longest_common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    let mut shared = first.chars().collect::<Vec<_>>();
    for value in values.iter().skip(1) {
        let candidate = value.chars().collect::<Vec<_>>();
        let keep = shared
            .iter()
            .zip(candidate.iter())
            .take_while(|(left, right)| left.eq_ignore_ascii_case(right))
            .count();
        shared.truncate(keep);
        if shared.is_empty() {
            break;
        }
    }
    shared.into_iter().collect()
}

fn leaf_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(index) => trimmed[index + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

fn plural(word: &str, count: usize) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}S")
    }
}

fn leaf_after(branch: &str) -> String {
    branch
        .rsplit_once('/')
        .map(|(_, leaf)| leaf.to_string())
        .unwrap_or_else(|| branch.to_string())
}

fn truncate(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return value.to_string();
    }
    chars
        .into_iter()
        .take(width.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

fn short_path(path: &Path, width: usize) -> String {
    let text = display_path(path);
    let length = text.chars().count();
    if length <= width {
        return text;
    }
    let tail = text
        .chars()
        .skip(length.saturating_sub(width.saturating_sub(1)))
        .collect::<String>();
    format!("…{tail}")
}

fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn keycap(text: &'static str) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(TERMINAL_GREEN)
            .bg(RAISED_BG)
            .add_modifier(Modifier::BOLD),
    )
}

fn label(text: &'static str) -> Span<'static> {
    Span::styled(format!(" {text}   "), Style::default().fg(MUTED))
}

fn panel_style() -> Style {
    Style::default().fg(TEXT).bg(PANEL_BG)
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(x),
        y: area.y.saturating_add(y),
        width: area.width.saturating_sub(x.saturating_mul(2)),
        height: area.height.saturating_sub(y.saturating_mul(2)),
    }
}

fn pulse(color: Color, tick: u64) -> Color {
    let phase = (tick % 24) as f32 / 24.0;
    let amount = (phase * std::f32::consts::TAU).sin().abs() * 0.45;
    lerp_color(color, Color::White, amount)
}

fn dim_color(color: Color) -> Color {
    lerp_color(color, CANVAS_BG, 0.45)
}

fn lerp_color(from: Color, to: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let (from_r, from_g, from_b) = rgb_parts(from);
    let (to_r, to_g, to_b) = rgb_parts(to);
    Color::Rgb(
        lerp_channel(from_r, to_r, amount),
        lerp_channel(from_g, to_g, amount),
        lerp_channel(from_b, to_b, amount),
    )
}

fn lerp_channel(from: u8, to: u8, amount: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * amount)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn rgb_parts(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(red, green, blue) => (red, green, blue),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        _ => (128, 128, 128),
    }
}

fn hash3(x: u64, y: u64, z: u64) -> u64 {
    let mut value = x.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ y.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ z.wrapping_mul(0x1656_67B1_9E37_79F9);
    value ^= value >> 29;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 32;
    value
}

fn square_preview_rects(area: Rect, grid: GridSize) -> Vec<Rect> {
    // The picker clamps its dimensions, but `GridSize` is a plain struct any
    // caller can fill in. Capping here keeps a bad size from becoming a
    // multi-billion-iteration loop and an allocation that cannot be satisfied.
    let rows = grid.rows.min(MAX_PANES) as u16;
    let columns = grid.columns.min(MAX_PANES) as u16;
    if rows == 0 || columns == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    // Cells sit flush against one another so the preview reads as a single grid
    // instead of scattered tiles. Each cell draws its own border, so touching
    // edges look like the interior rules of one table.
    let height_fit = area.height / rows;
    let width_fit = area.width / columns / 2;
    let side_height = height_fit.min(width_fit).max(1);
    let side_width = side_height.saturating_mul(2).max(1);
    let total_height = rows.saturating_mul(side_height).min(area.height);
    let total_width = columns.saturating_mul(side_width).min(area.width);
    let start_y = area
        .y
        .saturating_add(area.height.saturating_sub(total_height) / 2);
    let start_x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);

    let mut rects = Vec::with_capacity(usize::from(rows) * usize::from(columns));
    for row in 0..rows {
        for column in 0..columns {
            rects.push(Rect {
                x: start_x.saturating_add(column.saturating_mul(side_width)),
                y: start_y.saturating_add(row.saturating_mul(side_height)),
                width: side_width,
                height: side_height,
            });
        }
    }
    rects
}

// -------------------------------------------------------------- grid resizer

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridPickerAction {
    Continue,
    Confirm(GridSize),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DimensionField {
    Rows,
    Columns,
}

/// Resize overlay for a grid that is already running.
#[derive(Debug, Clone)]
pub struct GridPicker {
    initial: GridSize,
    rows: usize,
    columns: usize,
    active_field: DimensionField,
    pane_summaries: Vec<Option<String>>,
    opened: Instant,
}

impl GridPicker {
    pub fn new(grid: GridSize) -> Self {
        Self {
            initial: grid,
            rows: grid.rows,
            columns: grid.columns,
            active_field: DimensionField::Rows,
            pane_summaries: Vec::new(),
            opened: Instant::now(),
        }
    }

    pub fn with_pane_summaries(mut self, pane_summaries: Vec<Option<String>>) -> Self {
        self.pane_summaries = pane_summaries;
        self
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

    pub fn draw(&self, frame: &mut Frame<'_>, cwd: Option<&Path>) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(CANVAS_BG)),
            area,
        );
        let tick = self.opened.elapsed().as_millis() as u64 / FRAME_MS;

        let panel = if area.width >= 90 && area.height >= 26 {
            inset(area, 2, 1)
        } else {
            area
        };
        if panel.width < 10 || panel.height < 6 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(HAIRLINE_HI))
            .title(Line::from(vec![
                Span::styled("─┤ ", Style::default().fg(HAIRLINE_HI)),
                Span::styled("◆", Style::default().fg(RESIZE_ACCENT)),
                Span::styled(
                    " RESIZE GRID ",
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled("├─", Style::default().fg(HAIRLINE_HI)),
            ]))
            .style(panel_style());
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        paint_border_runner(frame, panel, tick);

        let content = inset(inner, if inner.width >= 8 { 2 } else { 0 }, 0);
        if content.width == 0 || content.height < 6 {
            return;
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(4),
                Constraint::Length(3),
            ])
            .split(content);

        self.draw_header(frame, chunks[0], cwd);
        self.draw_preview(frame, chunks[1], tick);
        self.draw_controls(frame, chunks[2]);
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect, cwd: Option<&Path>) {
        let context = cwd
            .map(display_path)
            .unwrap_or_else(|| format!("currently {}×{}", self.initial.rows, self.initial.columns));
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{:02}", self.rows),
                    Style::default()
                        .fg(RESIZE_ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ROWS  ×  ", Style::default().fg(MUTED)),
                Span::styled(
                    format!("{:02}", self.columns),
                    Style::default()
                        .fg(RESIZE_ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" COLS  =  ", Style::default().fg(MUTED)),
                Span::styled(
                    format!("{:02}", self.grid().count()),
                    Style::default().fg(SOFT_GREEN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" PANES", Style::default().fg(MUTED)),
            ]),
            Line::from(Span::styled(context, Style::default().fg(MUTED))),
        ];
        frame.render_widget(Paragraph::new(lines).style(panel_style()), area);
    }

    fn draw_preview(&self, frame: &mut Frame<'_>, area: Rect, tick: u64) {
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(SUNKEN_BG)),
            area,
        );
        paint_starfield(frame, area, tick, 0x2C4B);

        for (index, rect) in square_preview_rects(inset(area, 1, 0), self.grid())
            .into_iter()
            .enumerate()
        {
            let row = index / self.columns.max(1);
            let column = index % self.columns.max(1);
            draw_preview_cell(
                frame,
                rect,
                &PreviewCell {
                    accent: RESIZE_ACCENT,
                    revealed: true,
                    shimmer: shimmer_hits(tick, row, column, self.rows + self.columns),
                    title: format!("{:02}", index + 1),
                    subtitle: self.preview_summary(index).map(str::to_string),
                    fill: RESIZE_FILL,
                },
            );
        }
    }

    fn preview_summary(&self, index: usize) -> Option<&str> {
        // `GridSize` is a plain struct, and the loop that calls this already
        // guards its own division the same way. Dividing by a zero column count
        // here would panic mid-frame.
        let columns = self.columns.max(1);
        let row = index / columns;
        let column = index % columns;
        let old_index = (row < self.initial.rows && column < self.initial.columns)
            .then_some(row * self.initial.columns.max(1) + column)?;

        self.pane_summaries
            .get(old_index)
            .and_then(Option::as_deref)
            .filter(|summary| !summary.trim().is_empty())
    }

    fn draw_controls(&self, frame: &mut Frame<'_>, area: Rect) {
        let lines = vec![
            Line::from(vec![
                control_box(self.active_field == DimensionField::Rows, self.rows),
                Span::raw(" "),
                Span::styled("rows", Style::default().fg(MUTED)),
                Span::styled("  ×  ", Style::default().fg(HAIRLINE_HI)),
                control_box(self.active_field == DimensionField::Columns, self.columns),
                Span::raw(" "),
                Span::styled("cols", Style::default().fg(MUTED)),
            ]),
            Line::from(""),
            Line::from(vec![
                keycap("↑↓"),
                label("CHANGE"),
                keycap("←→"),
                label("SWITCH"),
                keycap("ENTER"),
                label("APPLY"),
                keycap("ESC"),
                label("CANCEL"),
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

fn control_box(active: bool, value: usize) -> Span<'static> {
    let style = if active {
        Style::default()
            .fg(Color::Black)
            .bg(RESIZE_ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(RESIZE_ACCENT)
            .bg(SUNKEN_BG)
            .add_modifier(Modifier::BOLD)
    };

    Span::styled(format!(" {value:>2} "), style)
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;

    use crate::auth::AgentKind;

    use super::*;

    /// `GridSize` is a plain struct, so a zero dimension can reach the picker.
    /// The preview loop already guarded its own division; the summary lookup it
    /// calls did not, and dividing by zero there would panic mid-frame.
    #[test]
    fn the_resize_preview_survives_a_zero_dimension() {
        let picker = GridPicker::new(GridSize {
            rows: 0,
            columns: 0,
        })
        .with_pane_summaries(vec![Some("busy".into())]);

        assert_eq!(picker.preview_summary(0), None);

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("build a test terminal");
        terminal
            .draw(|frame| picker.draw(frame, None))
            .expect("drawing a zero-sized grid must not panic");
    }

    const TEST_PROFILE: &str = "test-shell";

    fn test_profile() -> Profile {
        Profile {
            command: env::current_exe()
                .expect("test executable")
                .display()
                .to_string(),
            args: Vec::new(),
            title: Some("Test Shell".into()),
            agent_kind: None,
        }
    }

    fn shell_config() -> Config {
        let mut config = Config::default();
        config.profiles.insert(TEST_PROFILE.into(), test_profile());
        config.set_default_profile(TEST_PROFILE);
        config.auth.home = Some(env::temp_dir().join("gridbash-composer-no-auth-profiles"));
        config
    }

    /// Build a composer with a fixed profile so ambient `GRIDBASH_PROFILE` /
    /// `GRIDBASH_INVOKING_PROFILE` values cannot change what the test sees.
    fn composer_in(dir: PathBuf) -> Composer {
        Composer::with_profile(dir, None, "Grid 2", TEST_PROFILE.into(), test_profile())
            .expect("composer")
    }

    fn composer() -> Composer {
        composer_in(env::current_dir().expect("cwd"))
    }

    fn rendered(width: u16, height: u16, composer: &Composer, config: &Config) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|frame| composer.draw(frame, config))
            .expect("draw composer");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn press(composer: &mut Composer, code: KeyCode, config: &Config) {
        composer.handle_key(KeyEvent::new(code, KeyModifiers::NONE), config);
    }

    /// The scroll window is derived from the cursor. A cursor left past the end
    /// of a replaced value pushed `start` beyond `end`, and `clamp` panics when
    /// its bounds cross — mid-frame, with no way to recover the field.
    #[test]
    fn a_text_row_survives_a_cursor_past_the_end_of_its_value() {
        let long = "C:/Users/Jason/Documents/GitHub/gridbash/src/composer.rs";
        for cursor in [
            0,
            4,
            long.chars().count(),
            long.chars().count() + 500,
            usize::MAX,
        ] {
            for width in [0, 1, 6, 12, 400] {
                let field = TextField {
                    value: long.into(),
                    cursor,
                    limit: None,
                };
                let line = text_row("project", &field, true, 0, width, None, SOFT_GREEN);
                assert!(
                    !line.spans.is_empty(),
                    "cursor {cursor} at width {width} must still render"
                );
            }
        }

        // Non-ASCII values must window by character, not by byte.
        let field = TextField {
            value: "日本語のとてもながいパス/composer.rs".into(),
            cursor: usize::MAX,
            limit: None,
        };
        assert!(
            !text_row("project", &field, true, 0, 8, None, SOFT_GREEN)
                .spans
                .is_empty()
        );
    }

    /// A character index at the end of the value maps to a byte index equal to
    /// the value's length, and `String::remove` panics there.
    #[test]
    fn editing_a_text_field_with_a_desynced_cursor_does_not_panic() {
        let mut field = TextField {
            value: "café".into(),
            cursor: 99,
            limit: None,
        };

        assert!(!field.delete(), "there is nothing after the end to delete");
        assert!(!field.backspace(), "a cursor past the end removes nothing");
        assert_eq!(field.value, "café");

        field.cursor = field.len();
        assert!(field.backspace());
        assert_eq!(field.value, "caf");
        assert!(field.delete_word());
        assert!(field.value.is_empty());
    }

    #[test]
    fn launches_a_shell_grid_with_the_default_shape() {
        let config = shell_config();
        let composer = composer();
        let outcome = composer.build_outcome(&config).expect("outcome");

        assert_eq!(outcome.title, "Grid 2");
        assert_eq!(outcome.plan.grid.rows, DEFAULT_ROWS);
        assert_eq!(outcome.plan.grid.columns, DEFAULT_COLUMNS);
        assert_eq!(outcome.plan.panes.len(), DEFAULT_ROWS * DEFAULT_COLUMNS);
        assert!(
            outcome
                .plan
                .panes
                .iter()
                .all(|pane| pane.profile_name == TEST_PROFILE)
        );
    }

    #[test]
    fn agent_defaults_never_win_over_a_real_shell() {
        let mut config = Config::default();
        config.profiles.insert(
            "codex".into(),
            Profile {
                command: env::current_exe()
                    .expect("test executable")
                    .display()
                    .to_string(),
                args: Vec::new(),
                title: Some("Codex".into()),
                agent_kind: Some(AgentKind::Codex),
            },
        );
        config.set_default_profile("codex");
        config.auth.home = Some(env::temp_dir().join("gridbash-composer-agent-default"));

        let (name, _) = resolve_shell_profile_from(&config, None, None).expect("shell profile");
        assert_ne!(name, "codex");
        assert!(
            SHELL_PREFERENCE.contains(&name.as_str()),
            "{name} is not a shell"
        );
    }

    #[test]
    fn explicit_profile_environment_still_wins() {
        let config = shell_config();

        let (name, _) =
            resolve_shell_profile_from(&config, Some(TEST_PROFILE.into()), Some("cmd".into()))
                .expect("shell profile");
        assert_eq!(name, TEST_PROFILE);

        // A configured non-agent default is honoured when nothing overrides it.
        let (name, _) = resolve_shell_profile_from(&config, None, None).expect("shell profile");
        assert_eq!(name, TEST_PROFILE);

        // Unknown names fall through instead of failing the launch.
        let (name, _) =
            resolve_shell_profile_from(&config, Some("nope".into()), None).expect("shell profile");
        assert_eq!(name, TEST_PROFILE);
    }

    #[test]
    fn only_four_fields_are_reachable_and_enter_launches_from_each() {
        let config = shell_config();
        let mut composer = composer();

        assert_eq!(
            Field::ALL,
            [Field::Rows, Field::Columns, Field::Name, Field::Project]
        );
        for field in Field::ALL {
            composer.active = field;
            let event =
                composer.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &config);
            assert!(
                matches!(event, ComposerEvent::Launch(_)),
                "Enter did not launch from {field:?}"
            );
        }

        // Down wraps through exactly the four fields.
        composer.active = Field::Rows;
        for expected in [Field::Columns, Field::Name, Field::Project, Field::Rows] {
            press(&mut composer, KeyCode::Down, &config);
            assert_eq!(composer.active, expected);
        }

        // Tab advances everywhere except Project, where it completes instead.
        for expected in [Field::Columns, Field::Name, Field::Project] {
            press(&mut composer, KeyCode::Tab, &config);
            assert_eq!(composer.active, expected);
        }
        press(&mut composer, KeyCode::Tab, &config);
        assert_eq!(composer.active, Field::Project);
        press(&mut composer, KeyCode::BackTab, &config);
        assert_eq!(composer.active, Field::Name);
    }

    #[test]
    fn digits_and_arrows_shape_the_grid() {
        let config = shell_config();
        let mut composer = composer();

        composer.active = Field::Rows;
        press(&mut composer, KeyCode::Char('4'), &config);
        composer.active = Field::Columns;
        press(&mut composer, KeyCode::Char('0'), &config);
        assert_eq!(composer.grid().rows, 4);
        assert_eq!(composer.grid().columns, MAX_DIMENSION);

        press(&mut composer, KeyCode::Left, &config);
        assert_eq!(composer.grid().columns, MAX_DIMENSION - 1);
        for _ in 0..20 {
            press(&mut composer, KeyCode::Left, &config);
        }
        assert_eq!(composer.grid().columns, 1);
        assert_eq!(composer.grid().rows, 4);
    }

    #[test]
    fn typing_a_name_titles_the_grid_and_empty_names_fall_back() {
        let config = shell_config();
        let mut composer = composer();
        composer.active = Field::Name;
        for _ in 0..MAX_NAME_CHARS {
            press(&mut composer, KeyCode::Backspace, &config);
        }
        assert_eq!(composer.title(), "Grid 2");

        for ch in "  api  refactor ".chars() {
            press(&mut composer, KeyCode::Char(ch), &config);
        }
        assert_eq!(composer.title(), "api refactor");
    }

    #[test]
    fn tab_completes_and_then_cycles_project_folders() {
        let root = env::temp_dir().join(format!("gridbash-complete-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for name in ["alpha-one", "alpha-two", "beta"] {
            fs::create_dir_all(root.join(name)).expect("fixture dir");
        }
        fs::create_dir_all(root.join("alpha-one").join("nested")).expect("nested dir");

        let mut composer = composer_in(root.clone());
        composer.active = Field::Project;

        let base = display_path(&root.canonicalize().expect("canonical root"));
        composer
            .project
            .set(format!("{base}{}alpha", std::path::MAIN_SEPARATOR));
        composer.refresh_suggestions();
        assert_eq!(composer.suggestions.matches.len(), 2);

        // The first Tab only extends to the shared prefix instead of guessing.
        composer.complete_project();
        assert!(
            composer.project.value.ends_with("alpha-"),
            "got {}",
            composer.project.value
        );

        composer.complete_project();
        assert!(composer.project.value.ends_with("alpha-one"));
        composer.complete_project();
        assert!(composer.project.value.ends_with("alpha-two"));
        composer.complete_project();
        assert!(composer.project.value.ends_with("alpha-one"));
        assert!(composer.project_dir.is_some());

        // A unique match descends into the folder.
        composer
            .project
            .set(format!("{base}{}b", std::path::MAIN_SEPARATOR));
        composer.refresh_suggestions();
        composer.complete_project();
        assert!(composer.project.value.ends_with("beta"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unresolvable_projects_block_launch_with_a_visible_reason() {
        let config = shell_config();
        let mut composer = composer();
        composer.active = Field::Project;
        composer
            .project
            .set("definitely-not-a-gridbash-project".into());
        composer.refresh_project();
        assert!(composer.project_dir.is_none());

        let event = composer.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &config);
        assert!(matches!(event, ComposerEvent::Continue));
        let notice = composer.notice.clone().expect("notice");
        assert_eq!(notice.level, NoticeLevel::Error);
        assert!(notice.text.contains("does not exist"), "{}", notice.text);
    }

    #[test]
    fn worktrees_stay_on_by_default_and_fall_back_when_the_repo_cannot_host_them() {
        let config = shell_config();
        let mut composer = composer();
        assert!(composer.worktrees_enabled);

        composer.readiness = Some(WorktreeReadiness::Ready {
            base_slug: "main".into(),
        });
        assert!(composer.worktrees_active());
        assert_eq!(
            composer.branch_label(2).as_deref(),
            Some("gridbash/main-pane-03")
        );

        composer.readiness = Some(WorktreeReadiness::Blocked {
            reason: "uncommitted tracked changes in the base checkout".into(),
        });
        assert!(!composer.worktrees_active());
        assert!(composer.branch_label(0).is_none());

        composer.readiness = Some(WorktreeReadiness::Ready {
            base_slug: "main".into(),
        });
        composer.handle_key(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
            &config,
        );
        assert!(!composer.worktrees_active());
    }

    #[test]
    fn renders_the_full_dashboard_on_a_roomy_terminal() {
        let config = shell_config();
        let composer = composer();
        let text = rendered(140, 40, &composer, &config);

        assert!(text.contains("NEW GRID"));
        assert!(text.contains(WORDMARK[1]), "wordmark row missing");
        assert!(text.contains("SHAPE"));
        assert!(text.contains("IDENTITY"));
        assert!(text.contains("LIVE GRID"));
        assert!(text.contains("ROWS"));
        assert!(text.contains("COLUMNS"));
        assert!(text.contains("NAME"));
        assert!(text.contains("PROJECT"));
        assert!(text.contains("LAUNCH"));
        assert!(text.contains("PANES"));
        // The knobs this revamp removed must not come back.
        assert!(!text.contains("Auth"));
        assert!(!text.contains("START WORKSPACE"));
    }

    #[test]
    fn preview_cells_reveal_their_pane_number_and_branch() {
        let config = shell_config();
        let mut composer = composer();
        composer.readiness = Some(WorktreeReadiness::Ready {
            base_slug: "main".into(),
        });

        // Freshly changed shapes start dark and wake up in a diagonal wave. The
        // summary line always names the first and last branch, so count labels
        // rather than looking for any single one.
        let labels = |text: &str| text.matches("main-pane-").count();
        composer.shape_changed = Instant::now();
        let booting = labels(&rendered(140, 40, &composer, &config));

        composer.shape_changed = Instant::now() - Duration::from_secs(2);
        let settled = rendered(140, 40, &composer, &config);
        assert!(
            labels(&settled) > booting,
            "cells never revealed ({booting} labels before, {} after)",
            labels(&settled)
        );
        for pane in 1..=composer.grid().count() {
            assert!(
                settled.contains(&format!("main-pane-{pane:02}")),
                "pane {pane} missing its branch label"
            );
        }
    }

    /// The preview stands in for the real grid, so its cells sit flush: touching
    /// edges read as one table rather than a scatter of tiles.
    #[test]
    fn preview_cells_touch_without_gaps() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };
        let grid = GridSize {
            rows: 3,
            columns: 3,
        };
        let rects = square_preview_rects(area, grid);
        assert_eq!(rects.len(), 9);

        for row in 0..3 {
            for column in 0..3 {
                let cell = rects[row * 3 + column];
                if column > 0 {
                    let left = rects[row * 3 + column - 1];
                    assert_eq!(
                        left.x + left.width,
                        cell.x,
                        "column {column} leaves a horizontal gap"
                    );
                }
                if row > 0 {
                    let above = rects[(row - 1) * 3 + column];
                    assert_eq!(
                        above.y + above.height,
                        cell.y,
                        "row {row} leaves a vertical gap"
                    );
                }
            }
        }
    }

    /// The composer shares the resume picker's phosphor-green palette, so its
    /// accents must actually be green rather than the cyan they replaced.
    #[test]
    fn accent_palette_is_phosphor_green() {
        for (name, color) in [
            ("TERMINAL_GREEN", TERMINAL_GREEN),
            ("DIM_GREEN", DIM_GREEN),
            ("SOFT_GREEN", SOFT_GREEN),
        ] {
            let Color::Rgb(red, green, blue) = color else {
                panic!("{name} must be an rgb colour");
            };
            assert!(
                green > red && green > blue,
                "{name} is not green: rgb({red}, {green}, {blue})"
            );
        }
    }

    #[test]
    fn keeps_the_essentials_on_a_small_terminal() {
        let config = shell_config();
        let composer = composer();
        let text = rendered(72, 18, &composer, &config);

        assert!(text.contains("SHAPE"));
        assert!(text.contains("IDENTITY"));
        assert!(text.contains("LAUNCH"));
        assert!(text.contains("ENTER"));
    }

    #[test]
    fn draws_at_every_size_without_panicking() {
        let config = shell_config();
        let mut composer = composer();
        composer.active = Field::Project;
        composer.refresh_suggestions();

        for width in [8u16, 20, 40, 72, 96, 140, 200] {
            for height in [4u16, 8, 12, 18, 26, 40] {
                let _ = rendered(width, height, &composer, &config);
            }
        }
    }

    #[test]
    fn paints_the_animated_accents() {
        let config = shell_config();
        let composer = composer();
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).expect("test terminal");
        terminal
            .draw(|frame| composer.draw(frame, &config))
            .expect("draw composer");
        let buffer = terminal.backend().buffer();

        assert!(
            buffer.content().iter().any(|cell| cell.bg == RAISED_BG),
            "expected raised panels"
        );
        assert!(
            buffer.content().iter().any(|cell| cell.symbol() == "·"),
            "expected starfield dust"
        );
    }

    #[test]
    #[ignore = "developer aid: cargo test -- --ignored --nocapture dump_composer"]
    fn dump_composer() {
        let config = shell_config();
        let mut composer = composer();
        composer.rows = 2;
        composer.columns = 3;
        composer.readiness = Some(WorktreeReadiness::Ready {
            base_slug: "main".into(),
        });
        // Let the reveal animation finish so the dump shows the settled screen.
        composer.shape_changed = Instant::now() - Duration::from_secs(2);
        for (width, height, field) in [
            (140u16, 40u16, Field::Rows),
            (140, 40, Field::Project),
            (96, 30, Field::Name),
            (80, 24, Field::Rows),
        ] {
            composer.active = field;
            composer.refresh_suggestions();
            let mut terminal =
                Terminal::new(TestBackend::new(width, height)).expect("test terminal");
            terminal
                .draw(|frame| composer.draw(frame, &config))
                .expect("draw");
            let buffer = terminal.backend().buffer();
            println!("\n===== {width}x{height} focus={field:?} =====");
            for row in 0..height {
                let line = (0..width)
                    .map(|column| {
                        buffer
                            .cell((column, row))
                            .map(|cell| cell.symbol())
                            .unwrap_or(" ")
                    })
                    .collect::<String>();
                println!("|{}|", line.trim_end());
            }
        }
    }

    #[test]
    fn return_key_aliases_are_treated_as_enter() {
        assert!(is_enter_key(KeyEvent::new(
            KeyCode::Char('\r'),
            KeyModifiers::NONE,
        )));
        assert!(is_enter_key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::CONTROL,
        )));
        assert!(!is_enter_key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
        )));
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
    fn text_field_edits_stay_on_character_boundaries() {
        let mut field = TextField::new("héllo wörld", None);
        field.home();
        field.move_cursor(2);
        assert!(field.insert('X'));
        assert_eq!(field.value, "héXllo wörld");

        field.end();
        assert!(field.delete_word());
        assert_eq!(field.value, "héXllo ");

        let mut limited = TextField::new("", Some(2));
        assert!(limited.insert('a'));
        assert!(limited.insert('b'));
        assert!(!limited.insert('c'));
        assert_eq!(limited.value, "ab");
    }

    #[test]
    fn perimeter_walk_stays_on_the_border() {
        let rect = Rect {
            x: 3,
            y: 2,
            width: 9,
            height: 5,
        };
        for index in 0..64 {
            let (x, y) = perimeter_point(rect, index);
            assert!(x >= rect.x && x < rect.right(), "x {x} escaped {rect:?}");
            assert!(y >= rect.y && y < rect.bottom(), "y {y} escaped {rect:?}");
            assert!(
                x == rect.x || x == rect.right() - 1 || y == rect.y || y == rect.bottom() - 1,
                "({x},{y}) is not on the border of {rect:?}"
            );
        }
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
    fn resize_picker_renders_in_its_own_blue_and_shows_summaries() {
        let picker = GridPicker::new(GridSize {
            rows: 1,
            columns: 2,
        })
        .with_pane_summaries(vec![Some("reviewing code".into()), None]);
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).expect("test terminal");

        terminal
            .draw(|frame| picker.draw(frame, None))
            .expect("draw picker");

        let buffer = terminal.backend().buffer();
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.bg == RESIZE_FILL || cell.fg == RESIZE_ACCENT)
        );
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("RESIZE GRID"));
        assert!(text.contains("reviewing code"));
        assert!(!text.contains("waiting for output"));
    }

    #[test]
    fn resize_picker_keeps_summaries_at_retained_coordinates() {
        let mut picker = GridPicker::new(GridSize {
            rows: 2,
            columns: 3,
        })
        .with_pane_summaries(vec![
            Some("zero".into()),
            Some("one".into()),
            Some("removed two".into()),
            Some("three".into()),
            Some("four".into()),
            Some("removed five".into()),
        ]);

        picker.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

        assert_eq!(picker.preview_summary(0), Some("zero"));
        assert_eq!(picker.preview_summary(1), Some("one"));
        assert_eq!(picker.preview_summary(2), Some("three"));
        assert_eq!(picker.preview_summary(3), Some("four"));
    }
}
