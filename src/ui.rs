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
    app::{
        App, ExitedPaneRecoveryView, FollowUpDialog, GridPalette, PaneGroupView, PaneSelection,
        PaneSettingsView, PreviousPaneView, PreviousPanesView, PromptView, RenamePaneView,
        RenameTabView, SettingsGroup, SettingsRow, SettingsTab, SettingsValueKind, TabLabel,
    },
    auth::{AgentKind, AuthProfile},
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
    pub previous_panes_button: Option<Rect>,
    pub previous_pane_rows: Vec<(usize, Rect)>,
    pub pane_settings_button: Option<Rect>,
    pub pane_settings_rename_button: Option<Rect>,
    pub pane_settings_reload_button: Option<Rect>,
}

const QUIET_MARKER: &str = " *";
const STATUS_BRAND: &str = " GridBash ";
const PREVIOUS_PANES_BUTTON: &str = " Panes ";
const PANE_SETTINGS_BUTTON: &str = " Pane ";

pub fn draw(frame: &mut Frame<'_>, app: &App) -> DrawState {
    let area = frame.area();
    let output_height = if app.command_output_expanded() {
        command_output_height(area.height, app.command_output_lines().len())
    } else {
        0
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(output_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let tab_area = chunks[0];
    let grid_area = chunks[1];
    let command_output_area = chunks[2];
    let command_area = chunks[3];
    let status_area = chunks[4];
    let rects = app.pane_rects(grid_area);
    let palette = app.palette();
    let rename_view = app.rename_pane_view();
    let tab_rename_view = app.rename_tab_view();
    let previous_panes_view = app.previous_panes_view();
    let follow_up_dialog = app.follow_up_dialog();
    let prompt_view = app.prompt_view();
    let pane_settings_open = app.pane_settings_open();
    let image_overlay = app.image_overlay_view();
    let exited_recovery = if app.settings_open()
        || previous_panes_view.is_some()
        || pane_settings_open
        || rename_view.is_some()
        || tab_rename_view.is_some()
        || follow_up_dialog.is_some()
        || prompt_view.is_some()
        || image_overlay.is_some()
    {
        None
    } else {
        app.exited_recovery_view()
    };
    let modal_open = app.settings_open()
        || previous_panes_view.is_some()
        || pane_settings_open
        || rename_view.is_some()
        || tab_rename_view.is_some()
        || follow_up_dialog.is_some()
        || prompt_view.is_some()
        || image_overlay.is_some()
        || exited_recovery.is_some();
    let mut pane_settings_rename_button = None;
    let mut pane_settings_reload_button = None;
    render_tabs(frame, tab_area, &app.tab_labels(), palette);

    for (index, pane) in app.panes().iter().enumerate() {
        let Some(rect) = rects.get(index).copied() else {
            continue;
        };

        let focused = app.focused_pane() == Some(index);
        let selected = app.selected().contains(&index);
        let sleeping = app.pane_sleeping(index);
        let group = app.pane_group(index);
        let quiet = app.activity_badges_enabled() && pane.output_quiet();
        let chrome = pane_chrome(
            selected,
            focused,
            pane.exited,
            sleeping,
            group.map(|group| group.color.rgb),
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
            app.pane_profile(index),
            app.pane_auth(index),
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
        if let Some(view) = app.pane_settings_view(index) {
            let buttons = render_pane_settings(frame, inner, &view, palette);
            pane_settings_rename_button = buttons.rename;
            pane_settings_reload_button = buttons.reload;
        } else if sleeping {
            render_sleeping_screen(frame, inner);
        } else {
            render_screen(frame, inner, pane.screen(), app.selection_for_pane(index));
        }
        if let Some(group) = group {
            render_group_badge(frame, rect, group);
        }

        if focused && !sleeping && !modal_open {
            set_terminal_cursor(frame, inner, pane.screen());
        }
    }

    if output_height > 0 {
        render_command_output(frame, command_output_area, app);
    }
    render_command_line(frame, command_area, app);

    let input_scope = app.input_scope_label();
    let previous_panes_button = previous_panes_button_rect(status_area);
    let pane_settings_button = pane_settings_button_rect(status_area);
    let status = Line::from(vec![
        Span::styled(
            STATUS_BRAND,
            Style::default()
                .fg(Color::Black)
                .bg(palette.accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            PREVIOUS_PANES_BUTTON,
            previous_panes_button_style(app.previous_panes_open(), palette),
        ),
        Span::raw(" "),
        Span::styled(
            PANE_SETTINGS_BUTTON,
            pane_settings_button_style(app.pane_settings_open(), palette),
        ),
        Span::raw(" "),
        Span::styled(
            if app.voice_listening() { "MIC" } else { "LIVE" },
            Style::default()
                .fg(if app.voice_listening() {
                    palette.accent()
                } else {
                    palette.focus()
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(
            input_scope,
            Style::default().fg(if app.command_focused() {
                palette.accent()
            } else if app.selected().len() > 1 {
                palette.selected()
            } else {
                Color::Gray
            }),
        ),
        Span::raw(" | "),
        Span::raw(format!("{} selected", app.selected().len())),
        Span::raw(" | "),
        Span::raw(app.status().to_string()),
        Span::raw(
            " | Alt+n new | Alt+t tab | Alt+Shift+t restart | Alt+c command | Alt+v voice | Alt+e output | Alt+p panes | Alt+P pane | Alt+x swap | Alt+z sleep | Alt+q quit",
        ),
    ]);
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(APP_BG)),
        status_area,
    );

    if app.settings_open() {
        render_settings(frame, area, app, palette);
    } else if let Some(dialog) = follow_up_dialog.as_ref() {
        render_follow_up_dialog(frame, area, dialog);
    }
    let previous_pane_rows = if let Some(view) = previous_panes_view.as_ref() {
        render_previous_panes(frame, area, view, palette)
    } else {
        Vec::new()
    };
    if let Some(rename) = rename_view.as_ref() {
        render_rename_pane(frame, area, rename);
    }
    if let Some(rename) = tab_rename_view.as_ref() {
        render_rename_tab(frame, area, rename);
    }
    if let Some(prompt) = prompt_view.as_ref() {
        render_manager_prompt(frame, area, prompt);
    }
    if let Some(image) = image_overlay {
        render_image_overlay(frame, area, image);
    }
    if let Some(recovery) = exited_recovery.as_ref() {
        render_exited_recovery(frame, area, recovery, palette);
    }

    DrawState {
        grid_area,
        pane_rects: rects,
        previous_panes_button,
        previous_pane_rows,
        pane_settings_button,
        pane_settings_rename_button,
        pane_settings_reload_button,
    }
}

#[allow(clippy::too_many_arguments)]
fn pane_title(
    label: &str,
    quiet_marker: &str,
    folder: &str,
    worktree: Option<&str>,
    profile: Option<&str>,
    auth: Option<&str>,
    usage: Option<&str>,
    badge: &str,
) -> String {
    let mut parts = vec![format!("{label}{quiet_marker}"), folder.to_string()];
    if let Some(worktree) = worktree.filter(|value| !value.is_empty()) {
        parts.push(worktree.to_string());
    }
    if let Some(profile) = profile.filter(|value| !value.is_empty()) {
        parts.push(profile.to_string());
    }
    if let Some(auth) = auth.filter(|value| !value.is_empty()) {
        parts.push(auth.to_string());
    }
    if let Some(usage) = usage.filter(|value| !value.is_empty()) {
        parts.push(usage.to_string());
    }

    format!(" {}{} ", parts.join(" | "), badge)
}

fn render_tabs(frame: &mut Frame<'_>, area: Rect, tabs: &[TabLabel], palette: &GridPalette) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut spans = vec![
        Span::styled(
            STATUS_BRAND,
            Style::default()
                .fg(Color::Black)
                .bg(palette.accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

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
            Style::default().fg(palette.exited())
        } else if tab.activity {
            Style::default().fg(palette.quiet())
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label, style));
    }

    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        "Alt+n new",
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        "Alt+Shift+r rename tab",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(APP_BG)),
        area,
    );
}

fn render_group_badge(frame: &mut Frame<'_>, rect: Rect, group: PaneGroupView) {
    let label = format!(" G{} ", group.label);
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

fn render_manager_prompt(frame: &mut Frame<'_>, area: Rect, prompt: &PromptView) {
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
        Span::raw(prompt.input.as_str())
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" G{} ", prompt.label),
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
        Paragraph::new(line)
            .block(block)
            .style(Style::default().fg(Color::Rgb(230, 237, 243)).bg(APP_BG)),
        prompt_area,
    );
}

fn command_output_height(total_height: u16, line_count: usize) -> u16 {
    let available = total_height.saturating_sub(3);
    if available < 3 {
        return 0;
    }

    let max_height = (total_height / 3).clamp(3, 12).min(available);
    (line_count as u16).saturating_add(2).clamp(3, max_height)
}

fn render_command_output(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let border_style = if app.command_focused() {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title = if app.command_running() {
        " Command output | running "
    } else {
        " Command output "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = app.command_output_lines();
    let start = lines.len().saturating_sub(inner.height as usize);
    let visible = lines[start..]
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(visible).style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        ),
        inner,
    );
}

fn render_command_line(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let width = area.width as usize;
    let cwd = app.command_cwd().display().to_string();
    let cwd_budget = command_cwd_budget(width, app.command_input());
    let cwd = truncate_start(&cwd, cwd_budget);
    let prompt = format!(" {cwd} > ");
    let prompt_width = prompt.chars().count();
    let input_width = width.saturating_sub(prompt_width);
    let (input, cursor_offset) =
        visible_input(app.command_input(), app.command_cursor_chars(), input_width);
    let focused = app.command_focused();

    let prompt_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let input_style = if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Rgb(180, 190, 202))
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prompt, prompt_style),
            Span::styled(input, input_style),
        ]))
        .style(Style::default().bg(Color::Rgb(14, 20, 28))),
        area,
    );

    if focused {
        let x = area
            .x
            .saturating_add((prompt_width + cursor_offset).min(width.saturating_sub(1)) as u16);
        frame.set_cursor_position((x, area.y));
    }
}

