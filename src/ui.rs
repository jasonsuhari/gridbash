use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::path::Path;
use vt100::Cell;

use crate::app::{App, PaneGroupView, PromptView, SettingsRow};

pub struct DrawState {
    pub grid_area: Rect,
    pub pane_rects: Vec<Rect>,
}

pub fn draw(frame: &mut Frame<'_>, app: &App) -> DrawState {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let grid_area = chunks[0];
    let status_area = chunks[1];
    let rects = app.pane_rects(grid_area);

    for (index, pane) in app.panes().iter().enumerate() {
        let Some(rect) = rects.get(index).copied() else {
            continue;
        };

        let focused = app.focus() == index;
        let selected = app.selected().contains(&index);
        let group = app.pane_group(index);
        let border_style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if let Some(group) = group {
            Style::default()
                .fg(rgb_color(group.color.rgb))
                .add_modifier(Modifier::BOLD)
        } else if pane.active {
            Style::default().fg(Color::Green)
        } else if pane.exited {
            Style::default().fg(Color::Red)
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

        let folder = app
            .pane_folder(index)
            .map(label_name)
            .unwrap_or_else(|| folder_label(pane.cwd()));
        let title = if let Some(worktree) = app.pane_worktree(index) {
            format!(" {} | {} | {}{} ", index + 1, folder, worktree, badge)
        } else {
            format!(" {} | {}{} ", index + 1, folder, badge)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        render_screen(
            frame,
            inner,
            pane.screen(),
            group.map(|group| group.color.rgb),
        );
        if let Some(group) = group {
            render_group_badge(frame, rect, group);
        }

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
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "LIVE",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(
            broadcast,
            Style::default().fg(if app.broadcast() {
                Color::Cyan
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
        render_settings(frame, area, &app.settings_rows());
    }
    if let Some(prompt) = app.prompt_view() {
        render_manager_prompt(frame, area, prompt);
    }

    DrawState {
        grid_area,
        pane_rects: rects,
    }
}

fn render_group_badge(frame: &mut Frame<'_>, rect: Rect, group: PaneGroupView) {
    let label = format!(" :3 {} ", group.label);
    let width = label.len() as u16;
    if rect.width <= width.saturating_add(2) {
        return;
    }

    let area = Rect {
        x: rect.x + rect.width - width - 1,
        y: rect.y,
        width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(label).style(
            Style::default()
                .fg(Color::Black)
                .bg(rgb_color(group.color.rgb))
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
}

fn render_manager_prompt(frame: &mut Frame<'_>, area: Rect, prompt: PromptView) {
    let width = area.width.saturating_sub(4).max(24);
    let prompt_area = Rect {
        x: area.x.saturating_add(2),
        y: area.y.saturating_add(area.height.saturating_sub(5)),
        width: width.min(area.width),
        height: 3,
    };
    frame.render_widget(Clear, prompt_area);

    let input = if prompt.input.is_empty() {
        Span::styled(
            "type instruction, Enter sends, Esc cancels",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::raw(prompt.input)
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" :3 {} ", prompt.label),
            Style::default()
                .fg(Color::Black)
                .bg(rgb_color(prompt.color.rgb))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        input,
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(rgb_color(prompt.color.rgb)))
        .title(" Manager ");
    frame.render_widget(
        Paragraph::new(line).block(block).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        ),
        prompt_area,
    );
}

fn render_settings(frame: &mut Frame<'_>, area: Rect, rows: &[SettingsRow]) {
    let modal = centered_rect(area, 72, 64);
    frame.render_widget(Clear, modal);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Sample settings",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("not wired yet", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];

    for row in rows {
        lines.push(settings_row(row));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
        Span::raw(" move  "),
        Span::styled("Space", Style::default().fg(Color::Yellow)),
        Span::raw(" toggle  "),
        Span::styled("-/+", Style::default().fg(Color::Yellow)),
        Span::raw(" adjust  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" close"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
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
        Style::default().fg(Color::Yellow)
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
        Span::styled(
            format!("{:>10}", row.value),
            Style::default().fg(Color::LightCyan),
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

fn render_screen(
    frame: &mut Frame<'_>,
    area: Rect,
    screen: &vt100::Screen,
    tint: Option<(u8, u8, u8)>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let lines = (0..area.height)
        .map(|row| render_screen_row(screen, row, area.width, tint))
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(tint_color(Color::Rgb(11, 15, 20), tint)),
        ),
        area,
    );
}

fn render_screen_row<'a>(
    screen: &vt100::Screen,
    row: u16,
    width: u16,
    tint: Option<(u8, u8, u8)>,
) -> Line<'a> {
    let mut spans = Vec::new();
    let mut current_style: Option<Style> = None;
    let mut current_text = String::new();

    for column in 0..width {
        let Some(cell) = screen.cell(row, column) else {
            push_cell_text(
                &mut spans,
                &mut current_style,
                &mut current_text,
                Style::default().bg(tint_color(Color::Rgb(11, 15, 20), tint)),
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
            cell_style(cell, tint),
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

fn cell_style(cell: &Cell, tint: Option<(u8, u8, u8)>) -> Style {
    let mut style = Style::default()
        .fg(vt_color(cell.fgcolor(), Color::Rgb(230, 237, 243)))
        .bg(tint_color(
            vt_color(cell.bgcolor(), Color::Rgb(11, 15, 20)),
            tint,
        ));

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

fn rgb_color((red, green, blue): (u8, u8, u8)) -> Color {
    Color::Rgb(red, green, blue)
}

fn tint_color(color: Color, tint: Option<(u8, u8, u8)>) -> Color {
    let Some((tint_red, tint_green, tint_blue)) = tint else {
        return color;
    };
    let Some((red, green, blue)) = color_to_rgb(color) else {
        return color;
    };

    Color::Rgb(
        blend_channel(red, tint_red),
        blend_channel(green, tint_green),
        blend_channel(blue, tint_blue),
    )
}

fn blend_channel(base: u8, tint: u8) -> u8 {
    ((base as u16 * 4 + tint as u16) / 5) as u8
}

fn color_to_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((128, 0, 0)),
        Color::Green => Some((0, 128, 0)),
        Color::Yellow => Some((128, 128, 0)),
        Color::Blue => Some((0, 0, 128)),
        Color::Magenta => Some((128, 0, 128)),
        Color::Cyan => Some((0, 128, 128)),
        Color::Gray => Some((128, 128, 128)),
        Color::DarkGray => Some((80, 80, 80)),
        Color::LightRed => Some((255, 85, 85)),
        Color::LightGreen => Some((85, 255, 85)),
        Color::LightYellow => Some((255, 255, 85)),
        Color::LightBlue => Some((85, 85, 255)),
        Color::LightMagenta => Some((255, 85, 255)),
        Color::LightCyan => Some((85, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
        _ => None,
    }
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
