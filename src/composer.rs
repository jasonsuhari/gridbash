use std::{
    env,
    io::Stdout,
    path::{Path, PathBuf},
    time::Duration,
};

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

use crate::{config::Config, layout::GridSize, profiles::find_profile, setup::LaunchPlan};

type ComposerTerminal = Terminal<CrosstermBackend<Stdout>>;

const DEFAULT_ROWS: usize = 2;
const DEFAULT_COLUMNS: usize = 3;
const MAX_DIMENSION: usize = 10;

pub struct Composer {
    current_dir: PathBuf,
    rows: usize,
    columns: usize,
    active_field: DimensionField,
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
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut ComposerTerminal,
        config: &Config,
    ) -> Result<Option<LaunchPlan>> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

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
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.active_field = DimensionField::Columns;
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
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Char('c') => {
                self.active_field = DimensionField::Columns;
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

    fn draw(&self, frame: &mut Frame<'_>) {
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
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(5),
            ])
            .split(inner);

        self.draw_header(frame, chunks[0]);
        self.draw_preview(frame, chunks[1]);
        self.draw_controls(frame, chunks[2]);
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let cwd = display_path(&self.current_dir);
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
        ];

        frame.render_widget(Paragraph::new(lines).style(panel_style()), area);
    }

    fn draw_preview(&self, frame: &mut Frame<'_>, area: Rect) {
        let preview_area = inset(area, 1, 0);
        let grid = GridSize {
            rows: self.rows,
            columns: self.columns,
        };
        let rects = square_preview_rects(preview_area, grid);

        for (index, rect) in rects.into_iter().enumerate() {
            if rect.width == 0 || rect.height == 0 {
                continue;
            }

            frame.render_widget(dithered_square(index, rect), rect);
        }
    }

    fn draw_controls(&self, frame: &mut Frame<'_>, area: Rect) {
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

fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn startup_profile_name(config: &Config) -> String {
    env::var("GRIDBASH_PROFILE")
        .ok()
        .or_else(|| config.defaults.profile.clone())
        .unwrap_or_else(|| "git-bash".into())
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

fn dithered_square(index: usize, rect: Rect) -> Paragraph<'static> {
    let lines = (0..rect.height)
        .map(|y| {
            let mut text = String::with_capacity(rect.width as usize);
            for x in 0..rect.width {
                let ch = match (x + y + index as u16) % 4 {
                    0 => '#',
                    1 => '.',
                    2 => ':',
                    _ => '.',
                };
                text.push(ch);
            }
            Line::from(text)
        })
        .collect::<Vec<_>>();

    Paragraph::new(lines).style(
        Style::default()
            .fg(Color::Rgb(78, 198, 220))
            .bg(Color::Rgb(12, 58, 72)),
    )
}