fn command_cwd_budget(width: usize, input: &str) -> usize {
    if width <= 4 {
        return 0;
    }
    if input.is_empty() {
        return width.saturating_sub(4);
    }

    width.saturating_sub(14).min((width * 2) / 3)
}

fn truncate_start(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return chars[chars.len().saturating_sub(max_chars)..]
            .iter()
            .collect();
    }

    let tail = chars[chars.len() - (max_chars - 3)..]
        .iter()
        .collect::<String>();
    format!("...{tail}")
}

fn visible_input(input: &str, cursor_chars: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }

    let chars = input.chars().collect::<Vec<_>>();
    let cursor = cursor_chars.min(chars.len());
    if chars.len() <= width {
        return (input.to_string(), cursor);
    }

    let start = cursor.saturating_sub(width.saturating_sub(1));
    let end = (start + width).min(chars.len());
    (chars[start..end].iter().collect(), cursor - start)
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
    exited: bool,
    sleeping: bool,
    group_color: Option<(u8, u8, u8)>,
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
    } else if let Some(group_color) = group_color {
        Style::default()
            .fg(rgb_color(group_color))
            .add_modifier(Modifier::BOLD)
    } else if quiet {
        Style::default().fg(palette.quiet())
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

fn previous_panes_button_rect(status_area: Rect) -> Option<Rect> {
    let offset = STATUS_BRAND.len() as u16 + 1;
    let width = PREVIOUS_PANES_BUTTON.len() as u16;
    if status_area.height == 0 || status_area.width < offset.saturating_add(width) {
        return None;
    }

    Some(Rect {
        x: status_area.x.saturating_add(offset),
        y: status_area.y,
        width,
        height: 1,
    })
}

fn previous_panes_button_style(open: bool, palette: &GridPalette) -> Style {
    if open {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(palette.focus())
            .add_modifier(Modifier::BOLD)
    }
}

fn pane_settings_button_rect(status_area: Rect) -> Option<Rect> {
    let offset = STATUS_BRAND.len() as u16 + 1 + PREVIOUS_PANES_BUTTON.len() as u16 + 1;
    let width = PANE_SETTINGS_BUTTON.len() as u16;
    if status_area.height == 0 || status_area.width < offset.saturating_add(width) {
        return None;
    }

    Some(Rect {
        x: status_area.x.saturating_add(offset),
        y: status_area.y,
        width,
        height: 1,
    })
}

fn pane_settings_button_style(open: bool, palette: &GridPalette) -> Style {
    if open {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(palette.focus())
            .add_modifier(Modifier::BOLD)
    }
}

fn render_pane_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &PaneSettingsView,
    palette: &GridPalette,
) -> PaneSettingsButtons {
    if area.width == 0 || area.height == 0 {
        return PaneSettingsButtons::default();
    }

    frame.render_widget(Clear, area);
    let lines = pane_settings_lines(view, area.width, palette);
    frame.render_widget(Paragraph::new(lines).style(settings_panel_style()), area);

    PaneSettingsButtons {
        rename: pane_settings_rename_rect(area),
        reload: pane_settings_reload_rect(area),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PaneSettingsButtons {
    rename: Option<Rect>,
    reload: Option<Rect>,
}

fn pane_settings_state(view: &PaneSettingsView) -> (&'static str, Color) {
    if view.exited {
        ("exited", Color::Red)
    } else if view.sleeping {
        ("asleep", Color::DarkGray)
    } else if view.focused {
        ("focus", Color::Yellow)
    } else if view.selected {
        ("selected", Color::Cyan)
    } else {
        ("live", SETTINGS_TEXT)
    }
}

fn pane_settings_lines(
    view: &PaneSettingsView,
    width: u16,
    palette: &GridPalette,
) -> Vec<Line<'static>> {
    let (state, state_color) = pane_settings_state(view);
    let location = view
        .worktree
        .as_ref()
        .map(|worktree| format!("{} | {worktree}", view.folder))
        .unwrap_or_else(|| view.folder.clone());
    let mut lines = Vec::new();

    if width < 36 {
        lines.push(Line::from(Span::styled(
            fixed_width(" Pane Settings", width as usize),
            Style::default()
                .fg(palette.focus())
                .bg(SETTINGS_BG)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(pane_settings_rename_line(width, palette));
        lines.push(pane_settings_reload_line(width, palette));
        lines.push(pane_settings_command_bar(width));
        return lines;
    }

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Pane Settings",
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("pane {} {}", view.index + 1, view.label),
            Style::default().fg(SETTINGS_TEXT),
        ),
        Span::raw("  "),
        Span::styled(state, Style::default().fg(state_color)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            truncate_text(&location, width.saturating_sub(2) as usize),
            Style::default().fg(SETTINGS_MUTED),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(settings_section(
        "HISTORY",
        "visible conversation snapshot",
        width,
    ));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            truncate_text(&view.history_summary, width.saturating_sub(2) as usize),
            Style::default().fg(SETTINGS_TEXT),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(pane_settings_rename_line(width, palette));
    lines.push(pane_settings_reload_line(width, palette));
    lines.push(Line::from(""));
    lines.push(pane_settings_command_bar(width));

    lines
}

fn pane_settings_rename_line(width: u16, palette: &GridPalette) -> Line<'static> {
    pane_settings_action_line("[ Rename pane ]", width, palette.selected())
}

fn pane_settings_reload_line(width: u16, palette: &GridPalette) -> Line<'static> {
    pane_settings_action_line("[ Reload past history ]", width, palette.focus())
}

fn pane_settings_action_line(label: &str, width: u16, background: Color) -> Line<'static> {
    let text = if width as usize <= label.len() + 4 {
        fixed_width(label, width as usize)
    } else {
        let left = ((width as usize).saturating_sub(label.len())) / 2;
        let right = (width as usize).saturating_sub(left + label.len());
        format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
    };

    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Black)
            .bg(background)
            .add_modifier(Modifier::BOLD),
    ))
}

