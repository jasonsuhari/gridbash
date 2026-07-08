use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::path::Path;
use vt100::Cell;

use crate::{
    app::{App, GridPalette, PaneSelection, RenamePaneView, RenameTabView, SettingsRow, TabLabel},
    image_preview::ImagePreview,
};

const APP_BG: Color = Color::Rgb(11, 15, 20);
const SETTINGS_BG: Color = Color::Rgb(9, 14, 19);
const SETTINGS_SURFACE: Color = Color::Rgb(14, 22, 29);
const SETTINGS_ROW_ACTIVE: Color = Color::Rgb(25, 36, 44);
const SETTINGS_SHADOW: Color = Color::Rgb(4, 6, 10);
const SETTINGS_BORDER: Color = Color::Rgb(58, 210, 210);
const SETTINGS_MUTED: Color = Color::Rgb(118, 135, 149);
const SETTINGS_TEXT: Color = Color::Rgb(230, 237, 243);

pub struct DrawState {
    pub grid_area: Rect,
    pub pane_rects: Vec<Rect>,
}

const QUIET_MARKER: &str = " *";

pub fn draw(frame: &mut Frame<'_>, app: &App) -> DrawState {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let tab_area = chunks[0];
    let grid_area = chunks[1];
    let status_area = chunks[2];
    let rects = app.pane_rects(grid_area);
    let palette = app.palette();
    let rename_view = app.rename_pane_view();
    let tab_rename_view = app.rename_tab_view();
    let image_overlay = app.image_overlay_view();
    let modal_open = app.settings_open()
        || rename_view.is_some()
        || tab_rename_view.is_some()
        || image_overlay.is_some();

    render_tabs(frame, tab_area, &app.tab_labels());

    for (index, pane) in app.panes().iter().enumerate() {
        let Some(rect) = rects.get(index).copied() else {
            continue;
        };

        let focused = app.focus() == index;
        let selected = app.selected().contains(&index);
        let sleeping = app.pane_sleeping(index);
        let quiet = app.activity_badges_enabled() && pane.output_quiet();
        let chrome = pane_chrome(
            selected,
            focused,
            pane.active,
            pane.exited,
            sleeping,
            quiet,
            palette,
        );

        let folder = app
            .pane_folder(index)
            .map(label_name)
            .unwrap_or_else(|| folder_label(pane.cwd()));
        let usage = app.pane_usage_label(index);
        let title = pane_title(
            &app.pane_label(index),
            chrome.quiet_marker,
            &folder,
            app.pane_worktree(index),
            usage.as_deref(),
            chrome.badge,
        );

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(chrome.border_style)
            .title(title);
        if let Some(footer) =
            app.pane_conversation_footer(index, rect.width.saturating_sub(4) as usize)
        {
            block = block.title_bottom(conversation_footer(footer, focused || selected));
        }

        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if sleeping {
            render_sleeping_screen(frame, inner);
        } else {
            render_screen(frame, inner, pane.screen(), app.selection_for_pane(index));
        }

        if focused && !sleeping && !modal_open {
            set_terminal_cursor(frame, inner, pane.screen());
        }
    }

    let input_scope = if app.selected().len() > 1 {
        "selected panes"
    } else {
        "focused pane"
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
            input_scope,
            Style::default().fg(if app.selected().len() > 1 {
                palette.selected()
            } else {
                Color::Gray
            }),
        ),
        Span::raw(" | "),
        Span::raw(format!("{} selected", app.selected().len())),
        Span::raw(" | "),
        Span::raw(app.status().to_string()),
        Span::raw(" | Alt+x swap | Alt+z sleep | hover wakes | Alt+q quit"),
    ]);
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(APP_BG)),
        status_area,
    );

    if app.settings_open() {
        render_settings(frame, area, &app.settings_rows(), palette);
    }
    if let Some(rename) = rename_view.as_ref() {
        render_rename_pane(frame, area, rename);
    }
    if let Some(rename) = tab_rename_view.as_ref() {
        render_rename_tab(frame, area, rename);
    }
    if let Some(image) = image_overlay {
        render_image_overlay(frame, area, image);
    }

    DrawState {
        grid_area,
        pane_rects: rects,
    }
}

fn pane_title(
    label: &str,
    quiet_marker: &str,
    folder: &str,
    worktree: Option<&str>,
    usage: Option<&str>,
    badge: &str,
) -> String {
    let label = format!("{label}{quiet_marker}");
    let usage = usage.map(|label| format!(" | {label}")).unwrap_or_default();
    if let Some(worktree) = worktree {
        format!(" {label} | {folder} | {worktree}{usage}{badge} ")
    } else {
        format!(" {label} | {folder}{usage}{badge} ")
    }
}

