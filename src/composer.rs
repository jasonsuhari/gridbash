use std::{env, io::Stdout, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
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
    config::Config,
    layout::{GridLayout, GridSize},
    profiles::find_profile,
    setup::LaunchPlan,
};

type ComposerTerminal = Terminal<CrosstermBackend<Stdout>>;

const DEFAULT_ROWS: usize = 2;
const DEFAULT_COLUMNS: usize = 3;
const MAX_DIMENSION: usize = 10;

pub struct Composer {
    current_dir: PathBuf,
    rows: usize,
    columns: usize,
    active_field: DimensionField,
    status: String,
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

impl Composer {
    pub fn new(current_dir: PathBuf) -> Self {
        Self {
            current_dir,
            rows: DEFAULT_ROWS,
            columns: DEFAULT_COLUMNS,
            active_field: DimensionField::Rows,
            status: "Left/Right choose rows or columns | Up/Down changes | Enter launches".into(),
        }
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
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    self.handle_key(key, config)?
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

    fn handle_key(&mut self, key: KeyEvent, config: &Config) -> Result<ComposerEvent> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Ok(ComposerEvent::Quit),
            KeyCode::Enter => self.launch_plan(config).map(ComposerEvent::Launch),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                self.active_field = DimensionField::Rows;
                self.status = "Editing rows".into();
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.active_field = DimensionField::Columns;
                self.status = "Editing columns".into();
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('k') => {
                self.adjust_active(1);
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Down | KeyCode::Char('-') | KeyCode::Char('j') => {
                self.adjust_active(-1);
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Char('r') => {
                self.active_field = DimensionField::Rows;
                self.status = "Editing rows".into();
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Char('c') => {
                self.active_field = DimensionField::Columns;
                self.status = "Editing columns".into();
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                self.set_active_from_digit(ch);
                Ok(ComposerEvent::Continue)
            }
            _ => Ok(ComposerEvent::Continue),
        }
    }

    fn adjust_active(&mut self, delta: isize) {
        let value = match self.active_field {
            DimensionField::Rows => &mut self.rows,
            DimensionField::Columns => &mut self.columns,
        };
        *value = (*value as isize + delta).clamp(1, MAX_DIMENSION as isize) as usize;
        self.status = format!(
            "Grid set to {} row(s) x {} column(s): {} panes",
            self.rows,
            self.columns,
            self.rows * self.columns
        );
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
        self.status = format!(
            "Grid set to {} row(s) x {} column(s): {} panes",
            self.rows,
            self.columns,
            self.rows * self.columns
        );
    }

    fn launch_plan(&self, config: &Config) -> Result<LaunchPlan> {
        let grid = GridSize::new(self.rows, self.columns).context("invalid grid dimensions")?;
        let profile_name = startup_profile_name(config);
        let profile = find_profile(config, &profile_name)?;

        Ok(LaunchPlan::legacy(
            profile_name,
            profile,
            self.current_dir.clone(),
            grid.count(),
            grid,
        ))
    }

    fn draw(&self, frame: &mut Frame<'_>, config: &Config) {
        let area = frame.area();
        frame.render_widget(background(), area);

        let panel = centered_rect(area, 82, 28);
        frame.render_widget(Clear, panel);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" GridBash Startup ")
            .border_style(Style::default().fg(Color::Cyan))
            .style(panel_style());
        let inner = block.inner(panel);
        frame.render_widget(block, panel);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(8),
                Constraint::Length(5),
                Constraint::Length(2),
            ])
            .split(inner);

        self.draw_header(frame, chunks[0], config);
        self.draw_preview(frame, chunks[1]);
        self.draw_controls(frame, chunks[2]);
        frame.render_widget(status_bar(&self.status), chunks[3]);
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect, config: &Config) {
        let cwd = self.current_dir.display().to_string();
        let profile = startup_profile_name(config);
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    "Choose grid dimensions",
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
                Span::styled("cwd ", Style::default().fg(Color::DarkGray)),
                Span::styled(cwd, Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::styled("profile ", Style::default().fg(Color::DarkGray)),
                Span::styled(profile, Style::default().fg(Color::Gray)),
            ]),
        ];

        frame.render_widget(Paragraph::new(lines).style(panel_style()), area);
    }

    fn draw_preview(&self, frame: &mut Frame<'_>, area: Rect) {
        let preview_area = inset(area, 1, 0);
        let grid = GridSize {
            rows: self.rows,
            columns: self.columns,
        };
        let rects = GridLayout::new(grid).rects(preview_area, grid.count());

        for (index, rect) in rects.into_iter().enumerate() {
            if rect.width == 0 || rect.height == 0 {
                continue;
            }

            let border = if index == 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(54, 162, 183))
            };
            let title = if rect.width >= 8 {
                format!(" {} ", index + 1)
            } else {
                String::new()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border)
                .style(Style::default().bg(Color::Rgb(13, 24, 31)));
            frame.render_widget(block, rect);
        }
    }

    fn draw_controls(&self, frame: &mut Frame<'_>, area: Rect) {
        let lines = vec![
            Line::from(""),
            Line::from(vec![
                control_box(self.active_field == DimensionField::Rows, self.rows, "r"),
                Span::styled("  x  ", Style::default().fg(Color::DarkGray)),
                control_box(
                    self.active_field == DimensionField::Columns,
                    self.columns,
                    "c",
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
                Span::raw(" change  "),
                Span::styled("Left/Right", Style::default().fg(Color::Yellow)),
                Span::raw(" switch  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(" launch"),
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

fn startup_profile_name(config: &Config) -> String {
    env::var("GRIDBASH_PROFILE")
        .ok()
        .or_else(|| config.defaults.profile.clone())
        .unwrap_or_else(|| "git-bash".into())
}

fn control_box<'a>(active: bool, value: usize, label: &'static str) -> Span<'a> {
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

    Span::styled(format!(" {value:>2} {label} "), style)
}

fn status_bar<'a>(status: &str) -> Paragraph<'a> {
    Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(status.to_string(), Style::default().fg(Color::Gray)),
    ]))
    .style(panel_style())
}

fn background() -> Paragraph<'static> {
    Paragraph::new("").style(Style::default().bg(Color::Rgb(7, 11, 15)))
}

fn panel_style() -> Style {
    Style::default()
        .fg(Color::Rgb(230, 237, 243))
        .bg(Color::Rgb(11, 15, 20))
}

fn centered_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(x),
        y: area.y.saturating_add(y),
        width: area.width.saturating_sub(x.saturating_mul(2)),
        height: area.height.saturating_sub(y.saturating_mul(2)),
    }
}