fn pane_settings_command_bar(width: u16) -> Line<'static> {
    if width < 50 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("N"),
            Span::styled(" rename  ", Style::default().fg(Color::Gray)),
            command_key("R"),
            Span::styled(" reload  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }

    Line::from(vec![
        Span::raw("  "),
        command_key("N"),
        Span::styled(" rename  ", Style::default().fg(Color::Gray)),
        command_key("R/Enter"),
        Span::styled(" reload  ", Style::default().fg(Color::Gray)),
        command_key("Alt+o"),
        Span::styled(" global settings  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ])
}

fn pane_settings_rename_rect(area: Rect) -> Option<Rect> {
    pane_settings_action_rect(area, if area.width < 36 { 1 } else { 6 })
}

fn pane_settings_reload_rect(area: Rect) -> Option<Rect> {
    pane_settings_action_rect(area, if area.width < 36 { 2 } else { 7 })
}

fn pane_settings_action_rect(area: Rect, row: u16) -> Option<Rect> {
    if area.width == 0 || area.height <= row {
        return None;
    }

    Some(Rect {
        x: area.x,
        y: area.y.saturating_add(row),
        width: area.width,
        height: 1,
    })
}

fn render_previous_panes(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &PreviousPanesView,
    palette: &GridPalette,
) -> Vec<(usize, Rect)> {
    let modal = previous_panes_modal_rect(area, view.panes.len());
    let shadow = settings_shadow_rect(area, modal);
    let mut row_hits = Vec::new();

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
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Previous Panes ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    if inner.width == 0 || inner.height == 0 {
        return row_hits;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let header = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{} panes", view.panes.len()),
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  current session", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(
        Paragraph::new(vec![header, Line::from("")]).style(settings_panel_style()),
        chunks[0],
    );

    let list_area = chunks[1];
    let visible =
        visible_previous_pane_range(view.panes.len(), view.cursor, list_area.height as usize);
    let mut rows = Vec::new();

    for (row_offset, index) in visible.enumerate() {
        let Some(pane) = view.panes.get(index) else {
            continue;
        };
        let row_area = Rect {
            x: list_area.x,
            y: list_area.y.saturating_add(row_offset as u16),
            width: list_area.width,
            height: 1,
        };
        row_hits.push((index, row_area));
        rows.push(previous_pane_line(
            pane,
            view.cursor == index,
            list_area.width,
        ));
    }

    frame.render_widget(
        Paragraph::new(rows).style(settings_panel_style()),
        list_area,
    );
    frame.render_widget(
        Paragraph::new(previous_panes_command_bar(chunks[2].width)).style(settings_panel_style()),
        chunks[2],
    );

    row_hits
}

fn previous_pane_line(pane: &PreviousPaneView, active: bool, width: u16) -> Line<'static> {
    let (state, state_color) = previous_pane_state(pane);
    let label_width = if width < 62 { 10 } else { 16 };
    let location_width = if width < 62 { 14 } else { 24 };
    let marker = if active { ">" } else { " " };
    let location = pane
        .worktree
        .as_ref()
        .map(|worktree| format!("{} | {worktree}", pane.folder))
        .unwrap_or_else(|| pane.folder.clone());
    let text = format!(
        "{marker} {:>2} {:<label_width$} {:<8} {:<location_width$} {}",
        pane.index + 1,
        truncate_text(&pane.label, label_width),
        state,
        truncate_text(&location, location_width),
        pane.summary,
    );
    let bg = active.then_some(SETTINGS_ROW_ACTIVE);
    let fg = if active { SETTINGS_TEXT } else { state_color };

    Line::from(Span::styled(
        fixed_width(&text, width as usize),
        row_style(fg, bg, active || pane.focused),
    ))
}

fn previous_pane_state(pane: &PreviousPaneView) -> (&'static str, Color) {
    if pane.exited {
        ("exited", Color::Red)
    } else if pane.sleeping {
        ("asleep", Color::DarkGray)
    } else if pane.focused {
        ("focus", Color::Yellow)
    } else if pane.selected {
        ("selected", Color::Cyan)
    } else {
        ("live", SETTINGS_TEXT)
    }
}

fn previous_panes_command_bar(width: u16) -> Line<'static> {
    if width < 44 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Enter"),
            Span::styled(" focus  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }

    Line::from(vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" move  ", Style::default().fg(Color::Gray)),
        command_key("Enter"),
        Span::styled(" focus  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ])
}

fn previous_panes_modal_rect(area: Rect, pane_count: usize) -> Rect {
    let width = area.width.saturating_sub(4).min(96).max(area.width.min(1));
    let desired_height = (pane_count as u16).saturating_add(6).clamp(8, 24);
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

fn visible_previous_pane_range(
    pane_count: usize,
    cursor: usize,
    capacity: usize,
) -> std::ops::Range<usize> {
    if pane_count == 0 || capacity == 0 {
        return 0..0;
    }

    let capacity = capacity.min(pane_count);
    let cursor = cursor.min(pane_count - 1);
    let mut start = cursor.saturating_sub(capacity / 2);
    if start + capacity > pane_count {
        start = pane_count - capacity;
    }

    start..start + capacity
}

fn render_settings(frame: &mut Frame<'_>, area: Rect, app: &App, palette: &GridPalette) {
    let modal = settings_modal_rect(area, settings_content_row_count(app));
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
        Paragraph::new(settings_lines(app, inner.width)).style(settings_panel_style()),
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

fn render_exited_recovery(
    frame: &mut Frame<'_>,
    area: Rect,
    recovery: &ExitedPaneRecoveryView,
    palette: &GridPalette,
) {
    let modal = exited_recovery_modal_rect(area);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.exited())
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(SETTINGS_TEXT).bg(APP_BG))
        .title(format!(" Pane {} Exited ", recovery.pane_index + 1));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let target = if recovery.target_count == 1 {
        format!(
            "Pane {} ({}) is no longer running.",
            recovery.pane_index + 1,
            recovery.pane_label
        )
    } else {
        format!("{} panes are no longer running.", recovery.target_count)
    };
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                truncate_text(&target, inner.width.saturating_sub(2) as usize),
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            command_key("Enter"),
            Span::styled(" restart  ", Style::default().fg(Color::Gray)),
            command_key("r/t"),
            Span::styled(" restart  ", Style::default().fg(Color::Gray)),
            command_key("z"),
            Span::styled(" sleep", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            command_key("Alt+arrows"),
            Span::styled(" focus another pane", Style::default().fg(Color::Gray)),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(SETTINGS_TEXT).bg(APP_BG)),
        inner,
    );
}

fn render_follow_up_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &FollowUpDialog) {
    let modal = follow_up_modal_rect(area);
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
                .fg(SETTINGS_BORDER)
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Todo Follow-up ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    frame.render_widget(
        Paragraph::new(follow_up_lines(dialog, inner.width)).style(settings_panel_style()),
        inner,
    );
}

fn settings_content_row_count(app: &App) -> usize {
    match app.settings_tab() {
        SettingsTab::General => app.settings_rows().len(),
        SettingsTab::Auth => {
            app.auth_profiles().len().max(1) + usize::from(app.auth_create().is_some()) * 3
        }
    }
}

fn settings_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    match app.settings_tab() {
        SettingsTab::General => general_settings_lines(&app.settings_rows(), width),
        SettingsTab::Auth => auth_settings_lines(app, width),
    }
}

