use std::{
    env,
    io::Stdout,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::worktrees::ManagedWorktreeOptions;
use crate::{config::Config, layout::GridSize, profiles::find_profile, setup::LaunchPlan};

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
    worktrees: Option<ManagedWorktreeOptions>,
    picker: GridPicker,
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

impl Composer {
    pub fn new(current_dir: PathBuf, worktrees: Option<ManagedWorktreeOptions>) -> Self {
        let grid = GridSize {
            rows: DEFAULT_ROWS,
            columns: DEFAULT_COLUMNS,
        };
        Self {
            current_dir,
            worktrees,
            picker: GridPicker::new(grid),
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut ComposerTerminal,
        config: &Config,
    ) -> Result<Option<LaunchPlan>> {
        loop {
            terminal.draw(|frame| {
                self.picker
                    .draw(frame, GridPickerMode::Startup, Some(&self.current_dir))
            })?;

            if !event::poll(Duration::from_millis(50))? {
                continue;
            }

            let result = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match self.picker.handle_key(key) {
                        GridPickerAction::Continue => ComposerEvent::Continue,
                        GridPickerAction::Confirm(grid) => {
                            self.launch_plan(config, grid).map(ComposerEvent::Launch)?
                        }
                        GridPickerAction::Cancel => ComposerEvent::Quit,
                    }
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

    fn launch_plan(&self, config: &Config, grid: GridSize) -> Result<LaunchPlan> {
        let profile_name = startup_profile_name(config);
        let profile = find_profile(config, &profile_name)?;

        LaunchPlan::from_launch_options(
            profile_name,
            profile,
            self.current_dir.clone(),
            grid.count(),
            grid,
            self.worktrees.as_ref(),
        )
    }
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
    use std::env;

    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn launch_plan_uses_default_startup_grid_and_profile() {
        if env::var_os("GRIDBASH_PROFILE").is_some() {
            return;
        }

        let mut config = Config::default();
        config.set_default_profile("powershell");
        let current_dir = env::current_dir().expect("current dir");

        let composer = Composer::new(current_dir.clone(), None);
        let plan = composer
            .launch_plan(&config, composer.picker.grid())
            .expect("launch plan");

        assert_eq!(plan.grid.rows, DEFAULT_ROWS);
        assert_eq!(plan.grid.columns, DEFAULT_COLUMNS);
        assert_eq!(plan.panes.len(), DEFAULT_ROWS * DEFAULT_COLUMNS);
        assert!(
            plan.panes
                .iter()
                .all(|pane| { pane.profile_name == "powershell" && pane.cwd == current_dir })
        );
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
