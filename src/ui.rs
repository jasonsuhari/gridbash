use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::path::Path;
use vt100::Cell;

use crate::app::{App, GridPalette, SettingsRow};

pub struct DrawState {
    pub grid_area: Rect,
    pub pane_rects: Vec<Rect>,
}

const QUIET_MARKER: &str = " ●";

pub fn draw(frame: &mut Frame<'_>, app: &App) -> DrawState {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let grid_area = chunks[0];
    let status_area = chunks[1];
    let rects = app.pane_rects(grid_area);
    let palette = app.palette();

    for (index, pane) in app.panes().iter().enumerate() {
        let Some(rect) = rects.get(index).copied() else {
            continue;
        };

        let focused = app.focus() == index;
        let selected = app.selected().contains(&index);
        let border_style = if selected {
            Style::default()
                .fg(palette.selected())
                .add_modifier(Modifier::BOLD)
        } else if focused {
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD)
        } else if pane.exited {
            Style::default().fg(palette.exited())
        } else if pane.active {
            Style::default().fg(palette.active())
        } else if pane.output_quiet() {
            Style::default()
                .fg(palette.quiet())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let badge = if pane.exited {
            " exited"
        } else if pane.active {
            " active"
        } else if selected {
            " selected"
        } else {
            ""
        };
        let quiet_marker = if pane.output_quiet() {
            QUIET_MARKER
        } else {
            ""
        };

        let folder = app
            .pane_folder(index)
            .map(label_name)
            .unwrap_or_else(|| folder_label(pane.cwd()));
        let title = if let Some(worktree) = app.pane_worktree(index) {
            format!(
                " {}{} | {} | {}{} ",
                index + 1,
                quiet_marker,
                folder,
                worktree,
                badge
            )
        } else {
            format!(" {}{} | {}{} ", index + 1, quiet_marker, folder, badge)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        render_screen(frame, inner, pane.screen());

        if focused {
            set_terminal_cursor(frame, inner, pane.screen());
        }
    }

    let broadcast = if app.broadcast() {
        "BROADCAST"
    } else {
        "focused"
    };
    let status = Line::from(vec![
        Span::styled(
            " GridBash ",
            Style::default()
                .fg(Color::Black)
                .bg(palette.accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "LIVE",
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(
            broadcast,
            Style::default().fg(if app.broadcast() {
                palette.selected()
            } else {
                Color::Gray
            }),
        ),
        Span::raw(" | "),
        Span::raw(format!("{} selected", app.selected().len())),
        Span::raw(" | "),
        Span::raw(app.status().to_string()),
        Span::raw(" | Alt+q quit"),
    ]);
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(Color::Rgb(11, 15, 20))),
        status_area,
    );

    if app.settings_open() {
        render_settings(frame, area, &app.settings_rows(), palette);
    }

    DrawState {
        grid_area,
        pane_rects: rects,
    }
}

fn render_settings(frame: &mut Frame<'_>, area: Rect, rows: &[SettingsRow], palette: &GridPalette) {
    let modal = centered_rect(area, 72, 64);
    frame.render_widget(Clear, modal);

    let mut lines = vec![
        Line::from(vec![Span::styled(
            "Grid colors",
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    for row in rows {
        lines.push(settings_row(row));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Up/Down", Style::default().fg(palette.focus())),
        Span::raw(" move  "),
        Span::styled("Space", Style::default().fg(palette.focus())),
        Span::raw(" cycle  "),
        Span::styled("-/+", Style::default().fg(palette.focus())),
        Span::raw(" adjust  "),
        Span::styled("Esc", Style::default().fg(palette.focus())),
        Span::raw(" close"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent()))
        .title(" Settings ");
    frame.render_widget(
        Paragraph::new(lines).block(block).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        ),
        modal,
    );
}

fn settings_row(row: &SettingsRow) -> Line<'static> {
    let cursor = if row.selected { "> " } else { "  " };
    let cursor_style = if row.selected {
        Style::default().fg(row.value_color)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let label_style = if row.selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Line::from(vec![
        Span::styled(cursor, cursor_style),
        Span::styled(format!("{:<24}", row.label), label_style),
        Span::styled("■ ", Style::default().fg(row.value_color)),
        Span::styled(
            format!("{:<8}", row.value),
            Style::default().fg(row.value_color),
        ),
        Span::raw("   "),
        Span::styled(row.hint, Style::default().fg(Color::DarkGray)),
    ])
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = ratatui::layout::Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = ratatui::layout::Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn folder_label(cwd: &Path) -> String {
    let label = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| cwd.display().to_string());

    label_name(&label)
}

fn label_name(name: &str) -> String {
    let mut label = name.to_string();
    if !matches!(label.chars().last(), Some('/') | Some('\\')) {
        label.push('/');
    }

    label
}

fn render_screen(frame: &mut Frame<'_>, area: Rect, screen: &vt100::Screen) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let lines = (0..area.height)
        .map(|row| render_screen_row(screen, row, area.width))
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        ),
        area,
    );
}

fn render_screen_row<'a>(screen: &vt100::Screen, row: u16, width: u16) -> Line<'a> {
    let mut spans = Vec::new();
    let mut current_style: Option<Style> = None;
    let mut current_text = String::new();

    for column in 0..width {
        let Some(cell) = screen.cell(row, column) else {
            push_cell_text(
                &mut spans,
                &mut current_style,
                &mut current_text,
                Style::default(),
                " ",
            );
            continue;
        };

        if cell.is_wide_continuation() {
            continue;
        }

        let text = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        push_cell_text(
            &mut spans,
            &mut current_style,
            &mut current_text,
            cell_style(cell),
            text,
        );
    }

    flush_span(&mut spans, &mut current_style, &mut current_text);
    Line::from(spans)
}

fn push_cell_text<'a>(
    spans: &mut Vec<Span<'a>>,
    current_style: &mut Option<Style>,
    current_text: &mut String,
    style: Style,
    text: &str,
) {
    if current_style.is_some_and(|active| active == style) {
        current_text.push_str(text);
        return;
    }

    flush_span(spans, current_style, current_text);
    *current_style = Some(style);
    current_text.push_str(text);
}

fn flush_span<'a>(
    spans: &mut Vec<Span<'a>>,
    current_style: &mut Option<Style>,
    current_text: &mut String,
) {
    if current_text.is_empty() {
        return;
    }

    spans.push(Span::styled(
        std::mem::take(current_text),
        current_style.take().unwrap_or_default(),
    ));
}

fn cell_style(cell: &Cell) -> Style {
    let mut style = Style::default()
        .fg(vt_color(cell.fgcolor(), Color::Rgb(230, 237, 243)))
        .bg(vt_color(cell.bgcolor(), Color::Rgb(11, 15, 20)));

    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.dim() {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }

    style
}

fn vt_color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(index) => indexed_color(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

fn indexed_color(index: u8) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        15 => Color::White,
        16..=231 => {
            let index = index - 16;
            let red = ansi_cube_channel(index / 36);
            let green = ansi_cube_channel((index / 6) % 6);
            let blue = ansi_cube_channel(index % 6);
            Color::Rgb(red, green, blue)
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            Color::Rgb(gray, gray, gray)
        }
    }
}

fn ansi_cube_channel(value: u8) -> u8 {
    if value == 0 { 0 } else { 55 + value * 40 }
}

fn set_terminal_cursor(frame: &mut Frame<'_>, area: Rect, screen: &vt100::Screen) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (row, column) = screen.cursor_position();
    let x = area
        .x
        .saturating_add(column.min(area.width.saturating_sub(1)));
    let y = area
        .y
        .saturating_add(row.min(area.height.saturating_sub(1)));
    frame.set_cursor_position((x, y));
}