fn settings_tabs(active: SettingsTab) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        settings_tab("General", active == SettingsTab::General),
        Span::raw("  "),
        settings_tab("Auth", active == SettingsTab::Auth),
        Span::raw("  "),
        Span::styled("Tab switches", Style::default().fg(SETTINGS_MUTED)),
    ])
}

fn settings_tab(label: &'static str, active: bool) -> Span<'static> {
    let style = if active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::LightCyan).bg(SETTINGS_SURFACE)
    };
    Span::styled(format!(" {label} "), style)
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

fn follow_up_lines(dialog: &FollowUpDialog, width: u16) -> Vec<Line<'static>> {
    let quiet = format!(
        "Pane {} has been quiet for {}s.",
        dialog.pane_number, dialog.quiet_seconds
    );
    let count = format!("Todo {}/{}", dialog.todo_position, dialog.todo_count);
    let prompt_width = width.saturating_sub(4) as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                quiet,
                Style::default()
                    .fg(SETTINGS_BORDER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Send this queued prompt?",
                Style::default().fg(SETTINGS_TEXT),
            ),
            Span::raw("  "),
            Span::styled(
                count,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];

    for line in wrap_dialog_text(&dialog.prompt, prompt_width, 3) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(line, Style::default().fg(Color::LightCyan)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(follow_up_command_bar(width));
    lines
}