fn render_tabs(frame: &mut Frame<'_>, area: Rect, tabs: &[TabLabel]) {
    let mut spans = vec![Span::styled(
        " GridBash ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    spans.push(Span::raw(" "));
    for (index, tab) in tabs.iter().enumerate() {
        let marker = if tab.exited {
            "!"
        } else if tab.activity && !tab.active {
            "*"
        } else {
            ""
        };
        let label = format!(
            " {}:{}{} ",
            index + 1,
            truncate_text(&tab.title, 18),
            marker
        );
        let style = if tab.active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if tab.exited {
            Style::default().fg(Color::Red)
        } else if tab.activity {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label, style));
    }

    spans.push(Span::raw(" "));
    spans.push(Span::styled("Alt+t", Style::default().fg(Color::Yellow)));
    spans.push(Span::raw(" next  "));
    spans.push(Span::styled("Alt+n", Style::default().fg(Color::Yellow)));
    spans.push(Span::raw(" new  "));
    spans.push(Span::styled(
        "Alt+Shift+r",
        Style::default().fg(Color::Yellow),
    ));
    spans.push(Span::raw(" rename tab"));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(APP_BG)),
        area,
    );
}

#[derive(Debug, PartialEq)]
struct PaneChrome {
    border_style: Style,
    badge: &'static str,
    quiet_marker: &'static str,
}

fn pane_chrome(
    selected: bool,
    focused: bool,
    _active: bool,
    exited: bool,
    sleeping: bool,
    quiet: bool,
    palette: &GridPalette,
) -> PaneChrome {
    let border_style = if sleeping {
        Style::default().fg(Color::Rgb(32, 36, 42))
    } else if selected {
        Style::default()
            .fg(palette.selected())
            .add_modifier(Modifier::BOLD)
    } else if focused {
        Style::default()
            .fg(palette.focus())
            .add_modifier(Modifier::BOLD)
    } else if exited {
        Style::default().fg(palette.exited())
    } else if quiet {
        Style::default()
            .fg(palette.quiet())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let badge = if exited {
        " exited"
    } else if sleeping {
        " asleep"
    } else if selected {
        " selected"
    } else {
        ""
    };
    let quiet_marker = if quiet && !exited && !sleeping {
        QUIET_MARKER
    } else {
        ""
    };

    PaneChrome {
        border_style,
        badge,
        quiet_marker,
    }
}

fn render_settings(frame: &mut Frame<'_>, area: Rect, rows: &[SettingsRow], palette: &GridPalette) {
    let modal = settings_modal_rect(area, rows.len());
    let shadow = settings_shadow_rect(area, modal);

    if shadow != modal {
        frame.render_widget(Clear, shadow);
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(SETTINGS_SHADOW)),
            shadow,
        );
    }

    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" GridBash Settings ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    frame.render_widget(
        Paragraph::new(settings_lines(rows, inner.width)).style(settings_panel_style()),
        inner,
    );
}

fn render_rename_pane(frame: &mut Frame<'_>, area: Rect, rename: &RenamePaneView) {
    let modal = centered_rect(area, 62, 28);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Rename Pane ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    let header = Line::from(vec![
        Span::styled(
            format!("Pane {}", rename.pane_index + 1),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("currently {}", rename.pane_label),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(Color::Rgb(230, 237, 243))),
        chunks[0],
    );

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Name ");
    let input_inner = input_block.inner(chunks[1]);
    let input_line = if rename.value.is_empty() {
        Line::from(Span::styled(
            "blank restores number",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(rename.value.clone())
    };
    frame.render_widget(
        Paragraph::new(input_line).block(input_block).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        ),
        chunks[1],
    );

    let help = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" save  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" cancel  "),
        Span::styled("Ctrl+u", Style::default().fg(Color::Yellow)),
        Span::raw(" clear"),
    ]);
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Gray)),
        chunks[2],
    );

    if input_inner.width > 0 && input_inner.height > 0 {
        let cursor = rename.cursor.min(rename.value.chars().count()) as u16;
        let x = input_inner
            .x
            .saturating_add(cursor.min(input_inner.width.saturating_sub(1)));
        frame.set_cursor_position((x, input_inner.y));
    }
}