fn follow_up_command_bar(width: u16) -> Line<'static> {
    if width < 54 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Enter"),
            Span::styled(" send  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" no", Style::default().fg(Color::Gray)),
        ]);
    }

    Line::from(vec![
        Span::raw("  "),
        command_key("Enter/Y"),
        Span::styled(" send  ", Style::default().fg(Color::Gray)),
        command_key("Tab"),
        Span::styled(" next  ", Style::default().fg(Color::Gray)),
        command_key("Del"),
        Span::styled(" remove  ", Style::default().fg(Color::Gray)),
        command_key("Esc/N"),
        Span::styled(" no", Style::default().fg(Color::Gray)),
    ])
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
                "▀",
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

fn general_settings_lines(rows: &[SettingsRow], width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![
        settings_tabs(SettingsTab::General),
        Line::from(""),
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
    ];

    push_settings_group(
        &mut lines,
        rows,
        SettingsGroup::Display,
        "DISPLAY",
        "title bar and state signals",
        width,
    );
    push_settings_group(
        &mut lines,
        rows,
        SettingsGroup::Workflow,
        "WORKFLOW",
        "guard rails for high-speed sessions",
        width,
    );
    push_settings_group(
        &mut lines,
        rows,
        SettingsGroup::Todo,
        "TODO",
        "queued prompts for quiet panes",
        width,
    );
    push_settings_group(
        &mut lines,
        rows,
        SettingsGroup::Performance,
        "PERFORMANCE",
        "spacing and terminal budget",
        width,
    );
    push_settings_group(
        &mut lines,
        rows,
        SettingsGroup::Theme,
        "THEME",
        "runtime palette for grid chrome",
        width,
    );

    lines.push(Line::from(""));
    lines.push(settings_command_bar(width));
    lines
}

fn auth_settings_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![
        settings_tabs(SettingsTab::Auth),
        Line::from(""),
        settings_section(
            "AUTH PROFILES",
            if app.auth_refreshing() {
                "refreshing local account and usage status"
            } else {
                "Claude and Codex defaults"
            },
            width,
        ),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("home", Style::default().fg(SETTINGS_MUTED)),
            Span::raw("  "),
            Span::styled(
                truncate_text(&app.auth_home_label(), width.saturating_sub(8) as usize),
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(""),
    ];

    if app.auth_profiles().is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("No auth profiles found.", Style::default().fg(Color::Gray)),
            Span::raw("  "),
            Span::styled("n", Style::default().fg(Color::Yellow)),
            Span::styled(" creates one", Style::default().fg(SETTINGS_MUTED)),
        ]));
    } else {
        for (index, profile) in app.auth_profiles().iter().enumerate() {
            lines.push(auth_profile_row(
                profile,
                index == app.auth_cursor(),
                app.auth_default(profile.kind) == Some(profile.name.as_str()),
                width,
            ));
        }
    }

    if let Some(create) = app.auth_create() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Create", Style::default().fg(SETTINGS_MUTED)),
            Span::raw("  "),
            Span::styled(
                create.kind.display_name(),
                Style::default().fg(kind_color(create.kind)),
            ),
            Span::raw("  "),
            Span::styled(create.name.clone(), Style::default().fg(Color::Yellow)),
            Span::styled("_", Style::default().fg(Color::Yellow)),
        ]));
        lines.push(auth_create_command_bar());
    }

    lines.push(Line::from(""));
    lines.push(auth_command_bar(width));
    lines
}

fn auth_profile_row(
    profile: &AuthProfile,
    selected: bool,
    is_default: bool,
    width: u16,
) -> Line<'static> {
    let row_bg = selected.then_some(SETTINGS_ROW_ACTIVE);
    let marker = if selected { "> " } else { "  " };
    let default = if is_default { "default" } else { "" };
    let account = profile.account_label.as_deref().unwrap_or("no account");
    let detail = profile.account_detail.as_deref().unwrap_or("");
    let usage = profile
        .usage
        .as_ref()
        .map(|usage| usage.display_label())
        .unwrap_or_else(|| "usage n/a".into());
    let summary = format!(
        "{:<14} {:<7} {:<8} {:<12} {:<24} {:<8} {}",
        profile.name,
        profile.kind.as_str(),
        default,
        profile.status_label(),
        account,
        detail,
        usage
    );
    let available = width.saturating_sub(2) as usize;

    Line::from(vec![
        Span::styled(marker.to_string(), row_style(Color::Yellow, row_bg, false)),
        Span::styled(
            truncate_text(&summary, available),
            row_style(SETTINGS_TEXT, row_bg, selected),
        ),
    ])
}

fn auth_command_bar(width: u16) -> Line<'static> {
    if width < 58 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Up/Down"),
            Span::styled(" move  ", Style::default().fg(Color::Gray)),
            command_key("d"),
            Span::styled(" default  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }

    Line::from(vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" move  ", Style::default().fg(Color::Gray)),
        command_key("d"),
        Span::styled(" default  ", Style::default().fg(Color::Gray)),
        command_key("n"),
        Span::styled(" new  ", Style::default().fg(Color::Gray)),
        command_key("l"),
        Span::styled(" login  ", Style::default().fg(Color::Gray)),
        command_key("r"),
        Span::styled(" refresh  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ])
}

fn auth_create_command_bar() -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        command_key("Tab"),
        Span::styled(" kind  ", Style::default().fg(Color::Gray)),
        command_key("Enter"),
        Span::styled(" create  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" cancel", Style::default().fg(Color::Gray)),
    ])
}