fn render_rename_tab(frame: &mut Frame<'_>, area: Rect, rename: &RenameTabView) {
    let modal = centered_rect(area, 62, 28);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Rename Tab ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    let header = Line::from(vec![
        Span::styled(
            "Current tab",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(rename.title.clone(), Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(Color::Rgb(230, 237, 243))),
        chunks[0],
    );

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Title ");
    let input_inner = input_block.inner(chunks[1]);
    let input_line = if rename.value.is_empty() {
        Line::from(Span::styled(
            "tab title required",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(rename.value.clone())
    };
    frame.render_widget(
        Paragraph::new(input_line).block(input_block).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        ),
        chunks[1],
    );

    let help = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" save  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" cancel  "),
        Span::styled("Ctrl+u", Style::default().fg(Color::Yellow)),
        Span::raw(" clear"),
    ]);
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Gray)),
        chunks[2],
    );

    if input_inner.width > 0 && input_inner.height > 0 {
        let cursor = rename.cursor.min(rename.value.chars().count()) as u16;
        let x = input_inner
            .x
            .saturating_add(cursor.min(input_inner.width.saturating_sub(1)));
        frame.set_cursor_position((x, input_inner.y));
    }
}

fn render_image_overlay(frame: &mut Frame<'_>, area: Rect, image: &ImagePreview) {
    let modal = image_modal_rect(area, image);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(SETTINGS_TEXT).bg(APP_BG))
        .title(format!(" Image | {} ", truncate_text(&image.title, 48)));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines = Vec::new();
    lines.push(image_meta_line(image, inner.width));
    lines.push(Line::from(""));

    let available_image_rows = inner.height.saturating_sub(4) as usize;
    let max_columns = inner.width as usize;
    for row in image.rows.iter().take(available_image_rows) {
        lines.push(image_row(row, max_columns));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        command_key("Esc"),
        Span::styled(" close  ", Style::default().fg(Color::Gray)),
        command_key("q"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ]));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(SETTINGS_TEXT).bg(APP_BG)),
        inner,
    );
}

fn image_meta_line(image: &ImagePreview, width: u16) -> Line<'static> {
    let text = format!(
        "{}x{} -> {}x{} cells | {}",
        image.source_width,
        image.source_height,
        image.cell_width,
        image.cell_height,
        image.path.display()
    );

    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            truncate_text(&text, width.saturating_sub(2) as usize),
            Style::default().fg(Color::Gray),
        ),
    ])
}

fn image_row(row: &[crate::image_preview::ImageCell], max_columns: usize) -> Line<'static> {
    let spans = row
        .iter()
        .take(max_columns)
        .map(|cell| {
            Span::styled(
                "\u{2580}",
                Style::default()
                    .fg(rgb(cell.upper))
                    .bg(rgb(cell.lower))
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect::<Vec<_>>();

    Line::from(spans)
}

fn rgb(value: [u8; 3]) -> Color {
    Color::Rgb(value[0], value[1], value[2])
}

fn image_modal_rect(area: Rect, image: &ImagePreview) -> Rect {
    let desired_width = image.cell_width.saturating_add(4).clamp(36, 92);
    let desired_height = image.cell_height.saturating_add(6).clamp(10, 34);
    let width = area.width.saturating_sub(4).min(desired_width).max(1);
    let height = area.height.saturating_sub(2).min(desired_height).max(1);

    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn settings_lines(rows: &[SettingsRow], width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Grid controls",
                Style::default()
                    .fg(SETTINGS_BORDER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "session preview",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(settings_summary(width), Style::default().fg(Color::Gray)),
        ]),
        Line::from(""),
        settings_section("DISPLAY", "title bar and state signals", width),
    ];

    for row in rows.iter().take(2) {
        lines.push(settings_row(row, width));
    }

    lines.push(Line::from(""));
    lines.push(settings_section(
        "WORKFLOW",
        "guard rails for high-speed sessions",
        width,
    ));
    if let Some(row) = rows.get(2) {
        lines.push(settings_row(row, width));
    }

    lines.push(Line::from(""));
    lines.push(settings_section(
        "PERFORMANCE",
        "spacing and terminal budget",
        width,
    ));
    for row in rows.iter().skip(3).take(3) {
        lines.push(settings_row(row, width));
    }

    lines.push(Line::from(""));
    lines.push(settings_section(
        "THEME",
        "runtime palette for grid chrome",
        width,
    ));
    for row in rows.iter().skip(6) {
        lines.push(settings_row(row, width));
    }

    lines.push(Line::from(""));
    lines.push(settings_command_bar(width));
    lines
}

fn conversation_footer(summary: String, emphasized: bool) -> Line<'static> {
    let summary_style = if emphasized {
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Line::from(vec![
        Span::raw(" "),
        Span::styled("conv ", Style::default().fg(Color::Cyan)),
        Span::styled(summary, summary_style),
        Span::raw(" "),
    ])
}

fn settings_summary(width: u16) -> String {
    let text = if width < 70 {
        "Refine pane chrome, safety prompts, and highlight color."
    } else {
        "Refine pane chrome, safety prompts, performance, and highlight color."
    };
    truncate_text(text, width.saturating_sub(2) as usize)
}

fn settings_section(title: &'static str, helper: &'static str, width: u16) -> Line<'static> {
    let used = 2 + title.len() + 2;
    let helper = width
        .checked_sub(used as u16)
        .filter(|available| *available >= 10)
        .map(|available| truncate_text(helper, available as usize));
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            title,
            Style::default()
                .fg(SETTINGS_BORDER)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if let Some(helper) = helper {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(helper, Style::default().fg(SETTINGS_MUTED)));
    }

    Line::from(spans)
}