fn kind_color(kind: AgentKind) -> Color {
    match kind {
        AgentKind::Claude => Color::Magenta,
        AgentKind::Codex => Color::Cyan,
    }
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
        "Refine pane chrome, todo prompts, and highlight color."
    } else {
        "Refine pane chrome, idle follow-up todos, performance, and highlight color."
    };
    truncate_text(text, width.saturating_sub(2) as usize)
}

fn push_settings_group(
    lines: &mut Vec<Line<'static>>,
    rows: &[SettingsRow],
    group: SettingsGroup,
    title: &'static str,
    helper: &'static str,
    width: u16,
) {
    let group_rows = rows
        .iter()
        .filter(|row| row.group == group)
        .collect::<Vec<_>>();
    if group_rows.is_empty() {
        return;
    }

    if lines.last().is_none_or(|line| line.width() != 0) {
        lines.push(Line::from(""));
    }
    lines.push(settings_section(title, helper, width));
    for row in group_rows {
        lines.push(settings_row(row, width));
    }
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
    if row.group == SettingsGroup::Todo
        && matches!(
            row.value_kind,
            SettingsValueKind::Text | SettingsValueKind::Action
        )
    {
        return settings_todo_row(row, width);
    }

    let width = width as usize;
    let narrow = width < 66;
    let label_width = if narrow { 20 } else { 24 };
    let value_width = if narrow { 10 } else { 13 };
    let reserved = 2 + label_width + 2 + value_width + 2;
    let hint_width = width.saturating_sub(reserved);
    let marker = if row.selected { "> " } else { "  " };
    let label = fixed_width(&row.label, label_width);
    let value = fixed_width(&settings_value_label(row), value_width);
    let hint = if hint_width >= 10 {
        truncate_text(&row.hint, hint_width)
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

fn settings_todo_row(row: &SettingsRow, width: u16) -> Line<'static> {
    let width = width as usize;
    let marker = if row.selected { "> " } else { "  " };
    let label_width = if width < 66 { 10 } else { 12 };
    let hint_width = if row.selected && width >= 72 { 24 } else { 0 };
    let hint_gap = if hint_width > 0 { 2 } else { 0 };
    let reserved = marker.len() + label_width + 2 + hint_width + hint_gap;
    let value_width = width.saturating_sub(reserved);
    let row_bg = row.selected.then_some(SETTINGS_ROW_ACTIVE);
    let label = fixed_width(&row.label, label_width);
    let value = fixed_width(&settings_value_label(row), value_width);
    let mut used = marker.len() + label.len() + 2 + value.len();
    let mut spans = vec![
        Span::styled(marker.to_string(), row_style(Color::Yellow, row_bg, false)),
        Span::styled(label, row_style(SETTINGS_TEXT, row_bg, row.selected)),
        Span::styled("  ", row_style(SETTINGS_TEXT, row_bg, false)),
        Span::styled(value, settings_value_style(row)),
    ];

    if hint_width > 0 {
        let hint = fixed_width(&row.hint, hint_width);
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

    if width < 62 {
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
        command_key("Del"),
        Span::styled(" remove  ", Style::default().fg(Color::Gray)),
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
    match row.value_kind {
        SettingsValueKind::Switch => format!("[ {} ]", row.value),
        SettingsValueKind::Choice => format!("< {} >", row.value),
        SettingsValueKind::Stepper => format!("- {} +", row.value),
        SettingsValueKind::Action => format!("[ {} ]", row.value),
        SettingsValueKind::Text if row.value.is_empty() => "(empty)".into(),
        SettingsValueKind::Text => row.value.clone(),
    }
}

fn settings_value_style(row: &SettingsRow) -> Style {
    if let Some(color) = row.value_color {
        return Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD);
    }

    let mut style = match row.value_kind {
        SettingsValueKind::Switch if row.value == "on" => Style::default()
            .fg(Color::Black)
            .bg(SETTINGS_BORDER)
            .add_modifier(Modifier::BOLD),
        SettingsValueKind::Switch => Style::default().fg(SETTINGS_MUTED).bg(SETTINGS_SURFACE),
        SettingsValueKind::Choice if row.value == "cyan" => Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        SettingsValueKind::Choice if row.value == "yellow" => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        SettingsValueKind::Choice if row.value == "green" => Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        SettingsValueKind::Choice if row.value == "magenta" => Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        SettingsValueKind::Text if row.editing => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        SettingsValueKind::Action if row.selected => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        _ if row.selected => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::LightCyan)
            .bg(SETTINGS_SURFACE)
            .add_modifier(Modifier::BOLD),
    };

    if row.selected && row.value_kind == SettingsValueKind::Switch && row.value == "off" {
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

fn exited_recovery_modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(62).max(area.width.min(1));
    let height = area.height.saturating_sub(2).min(9).max(area.height.min(1));

    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn follow_up_modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(74).max(area.width.min(1));
    let height = area
        .height
        .saturating_sub(2)
        .min(12)
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
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    format!("{}...", text.chars().take(width - 3).collect::<String>())
}

fn wrap_dialog_text(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let next_len = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };

        if next_len <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }

        if !current.is_empty() {
            lines.push(current);
            current = String::new();
        }

        if word.len() > width {
            lines.push(truncate_text(word, width));
        } else {
            current.push_str(word);
        }

        if lines.len() == max_lines {
            break;
        }
    }

    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }

    if lines.is_empty() {
        lines.push("(empty prompt)".into());
    }
    if lines.len() == max_lines
        && text.len() > lines.join(" ").len()
        && let Some(last) = lines.last_mut()
    {
        *last = truncate_text(last, width.saturating_sub(3));
        last.push_str("...");
    }

    lines
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