fn settings_row(row: &SettingsRow, width: u16) -> Line<'static> {
    let width = width as usize;
    let narrow = width < 66;
    let label_width = if narrow { 20 } else { 24 };
    let value_width = if narrow { 10 } else { 13 };
    let reserved = 2 + label_width + 2 + value_width + 2;
    let hint_width = width.saturating_sub(reserved);
    let marker = if row.selected { "> " } else { "  " };
    let label = fixed_width(row.label, label_width);
    let value = fixed_width(&settings_value_label(row), value_width);
    let hint = if hint_width >= 10 {
        truncate_text(row.hint, hint_width)
    } else {
        String::new()
    };
    let row_bg = row.selected.then_some(SETTINGS_ROW_ACTIVE);
    let mut used = marker.len() + label.len() + 2 + value.len();
    let mut spans = vec![
        Span::styled(marker.to_string(), row_style(Color::Yellow, row_bg, false)),
        Span::styled(label, row_style(SETTINGS_TEXT, row_bg, row.selected)),
        Span::styled("  ", row_style(SETTINGS_TEXT, row_bg, false)),
        Span::styled(value, settings_value_style(row)),
    ];

    if !hint.is_empty() {
        used += 2 + hint.len();
        spans.push(Span::styled("  ", row_style(SETTINGS_TEXT, row_bg, false)));
        spans.push(Span::styled(hint, row_style(SETTINGS_MUTED, row_bg, false)));
    }

    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            row_style(SETTINGS_TEXT, row_bg, false),
        ));
    }

    Line::from(spans)
}

fn settings_command_bar(width: u16) -> Line<'static> {
    if width < 50 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Arrows"),
            Span::styled(" adjust  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }

    if width < 58 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Up/Down"),
            Span::styled(" move  ", Style::default().fg(Color::Gray)),
            command_key("Left/Right"),
            Span::styled(" adjust  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }

    Line::from(vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" move  ", Style::default().fg(Color::Gray)),
        command_key("Enter/Space"),
        Span::styled(" toggle  ", Style::default().fg(Color::Gray)),
        command_key("Left/Right"),
        Span::styled(" adjust  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ])
}

fn command_key(label: &'static str) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

fn settings_value_label(row: &SettingsRow) -> String {
    match row.value.as_str() {
        "on" | "off" => format!("[ {} ]", row.value),
        _ if row.value_color.is_some() => format!("< {} >", row.value),
        _ => format!("- {} +", row.value),
    }
}

fn settings_value_style(row: &SettingsRow) -> Style {
    if let Some(color) = row.value_color {
        return Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD);
    }

    let mut style = match row.value.as_str() {
        "on" => Style::default()
            .fg(Color::Black)
            .bg(SETTINGS_BORDER)
            .add_modifier(Modifier::BOLD),
        "off" => Style::default().fg(SETTINGS_MUTED).bg(SETTINGS_SURFACE),
        _ if row.selected => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::LightCyan)
            .bg(SETTINGS_SURFACE)
            .add_modifier(Modifier::BOLD),
    };

    if row.selected && matches!(row.value.as_str(), "off") {
        style = style.fg(Color::White);
    }

    style
}

fn row_style(fg: Color, bg: Option<Color>, bold: bool) -> Style {
    let style = if let Some(bg) = bg {
        Style::default().fg(fg).bg(bg)
    } else {
        Style::default().fg(fg)
    };

    if bold {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn settings_panel_style() -> Style {
    Style::default().fg(SETTINGS_TEXT).bg(SETTINGS_BG)
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn settings_modal_rect(area: Rect, row_count: usize) -> Rect {
    let width = area.width.saturating_sub(4).min(88).max(area.width.min(1));
    let desired_height = (row_count as u16).saturating_add(14).max(21);
    let height = area
        .height
        .saturating_sub(2)
        .min(desired_height)
        .max(area.height.min(1));

    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn settings_shadow_rect(area: Rect, modal: Rect) -> Rect {
    let offset_x = if modal.x.saturating_add(modal.width).saturating_add(2)
        <= area.x.saturating_add(area.width)
    {
        2
    } else {
        0
    };
    let offset_y = if modal.y.saturating_add(modal.height).saturating_add(1)
        <= area.y.saturating_add(area.height)
    {
        1
    } else {
        0
    };

    Rect {
        x: modal.x.saturating_add(offset_x),
        y: modal.y.saturating_add(offset_y),
        width: modal.width,
        height: modal.height,
    }
}

fn fixed_width(text: &str, width: usize) -> String {
    let text = truncate_text(text, width);
    format!("{text:<width$}")
}

fn truncate_text(text: &str, width: usize) -> String {
    if text.len() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    format!("{}...", &text[..width - 3])
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

fn render_sleeping_screen(frame: &mut Frame<'_>, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let style = Style::default().fg(Color::Black).bg(Color::Black);
    let blank = " ".repeat(area.width as usize);
    let lines = (0..area.height)
        .map(|_| Line::from(Span::styled(blank.clone(), style)))
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines).style(style), area);
}

fn render_screen(
    frame: &mut Frame<'_>,
    area: Rect,
    screen: &vt100::Screen,
    selection: Option<PaneSelection>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let lines = (0..area.height)
        .map(|row| render_screen_row(screen, row, area.width, selection))
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(Color::Rgb(230, 237, 243)).bg(APP_BG)),
        area,
    );
}

fn render_screen_row<'a>(
    screen: &vt100::Screen,
    row: u16,
    width: u16,
    selection: Option<PaneSelection>,
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
                selection_style(Style::default(), selection, row, column),
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
            selection_style(cell_style(cell), selection, row, column),
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

fn selection_style(style: Style, selection: Option<PaneSelection>, row: u16, column: u16) -> Style {
    if selection.is_some_and(|selection| selection.contains(row, column)) {
        style
            .fg(Color::Black)
            .bg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    } else {
        style
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_activity_does_not_change_idle_pane_chrome() {
        let palette = GridPalette::default();

        assert_eq!(
            pane_chrome(false, false, false, false, false, false, &palette),
            pane_chrome(false, false, true, false, false, false, &palette)
        );
    }

    #[test]
    fn selected_and_exited_badges_remain_visible() {
        let palette = GridPalette::default();

        assert_eq!(
            pane_chrome(true, false, true, false, false, true, &palette).badge,
            " selected"
        );
        assert_eq!(
            pane_chrome(true, false, true, true, false, true, &palette).badge,
            " exited"
        );
    }

    #[test]
    fn sleeping_panes_show_sleep_badge() {
        let palette = GridPalette::default();

        assert_eq!(
            pane_chrome(false, false, true, false, true, true, &palette).badge,
            " asleep"
        );
    }

    #[test]
    fn pane_title_uses_custom_label_in_number_slot() {
        assert_eq!(
            pane_title("api", "", "gridbash/", Some("feat/rename-panes"), None, ""),
            " api | gridbash/ | feat/rename-panes "
        );
        assert_eq!(
            pane_title("1", "", "gridbash/", None, None, " selected"),
            " 1 | gridbash/ selected "
        );
        assert_eq!(
            pane_title("2", "", "gridbash/", None, Some("5h 80% left"), " selected"),
            " 2 | gridbash/ | 5h 80% left selected "
        );
    }

    #[test]
    fn pane_title_keeps_quiet_marker_with_custom_label() {
        assert_eq!(
            pane_title("api", QUIET_MARKER, "gridbash/", None, None, ""),
            " api * | gridbash/ "
        );
    }

    #[test]
    fn quiet_output_marks_idle_pane_without_active_chrome() {
        let palette = GridPalette::default();
        let quiet = pane_chrome(false, false, false, false, false, true, &palette);
        let active_quiet = pane_chrome(false, false, true, false, false, true, &palette);

        assert_eq!(quiet.quiet_marker, QUIET_MARKER);
        assert_eq!(quiet.border_style, active_quiet.border_style);
    }
}