fn rgb_color((red, green, blue): (u8, u8, u8)) -> Color {
    Color::Rgb(red, green, blue)
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
    fn idle_pane_has_no_state_badges() {
        let palette = GridPalette::default();
        let chrome = pane_chrome(false, false, false, false, None, false, &palette);

        assert_eq!(chrome.badge, "");
        assert_eq!(chrome.quiet_marker, "");
    }

    #[test]
    fn selected_and_exited_badges_remain_visible() {
        let palette = GridPalette::default();

        assert_eq!(
            pane_chrome(true, false, false, false, None, true, &palette).badge,
            " selected"
        );
        assert_eq!(
            pane_chrome(true, false, true, false, None, true, &palette).badge,
            " exited"
        );
    }

    #[test]
    fn sleeping_panes_show_sleep_badge() {
        let palette = GridPalette::default();

        assert_eq!(
            pane_chrome(false, false, false, true, None, true, &palette).badge,
            " asleep"
        );
    }

    #[test]
    fn pane_title_uses_custom_label_in_number_slot() {
        assert_eq!(
            pane_title(
                "api",
                "",
                "gridbash/",
                Some("feat/rename-panes"),
                None,
                None,
                None,
                ""
            ),
            " api | gridbash/ | feat/rename-panes "
        );
        assert_eq!(
            pane_title("1", "", "gridbash/", None, None, None, None, " selected"),
            " 1 | gridbash/ selected "
        );
        assert_eq!(
            pane_title(
                "2",
                "",
                "gridbash/",
                None,
                None,
                None,
                Some("5h 80% left"),
                " selected"
            ),
            " 2 | gridbash/ | 5h 80% left selected "
        );
        assert_eq!(
            pane_title(
                "api",
                "",
                "gridbash/",
                Some("main"),
                Some("codex"),
                Some("codex-2"),
                Some("5h 80% left"),
                " selected"
            ),
            " api | gridbash/ | main | codex | codex-2 | 5h 80% left selected "
        );
    }

    #[test]
    fn pane_title_keeps_quiet_marker_with_custom_label() {
        assert_eq!(
            pane_title("api", QUIET_MARKER, "gridbash/", None, None, None, None, ""),
            " api * | gridbash/ "
        );
    }

    #[test]
    fn pane_settings_action_buttons_use_expected_rows() {
        assert_eq!(
            pane_settings_rename_rect(Rect::new(5, 10, 40, 12)),
            Some(Rect::new(5, 16, 40, 1))
        );
        assert_eq!(
            pane_settings_reload_rect(Rect::new(5, 10, 40, 12)),
            Some(Rect::new(5, 17, 40, 1))
        );
        assert_eq!(
            pane_settings_rename_rect(Rect::new(5, 10, 20, 4)),
            Some(Rect::new(5, 11, 20, 1))
        );
        assert_eq!(
            pane_settings_reload_rect(Rect::new(5, 10, 20, 4)),
            Some(Rect::new(5, 12, 20, 1))
        );
        assert_eq!(pane_settings_rename_rect(Rect::new(5, 10, 40, 6)), None);
        assert_eq!(pane_settings_reload_rect(Rect::new(5, 10, 40, 6)), None);
    }

    #[test]
    fn truncates_non_ascii_text_without_slicing_inside_a_character() {
        assert_eq!(truncate_text("alpha beta", 8), "alpha...");
        assert_eq!(
            truncate_text("codex says 東京 ready", 15),
            "codex says 東..."
        );
    }

    #[test]
    fn quiet_output_marks_idle_pane_without_active_chrome() {
        let palette = GridPalette::default();
        let quiet = pane_chrome(false, false, false, false, None, true, &palette);

        assert_eq!(quiet.quiet_marker, QUIET_MARKER);
        assert_eq!(quiet.border_style, Style::default().fg(palette.quiet()));
    }

    #[test]
    fn grouped_quiet_pane_keeps_group_border_and_marker() {
        let palette = GridPalette::default();
        let group_color = (82, 166, 255);
        let chrome = pane_chrome(
            false,
            false,
            false,
            false,
            Some(group_color),
            true,
            &palette,
        );

        assert_eq!(chrome.quiet_marker, QUIET_MARKER);
        assert_eq!(
            chrome.border_style,
            Style::default()
                .fg(rgb_color(group_color))
                .add_modifier(Modifier::BOLD)
        );
    }
}
