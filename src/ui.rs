use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::merge::MergeStrategy,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use vt100::Cell;

use crate::{
    app::{
        AccountMeterView, AccountSheetView, AdoptTerminalView, App, AssistantMessageRole,
        BackgroundJobState, BackgroundJobView, BackgroundJobsView, CloseGridConfirmationView,
        CommandPaletteView, ExitedPaneRecoveryView, FollowUpDialog, GridPalette, PaneSelection,
        PaneSettingsTarget, PaneSettingsView, ParkSheetView, ParkToastView, PortInspectorView,
        PreviousPaneView, PreviousPanesView, QuitConfirmationView, RenamePaneView, RenameTabView,
        SettingsGroup, SettingsRow, SettingsTab, SettingsValueKind, ShelfState, ShelfView,
        TabLabel, WorkspaceAssistantView, shelf_areas,
    },
    auth::{AgentKind, AuthProfile},
    copy_mode::{CopyCellKind, CopyModeView, TextPoint},
    image_preview::ImagePreview,
    keeper,
};

// ---------------------------------------------------------------------------
// Design tokens
//
// One ramp, used everywhere. The chrome used to reach for `Color::DarkGray` and
// `Color::Yellow` alongside hand-written RGB triples, which put terminal-theme
// colours next to fixed ones — the same "grey" border landed anywhere from black
// to near-white depending on the user's scheme, and never matched the panel it
// framed. Everything structural is a fixed value from this ramp so the shell
// looks the same in every terminal; only the five palette roles the user
// actually chose stay configurable.
// ---------------------------------------------------------------------------

/// Deepest surface: the gutter behind the grid, and the dividers between panes.
pub(crate) const INK: Color = Color::Rgb(7, 10, 14);
/// The app background, and the default background of terminal cells.
const APP_BG: Color = Color::Rgb(11, 15, 20);
/// Raised chrome: the tab strip and the status bar.
pub(crate) const SURFACE: Color = Color::Rgb(16, 22, 29);
/// Chrome under the cursor or otherwise active.
pub(crate) const SURFACE_HI: Color = Color::Rgb(26, 35, 45);

/// Idle pane border. Present enough to read as a frame, quiet enough that the
/// focused pane is the only thing on screen drawing the eye.
pub(crate) const LINE: Color = Color::Rgb(42, 53, 66);
/// Divider between two panes that are both idle.
const LINE_SOFT: Color = Color::Rgb(30, 39, 49);

pub(crate) const TEXT: Color = Color::Rgb(230, 237, 243);
pub(crate) const TEXT_DIM: Color = Color::Rgb(150, 165, 180);
pub(crate) const TEXT_FAINT: Color = Color::Rgb(98, 113, 129);

/// Terminal-cell defaults, applied to any cell whose own colour is "default".
const PANE_FG: Color = TEXT;
const PANE_BG: Color = APP_BG;

/// Mouse text selection inside a pane.
const SELECTION_FG: Color = Color::Rgb(8, 12, 16);
const SELECTION_BG: Color = Color::Rgb(126, 231, 235);

/// A pane whose agent is waiting on the user. The one colour allowed to shout.
const WAITING: Color = Color::Rgb(255, 196, 61);

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
    pub tab_rects: Vec<(usize, Rect)>,
    pub previous_pane_rows: Vec<(usize, Rect)>,
    pub pane_settings_rename_button: Option<Rect>,
    pub pane_settings_reload_button: Option<Rect>,
    pub pane_settings_sleep_button: Option<Rect>,
    pub pane_settings_deactivate_button: Option<Rect>,
    pub pane_settings_goal_button: Option<Rect>,
    pub pane_settings_stop_goal_button: Option<Rect>,
    pub background_job_rows: Vec<(usize, Rect)>,
    pub port_rows: Vec<(usize, Rect)>,
    pub shelf_rows: Vec<(usize, Rect)>,
    pub park_toast_rect: Option<Rect>,
}

#[derive(Debug, Clone, Default)]
pub struct PaneRenderCache {
    revision: u64,
    width: u16,
    height: u16,
    selection: Option<PaneSelection>,
    buffer: Buffer,
}

pub fn draw(frame: &mut Frame<'_>, app: &App) -> DrawState {
    let area = frame.area();
    let command_center_height = command_center_height(
        area.height,
        app.command_center_open(),
        app.command_center_height(),
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(command_center_height),
        ])
        .split(area);

    let tab_area = chunks[0];
    let grid_area = chunks[1];
    let command_center_area = chunks[2];
    let rects = app.pane_rects(grid_area);
    let palette = app.palette();
    let rename_view = app.rename_pane_view();
    let tab_rename_view = app.rename_tab_view();
    let previous_panes_view = app.previous_panes_view();
    let background_jobs_view = app.background_jobs_view();
    let port_inspector_view = app.port_inspector_view();
    let follow_up_dialog = app.follow_up_dialog();
    let pane_settings_view = app.pane_settings_view();
    let command_palette_view = app.command_palette_view();
    let pane_settings_open = pane_settings_view.is_some();
    let grid_resizer = app.grid_resizer();
    let image_overlay = app.image_overlay_view();
    let assistant_view = app.workspace_assistant_view();
    let adopt_terminal_view = app.adopt_terminal_view();
    let quit_confirmation = app.quit_confirmation_view();
    let close_grid_confirmation = app.close_grid_confirmation_view();
    let help_open = app.help_open();
    let copy_mode_open = app.copy_mode_open();
    let park_sheet = app.park_sheet_view();
    let account_sheet = app.account_sheet_view();
    let exited_recovery = if command_palette_view.is_some()
        || help_open
        || copy_mode_open
        || app.settings_open()
        || previous_panes_view.is_some()
        || background_jobs_view.is_some()
        || port_inspector_view.is_some()
        || pane_settings_open
        || rename_view.is_some()
        || tab_rename_view.is_some()
        || follow_up_dialog.is_some()
        || grid_resizer.is_some()
        || image_overlay.is_some()
        || assistant_view.is_some()
        || quit_confirmation.is_some()
        || close_grid_confirmation.is_some()
    {
        None
    } else {
        app.exited_recovery_view()
    };
    let modal_open = command_palette_view.is_some()
        || help_open
        || copy_mode_open
        || app.settings_open()
        || previous_panes_view.is_some()
        || background_jobs_view.is_some()
        || port_inspector_view.is_some()
        || pane_settings_open
        || rename_view.is_some()
        || tab_rename_view.is_some()
        || follow_up_dialog.is_some()
        || grid_resizer.is_some()
        || image_overlay.is_some()
        || assistant_view.is_some()
        || quit_confirmation.is_some()
        || close_grid_confirmation.is_some()
        || park_sheet.is_some()
        || account_sheet.is_some()
        || exited_recovery.is_some();
    let mut pane_settings_rename_button = None;
    let mut pane_settings_reload_button = None;
    let mut pane_settings_sleep_button = None;
    let mut pane_settings_deactivate_button = None;
    let mut pane_settings_goal_button = None;
    let mut pane_settings_stop_goal_button = None;
    let tab_rects = render_tabs(
        frame,
        tab_area,
        &app.tab_labels(),
        palette,
        &app.account_meters(),
        app.tab_strip_note().as_deref(),
    );

    if let Some(hint) = app.empty_parked_hint() {
        render_empty_parked(frame, grid_area, &hint);
    }

    let mut shelf_rows = Vec::new();
    if let Some(shelf) = app.shelf_view() {
        let banner = app.shelf_banner();
        let areas = shelf_areas(grid_area, banner.is_some());
        shelf_rows = render_shelf(frame, areas.list, &shelf, palette);
        if let (Some(area), Some((condition, reason))) = (areas.banner, banner) {
            render_shelf_banner(frame, area, &condition, &reason);
        }
    }
    if let Some(view) = app.keeper_view() {
        keeper::render(
            frame,
            keeper::areas(grid_area).table,
            view.command,
            view.snapshot,
            palette,
        );
    }

    // Panes share the border cells that divide them, so whichever pane draws
    // last owns the colour of the line between them. Drawing in state order lets
    // a focused or selected pane keep an unbroken outline instead of having half
    // of it repainted grey by the neighbour that happens to come after it.
    let mut draw_order = (0..app.panes().len()).collect::<Vec<_>>();
    draw_order.sort_by_cached_key(|index| pane_draw_layer(app, *index));

    for index in draw_order {
        let Some(pane) = app.panes().get(index) else {
            continue;
        };
        let Some(rect) = rects.get(index).copied() else {
            continue;
        };
        if rect.width == 0 || rect.height == 0 {
            continue;
        }

        let sleeping = app.pane_sleeping(index);
        let frame_view = PaneFrame {
            number: index + 1,
            label: app.pane_label(index),
            summary: app.pane_header_summary(index, rect.width as usize),
            usage: app.pane_usage_label(index),
            state: pane_state(app, index),
            focused: app.focused_pane() == Some(index),
            selected: app.selected().contains(&index),
            logging: app.pane_logging(index),
            compact: app.compact_titles_enabled(),
        };

        let inner = render_pane_frame(frame, rect, &frame_view, palette);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        if let Some(copy_mode) = app.copy_mode_view(index, inner.width, inner.height) {
            render_copy_mode(frame, inner, &copy_mode, palette);
        } else if sleeping {
            render_sleeping_screen(frame, inner);
        } else {
            let selection = app.selection_for_pane(index);
            app.render_pane_screen(frame, index, inner, selection);
            if !frame_view.focused {
                fade_toward_background(frame.buffer_mut(), inner);
            }
        }

        if frame_view.focused && !sleeping && !modal_open && pane.screen().scrollback() == 0 {
            set_terminal_cursor(frame, inner, pane.screen());
        }
    }

    if let Some(bounds) = rects
        .iter()
        .copied()
        .filter(|rect| !rect.is_empty())
        .reduce(|all, rect| all.union(rect))
    {
        round_outer_corners(frame.buffer_mut(), bounds);
    }

    if let Some(sheet) = park_sheet.as_ref()
        && let Some(pane) = rects.get(sheet.pane_index).copied()
    {
        render_park_sheet(frame, pane, sheet, palette);
    }
    if let Some(sheet) = account_sheet.as_ref()
        && let Some(pane) = rects.get(sheet.pane_index).copied()
    {
        render_account_sheet(frame, pane, sheet, palette);
    }
    let park_toast_rect = app
        .park_toast_view()
        .and_then(|toast| render_park_toast(frame, grid_area, &toast));

    if let Some(assistant) = assistant_view.as_ref() {
        render_workspace_assistant(frame, command_center_area, assistant, palette);
    } else if app.command_focused() {
        render_shell_command_center(frame, command_center_area, app, palette);
    }

    if app.settings_open() {
        render_settings(frame, area, app, palette);
    } else if let Some(view) = pane_settings_view.as_ref() {
        let buttons = render_pane_settings(frame, area, view, palette);
        pane_settings_rename_button = buttons.rename;
        pane_settings_reload_button = buttons.reload;
        pane_settings_sleep_button = buttons.sleep;
        pane_settings_deactivate_button = buttons.deactivate;
        pane_settings_goal_button = buttons.goal;
        pane_settings_stop_goal_button = buttons.stop_goal;
    } else if let Some(dialog) = follow_up_dialog.as_ref() {
        render_follow_up_dialog(frame, area, dialog);
    }
    let previous_pane_rows = if let Some(view) = previous_panes_view.as_ref() {
        render_previous_panes(frame, area, view, palette)
    } else {
        Vec::new()
    };
    let background_job_rows = if let Some(view) = background_jobs_view.as_ref() {
        render_background_jobs(frame, area, view, palette)
    } else {
        Vec::new()
    };
    let port_rows = if let Some(view) = port_inspector_view.as_ref() {
        render_port_inspector(frame, area, view, palette)
    } else {
        Vec::new()
    };
    if let Some(rename) = rename_view.as_ref() {
        render_rename_pane(frame, area, rename);
    }
    if let Some(rename) = tab_rename_view.as_ref() {
        render_rename_tab(frame, area, rename);
    }
    if let Some(adopt) = adopt_terminal_view.as_ref() {
        render_adopt_terminal(frame, area, adopt, palette);
    }
    if let Some(image) = image_overlay {
        render_image_overlay(frame, area, image);
    }
    if let Some(recovery) = exited_recovery.as_ref() {
        render_exited_recovery(frame, area, recovery, palette);
    }
    if let Some(picker) = grid_resizer {
        picker.draw(frame, None);
    }
    if help_open {
        render_help(frame, area, app, palette);
    }
    if let Some(view) = command_palette_view.as_ref() {
        render_command_palette(frame, area, view, palette);
    }
    if let Some(confirmation) = quit_confirmation.as_ref() {
        render_quit_confirmation(frame, area, confirmation, palette);
    }
    if let Some(confirmation) = close_grid_confirmation.as_ref() {
        render_close_grid_confirmation(frame, area, confirmation, palette);
    }

    DrawState {
        grid_area,
        pane_rects: rects,
        tab_rects,
        previous_pane_rows,
        pane_settings_rename_button,
        pane_settings_reload_button,
        pane_settings_sleep_button,
        pane_settings_deactivate_button,
        pane_settings_goal_button,
        pane_settings_stop_goal_button,
        background_job_rows,
        port_rows,
        shelf_rows,
        park_toast_rect,
    }
}

/// The blank column between two tabs.
///
/// Without it the strip is one unbroken run of coloured cells: the eye reads a
/// highlight bar with words in it rather than a row of separate things you can
/// click. The gap is what makes them tabs.
const TAB_GAP: u16 = 1;

/// Half-block caps. Filling half of the end cell rounds a tab off instead of
/// letting it stop dead on a column boundary, which is as close to a real tab
/// shape as a single-row strip can get.
const TAB_CAP_LEFT: &str = "▐";
const TAB_CAP_RIGHT: &str = "▌";

/// The longest grid name a tab will show before it is cut short.
const TAB_TITLE_CHARS: usize = 18;

/// `‹ ` and ` ›`: what the strip spends to admit there are tabs off-screen.
const TAB_OVERFLOW_WIDTH: u16 = 2;

/// One tab, styled and measured before the strip works out which ones fit.
struct TabShape {
    index: usize,
    spans: Vec<Span<'static>>,
    width: u16,
}

fn render_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    tabs: &[TabLabel],
    palette: &GridPalette,
    accounts: &[AccountMeterView],
    note: Option<&str>,
) -> Vec<(usize, Rect)> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    frame.render_widget(Block::default().style(Style::default().bg(SURFACE)), area);

    let shapes = tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| tab_shape(index, tab, palette))
        .collect::<Vec<_>>();
    let (pinned, grids): (Vec<_>, Vec<_>) = shapes
        .into_iter()
        .partition(|shape| tabs[shape.index].pinned);

    let mut tab_rects = Vec::with_capacity(tabs.len());
    let area_right = area.right();
    let mut right = area_right;
    for shape in pinned.iter().rev() {
        let x = right.saturating_sub(shape.width).max(area.x);
        tab_rects.push((shape.index, Rect::new(x, area.y, right - x, 1)));
        frame.render_widget(
            Paragraph::new(Line::from(shape.spans.clone())),
            Rect::new(x, area.y, right - x, 1),
        );
        right = x.saturating_sub(TAB_GAP);
    }
    let strip = account_strip(accounts);
    let strip_width = u16::try_from(strip.width()).unwrap_or(u16::MAX);
    if !accounts.is_empty() && right.saturating_sub(area.x) > strip_width + ACCOUNT_STRIP_TAB_ROOM {
        right -= strip_width;
        frame.render_widget(
            Paragraph::new(strip),
            Rect::new(right, area.y, strip_width, 1),
        );
    }
    if let Some(note) = note {
        let width = u16::try_from(note.chars().count() + 2).unwrap_or(u16::MAX);
        if right.saturating_sub(area.x) > width + 20 {
            right -= width;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("{note}  "),
                    Style::default().fg(TEXT_FAINT),
                )),
                Rect::new(right, area.y, width, 1),
            );
        }
    }

    let widths = grids.iter().map(|shape| shape.width).collect::<Vec<_>>();
    let active = grids
        .iter()
        .position(|shape| tabs[shape.index].active)
        .unwrap_or(0);
    let mut tab_x = area.x.saturating_add(1);
    let (first, last) = visible_tabs(&widths, active, right.saturating_sub(tab_x));
    let overflow = Style::default().fg(TEXT_FAINT).bg(SURFACE);
    let mut spans = Vec::new();
    if first > 0 {
        spans.push(Span::styled("‹ ", overflow));
        tab_x = tab_x.saturating_add(TAB_OVERFLOW_WIDTH);
    }
    for (position, shape) in grids.iter().enumerate().skip(first).take(last - first) {
        if position > first {
            spans.push(Span::raw(" "));
            tab_x = tab_x.saturating_add(TAB_GAP);
        }
        if tab_x < right {
            tab_rects.push((
                shape.index,
                Rect::new(
                    tab_x,
                    area.y,
                    shape.width.min(right.saturating_sub(tab_x)),
                    1,
                ),
            ));
        }
        tab_x = tab_x.saturating_add(shape.width);
        spans.extend(shape.spans.iter().cloned());
    }
    if last < grids.len() {
        spans.push(Span::styled(" ›", overflow));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x + 1, area.y, right.saturating_sub(area.x + 1), 1),
    );

    tab_rects.sort_by_key(|(index, _)| *index);
    tab_rects
}

/// The run of tabs to draw, as a half-open range that always contains `active`.
///
/// A strip that simply ran off the right edge hid the one tab the user was
/// looking at as soon as they opened a seventh grid — and hid it silently, since
/// a clipped `Paragraph` leaves nothing behind to say there was more.
fn visible_tabs(widths: &[u16], active: usize, budget: u16) -> (usize, usize) {
    if widths.is_empty() {
        return (0, 0);
    }

    let gaps = u16::try_from(widths.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let total = widths
        .iter()
        .fold(gaps.saturating_mul(TAB_GAP), |acc, width| {
            acc.saturating_add(*width)
        });
    // The overflow markers only have to be paid for when there is overflow, and
    // reserving them up front is what keeps the last tab from being pushed out
    // by the very marker that says it was pushed out.
    let budget = if total <= budget {
        budget
    } else {
        budget.saturating_sub(2 * TAB_OVERFLOW_WIDTH)
    };

    let active = active.min(widths.len() - 1);
    let (mut first, mut last) = (active, active + 1);
    let mut used = widths[active];
    loop {
        let mut grew = false;
        // Rightwards first, so the common case — an early tab selected out of
        // many — reads in the order the user numbered them.
        if last < widths.len() {
            let next = used.saturating_add(TAB_GAP).saturating_add(widths[last]);
            if next <= budget {
                used = next;
                last += 1;
                grew = true;
            }
        }
        if first > 0 {
            let next = used
                .saturating_add(TAB_GAP)
                .saturating_add(widths[first - 1]);
            if next <= budget {
                used = next;
                first -= 1;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    (first, last)
}

/// The selection colour marks the one grid on screen and nothing else; every other tab is
/// the same dark grey whatever its panes are doing.
fn tab_fill(tab: &TabLabel, palette: &GridPalette) -> Color {
    if tab.active {
        palette.accent()
    } else {
        SURFACE_HI
    }
}

fn tab_text_color(tab: &TabLabel, palette: &GridPalette) -> Color {
    if tab.active {
        text_on(palette.accent())
    } else {
        TEXT_DIM
    }
}

/// Dark text on a light fill and light text on a dark one, so a label stays
/// readable whatever colour the user picked for selection.
fn text_on(background: Color) -> Color {
    match color_rgb(background) {
        Some((red, green, blue))
            if 0.2126 * f32::from(red) + 0.7152 * f32::from(green) + 0.0722 * f32::from(blue)
                < 115.0 =>
        {
            TEXT
        }
        _ => INK,
    }
}

fn tab_shape(index: usize, tab: &TabLabel, palette: &GridPalette) -> TabShape {
    let fill = tab_fill(tab, palette);
    let cap = Style::default().fg(fill).bg(SURFACE);
    let mut body = Style::default().fg(tab_text_color(tab, palette)).bg(fill);
    // Selection is a mark the user put there by hand, so it has to survive
    // whatever the grid's own state is doing to the colours.
    if tab.selected {
        body = body.add_modifier(Modifier::UNDERLINED);
    }

    // One glyph carries the state that used to need a `!`/`*`/`+` cipher; the
    // rest is conveyed by colour, which needs no legend.
    let marker = if tab.exited {
        " ! "
    } else if tab.done {
        " ✓ "
    } else {
        " "
    };
    if tab.done && !tab.active {
        body = body.fg(TEXT);
    }
    // The number is how the user reaches the grid from the keyboard, so it is
    // always there — just never louder than the name it belongs to.
    let number = Style::default()
        .fg(if tab.active {
            text_on(fill)
        } else {
            TEXT_FAINT
        })
        .bg(fill);

    let spans = vec![
        Span::styled(TAB_CAP_LEFT, cap),
        Span::styled(format!(" {} ", index + 1), number),
        Span::styled(
            truncate_text(&tab.title, TAB_TITLE_CHARS),
            body.add_modifier(Modifier::BOLD),
        ),
        Span::styled(marker, body),
        Span::styled(TAB_CAP_RIGHT, cap),
    ];
    let width = u16::try_from(spans.iter().map(Span::width).sum::<usize>()).unwrap_or(u16::MAX);

    TabShape {
        index,
        spans,
        width,
    }
}

/// Columns the grid tabs keep before the account strip gives way to them.
const ACCOUNT_STRIP_TAB_ROOM: u16 = 30;
const ACCOUNT_BATTERY_CELLS: usize = 4;
const ACCOUNT_NEARLY_EMPTY: u8 = 15;
const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

fn account_strip(accounts: &[AccountMeterView]) -> Line<'static> {
    let mut spans = Vec::new();
    for account in accounts {
        let name = if account.current {
            Style::default().fg(TEXT_DIM).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT_FAINT)
        };
        spans.push(Span::styled(format!("{} ", account.name), name));
        let left = account_left_label(account);
        spans.push(Span::styled(
            format!("{left} "),
            Style::default().fg(TEXT_DIM),
        ));
        spans.extend(account_battery(account));
        spans.push(Span::raw("   "));
    }
    if let Some(next) = accounts
        .iter()
        .filter_map(|account| Some((account.resets_in_secs?, account)))
        .min_by_key(|(left, _)| *left)
    {
        spans.push(Span::styled(
            format!("↻ {} {}", next.1.name, short_duration(next.0)),
            Style::default().fg(TEXT_FAINT),
        ));
        spans.push(Span::raw("  "));
    }
    Line::from(spans).style(Style::default().bg(SURFACE))
}

fn account_left_percent(account: &AccountMeterView) -> Option<u8> {
    account
        .signed_in
        .then_some(account.used_percent)
        .flatten()
        .map(|used| 100 - used.min(100))
}

fn account_left_label(account: &AccountMeterView) -> String {
    if !account.signed_in {
        return "signed out".into();
    }
    account_left_percent(account).map_or("?".into(), |left| format!("{left}%"))
}

fn account_battery(account: &AccountMeterView) -> Vec<Span<'static>> {
    let left = account_left_percent(account).unwrap_or(0);
    let eighths =
        ((usize::from(left) * ACCOUNT_BATTERY_CELLS * 8 + 50) / 100).max(usize::from(left > 0));
    let full = eighths / 8;
    let partial = EIGHTHS[eighths % 8];
    let empty = ACCOUNT_BATTERY_CELLS - full - usize::from(!partial.is_empty());
    let charge = match account_left_percent(account) {
        Some(left) if left <= ACCOUNT_NEARLY_EMPTY => WAITING,
        _ if account.current => TEXT,
        _ => TEXT_DIM,
    };
    let body = Style::default().fg(charge).bg(LINE);
    vec![
        Span::styled("█".repeat(full), body),
        Span::styled(partial, body),
        Span::styled(" ".repeat(empty), body),
        Span::styled("╸", Style::default().fg(LINE)),
    ]
}

fn short_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    if hours >= 48 {
        format!("{}d", hours / 24)
    } else if hours >= 1 {
        format!("{hours}h")
    } else {
        format!("{}m", (seconds / 60).max(1))
    }
}

fn render_account_sheet(
    frame: &mut Frame<'_>,
    pane: Rect,
    view: &AccountSheetView,
    palette: &GridPalette,
) {
    let rows = u16::try_from(view.accounts.len()).unwrap_or(u16::MAX);
    let height = (rows + 3).min(pane.height.saturating_sub(2));
    if height < 4 || pane.width < 30 {
        return;
    }
    let area = Rect::new(
        pane.x + 1,
        pane.bottom() - 1 - height,
        pane.width - 2,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(palette.accent()))
        .title(Span::styled(
            " Account for this pane ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(SURFACE));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let name_width = view
        .accounts
        .iter()
        .map(|account| account.name.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for (index, account) in view.accounts.iter().enumerate() {
        let background = if index == view.cursor {
            SURFACE_HI
        } else {
            SURFACE
        };
        let mut spans = vec![
            Span::styled(format!(" {} ", index + 1), Style::default().fg(TEXT_FAINT)),
            Span::styled(
                format!("{:name_width$}  ", account.name),
                Style::default()
                    .fg(TEXT)
                    .add_modifier(if index == view.cursor {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ];
        spans.extend(account_battery(account));
        let detail = if !account.signed_in {
            "  not signed in".to_string()
        } else {
            let used = account_left_percent(account)
                .map_or("  usage unknown".to_string(), |left| {
                    format!("  {left:>3}% left")
                });
            let reset = account
                .resets_in_secs
                .map(|left| format!("   resets in {}", short_duration(left)))
                .unwrap_or_default();
            format!("{used}{reset}")
        };
        spans.push(Span::styled(detail, Style::default().fg(TEXT_DIM)));
        if account.current {
            spans.push(Span::styled(
                "   this pane",
                Style::default().fg(TEXT_FAINT),
            ));
        }
        lines.push(Line::from(spans).style(Style::default().bg(background)));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        " Enter or 1-9 move it there, keeping the conversation   Esc cancel",
        Style::default().fg(TEXT_FAINT),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn command_center_height(total_height: u16, open: bool, requested: u16) -> u16 {
    if !open {
        return 0;
    }
    requested.min(total_height.saturating_sub(3))
}

fn render_shell_command_center(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    palette: &GridPalette,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let title = if app.command_running() {
        " BashBot Director · Shell [ Chat | Shell ] · running "
    } else {
        " BashBot Director · Shell [ Chat | Shell ] "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )
        .title(title)
        .title_bottom(" Alt+C or Esc closes ");
    let inner = block.inner(area);
    frame.render_widget(block.style(Style::default().bg(SETTINGS_BG)), area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if inner.height < 3 {
        frame.render_widget(
            Paragraph::new(" Shell · enlarge the terminal for output")
                .style(Style::default().fg(SETTINGS_MUTED).bg(SETTINGS_BG)),
            inner,
        );
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let lines = app.command_output_lines();
    let end = lines
        .len()
        .saturating_sub(app.command_output_scroll_from_bottom())
        .max(lines.len().min(chunks[0].height as usize));
    let start = end.saturating_sub(chunks[0].height as usize);
    let visible = lines[start..end]
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(visible).style(Style::default().fg(TEXT).bg(APP_BG)),
        chunks[0],
    );

    let width = chunks[1].width as usize;
    let cwd = app.command_cwd().display().to_string();
    let cwd_budget = command_cwd_budget(width, app.command_input());
    let cwd = truncate_start(&cwd, cwd_budget);
    let prompt = format!(" {cwd} > ");
    let prompt_width = prompt.chars().count();
    let input_width = width.saturating_sub(prompt_width);
    let (input, cursor_offset) =
        visible_input(app.command_input(), app.command_cursor_chars(), input_width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                prompt,
                Style::default()
                    .fg(palette.accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(input, Style::default().fg(TEXT)),
        ]))
        .style(Style::default().bg(SETTINGS_SURFACE)),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            command_key("Tab"),
            Span::styled(" chat  ", Style::default().fg(SETTINGS_MUTED)),
            command_key("Enter"),
            Span::styled(" run  ", Style::default().fg(SETTINGS_MUTED)),
            command_key("Ctrl+↑/↓"),
            Span::styled(" resize  ", Style::default().fg(SETTINGS_MUTED)),
            command_key("PgUp/PgDn"),
            Span::styled(" scroll", Style::default().fg(SETTINGS_MUTED)),
        ]))
        .style(Style::default().bg(SETTINGS_BG)),
        chunks[2],
    );
    if input_width > 0 {
        let x = chunks[1]
            .x
            .saturating_add((prompt_width + cursor_offset).min(width.saturating_sub(1)) as u16);
        frame.set_cursor_position((x, chunks[1].y));
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

/// The one state a pane's header names.
///
/// A pane is usually several of these at once — an exited pane is also quiet, a
/// sleeping pane is also idle — so the header names the most urgent one instead
/// of stacking badges until they crowd out the pane's own name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    /// Producing output.
    Live,
    /// An agent that has stopped producing output. This is how an agent asks for
    /// the user's attention, and in a grid full of agents it is the single most
    /// useful thing on screen — so it is the one state allowed to shout.
    Waiting,
    /// Quiet, but not an agent waiting on anyone.
    Idle,
    Sleeping,
    Exited,
}

impl PaneState {
    fn badge(self) -> &'static str {
        match self {
            Self::Live => "",
            Self::Waiting => "needs you",
            Self::Idle => "idle",
            Self::Sleeping => "asleep",
            Self::Exited => "exited",
        }
    }

    fn color(self, palette: &GridPalette) -> Color {
        match self {
            Self::Live => TEXT_FAINT,
            Self::Waiting => TEXT_DIM,
            Self::Idle => palette.quiet(),
            Self::Sleeping => TEXT_FAINT,
            Self::Exited => palette.exited(),
        }
    }
}

/// Everything the chrome around one pane draws, with nothing left to look up.
///
/// Keeping this separate from `App` is what lets the frame be rendered — and
/// therefore seen and tested — without a live PTY behind it.
#[derive(Debug, Clone)]
pub struct PaneFrame {
    pub number: usize,
    pub label: String,
    pub summary: String,
    pub usage: Option<String>,
    pub state: PaneState,
    pub focused: bool,
    pub selected: bool,
    pub logging: bool,
    pub compact: bool,
}

/// Which panes get to keep their outline where two panes share a border cell.
///
/// Higher draws later, and the last pane to touch a shared cell owns its colour.
/// Focus outranks everything: knowing where your keystrokes are going matters
/// more than any other signal the grid shows.
fn pane_draw_layer(app: &App, index: usize) -> u8 {
    if app.focused_pane() == Some(index) {
        3
    } else if app.selected().contains(&index) {
        2
    } else if pane_state(app, index) == PaneState::Waiting {
        1
    } else {
        0
    }
}

fn pane_state(app: &App, index: usize) -> PaneState {
    let Some(pane) = app.panes().get(index) else {
        return PaneState::Exited;
    };
    if pane.exited {
        return PaneState::Exited;
    }
    if app.pane_sleeping(index) {
        return PaneState::Sleeping;
    }
    if !app.activity_badges_enabled() || !pane.awaiting_input() {
        return PaneState::Live;
    }
    if app.pane_needs_input(index) {
        PaneState::Waiting
    } else {
        PaneState::Idle
    }
}

fn pane_border_style(view: &PaneFrame, palette: &GridPalette) -> Style {
    if view.focused {
        return Style::default()
            .fg(palette.focus())
            .add_modifier(Modifier::BOLD);
    }
    if view.selected {
        return Style::default()
            .fg(palette.selected())
            .add_modifier(Modifier::BOLD);
    }

    match view.state {
        PaneState::Waiting => Style::default().fg(LINE),
        PaneState::Exited => Style::default().fg(palette.exited()),
        PaneState::Idle => Style::default().fg(LINE),
        PaneState::Sleeping => Style::default().fg(LINE_SOFT),
        PaneState::Live => Style::default().fg(LINE),
    }
}

/// The pane number, as a filled chip when the pane is focused or selected.
///
/// A filled chip is reserved for the two things the user chose — where input
/// goes, and what a bulk action would hit — so it never competes with a state
/// the pane arrived at on its own.
fn pane_number_style(view: &PaneFrame, palette: &GridPalette) -> Style {
    if view.focused {
        return Style::default()
            .fg(text_on(palette.focus()))
            .bg(palette.focus())
            .add_modifier(Modifier::BOLD);
    }
    if view.selected {
        return Style::default()
            .fg(text_on(palette.selected()))
            .bg(palette.selected())
            .add_modifier(Modifier::BOLD);
    }

    Style::default()
        .fg(view.state.color(palette))
        .add_modifier(Modifier::BOLD)
}

/// Draws a pane's frame and returns the area left for its terminal.
fn render_pane_frame(
    frame: &mut Frame<'_>,
    rect: Rect,
    view: &PaneFrame,
    palette: &GridPalette,
) -> Rect {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(pane_border_style(view, palette))
        // Panes are laid out overlapping by one cell, so each shared cell is
        // drawn twice. Merging resolves the pair into a single line with the
        // junction glyph the crossing actually calls for.
        .merge_borders(MergeStrategy::Exact)
        .style(Style::default().bg(APP_BG));

    // Below about eight columns a header is all truncation and no information,
    // and the number alone tells the user more than a sliced-up word would.
    if rect.width >= 8 {
        let budget = rect.width.saturating_sub(2);
        // The state is reserved out of the budget before the name and summary
        // are measured, rather than fitted around them afterwards. A pane whose
        // agent is waiting has to say so at every width — that signal is the
        // whole reason to look at a grid of nine panes — and sizing it last is
        // what made it the first thing a narrow header dropped.
        let trailing = pane_header_trailing(view, palette, rect.width);
        let trailing_width = trailing.width() as u16;
        // The two titles are drawn independently, one from each end, so the
        // state only goes up if the number it would sit beside still fits too.
        // Without that floor a narrow pane draws both and they overlap.
        let fits = trailing_width > 0 && pane_number_width(view) + trailing_width <= budget;
        let leading = pane_header_leading(
            view,
            palette,
            if fits {
                budget - trailing_width
            } else {
                budget
            },
        );
        if fits {
            block = block.title_top(trailing.right_aligned());
        }
        block = block.title_top(leading);
    }

    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}

/// Width of the pane-number chip, which is the one part of a header that is
/// never dropped — it is how the user names a pane to a keyboard shortcut.
fn pane_number_width(view: &PaneFrame) -> u16 {
    // Panes are capped well below four digits, so this cannot overflow.
    view.number.to_string().chars().count() as u16 + 2
}

/// The pane's identity, fitted to whatever the state left behind.
///
/// Every piece is measured against the running total, so the line never exceeds
/// `budget` — which is what lets the caller reserve the state's width up front
/// and trust that it still fits.
fn pane_header_leading(view: &PaneFrame, palette: &GridPalette, budget: u16) -> Line<'static> {
    let budget = budget as usize;
    let number = format!(" {} ", view.number);
    let mut used = number.chars().count();
    let mut spans = vec![Span::styled(number, pane_number_style(view, palette))];
    if used >= budget {
        return Line::from(spans);
    }

    // The name is how the user tells one pane from another, so it is the last
    // thing to go.
    let label = if view.label == view.number.to_string() {
        String::new()
    } else {
        truncate_text(&view.label, budget - used - 1)
    };
    if !label.is_empty() {
        used += label.chars().count() + 1;
        spans.push(Span::styled(
            format!("{label} "),
            if view.focused {
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT_DIM)
            },
        ));
    }

    // The activity summary is the header's least urgent text: useful to read,
    // never needed to act. It appears only once everything else is safely on
    // screen, and only if enough of it survives to still mean something.
    let summary_budget = budget.saturating_sub(used + 3);
    if !view.compact && !view.summary.is_empty() && summary_budget >= 6 {
        let summary = truncate_text(&view.summary, summary_budget);
        spans.push(Span::styled(
            format!("· {summary} "),
            Style::default().fg(TEXT_FAINT),
        ));
    }

    Line::from(spans)
}

/// The width below which a pane's header has no room for its usage figure.
///
/// Usage is reference material — you go looking for it. Below this the header is
/// down to the pane's name and its state, and spending a third of a narrow
/// header on a quota reading buys nothing.
const USAGE_HEADER_WIDTH: u16 = 48;

fn pane_header_trailing(view: &PaneFrame, palette: &GridPalette, width: u16) -> Line<'static> {
    let mut spans = Vec::new();

    if view.logging {
        spans.push(Span::styled(
            "rec ",
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(usage) = view.usage.as_deref().filter(|usage| !usage.is_empty())
        && !view.compact
        && width >= USAGE_HEADER_WIDTH
    {
        let separator = if view.state.badge().is_empty() {
            " "
        } else {
            " · "
        };
        spans.push(Span::styled(
            format!("{usage}{separator}"),
            Style::default().fg(TEXT_FAINT),
        ));
    }

    let badge = view.state.badge();
    if !badge.is_empty() {
        spans.push(Span::styled(
            format!("{badge} "),
            Style::default().fg(view.state.color(palette)),
        ));
    }

    // Right-aligned text lands flush against the border rule that fills the rest
    // of the header, so it needs a gap of its own to read as a separate thing.
    if !spans.is_empty() {
        spans.insert(0, Span::raw(" "));
    }

    Line::from(spans)
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

    let width = area.width.saturating_sub(4).min(100).max(area.width.min(1));
    let inner_width = width.saturating_sub(2);
    let lines = pane_settings_lines(view, inner_width, palette);
    let desired_height = (lines.len() as u16).saturating_add(2);
    let height = area
        .height
        .saturating_sub(2)
        .min(desired_height)
        .max(area.height.min(1));
    let modal = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
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
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Pane Activity ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    frame.render_widget(Paragraph::new(lines).style(settings_panel_style()), inner);

    PaneSettingsButtons {
        rename: pane_settings_rename_rect(inner, view.auth_kind.is_some()),
        reload: pane_settings_reload_rect(inner, view.auth_kind.is_some()),
        sleep: pane_settings_sleep_rect(inner, view.auth_kind.is_some(), view.goal.is_some()),
        deactivate: pane_settings_deactivate_rect(
            inner,
            view.auth_kind.is_some(),
            view.goal.is_some(),
        ),
        goal: pane_settings_goal_rect(inner, view.auth_kind.is_some(), view.goal.is_some()),
        stop_goal: pane_settings_stop_goal_rect(
            inner,
            view.auth_kind.is_some(),
            view.goal.is_some(),
        ),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PaneSettingsButtons {
    rename: Option<Rect>,
    reload: Option<Rect>,
    sleep: Option<Rect>,
    deactivate: Option<Rect>,
    goal: Option<Rect>,
    stop_goal: Option<Rect>,
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
            fixed_width(
                &format!(" Pane {} {}", view.index + 1, view.label),
                width as usize,
            ),
            Style::default()
                .fg(palette.focus())
                .bg(SETTINGS_BG)
                .add_modifier(Modifier::BOLD),
        )));
        if let Some(kind) = view.auth_kind {
            let auth = view
                .auth_options
                .get(view.auth_cursor)
                .map(|option| option.name.as_str())
                .unwrap_or("none");
            let selected = view.selected_target == PaneSettingsTarget::Auth;
            lines.push(Line::from(Span::styled(
                fixed_width(
                    &format!(
                        "{} {} auth: {auth}",
                        if selected { ">" } else { " " },
                        kind.display_name()
                    ),
                    width as usize,
                ),
                Style::default().fg(SETTINGS_TEXT).bg(if selected {
                    SETTINGS_ROW_ACTIVE
                } else {
                    SETTINGS_BG
                }),
            )));
        }
        lines.push(Line::from(Span::styled(
            fixed_width(
                &format!(" latest: {}", view.history_summary),
                width as usize,
            ),
            Style::default().fg(SETTINGS_TEXT),
        )));
        lines.push(pane_settings_rename_line(
            width,
            palette,
            view.selected_target == PaneSettingsTarget::Rename,
        ));
        lines.push(pane_settings_reload_line(
            width,
            palette,
            view.selected_target == PaneSettingsTarget::Reload,
        ));
        lines.push(pane_settings_sleep_line(
            width,
            view.sleeping,
            palette,
            view.selected_target == PaneSettingsTarget::Sleep,
        ));
        lines.push(pane_settings_deactivate_line(
            width,
            palette,
            view.selected_target == PaneSettingsTarget::Deactivate,
        ));
        lines.push(pane_settings_goal_line(
            width,
            view.goal.is_some(),
            palette,
            view.selected_target == PaneSettingsTarget::Goal,
        ));
        if view.goal.is_some() {
            lines.push(pane_settings_stop_goal_line(
                width,
                palette,
                view.selected_target == PaneSettingsTarget::StopGoal,
            ));
        }
        lines.push(pane_settings_command_bar(
            width,
            !view.auth_options.is_empty(),
        ));
        return lines;
    }

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Pane Activity",
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
    if let Some(kind) = view.auth_kind {
        lines.push(settings_section(
            "AUTH ACCOUNT",
            "Left/Right selects; Enter applies and restarts",
            width,
        ));
        if let Some(option) = view.auth_options.get(view.auth_cursor) {
            let account = option.account_label.as_deref().unwrap_or("no account");
            let current = if option.current { " current" } else { "" };
            let status = if option.ready {
                "ready"
            } else {
                "login needed"
            };
            let selected = view.selected_target == PaneSettingsTarget::Auth;
            let account = truncate_text(
                &format!("{} | {} | {}{}", option.name, account, status, current),
                width.saturating_sub(8) as usize,
            );
            lines.push(Line::from(Span::styled(
                fixed_width(
                    &format!("{} < {account} >", if selected { ">" } else { " " }),
                    width as usize,
                ),
                Style::default()
                    .fg(kind_color(kind))
                    .bg(if selected {
                        SETTINGS_ROW_ACTIVE
                    } else {
                        SETTINGS_BG
                    })
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!(
                        "No {} auth profiles. Open global Auth settings.",
                        kind.as_str()
                    ),
                    Style::default().fg(SETTINGS_MUTED),
                ),
            ]));
        }
    } else {
        lines.push(Line::from(""));
    }
    lines.push(settings_section(
        "RECENT ACTIVITY",
        view.history_notice
            .as_deref()
            .unwrap_or("latest AI activity summary"),
        width,
    ));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("summary  ", Style::default().fg(SETTINGS_MUTED)),
        Span::styled(
            truncate_text(&view.history_summary, width.saturating_sub(11) as usize),
            Style::default().fg(SETTINGS_TEXT),
        ),
    ]));
    if view.auth_kind.is_none() {
        lines.push(Line::from(""));
    }
    lines.push(pane_settings_rename_line(
        width,
        palette,
        view.selected_target == PaneSettingsTarget::Rename,
    ));
    lines.push(pane_settings_reload_line(
        width,
        palette,
        view.selected_target == PaneSettingsTarget::Reload,
    ));
    lines.push(settings_section(
        "PANE CONTROLS",
        if view.manager_configured {
            "BashBot Director ready in Alt+C"
        } else {
            "configure the grid Manager in global settings"
        },
        width,
    ));
    if let Some(goal) = &view.goal {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                truncate_text(
                    &format!("grid goal: {} | {}", goal.objective, goal.status),
                    width.saturating_sub(2) as usize,
                ),
                Style::default().fg(Color::LightCyan),
            ),
        ]));
    }
    lines.push(pane_settings_sleep_line(
        width,
        view.sleeping,
        palette,
        view.selected_target == PaneSettingsTarget::Sleep,
    ));
    lines.push(pane_settings_deactivate_line(
        width,
        palette,
        view.selected_target == PaneSettingsTarget::Deactivate,
    ));
    lines.push(pane_settings_goal_line(
        width,
        view.goal.is_some(),
        palette,
        view.selected_target == PaneSettingsTarget::Goal,
    ));
    if view.goal.is_some() {
        lines.push(pane_settings_stop_goal_line(
            width,
            palette,
            view.selected_target == PaneSettingsTarget::StopGoal,
        ));
    }
    if view.auth_kind.is_none() {
        lines.push(Line::from(""));
    }
    lines.push(pane_settings_command_bar(
        width,
        !view.auth_options.is_empty(),
    ));

    lines
}

fn pane_settings_rename_line(width: u16, palette: &GridPalette, selected: bool) -> Line<'static> {
    pane_settings_action_line("[ Rename pane ]", width, palette.selected(), selected)
}

fn pane_settings_reload_line(width: u16, palette: &GridPalette, selected: bool) -> Line<'static> {
    pane_settings_action_line("[ Refresh activity ]", width, palette.focus(), selected)
}

fn render_workspace_assistant(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &WorkspaceAssistantView,
    palette: &GridPalette,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )
        .title(format!(
            " BashBot Director · {} · Chat [ Chat | Shell ] ",
            view.grid_title
        ))
        .title_bottom(" Alt+C or Esc closes ");
    let inner = block.inner(area);
    frame.render_widget(
        block.style(Style::default().fg(SETTINGS_TEXT).bg(SETTINGS_BG)),
        area,
    );

    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if inner.height < 5 {
        frame.render_widget(
            Paragraph::new(" [>_] BashBot Director · enlarge the terminal to chat")
                .style(Style::default().fg(palette.focus()).bg(SETTINGS_BG)),
            inner,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let state = if view.busy {
        "thinking..."
    } else if view.configured {
        "ready"
    } else {
        "setup needed"
    };
    let header = vec![
        Line::from(vec![
            Span::styled(" [>_]  ", Style::default().fg(palette.focus())),
            Span::styled(
                format!("BashBot Director · {state} · {} panes", view.pane_count),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            view.goal
                .as_deref()
                .map(|goal| format!(" goal · {goal}"))
                .unwrap_or_else(|| " /goal start · /stop end · brief · delegate".into()),
            Style::default().fg(SETTINGS_MUTED),
        )]),
    ];
    frame.render_widget(
        Paragraph::new(header).style(Style::default().bg(SETTINGS_BG)),
        chunks[0],
    );

    let transcript = assistant_transcript_lines(
        view,
        chunks[1].width as usize,
        chunks[1].height as usize,
        view.scroll_from_bottom,
        palette,
    );
    frame.render_widget(
        Paragraph::new(transcript).style(Style::default().bg(SETTINGS_BG)),
        chunks[1],
    );

    let prefix = "you › ";
    let input_width = chunks[2]
        .width
        .saturating_sub(prefix.chars().count() as u16) as usize;
    let (visible, cursor_offset) = visible_input(&view.input, view.cursor_chars, input_width);
    let input = if view.input.is_empty() {
        "Ask about this grid or use /goal...".to_string()
    } else {
        visible
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                prefix,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_text(&input, input_width),
                Style::default().fg(if view.input.is_empty() {
                    SETTINGS_MUTED
                } else {
                    SETTINGS_TEXT
                }),
            ),
        ]))
        .style(Style::default().bg(SETTINGS_SURFACE)),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            command_key("Enter"),
            Span::styled(" send  ", Style::default().fg(SETTINGS_MUTED)),
            command_key("Tab"),
            Span::styled(" shell  ", Style::default().fg(SETTINGS_MUTED)),
            command_key("Ctrl+U"),
            Span::styled(" clear  ", Style::default().fg(SETTINGS_MUTED)),
            command_key("Ctrl+↑/↓"),
            Span::styled(" resize", Style::default().fg(SETTINGS_MUTED)),
        ]))
        .style(Style::default().bg(SETTINGS_BG)),
        chunks[3],
    );

    if input_width > 0 {
        let cursor_x = chunks[2]
            .x
            .saturating_add(prefix.chars().count() as u16)
            .saturating_add(cursor_offset.min(input_width.saturating_sub(1)) as u16);
        frame.set_cursor_position((cursor_x, chunks[2].y));
    }
}

fn assistant_transcript_lines(
    view: &WorkspaceAssistantView,
    width: usize,
    height: usize,
    scroll_from_bottom: usize,
    palette: &GridPalette,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    if view.messages.is_empty() {
        let welcome = if view.configured {
            "Ask me to brief this grid, sharpen a prompt, delegate work, or start a /goal."
        } else {
            "Set the Manager endpoint, model, and API key in Settings to start chatting."
        };
        push_assistant_message_lines(
            &mut lines,
            "bot › ",
            welcome,
            width,
            Style::default().fg(palette.accent()),
        );
    } else {
        for message in &view.messages {
            let (prefix, style) = match message.role {
                AssistantMessageRole::User => (
                    "you › ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                AssistantMessageRole::BashBot => (
                    "bot › ",
                    Style::default()
                        .fg(palette.accent())
                        .add_modifier(Modifier::BOLD),
                ),
            };
            push_assistant_message_lines(&mut lines, prefix, &message.text, width, style);
        }
    }
    if view.busy {
        push_assistant_message_lines(
            &mut lines,
            "bot › ",
            "reviewing this grid...",
            width,
            Style::default().fg(palette.accent()),
        );
    }

    let end = lines
        .len()
        .saturating_sub(scroll_from_bottom)
        .max(lines.len().min(height));
    let start = end.saturating_sub(height);
    lines.into_iter().skip(start).take(end - start).collect()
}

fn push_assistant_message_lines(
    lines: &mut Vec<Line<'static>>,
    prefix: &'static str,
    text: &str,
    width: usize,
    prefix_style: Style,
) {
    let prefix_width = prefix.chars().count();
    let content_width = width.saturating_sub(prefix_width).max(1);
    for (index, wrapped) in wrap_text(text, content_width).into_iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                if index == 0 {
                    prefix.to_string()
                } else {
                    " ".repeat(prefix_width)
                },
                prefix_style,
            ),
            Span::styled(wrapped, Style::default().fg(SETTINGS_TEXT)),
        ]));
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_width = word.chars().count();
        let next_width = current.chars().count() + usize::from(!current.is_empty()) + word_width;
        if next_width <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            if word_width <= width {
                current.push_str(word);
            } else {
                let chars = word.chars().collect::<Vec<_>>();
                for chunk in chars.chunks(width) {
                    lines.push(chunk.iter().collect());
                }
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn pane_settings_sleep_line(
    width: u16,
    sleeping: bool,
    palette: &GridPalette,
    selected: bool,
) -> Line<'static> {
    pane_settings_action_line(
        if sleeping {
            "[ Wake pane ]"
        } else {
            "[ Sleep pane ]"
        },
        width,
        palette.quiet(),
        selected,
    )
}

fn pane_settings_deactivate_line(
    width: u16,
    palette: &GridPalette,
    selected: bool,
) -> Line<'static> {
    pane_settings_action_line("[ Deactivate pane ]", width, palette.exited(), selected)
}

fn pane_settings_goal_line(
    width: u16,
    has_goal: bool,
    palette: &GridPalette,
    selected: bool,
) -> Line<'static> {
    pane_settings_action_line(
        if has_goal {
            "[ Edit goal in Director ]"
        } else {
            "[ Set goal in Director ]"
        },
        width,
        palette.accent(),
        selected,
    )
}

fn pane_settings_stop_goal_line(
    width: u16,
    palette: &GridPalette,
    selected: bool,
) -> Line<'static> {
    pane_settings_action_line("[ Stop Director goal ]", width, palette.exited(), selected)
}

fn pane_settings_action_line(
    label: &str,
    width: u16,
    background: Color,
    selected: bool,
) -> Line<'static> {
    let label = if selected {
        format!("> {label} <")
    } else {
        label.to_string()
    };
    let text = if width as usize <= label.len() + 4 {
        fixed_width(&label, width as usize)
    } else {
        let left = ((width as usize).saturating_sub(label.len())) / 2;
        let right = (width as usize).saturating_sub(left + label.len());
        format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
    };

    Line::from(Span::styled(
        text,
        Style::default()
            .fg(if selected {
                SETTINGS_TEXT
            } else {
                Color::Black
            })
            .bg(if selected {
                SETTINGS_ROW_ACTIVE
            } else {
                background
            })
            .add_modifier(Modifier::BOLD),
    ))
}

fn pane_settings_command_bar(width: u16, has_auth: bool) -> Line<'static> {
    if width < 64 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Up/Down"),
            Span::styled(" select  ", Style::default().fg(Color::Gray)),
            command_key("Enter"),
            Span::styled(" use  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }

    let mut spans = vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" select  ", Style::default().fg(Color::Gray)),
        command_key(if width >= 72 { "Enter/Space" } else { "Enter" }),
        Span::styled(" use  ", Style::default().fg(Color::Gray)),
    ];
    if has_auth {
        spans.push(command_key("Left/Right"));
        spans.push(Span::styled(" auth  ", Style::default().fg(Color::Gray)));
    }
    spans.extend([
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ]);
    Line::from(spans)
}

fn pane_settings_rename_rect(area: Rect, has_auth: bool) -> Option<Rect> {
    let row = if area.width < 36 && has_auth {
        3
    } else if area.width < 36 {
        2
    } else {
        6
    };
    pane_settings_action_rect(area, row)
}

fn pane_settings_reload_rect(area: Rect, has_auth: bool) -> Option<Rect> {
    let row = if area.width < 36 && has_auth {
        4
    } else if area.width < 36 {
        3
    } else {
        7
    };
    pane_settings_action_rect(area, row)
}

fn pane_settings_sleep_rect(area: Rect, has_auth: bool, has_goal: bool) -> Option<Rect> {
    let row = if area.width < 36 {
        if has_auth { 5 } else { 4 }
    } else if has_goal {
        10
    } else {
        9
    };
    pane_settings_action_rect(area, row)
}

fn pane_settings_deactivate_rect(area: Rect, has_auth: bool, has_goal: bool) -> Option<Rect> {
    let row = if area.width < 36 {
        if has_auth { 6 } else { 5 }
    } else if has_goal {
        11
    } else {
        10
    };
    pane_settings_action_rect(area, row)
}

fn pane_settings_goal_rect(area: Rect, has_auth: bool, has_goal: bool) -> Option<Rect> {
    let row = if area.width < 36 {
        if has_auth { 7 } else { 6 }
    } else if has_goal {
        12
    } else {
        11
    };
    pane_settings_action_rect(area, row)
}

fn pane_settings_stop_goal_rect(area: Rect, has_auth: bool, has_goal: bool) -> Option<Rect> {
    if !has_goal {
        return None;
    }
    let row = if area.width < 36 {
        if has_auth { 8 } else { 7 }
    } else {
        13
    };
    pane_settings_action_rect(area, row)
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

fn render_port_inspector(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &PortInspectorView,
    palette: &GridPalette,
) -> Vec<(usize, Rect)> {
    let modal = previous_panes_modal_rect(area, view.ports.len().max(1));
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
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Agent Ports ");
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
    let count = match view.ports.len() {
        1 => "1 TCP listener".to_string(),
        count => format!("{count} TCP listeners"),
    };
    let scan_state = if view.refreshing {
        "  refreshing..."
    } else {
        "  launched by GridBash agents"
    };
    let header = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                count,
                Style::default()
                    .fg(palette.accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(scan_state, Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::styled(
            view.error
                .as_deref()
                .map(|error| format!("  Scan error: {error}"))
                .unwrap_or_else(|| "  PORT   PROCESS              PID  PANE / TAB".into()),
            Style::default().fg(if view.error.is_some() {
                Color::LightRed
            } else {
                SETTINGS_MUTED
            }),
        )),
    ];
    frame.render_widget(
        Paragraph::new(header).style(settings_panel_style()),
        chunks[0],
    );

    let list_area = chunks[1];
    if view.ports.is_empty() {
        let message = if view.refreshing {
            "  Looking for localhost listeners in agent process trees..."
        } else {
            "  No agent-owned localhost listeners are active."
        };
        frame.render_widget(
            Paragraph::new(Span::styled(message, Style::default().fg(SETTINGS_MUTED)))
                .style(settings_panel_style()),
            list_area,
        );
    } else {
        let visible =
            visible_previous_pane_range(view.ports.len(), view.cursor, list_area.height as usize);
        let mut rows = Vec::new();
        for (row_offset, index) in visible.enumerate() {
            let Some(port) = view.ports.get(index) else {
                continue;
            };
            let row_area = Rect {
                x: list_area.x,
                y: list_area.y.saturating_add(row_offset as u16),
                width: list_area.width,
                height: 1,
            };
            row_hits.push((index, row_area));
            rows.push(agent_port_line(
                port.port,
                port.pid,
                &port.process,
                &port.owner,
                view.cursor == index,
                view.pending_terminate == Some(port.pid),
                list_area.width,
            ));
        }
        frame.render_widget(
            Paragraph::new(rows).style(settings_panel_style()),
            list_area,
        );
    }

    frame.render_widget(
        Paragraph::new(port_inspector_command_bar(
            chunks[2].width,
            view.pending_terminate.is_some(),
        ))
        .style(settings_panel_style()),
        chunks[2],
    );
    row_hits
}

#[allow(clippy::too_many_arguments)]
fn agent_port_line(
    port: u16,
    pid: u32,
    process: &str,
    owner: &str,
    active: bool,
    pending_terminate: bool,
    width: u16,
) -> Line<'static> {
    let process_width = if width < 62 { 12 } else { 18 };
    let owner = if active && pending_terminate {
        "Press Enter to terminate"
    } else {
        owner
    };
    let text = format!(
        "{} {:>5}   {:<process_width$} {:>7}  {}",
        if active { ">" } else { " " },
        port,
        truncate_text(process, process_width),
        pid,
        owner,
    );
    let fg = if active && pending_terminate {
        Color::LightRed
    } else if active {
        SETTINGS_TEXT
    } else {
        Color::LightCyan
    };
    Line::from(Span::styled(
        fixed_width(&text, width as usize),
        row_style(fg, active.then_some(SETTINGS_ROW_ACTIVE), active),
    ))
}

fn port_inspector_command_bar(width: u16, pending_terminate: bool) -> Line<'static> {
    if pending_terminate {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Enter"),
            Span::styled(" terminate process  ", Style::default().fg(Color::LightRed)),
            command_key("Esc"),
            Span::styled(" cancel", Style::default().fg(Color::Gray)),
        ]);
    }
    if width < 60 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Enter"),
            Span::styled(" stop  ", Style::default().fg(Color::Gray)),
            command_key("R"),
            Span::styled(" refresh  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }
    Line::from(vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" move  ", Style::default().fg(Color::Gray)),
        command_key("Enter/Delete"),
        Span::styled(" terminate  ", Style::default().fg(Color::Gray)),
        command_key("R"),
        Span::styled(" refresh  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ])
}

fn render_background_jobs(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &BackgroundJobsView,
    palette: &GridPalette,
) -> Vec<(usize, Rect)> {
    let modal = previous_panes_modal_rect(area, view.jobs.len().max(1));
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
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Background Agents ");
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
    let count_label = if view.jobs.len() == 1 {
        "1 background agent".into()
    } else {
        format!("{} background agents", view.jobs.len())
    };
    let header = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            count_label,
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  session-wide pool", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(
        Paragraph::new(vec![header, Line::from("")]).style(settings_panel_style()),
        chunks[0],
    );

    let list_area = chunks[1];
    if view.jobs.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "No background agents. Press Alt+Shift+B from the grid to add one.",
                    Style::default().fg(SETTINGS_MUTED),
                ),
            ]))
            .style(settings_panel_style()),
            list_area,
        );
    } else {
        let visible =
            visible_previous_pane_range(view.jobs.len(), view.cursor, list_area.height as usize);
        let mut rows = Vec::new();
        for (row_offset, index) in visible.enumerate() {
            let Some(job) = view.jobs.get(index) else {
                continue;
            };
            let row_area = Rect {
                x: list_area.x,
                y: list_area.y.saturating_add(row_offset as u16),
                width: list_area.width,
                height: 1,
            };
            row_hits.push((index, row_area));
            rows.push(background_job_line(
                job,
                view.cursor == index,
                view.pending_delete == Some(job.id),
                list_area.width,
            ));
        }
        frame.render_widget(
            Paragraph::new(rows).style(settings_panel_style()),
            list_area,
        );
    }

    frame.render_widget(
        Paragraph::new(background_jobs_command_bar(chunks[2].width)).style(settings_panel_style()),
        chunks[2],
    );
    row_hits
}

fn background_job_line(
    job: &BackgroundJobView,
    active: bool,
    pending_delete: bool,
    width: u16,
) -> Line<'static> {
    let (state, state_color) = background_job_state(job.state, pending_delete);
    let label_width = if width < 72 { 12 } else { 16 };
    let agent_width = if width < 72 { 8 } else { 12 };
    let location_width = if width < 72 { 16 } else { 28 };
    let marker = if active { ">" } else { " " };
    let mut location = format!("{} | {}", job.source_tab, job.folder);
    if let Some(worktree) = job.worktree.as_deref() {
        location.push_str(" | ");
        location.push_str(worktree);
    }
    let summary = if pending_delete {
        "Delete again to stop this live job"
    } else {
        &job.summary
    };
    let text = format!(
        "{marker} {:<label_width$} {:<8} {:<agent_width$} {:<location_width$} {}",
        truncate_text(&job.label, label_width),
        state,
        truncate_text(&job.agent, agent_width),
        truncate_text(&location, location_width),
        summary,
    );
    let bg = active.then_some(SETTINGS_ROW_ACTIVE);
    let fg = if active { SETTINGS_TEXT } else { state_color };
    Line::from(Span::styled(
        fixed_width(&text, width as usize),
        row_style(fg, bg, active || pending_delete),
    ))
}

fn background_job_state(state: BackgroundJobState, pending_delete: bool) -> (&'static str, Color) {
    if pending_delete {
        return ("stop?", Color::Red);
    }
    match state {
        BackgroundJobState::Working => ("working", Color::Green),
        BackgroundJobState::Quiet => ("quiet", Color::Cyan),
        BackgroundJobState::Exited => ("exited", Color::Red),
        BackgroundJobState::Offline => ("offline", Color::DarkGray),
    }
}

fn background_jobs_command_bar(width: u16) -> Line<'static> {
    if width < 64 {
        return Line::from(vec![
            Span::raw("  "),
            command_key("Enter"),
            Span::styled(" insert  ", Style::default().fg(Color::Gray)),
            command_key("R"),
            Span::styled(" restart  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]);
    }
    Line::from(vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" move  ", Style::default().fg(Color::Gray)),
        command_key("Enter"),
        Span::styled(" insert  ", Style::default().fg(Color::Gray)),
        command_key("R"),
        Span::styled(" restart  ", Style::default().fg(Color::Gray)),
        command_key("Delete"),
        Span::styled(" stop/remove  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" close", Style::default().fg(Color::Gray)),
    ])
}

/// The picker that re-roots a pane at an outside terminal's folder.
///
/// It is careful to promise only what it does. "Adopting" a window moves the
/// pane to that window's folder; the window itself keeps its process and is left
/// running, because a live console cannot be handed to another pseudoconsole.
fn render_adopt_terminal(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &AdoptTerminalView,
    palette: &GridPalette,
) {
    let modal = previous_panes_modal_rect(area, view.rows.len().saturating_add(2));
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
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Adopt A Terminal ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let budget = inner.width.saturating_sub(4) as usize;
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("into pane {}", view.target_label),
                    Style::default()
                        .fg(palette.focus())
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    truncate_text(
                        "opens a shell where the window is; the window keeps running",
                        budget,
                    ),
                    Style::default().fg(SETTINGS_MUTED),
                ),
            ]),
        ])
        .style(settings_panel_style()),
        chunks[0],
    );

    let rows = chunks[1];
    let visible = rows.height as usize;
    let first = view.cursor.saturating_sub(visible.saturating_sub(1));
    let lines = view
        .rows
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .map(|(index, row)| {
            let selected = index == view.cursor;
            // A window whose title hides its folder is still listed, so the user
            // is not left wondering why it is missing — it just cannot be picked.
            let style = if !row.adoptable {
                Style::default().fg(TEXT_FAINT)
            } else if selected {
                Style::default()
                    .fg(INK)
                    .bg(palette.focus())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(SETTINGS_TEXT)
            };
            Line::from(vec![
                Span::styled(if selected { " ▸ " } else { "   " }, style),
                Span::styled(truncate_text(&row.label, budget.saturating_sub(10)), style),
                Span::styled(
                    format!("  pid {}", row.pid),
                    if selected {
                        style
                    } else {
                        Style::default().fg(SETTINGS_MUTED)
                    },
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(settings_panel_style()), rows);

    let footer = if view.confirming {
        Line::from(vec![
            Span::styled(
                "  replace this pane? ",
                Style::default().fg(WAITING).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "its shell closes · Enter confirms · Esc goes back",
                Style::default().fg(SETTINGS_MUTED),
            ),
        ])
    } else {
        Line::from(Span::styled(
            "  ↑↓ choose · Enter adopts · Esc cancels",
            Style::default().fg(SETTINGS_MUTED),
        ))
    };
    frame.render_widget(
        Paragraph::new(footer).style(settings_panel_style()),
        chunks[2],
    );
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

    let title = if app.settings_tab() == SettingsTab::Auth {
        " Auth Profiles | Alt+Shift+A "
    } else {
        " GridBash Settings "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(title);
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

fn render_quit_confirmation(
    frame: &mut Frame<'_>,
    area: Rect,
    confirmation: &QuitConfirmationView,
    palette: &GridPalette,
) {
    let modal = quit_confirmation_modal_rect(area);
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
        .title(" Quit GridBash? ")
        .title_bottom(" Alt+Q confirms | Any other key cancels ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let behavior = if confirmation.keeps_terminals_running {
        "Your exact workspace has been saved. Live terminals will stay running."
    } else {
        "Your exact workspace has been saved. Pane processes will close with GridBash."
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(behavior, Style::default().fg(SETTINGS_TEXT))),
        Line::from(""),
        Line::from(Span::styled(
            "Resume this setup directly with:",
            Style::default().fg(SETTINGS_MUTED),
        )),
        Line::from(Span::styled(
            confirmation.resume_command.clone(),
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).style(settings_panel_style()), inner);
}

fn render_close_grid_confirmation(
    frame: &mut Frame<'_>,
    area: Rect,
    confirmation: &CloseGridConfirmationView,
    palette: &GridPalette,
) {
    let modal = close_grid_confirmation_modal_rect(area);
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
                .fg(palette.exited())
                .add_modifier(Modifier::BOLD),
        )
        .style(settings_panel_style())
        .title(" Close current grid? ")
        .title_bottom(" Enter / Y closes | Esc / N cancels ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let pane_label = if confirmation.pane_count == 1 {
        "pane"
    } else {
        "panes"
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "Close \"{}\" and terminate {} {pane_label}?",
                confirmation.title, confirmation.pane_count
            ),
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Every visible process in this grid will stop.",
            Style::default().fg(SETTINGS_TEXT),
        )),
        Line::from(Span::styled(
            "Managed worktrees and branches will stay on disk.",
            Style::default().fg(SETTINGS_MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(settings_panel_style()),
        inner,
    );
}

fn settings_content_row_count(app: &App) -> usize {
    match app.settings_tab() {
        SettingsTab::General => app.settings_rows().len(),
        SettingsTab::Auth => {
            app.auth_profiles().len().max(1) + usize::from(app.auth_create().is_some()) * 3 + 3
        }
        SettingsTab::Manager => app.manager_settings_rows().len() + 5,
    }
}

fn settings_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    match app.settings_tab() {
        SettingsTab::General => general_settings_lines(&app.settings_rows(), width),
        SettingsTab::Auth => auth_settings_lines(app, width),
        SettingsTab::Manager => manager_settings_lines(app, width),
    }
}

fn settings_tabs(active: SettingsTab) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        settings_tab("General", active == SettingsTab::General),
        Span::raw("  "),
        settings_tab("Auth", active == SettingsTab::Auth),
        Span::raw("  "),
        settings_tab("Manager", active == SettingsTab::Manager),
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
            "FOCUSED PANE",
            "Enter applies the highlighted compatible profile and restarts this pane",
            width,
        ),
        auth_focused_pane_line(app, width),
        Line::from(""),
        settings_section(
            "NEW PANE POLICY",
            "only affects panes when they start; running panes keep their current profile",
            width,
        ),
        auth_new_pane_policy_line(app, width),
        Line::from(""),
        settings_section(
            "AUTH PROFILES",
            if app.auth_refreshing() {
                "refreshing local account and usage status"
            } else {
                "isolated Claude/Codex homes; each keeps its own login and usage"
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
    lines.extend(auth_command_bar(width));
    lines
}

fn auth_focused_pane_line(app: &App, width: u16) -> Line<'static> {
    let Some(pane) = app.auth_pane_view() else {
        return Line::from(vec![
            Span::raw("  "),
            Span::styled("No focused pane.", Style::default().fg(SETTINGS_MUTED)),
        ]);
    };
    let Some(kind) = pane.kind else {
        return Line::from(vec![
            Span::raw("  "),
            Span::styled(
                truncate_text(
                    &format!(
                        "pane {} ({}) | managed auth only applies to Claude and Codex panes",
                        pane.index + 1,
                        pane.label
                    ),
                    width.saturating_sub(2) as usize,
                ),
                Style::default().fg(SETTINGS_MUTED),
            ),
        ]);
    };

    let current = pane.current_profile.as_deref().unwrap_or("normal login");
    let action = app
        .auth_profiles()
        .get(app.auth_cursor())
        .map(|profile| {
            if profile.kind != kind {
                format!("select a {} profile", kind.display_name())
            } else if pane.current_profile.as_deref() == Some(profile.name.as_str()) {
                "highlighted profile is current".into()
            } else {
                format!("Enter uses {} + restarts", profile.name)
            }
        })
        .unwrap_or_else(|| "create or select a profile below".into());
    let summary = format!(
        "pane {} ({}) | {} | current: {} | {}",
        pane.index + 1,
        pane.label,
        kind.display_name(),
        current,
        action
    );
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            truncate_text(&summary, width.saturating_sub(2) as usize),
            Style::default()
                .fg(kind_color(kind))
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn auth_new_pane_policy_line(app: &App, width: u16) -> Line<'static> {
    let (mode, detail) = if app.auth_auto_cycle() {
        (
            "[ round-robin ]",
            "rotate through every ready profile of the matching agent kind".to_string(),
        )
    } else {
        let claude = app
            .auth_default(AgentKind::Claude)
            .unwrap_or("normal login");
        let codex = app.auth_default(AgentKind::Codex).unwrap_or("normal login");
        (
            "[ per-agent defaults ]",
            format!("Claude: {claude} | Codex: {codex}"),
        )
    };
    let summary = truncate_text(
        &format!("{mode}  {detail}"),
        width.saturating_sub(2) as usize,
    );
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            summary,
            Style::default()
                .fg(if app.auth_auto_cycle() {
                    Color::Black
                } else {
                    Color::LightCyan
                })
                .bg(if app.auth_auto_cycle() {
                    SETTINGS_BORDER
                } else {
                    SETTINGS_SURFACE
                })
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn manager_settings_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let rows = app.manager_settings_rows();
    let mut lines = vec![
        settings_tabs(SettingsTab::Manager),
        Line::from(""),
        settings_section(
            "GRID MANAGER API",
            "powers grid goals and optional AI activity summaries",
            width,
        ),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "AI summaries are opt-in. When enabled, bounded active-tab output is sent to this endpoint. The key stays in your local config.",
                Style::default().fg(SETTINGS_MUTED),
            ),
        ]),
        Line::from(""),
    ];
    for row in &rows {
        lines.push(settings_row(row, width));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        command_key("Up/Down"),
        Span::styled(" move  ", Style::default().fg(Color::Gray)),
        command_key("Enter"),
        Span::styled(" toggle/edit/save  ", Style::default().fg(Color::Gray)),
        command_key("Esc"),
        Span::styled(" cancel/close", Style::default().fg(Color::Gray)),
    ]));
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

fn auth_command_bar(width: u16) -> Vec<Line<'static>> {
    if width < 58 {
        return vec![
            Line::from(vec![
                Span::raw("  "),
                command_key("Up/Down"),
                Span::styled(" move  ", Style::default().fg(Color::Gray)),
                command_key("Enter"),
                Span::styled(" assign", Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::raw("  "),
                command_key("d"),
                Span::styled(" default  ", Style::default().fg(Color::Gray)),
                command_key("c"),
                Span::styled(" policy  ", Style::default().fg(Color::Gray)),
                command_key("Esc"),
                Span::styled(" close", Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::raw("  "),
                command_key("n"),
                Span::styled(" new  ", Style::default().fg(Color::Gray)),
                command_key("l"),
                Span::styled(" login  ", Style::default().fg(Color::Gray)),
                command_key("r"),
                Span::styled(" refresh", Style::default().fg(Color::Gray)),
            ]),
        ];
    }

    vec![
        Line::from(vec![
            Span::raw("  "),
            command_key("Up/Down"),
            Span::styled(" move  ", Style::default().fg(Color::Gray)),
            command_key("Enter"),
            Span::styled(" assign  ", Style::default().fg(Color::Gray)),
            command_key("Esc"),
            Span::styled(" close", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            command_key("d"),
            Span::styled(" default  ", Style::default().fg(Color::Gray)),
            command_key("c"),
            Span::styled(" policy  ", Style::default().fg(Color::Gray)),
            command_key("n"),
            Span::styled(" new  ", Style::default().fg(Color::Gray)),
            command_key("l"),
            Span::styled(" login  ", Style::default().fg(Color::Gray)),
            command_key("r"),
            Span::styled(" refresh", Style::default().fg(Color::Gray)),
        ]),
    ]
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

fn settings_section(title: &str, helper: &str, width: u16) -> Line<'static> {
    let used = 2 + title.len() + 2;
    let helper = width
        .checked_sub(used as u16)
        .filter(|available| *available >= 10)
        .map(|available| truncate_text(helper, available as usize));
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            title.to_string(),
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

fn render_help(frame: &mut Frame<'_>, area: Rect, app: &App, palette: &GridPalette) {
    let controls = app.shortcut_help_entries();
    let modal = help_modal_rect(area);
    frame.render_widget(Clear, modal);
    let inner_width = modal.width.saturating_sub(4) as usize;
    let mut lines = vec![
        Line::from(Span::styled(
            "MODELLESS CONTROLS",
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Input stays in the terminal unless a GridBash shortcut is pressed."),
        Line::from(""),
    ];

    if inner_width >= 62 {
        let rows = controls.len().div_ceil(2);
        let column_width = inner_width.saturating_sub(3) / 2;
        for (row, control) in controls.iter().take(rows).enumerate() {
            let left = help_control(&control.0, control.1, column_width);
            let right = controls
                .get(row + rows)
                .map(|control| help_control(&control.0, control.1, column_width))
                .unwrap_or_default();
            lines.push(Line::from(format!("{left:<column_width$}   {right}")));
        }
    } else {
        let available = modal.height.saturating_sub(7) as usize;
        for control in controls.iter().take(available) {
            lines.push(Line::from(help_control(&control.0, control.1, inner_width)));
        }
        if available < controls.len() {
            lines.push(Line::from(
                "More controls: enlarge the terminal or see README.md",
            ));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent()))
        .title(" GridBash Help ")
        .title_bottom(" Esc, Enter, q, Alt+h, or F1 closes ");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().fg(SETTINGS_TEXT).bg(SETTINGS_BG)),
        modal,
    );
}

fn render_command_palette(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &CommandPaletteView,
    palette: &GridPalette,
) {
    let width = area.width.saturating_sub(2).min(86).max(area.width.min(1));
    let desired_height = (view.items.len() as u16).saturating_add(4).clamp(6, 18);
    let height = desired_height
        .min(area.height.saturating_sub(2))
        .max(area.height.min(1));
    let modal = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 3,
        width,
        height,
    };
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent()))
        .title(" GridBash Commands ")
        .title_bottom(" Enter runs | Up/Down selects | Esc closes ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let input_width = inner.width.saturating_sub(3) as usize;
    let (query, cursor_offset) = visible_input(&view.query, view.cursor_chars, input_width);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query, Style::default().fg(SETTINGS_TEXT)),
    ])];

    let item_count = inner.height.saturating_sub(1) as usize;
    if view.items.is_empty() && item_count > 0 {
        lines.push(Line::from(Span::styled(
            "  No matching commands",
            Style::default().fg(SETTINGS_MUTED),
        )));
    } else {
        let start = view.selected.saturating_sub(item_count.saturating_sub(1));
        for (index, item) in view.items.iter().enumerate().skip(start).take(item_count) {
            let marker = if index == view.selected { ">" } else { " " };
            let shortcut_width = item.shortcut.chars().count().min(18);
            let label_width = inner
                .width
                .saturating_sub(shortcut_width as u16)
                .saturating_sub(4) as usize;
            let label = truncate_text(item.label, label_width);
            let padding = " ".repeat(label_width.saturating_sub(label.chars().count()));
            let style = if index == view.selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(palette.focus())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(SETTINGS_TEXT).bg(SETTINGS_BG)
            };
            lines.push(
                Line::from(format!(
                    "{marker} {label}{padding}  {}",
                    truncate_text(&item.shortcut, shortcut_width)
                ))
                .style(style),
            );
        }
    }

    frame.render_widget(Paragraph::new(lines).style(settings_panel_style()), inner);
    let cursor_x = inner
        .x
        .saturating_add(2)
        .saturating_add(cursor_offset.min(input_width) as u16)
        .min(inner.x.saturating_add(inner.width.saturating_sub(1)));
    frame.set_cursor_position((cursor_x, inner.y));
}

fn help_control(key: &str, action: &str, width: usize) -> String {
    truncate_text(&format!("{key:<13} {action}"), width)
}

fn help_modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(2).min(92).max(area.width.min(1));
    let height = area
        .height
        .saturating_sub(2)
        .min(22)
        .max(area.height.min(1));
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
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

fn close_grid_confirmation_modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(66).max(area.width.min(1));
    let height = area
        .height
        .saturating_sub(2)
        .min(11)
        .max(area.height.min(1));

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

fn quit_confirmation_modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(78).max(area.width.min(1));
    let height = area
        .height
        .saturating_sub(2)
        .min(10)
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

fn render_empty_parked(frame: &mut Frame<'_>, area: Rect, hint: &str) {
    if area.height == 0 {
        return;
    }
    let middle = Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(hint.to_string())
            .style(Style::default().fg(TEXT_DIM))
            .alignment(Alignment::Center),
        middle,
    );
}

fn render_sleeping_screen(frame: &mut Frame<'_>, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // A shade below the app background rather than pure black. A sleeping pane
    // should read as dormant, not as a hole cut in the grid — and on a terminal
    // whose own background is not black, a black rectangle is exactly that.
    let style = Style::default().fg(INK).bg(INK);
    let buffer = frame.buffer_mut();
    // Indexing a `Buffer` outside its area panics, so clip first and still take
    // the `Option` path: a pane rect can outlive the frame it was measured in.
    let area = area.intersection(buffer.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.reset();
                cell.set_style(style);
            }
        }
    }
}

pub fn render_cached_screen(
    frame: &mut Frame<'_>,
    area: Rect,
    cache: &mut PaneRenderCache,
    revision: u64,
    screen: &vt100::Screen,
    selection: Option<PaneSelection>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    refresh_screen_cache(cache, revision, screen, area.width, area.height, selection);
    blit_buffer(&cache.buffer, frame.buffer_mut(), area);
}

fn render_copy_mode(frame: &mut Frame<'_>, area: Rect, view: &CopyModeView, palette: &GridPalette) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let footer_height = u16::from(area.height > 1);
    let body = Rect {
        height: area.height.saturating_sub(footer_height),
        ..area
    };
    let footer = Rect {
        y: area.y.saturating_add(body.height),
        height: footer_height,
        ..area
    };
    let lines = view
        .rows
        .iter()
        .map(|row| {
            // Coalesce runs of equally styled characters. Rendering is
            // unchanged, but a full-width row costs a handful of spans instead
            // of one heap-allocated `Span` per character, every frame.
            let mut spans = Vec::new();
            let mut current_style: Option<Style> = None;
            let mut current_text = String::new();
            for (offset, ch) in row.text.chars().enumerate() {
                let point = TextPoint {
                    line: row.line,
                    column: view.left_column + offset,
                };
                let style = match view.cell_kind(point) {
                    CopyCellKind::Normal => Style::default().fg(TEXT).bg(APP_BG),
                    // Every match is highlighted, so the one the cursor is on
                    // has to be the brighter of the two or "next match" gives no
                    // feedback.
                    CopyCellKind::Match => Style::default()
                        .fg(TEXT)
                        .bg(SURFACE_HI)
                        .add_modifier(Modifier::BOLD),
                    CopyCellKind::ActiveMatch => Style::default()
                        .fg(INK)
                        .bg(WAITING)
                        .add_modifier(Modifier::BOLD),
                    CopyCellKind::Selection => Style::default()
                        .fg(SELECTION_FG)
                        .bg(SELECTION_BG)
                        .add_modifier(Modifier::BOLD),
                    CopyCellKind::Cursor => Style::default()
                        .fg(INK)
                        .bg(palette.focus())
                        .add_modifier(Modifier::BOLD),
                };
                if current_style.is_some_and(|active| active == style) {
                    current_text.push(ch);
                    continue;
                }
                flush_span(&mut spans, &mut current_style, &mut current_text);
                current_style = Some(style);
                current_text.push(ch);
            }
            flush_span(&mut spans, &mut current_style, &mut current_text);
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(TEXT).bg(APP_BG)),
        body,
    );

    if footer.height > 0 {
        let search = if view.searching {
            format!(
                " /{}▌ {}/{}",
                view.query,
                view.active_match.map_or(0, |i| i + 1),
                view.match_count
            )
        } else if view.query.is_empty() {
            String::new()
        } else {
            format!(
                " /{} {}/{}",
                view.query,
                view.active_match.map_or(0, |i| i + 1),
                view.match_count
            )
        };
        let selection = view
            .selection_label
            .map(|kind| format!(" {kind}"))
            .unwrap_or_default();
        let label = format!(
            " COPY {}:{}/{}{}{} | / search n/N next Space/V select y copy Esc close ",
            view.pane + 1,
            view.cursor.line + 1,
            view.total_lines,
            search,
            selection
        );
        frame.render_widget(
            Paragraph::new(truncate_text(&label, footer.width as usize)).style(
                Style::default()
                    .fg(INK)
                    .bg(palette.focus())
                    .add_modifier(Modifier::BOLD),
            ),
            footer,
        );
    }
}

fn refresh_screen_cache(
    cache: &mut PaneRenderCache,
    revision: u64,
    screen: &vt100::Screen,
    width: u16,
    height: u16,
    selection: Option<PaneSelection>,
) {
    if cache.revision == revision
        && cache.width == width
        && cache.height == height
        && cache.selection == selection
    {
        return;
    }

    let area = Rect::new(0, 0, width, height);
    // Reuse the cached allocation instead of building a fresh pane-sized buffer
    // on every screen revision, which for an active pane is once per frame.
    if cache.buffer.area != area {
        cache.buffer.resize(area);
    }
    rasterize_screen(&mut cache.buffer, screen, selection);
    cache.revision = revision;
    cache.width = width;
    cache.height = height;
    cache.selection = selection;
}

/// Copy a cached pane buffer into the frame at `area`.
///
/// Rendering must never take GridBash down. A pane rect can outlive the frame it
/// was measured against — the terminal can shrink between layout and draw — and
/// both `Buffer::index_of` and slice indexing panic outright on an out-of-range
/// position. Because those checks were only `debug_assert!`s, a release build
/// aborted the whole process. Clamping to what the two buffers actually cover
/// makes a stale rect drop cells for one frame instead.
fn blit_buffer(source: &Buffer, target: &mut Buffer, area: Rect) {
    debug_assert_eq!(source.area.width, area.width);
    debug_assert_eq!(source.area.height, area.height);

    let area = area.intersection(target.area);
    let source_stride = source.area.width as usize;
    let width = (area.width as usize).min(source_stride);
    if width == 0 {
        return;
    }

    // Index arithmetic mirrors Buffer::index_of without its out-of-range panic.
    let target_stride = target.area.width as usize;
    let target_column = area.x.saturating_sub(target.area.x) as usize;
    for row in 0..area.height.min(source.area.height) {
        let source_start = row as usize * source_stride;
        let target_start =
            (area.y.saturating_sub(target.area.y) + row) as usize * target_stride + target_column;
        let Some(source_row) = source.content.get(source_start..source_start + width) else {
            break;
        };
        let Some(target_row) = target.content.get_mut(target_start..target_start + width) else {
            break;
        };
        target_row.clone_from_slice(source_row);
    }
}

/// Writes a terminal screen straight into buffer cells.
///
/// Terminal cells already *are* cells. The path this replaced turned every row
/// into a `Line` of `Span`s — a heap `String` per style run and a `Vec` per row —
/// then handed the pane to `Paragraph`, which re-segmented each row into
/// graphemes and re-measured their widths to lay out text that was already laid
/// out on a grid. For a pane producing output that ran once per frame. Copying
/// cell to cell skips the whole text layer and allocates nothing.
///
/// Every cell in `buffer` is written, so the caller does not clear it first. That
/// matters: keeping the previous frame's cells lets the symbol comparison below
/// skip the write for the cells that did not change, which in a terminal is most
/// of them.
fn rasterize_screen(buffer: &mut Buffer, screen: &vt100::Screen, selection: Option<PaneSelection>) {
    let width = buffer.area.width;
    let height = buffer.area.height;
    let stride = width as usize;
    let mut colors = ColorMemo::default();

    for row in 0..height {
        let base = row as usize * stride;
        let mut column = 0;
        while column < width {
            let source = screen.cell(row, column);
            // vt100 stores a wide character as a cell holding both columns'
            // worth of content plus a contentless continuation cell. Ratatui
            // wants the symbol on the lead cell and a blank behind it, so the
            // pair is emitted together and the continuation column skipped.
            let wide = source.is_some_and(|cell| cell.is_wide()) && column + 1 < width;
            let (symbol, style) = match source {
                // A continuation cell reached on its own means its lead was
                // clipped away; it has no content of its own to show.
                Some(cell) if cell.is_wide_continuation() => (" ", colors.style_for(cell)),
                Some(cell) => {
                    let symbol = match cell.has_contents() {
                        // A wide glyph in the last column has nowhere to put its
                        // second half. Emitting it would let the terminal spill
                        // it over whatever the blit puts to the right.
                        true if cell.is_wide() && !wide => " ",
                        true => cell.contents(),
                        false => " ",
                    };
                    (symbol, colors.style_for(cell))
                }
                None => (" ", CellStyle::default()),
            };
            let style = style.with_selection(selection, row, column);

            if let Some(target) = buffer.content.get_mut(base + column as usize) {
                style.apply(target, symbol);
            }
            if wide {
                if let Some(trailing) = buffer.content.get_mut(base + column as usize + 1) {
                    // Ratatui's diff assumes a wide symbol is followed by a blank
                    // and skips that column; anything else would be printed over
                    // the second half of the glyph.
                    style.apply(trailing, " ");
                }
                column += 2;
            } else {
                column += 1;
            }
        }
    }
}

/// A terminal cell's appearance, in the form a buffer cell actually stores.
///
/// Deliberately not a `Style`: a `Style` carries `Option` colours and an
/// add/remove modifier pair, and patching one onto a cell that persists between
/// frames leaves last frame's modifiers set. These three fields are assigned, so
/// dropping bold is as reliable as adding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    fg: Color,
    bg: Color,
    modifier: Modifier,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            fg: PANE_FG,
            bg: PANE_BG,
            modifier: Modifier::empty(),
        }
    }
}

impl CellStyle {
    fn with_selection(self, selection: Option<PaneSelection>, row: u16, column: u16) -> Self {
        if selection.is_some_and(|selection| selection.contains(row, column)) {
            Self {
                fg: SELECTION_FG,
                bg: SELECTION_BG,
                modifier: self.modifier | Modifier::BOLD,
            }
        } else {
            self
        }
    }

    fn apply(self, target: &mut ratatui::buffer::Cell, symbol: &str) {
        // Building a `CompactString` is the most expensive part of writing a
        // cell, and in terminal output the symbol is usually the one already
        // there.
        if target.symbol() != symbol {
            target.set_symbol(symbol);
        }
        target.fg = self.fg;
        target.bg = self.bg;
        target.modifier = self.modifier;
    }
}

/// Remembers the last colour pair translated out of vt100.
///
/// Styled output arrives in runs — a word, usually a whole line, shares one
/// colour pair — so this turns the palette lookup for almost every cell into two
/// comparisons. The keys start at vt100's own default, which is what an untouched
/// screen is full of.
#[derive(Debug, Clone, Copy)]
struct ColorMemo {
    fg_key: vt100::Color,
    bg_key: vt100::Color,
    fg: Color,
    bg: Color,
}

impl Default for ColorMemo {
    fn default() -> Self {
        // There is no "nothing remembered yet" state to guard against: vt100's
        // default colour maps to the pane defaults, so seeding the memo with
        // that pair starts it already correct for an untouched screen.
        Self {
            fg_key: vt100::Color::Default,
            bg_key: vt100::Color::Default,
            fg: PANE_FG,
            bg: PANE_BG,
        }
    }
}

impl ColorMemo {
    fn style_for(&mut self, cell: &Cell) -> CellStyle {
        let fg_key = cell.fgcolor();
        if self.fg_key != fg_key {
            self.fg_key = fg_key;
            self.fg = vt_color(fg_key, PANE_FG);
        }
        let bg_key = cell.bgcolor();
        if self.bg_key != bg_key {
            self.bg_key = bg_key;
            self.bg = vt_color(bg_key, PANE_BG);
        }

        let mut modifier = Modifier::empty();
        if cell.bold() {
            modifier |= Modifier::BOLD;
        }
        if cell.dim() {
            modifier |= Modifier::DIM;
        }
        if cell.italic() {
            modifier |= Modifier::ITALIC;
        }
        if cell.underline() {
            modifier |= Modifier::UNDERLINED;
        }
        if cell.inverse() {
            modifier |= Modifier::REVERSED;
        }

        CellStyle {
            fg: self.fg,
            bg: self.bg,
            modifier,
        }
    }
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

/// Rounds the four corners of the whole grid. Corners where panes meet stay
/// square, because border merging only knows how to join square corners.
const SHELF_ROW_HEIGHT: u16 = 4;
const SHELF_FOOTER_HEIGHT: u16 = 5;

fn render_shelf(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &ShelfView,
    palette: &GridPalette,
) -> Vec<(usize, Rect)> {
    if area.width < 8 || area.height < 4 {
        return Vec::new();
    }
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(LINE))
        .style(Style::default().bg(APP_BG));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Parked",
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {}", view.rows.len()),
                Style::default().fg(TEXT_FAINT),
            ),
        ])),
        Rect { height: 1, ..inner },
    );

    let footer_height = SHELF_FOOTER_HEIGHT.min(inner.height.saturating_sub(2));
    let list = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2 + footer_height),
        ..inner
    };
    let visible = usize::from(list.height / SHELF_ROW_HEIGHT).max(1);
    let focused = view.rows.iter().position(|row| row.focused).unwrap_or(0);
    let first = focused.saturating_sub(visible - 1);
    let width = usize::from(list.width.saturating_sub(3));

    let mut rows = Vec::new();
    for (offset, row) in view.rows.iter().enumerate().skip(first).take(visible) {
        let y = list.y + u16::try_from(offset - first).unwrap_or(0) * SHELF_ROW_HEIGHT;
        let rect = Rect::new(
            list.x,
            y,
            list.width,
            SHELF_ROW_HEIGHT.min(list.bottom() - y),
        );
        let background = if row.focused { SURFACE_HI } else { APP_BG };
        frame.render_widget(
            Block::default().style(Style::default().bg(background)),
            Rect {
                height: rect.height.saturating_sub(1),
                ..rect
            },
        );
        if row.focused {
            for line in 0..rect.height.saturating_sub(1) {
                frame.buffer_mut()[(rect.x, rect.y + line)]
                    .set_symbol("▌")
                    .set_fg(palette.accent())
                    .set_bg(background);
            }
        }
        let (mark, state) = match row.state {
            ShelfState::Done => ("✓", "DONE"),
            ShelfState::Checking => ("◐", "checking"),
            ShelfState::Watching => ("○", "watching"),
            ShelfState::Parked => ("·", ""),
        };
        let done = row.state == ShelfState::Done;
        let head = Line::from(vec![
            Span::styled(
                format!("{mark} "),
                Style::default().fg(if done { TEXT } else { TEXT_FAINT }),
            ),
            Span::styled(
                truncate_text(&row.name, width.saturating_sub(state.len() + 4)),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_text(
                    &format!(" from {}", row.from),
                    width.saturating_sub(row.name.chars().count() + state.len() + 4),
                ),
                Style::default().fg(TEXT_FAINT),
            ),
        ]);
        let text_area = Rect::new(rect.x + 2, rect.y, rect.width.saturating_sub(3), 1);
        frame.render_widget(Paragraph::new(head), text_area);
        if !state.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    state,
                    Style::default()
                        .fg(if done { TEXT } else { TEXT_FAINT })
                        .add_modifier(if done {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ))
                .alignment(Alignment::Right),
                text_area,
            );
        }
        let condition = row.condition.as_ref().map_or_else(
            || "no condition".to_string(),
            |condition| format!("when {condition}"),
        );
        let lines = [
            (condition, TEXT_DIM),
            (row.detail.clone(), if done { TEXT } else { TEXT_FAINT }),
        ];
        for (line, (text, color)) in lines.into_iter().enumerate() {
            let y = rect.y + 1 + u16::try_from(line).unwrap_or(0);
            if y >= rect.bottom() || y >= list.bottom() {
                break;
            }
            frame.render_widget(
                Paragraph::new(Span::styled(
                    truncate_text(&text, width.saturating_sub(2)),
                    Style::default().fg(color),
                )),
                Rect::new(rect.x + 4, y, rect.width.saturating_sub(5), 1),
            );
        }
        rows.push((offset, rect));
    }

    if footer_height > 0 {
        let footer = Rect {
            y: inner.bottom() - footer_height,
            height: footer_height,
            ..inner
        };
        let lines = vec![
            Line::from(Span::styled(
                " Watcher",
                Style::default().fg(TEXT_DIM).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                " Claude reads a pane each time it goes still",
                Style::default().fg(TEXT_FAINT),
            )),
            Line::from(Span::styled(
                format!(
                    " {} checks · {:.1}k tokens since GridBash started",
                    view.checks,
                    view.tokens as f64 / 1000.0
                ),
                Style::default().fg(TEXT_FAINT),
            )),
            Line::from(Span::styled(
                match &view.watch_key {
                    Some(watch_key) => format!(
                        " {} back to its grid · {watch_key} new goal",
                        view.return_key
                    ),
                    None => format!(" {} back to its grid", view.return_key),
                },
                Style::default().fg(TEXT_FAINT),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), footer);
    }
    rows
}

fn render_shelf_banner(frame: &mut Frame<'_>, area: Rect, condition: &str, reason: &str) {
    let width = usize::from(area.width.saturating_sub(4));
    frame.render_widget(Block::default().style(Style::default().bg(SURFACE)), area);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " ✓ Done  ",
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_text(condition, width.saturating_sub(9)),
                Style::default().fg(TEXT_DIM),
            ),
        ]),
        Line::from(Span::styled(
            format!(" {}", truncate_text(reason, width)),
            Style::default().fg(TEXT),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), Rect { height: 2, ..area });
}

const PARK_SHEET_HEIGHT: u16 = 7;

fn render_park_sheet(
    frame: &mut Frame<'_>,
    pane: Rect,
    view: &ParkSheetView,
    palette: &GridPalette,
) {
    let height = PARK_SHEET_HEIGHT.min(pane.height.saturating_sub(2));
    if height < 3 || pane.width < 12 {
        return;
    }
    let area = Rect::new(
        pane.x + 1,
        pane.bottom() - 1 - height,
        pane.width - 2,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(palette.accent()))
        .title(Span::styled(
            if view.parking {
                " Park · notify me when "
            } else {
                " Notify me when "
            },
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(SURFACE));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field = Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1);
    let room = usize::from(field.width.saturating_sub(3));
    let skip = view.cursor.saturating_sub(room);
    let shown = view.value.chars().skip(skip).take(room).collect::<String>();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("❯ ", Style::default().fg(TEXT_DIM)),
            if view.value.is_empty() {
                Span::styled(
                    "in your words, e.g. the PR is merged and CI is green",
                    Style::default().fg(TEXT_FAINT),
                )
            } else {
                Span::styled(shown, Style::default().fg(TEXT))
            },
        ]))
        .style(Style::default().bg(SURFACE_HI)),
        field,
    );
    let cursor_x = field.x + 2 + u16::try_from(view.cursor - skip).unwrap_or(0);
    frame.set_cursor_position((cursor_x.min(field.right().saturating_sub(1)), field.y));

    let empty = if view.parking {
        "Leave it empty to just park it until you bring it back."
    } else {
        "Leave it empty to stop watching it."
    };
    let lines = vec![
        Line::from(Span::styled(
            "Claude reads the pane each time it goes still: one Haiku call on your login.",
            Style::default().fg(TEXT_FAINT),
        )),
        Line::from(Span::styled(empty, Style::default().fg(TEXT_FAINT))),
        Line::from(Span::styled(
            if view.parking {
                "Enter park   Esc cancel"
            } else {
                "Enter save   Esc cancel"
            },
            Style::default().fg(TEXT_DIM),
        )),
    ];
    let text = Rect::new(
        inner.x + 1,
        field.y + 2,
        inner.width.saturating_sub(2),
        inner.bottom().saturating_sub(field.y + 2),
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text);
}

fn render_park_toast(frame: &mut Frame<'_>, grid: Rect, view: &ParkToastView) -> Option<Rect> {
    let head = format!("✓ {} is done", view.name);
    let tail = format!("{} view", view.key);
    let fixed = head.chars().count() + tail.chars().count() + 10;
    let max_width = usize::from(grid.width.saturating_sub(4));
    if max_width < fixed + 8 || grid.height < 6 {
        return None;
    }
    let reason = truncate_text(&view.reason, (max_width - fixed).min(70));
    let width = u16::try_from(fixed + reason.chars().count()).ok()?;
    let area = Rect::new(grid.right() - width - 2, grid.bottom() - 4, width, 3);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .style(Style::default().bg(SURFACE));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {head}   "),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{reason}   "), Style::default().fg(TEXT_DIM)),
            Span::styled(tail, Style::default().fg(TEXT_FAINT)),
        ])),
        inner,
    );
    Some(area)
}

fn round_outer_corners(buffer: &mut Buffer, bounds: Rect) {
    if bounds.width < 2 || bounds.height < 2 {
        return;
    }
    let (left, top) = (bounds.left(), bounds.top());
    let (right, bottom) = (bounds.right() - 1, bounds.bottom() - 1);
    for (x, y, square, round) in [
        (left, top, "┌", "╭"),
        (right, top, "┐", "╮"),
        (left, bottom, "└", "╰"),
        (right, bottom, "┘", "╯"),
    ] {
        if let Some(cell) = buffer.cell_mut((x, y))
            && cell.symbol() == square
        {
            cell.set_symbol(round);
        }
    }
}

/// How much of a pane's own colour survives when it is not focused, so the
/// pane taking keystrokes is the brightest thing on screen.
const UNFOCUSED_TEXT_STRENGTH: f32 = 0.68;

pub(crate) fn fade_toward_background(buffer: &mut Buffer, area: Rect) {
    let area = area.intersection(buffer.area);
    let Some((bg_red, bg_green, bg_blue)) = color_rgb(PANE_BG) else {
        return;
    };
    let blend = |value: u8, background: u8| -> u8 {
        (f32::from(value) * UNFOCUSED_TEXT_STRENGTH
            + f32::from(background) * (1.0 - UNFOCUSED_TEXT_STRENGTH)) as u8
    };
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buffer[(x, y)];
            let fg = if cell.fg == Color::Reset {
                PANE_FG
            } else {
                cell.fg
            };
            if let Some((red, green, blue)) = color_rgb(fg) {
                cell.fg = Color::Rgb(
                    blend(red, bg_red),
                    blend(green, bg_green),
                    blend(blue, bg_blue),
                );
            }
        }
    }
}

/// The xterm default for the sixteen named colours.
/// A terminal profile can redefine the first sixteen; that only shifts how a
/// faded colour looks, not whether it reads as faded.
fn color_rgb(color: Color) -> Option<(u8, u8, u8)> {
    const NAMED: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    Some(match color {
        Color::Rgb(red, green, blue) => (red, green, blue),
        Color::Indexed(index) => return color_rgb(indexed_color(index)),
        Color::Black => NAMED[0],
        Color::Red => NAMED[1],
        Color::Green => NAMED[2],
        Color::Yellow => NAMED[3],
        Color::Blue => NAMED[4],
        Color::Magenta => NAMED[5],
        Color::Cyan => NAMED[6],
        Color::Gray => NAMED[7],
        Color::DarkGray => NAMED[8],
        Color::LightRed => NAMED[9],
        Color::LightGreen => NAMED[10],
        Color::LightYellow => NAMED[11],
        Color::LightBlue => NAMED[12],
        Color::LightMagenta => NAMED[13],
        Color::LightCyan => NAMED[14],
        Color::White => NAMED[15],
        Color::Reset => return None,
    })
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
    use crate::app::AdoptTerminalRow;
    use crate::{
        cli::Cli,
        config::Config,
        layout::{GridLayout, GridSize},
    };
    use clap::Parser;
    use ratatui::{Terminal, backend::TestBackend};

    /// One pane in a rendered preview of the shell.
    struct PreviewPane {
        frame: PaneFrame,
        body: &'static str,
    }

    fn preview_panes() -> Vec<PreviewPane> {
        let pane =
            |number: usize, label: &str, summary: &str, state: PaneState, body: &'static str| {
                PreviewPane {
                    frame: PaneFrame {
                        number,
                        label: label.into(),
                        summary: summary.into(),
                        usage: Some("5h 80% left".into()),
                        state,
                        focused: number == 1,
                        selected: number == 4,
                        logging: number == 6,
                        compact: false,
                    },
                    body,
                }
            };

        vec![
            pane(
                1,
                "api",
                "editing src/routes.rs",
                PaneState::Live,
                "$ cargo test",
            ),
            pane(
                2,
                "web",
                "waiting for review",
                PaneState::Waiting,
                "Continue? [y/N]",
            ),
            pane(3, "docs", "wrote the changelog", PaneState::Idle, "$ "),
            pane(
                4,
                "infra",
                "terraform plan clean",
                PaneState::Live,
                "$ tf apply",
            ),
            pane(
                5,
                "tests",
                "3 failures remain",
                PaneState::Waiting,
                "FAILED 3",
            ),
            pane(
                6,
                "bench",
                "recording a trace",
                PaneState::Live,
                "sampling...",
            ),
            pane(7, "spike", "paused by the user", PaneState::Sleeping, ""),
            pane(8, "old", "process exited", PaneState::Exited, "exit 130"),
            pane(9, "shell", "", PaneState::Live, "$ git status"),
        ]
    }

    /// Renders the main shell, tab strip and pane lattice, to text.
    ///
    /// The grid is the one part of GridBash that cannot be judged from its
    /// source: whether nine panes read as a workspace or as noise is a question
    /// about glyphs on a screen. This draws the real chrome functions so that
    /// question can be answered without a terminal full of live PTYs behind it.
    fn preview_shell(width: u16, height: u16, grid: GridSize) -> String {
        let palette = GridPalette::default();
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.render_widget(Paragraph::new("").style(Style::default().bg(INK)), area);

                let tabs = ["Frontend", "Backend", "Review"]
                    .iter()
                    .enumerate()
                    .map(|(index, title)| TabLabel {
                        title: (*title).into(),
                        active: index == 0,
                        selected: index == 1,
                        exited: false,
                        pinned: false,
                        done: false,
                    })
                    .collect::<Vec<_>>();
                render_tabs(frame, Rect::new(0, 0, width, 1), &tabs, &palette, &[], None);

                let grid_area = Rect::new(0, 1, width, height.saturating_sub(1));
                let panes = preview_panes();
                let rects = GridLayout::new(grid).rects(grid_area, panes.len());
                let mut order = (0..panes.len()).collect::<Vec<_>>();
                order.sort_by_key(|index| {
                    let view = &panes[*index].frame;
                    u8::from(view.selected) + 2 * u8::from(view.focused)
                });
                for index in order {
                    let Some(rect) = rects.get(index).copied() else {
                        continue;
                    };
                    let pane = &panes[index];
                    let inner = render_pane_frame(frame, rect, &pane.frame, &palette);
                    if inner.width > 0 && inner.height > 0 {
                        frame.render_widget(
                            Paragraph::new(pane.body).style(Style::default().fg(TEXT).bg(APP_BG)),
                            inner,
                        );
                    }
                }
            })
            .expect("render preview");

        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Prints the shell so the layout can be looked at. Run with:
    /// `cargo test -- --ignored --nocapture main_tui_preview`
    #[test]
    #[ignore = "visual preview"]
    fn main_tui_preview() {
        for (width, height, rows, columns) in [(120, 34, 3, 3), (100, 28, 2, 2), (72, 20, 2, 2)] {
            let grid = GridSize::new(rows, columns).expect("grid");
            eprintln!("\n===== {width}x{height} · {rows}x{columns} =====");
            eprintln!("{}", preview_shell(width, height, grid));
        }
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The event loop catches a panic thrown inside `Terminal::draw`, which
    /// abandons the frame half-written and never swaps the buffers. Resetting the
    /// current buffer and clearing is what makes the next frame trustworthy.
    #[test]
    fn a_frame_abandoned_by_a_panic_redraws_cleanly() {
        let backend = TestBackend::new(6, 1);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("aaaaaa"), frame.area());
            })
            .expect("first frame");
        assert_eq!(buffer_text(&terminal), "aaaaaa");

        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = terminal.draw(|frame| {
                frame.render_widget(Paragraph::new("bbbbbb"), frame.area());
                panic!("half-drawn frame");
            });
        }));
        std::panic::set_hook(previous_hook);
        assert!(panicked.is_err(), "the draw must have unwound");

        // What the event loop does on recovery.
        terminal.current_buffer_mut().reset();
        terminal.clear().expect("clear after a caught panic");

        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("cc"), frame.area());
            })
            .expect("frame after recovery");
        assert_eq!(
            buffer_text(&terminal),
            "cc    ",
            "the abandoned frame must not bleed into the next one"
        );
    }

    /// Pane rects are stored between frames, so a rect measured before a resize
    /// can name cells the current frame does not have. Indexing a `Buffer`
    /// outside its area panics, and this runs on every sleeping pane.
    #[test]
    fn the_sleeping_screen_clips_rects_that_outgrew_the_frame() {
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        terminal
            .draw(|frame| {
                render_sleeping_screen(frame, Rect::new(0, 0, 200, 200));
                render_sleeping_screen(frame, Rect::new(8, 3, 40, 40));
                render_sleeping_screen(frame, Rect::new(50, 50, 4, 4));
                render_sleeping_screen(frame, Rect::new(0, 0, 0, 0));
                render_sleeping_screen(frame, Rect::new(u16::MAX, u16::MAX, u16::MAX, u16::MAX));
            })
            .expect("drawing a sleeping pane must not panic");

        assert_eq!(buffer_text(&terminal).chars().count(), 40);
    }

    #[test]
    fn tab_rendering_returns_click_targets_and_marks_selected_grids() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let tabs = vec![
            TabLabel {
                title: "Frontend".into(),
                active: true,
                selected: false,
                exited: false,
                pinned: false,
                done: false,
            },
            TabLabel {
                title: "Tests".into(),
                active: false,
                selected: true,
                exited: false,
                pinned: false,
                done: false,
            },
            TabLabel {
                title: "Review".into(),
                active: false,
                selected: false,
                exited: false,
                pinned: false,
                done: false,
            },
        ];
        let mut targets = Vec::new();

        terminal
            .draw(|frame| {
                targets = render_tabs(
                    frame,
                    frame.area(),
                    &tabs,
                    &GridPalette::default(),
                    &[],
                    None,
                );
            })
            .expect("render tabs");

        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].0, 0);
        assert_eq!(targets[1].0, 1);
        // Tabs are separate shapes with a blank column between them, and the
        // click target covers the whole shape including its caps.
        assert_eq!(targets[0].1.right() + TAB_GAP, targets[1].1.x);
        assert_eq!(targets[1].1.right() + TAB_GAP, targets[2].1.x);

        let rendered = buffer_text(&terminal);
        assert!(rendered.contains(" 1 Frontend "), "tabs: {rendered}");
        assert!(rendered.contains(" 2 Tests "), "tabs: {rendered}");
        assert!(
            rendered.contains(&format!("{TAB_CAP_LEFT} 1 Frontend {TAB_CAP_RIGHT}")),
            "the current tab must be drawn as a capped shape: {rendered}"
        );
        // Only the current grid is coloured; a waiting grid stays grey.
        let cells = terminal.backend().buffer().content();
        assert!(
            cells
                .iter()
                .any(|cell| cell.symbol() == "F" && cell.bg == GridPalette::default().accent())
        );
        assert!(
            !cells
                .iter()
                .any(|cell| cell.symbol() == "R" && cell.bg == GridPalette::default().accent())
        );
    }

    /// Every tab is a filled shape, not just the current one — otherwise the
    /// strip reads as one highlighted word among plain ones.
    #[test]
    fn background_tabs_are_drawn_as_tabs_too() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let tabs = vec![
            TabLabel {
                title: "Frontend".into(),
                active: true,
                selected: false,
                exited: false,
                pinned: false,
                done: false,
            },
            TabLabel {
                title: "Tests".into(),
                active: false,
                selected: false,
                exited: false,
                pinned: false,
                done: false,
            },
        ];

        let mut targets = Vec::new();
        terminal
            .draw(|frame| {
                targets = render_tabs(
                    frame,
                    frame.area(),
                    &tabs,
                    &GridPalette::default(),
                    &[],
                    None,
                );
            })
            .expect("render tabs");

        let buffer = terminal.backend().buffer().clone();
        let backdrop = targets[1].1;
        assert!(
            (backdrop.x + 1..backdrop.right() - 1)
                .all(|x| buffer[(x, 0)].bg == SURFACE_HI && buffer[(x, 0)].bg != SURFACE),
            "an unselected tab must still sit on a raised surface"
        );
        // The gap between two tabs belongs to the strip, not to either tab.
        assert_eq!(buffer[(targets[0].1.right(), 0)].bg, SURFACE);
    }

    /// A strip that simply ran off the right edge hid the active tab as soon as
    /// there were enough grids, and said nothing about it.
    #[test]
    fn the_tab_strip_scrolls_to_keep_the_active_tab_on_screen() {
        let widths = [10u16; 8];

        assert_eq!(visible_tabs(&widths, 0, 200), (0, 8), "everything fits");
        assert_eq!(visible_tabs(&[], 0, 200), (0, 0));

        for active in 0..widths.len() {
            for budget in [12u16, 20, 33, 44, 60] {
                let (first, last) = visible_tabs(&widths, active, budget);
                assert!(
                    first <= active && active < last,
                    "the active tab fell out of the window at {budget} columns"
                );
                let used = (last - first) as u16 * (10 + TAB_GAP) - TAB_GAP;
                assert!(
                    used + 2 * TAB_OVERFLOW_WIDTH <= budget || last - first == 1,
                    "the window outgrew its budget: {used} in {budget}"
                );
            }
        }

        // An active tab wider than the whole strip is still the one that is
        // drawn; the paragraph clips it rather than the strip dropping it.
        assert_eq!(visible_tabs(&[80, 10], 0, 20), (0, 1));
    }

    /// The picker must say what it actually does. Adopting a window moves the
    /// pane to that window's folder; the window keeps its process, because a
    /// live console cannot be handed to another pseudoconsole.
    #[test]
    fn the_adopt_picker_names_the_target_and_asks_before_replacing() {
        let view = AdoptTerminalView {
            cursor: 1,
            target_label: "2 Fluent".into(),
            rows: vec![
                AdoptTerminalRow {
                    label: "MINGW64:~ - npm run dev".into(),
                    pid: 41,
                    adoptable: false,
                },
                AdoptTerminalRow {
                    label: "C:\\repos\\api".into(),
                    pid: 42,
                    adoptable: true,
                },
            ],
            confirming: false,
        };

        let render = |view: &AdoptTerminalView| {
            let mut terminal = Terminal::new(TestBackend::new(90, 20)).expect("test terminal");
            terminal
                .draw(|frame| {
                    render_adopt_terminal(frame, frame.area(), view, &GridPalette::default());
                })
                .expect("render adopt picker");
            buffer_text(&terminal)
        };

        let listing = render(&view);
        assert!(listing.contains("into pane 2 Fluent"), "{listing}");
        assert!(listing.contains("C:\\repos\\api"), "{listing}");
        assert!(listing.contains("pid 42"), "{listing}");
        // A window whose folder could not be read is still listed rather than
        // silently dropped, so its absence is never a mystery.
        assert!(listing.contains("npm run dev"), "{listing}");
        assert!(listing.contains("the window keeps running"), "{listing}");
        assert!(!listing.contains("its shell closes"), "{listing}");

        // Confirming is the second half of "ask first": it says what is lost.
        let confirming = render(&AdoptTerminalView {
            confirming: true,
            ..view
        });
        assert!(confirming.contains("replace this pane?"), "{confirming}");
        assert!(confirming.contains("its shell closes"), "{confirming}");
    }

    #[test]
    fn command_palette_renders_at_narrow_sizes_and_handles_no_matches() {
        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let view = CommandPaletteView {
            query: "missing 東京".into(),
            cursor_chars: 10,
            selected: 0,
            items: Vec::new(),
        };

        terminal
            .draw(|frame| {
                render_command_palette(frame, frame.area(), &view, &GridPalette::default());
            })
            .expect("render palette");

        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("GridBash Commands"));
        assert!(rendered.contains("No matching"));
    }

    #[test]
    fn quit_confirmation_shows_the_exact_resume_command() {
        let backend = TestBackend::new(90, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let view = QuitConfirmationView {
            resume_command: "gridbash resume 1777777777777-42".into(),
            keeps_terminals_running: true,
        };

        terminal
            .draw(|frame| {
                render_quit_confirmation(frame, frame.area(), &view, &GridPalette::default());
            })
            .expect("render quit confirmation");

        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("Quit GridBash?"));
        assert!(rendered.contains("gridbash resume 1777777777777-42"));
        assert!(rendered.contains("Alt+Q confirms"));
        assert!(rendered.contains("Live terminals will stay running"));
    }

    #[test]
    fn close_grid_confirmation_names_the_grid_and_consequences() {
        let backend = TestBackend::new(90, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let view = CloseGridConfirmationView {
            title: "Backend agents".into(),
            pane_count: 3,
        };

        terminal
            .draw(|frame| {
                render_close_grid_confirmation(frame, frame.area(), &view, &GridPalette::default());
            })
            .expect("render close-grid confirmation");

        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("Close current grid?"));
        assert!(rendered.contains("Backend agents"));
        assert!(rendered.contains("terminate 3 panes"));
        assert!(rendered.contains("worktrees and branches will stay on disk"));
        assert!(rendered.contains("Enter / Y closes"));
    }

    #[test]
    fn command_center_is_hidden_and_clamped_to_the_available_height() {
        assert_eq!(command_center_height(24, false, 12), 0);
        assert_eq!(command_center_height(24, true, 12), 12);
        assert_eq!(command_center_height(8, true, 12), 5);
    }

    #[test]
    fn hidden_command_line_renders_safely_at_small_terminal_sizes() {
        let cli = Cli::parse_from(["gridbash"]);
        let app = App::new(cli, Config::default()).expect("app");

        for (width, height) in [(1, 1), (2, 2), (40, 6)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            terminal
                .draw(|frame| {
                    draw(frame, &app);
                })
                .expect("render hidden command line");

            assert!(!buffer_text(&terminal).contains(" > "));
        }
    }

    /// The pane path as it stood before the rasterizer: a `Line` of `Span`s per
    /// row, laid out by `Paragraph`. Kept only so the benchmark below can put a
    /// number on what replacing it bought.
    fn legacy_screen_render(buffer: &mut Buffer, screen: &vt100::Screen, width: u16, height: u16) {
        use ratatui::widgets::Widget;

        fn legacy_cell_style(cell: &Cell) -> Style {
            let mut style = Style::default()
                .fg(vt_color(cell.fgcolor(), PANE_FG))
                .bg(vt_color(cell.bgcolor(), PANE_BG));
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

        let lines = (0..height)
            .map(|row| {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut current_style: Option<Style> = None;
                let mut current_text = String::new();
                for column in 0..width {
                    let (style, text) = match screen.cell(row, column) {
                        Some(cell) if cell.is_wide_continuation() => continue,
                        Some(cell) => (
                            legacy_cell_style(cell),
                            if cell.has_contents() {
                                cell.contents()
                            } else {
                                " "
                            },
                        ),
                        None => (Style::default(), " "),
                    };
                    if current_style.is_some_and(|active| active == style) {
                        current_text.push_str(text);
                        continue;
                    }
                    flush_span(&mut spans, &mut current_style, &mut current_text);
                    current_style = Some(style);
                    current_text.push_str(text);
                }
                flush_span(&mut spans, &mut current_style, &mut current_text);
                Line::from(spans)
            })
            .collect::<Vec<_>>();

        let area = Rect::new(0, 0, width, height);
        if buffer.area != area {
            buffer.resize(area);
        }
        buffer.reset();
        Widget::render(
            Paragraph::new(lines).style(Style::default().fg(PANE_FG).bg(APP_BG)),
            area,
            buffer,
        );
    }

    /// Times the pane hot path against the one it replaced.
    ///
    /// The version of this benchmark that shipped before held `revision` at 1,
    /// so every iteration after the first hit the cache's early return and the
    /// loop timed `blit_buffer` alone — the rasterizer it was named after never
    /// ran. Varying the revision is what makes the number mean anything.
    #[test]
    #[ignore = "manual performance benchmark"]
    fn benchmark_cached_screen_render() {
        use std::{hint::black_box, time::Instant};

        const ITERATIONS: usize = 5_000;
        let mut parser = vt100::Parser::new(40, 120, 10_000);
        let output = (0..40)
            .map(|row| {
                format!(
                    "\x1b[38;5;{}mrow {row:02}: GridBash performance benchmark output with styled terminal cells\x1b[0m\r\n",
                    32 + row
                )
            })
            .collect::<String>();
        parser.process(output.as_bytes());
        let screen = parser.screen();

        let area = Rect::new(0, 0, 120, 40);
        let mut frame_buffer = Buffer::empty(area);

        let mut legacy = Buffer::empty(area);
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            legacy_screen_render(&mut legacy, screen, area.width, area.height);
            black_box(&legacy);
        }
        let legacy_elapsed = start.elapsed() / ITERATIONS as u32;

        let mut cache = PaneRenderCache::default();
        let start = Instant::now();
        for iteration in 0..ITERATIONS {
            // A fresh revision every pass, so the cache never short-circuits and
            // the rasterizer actually runs.
            refresh_screen_cache(
                &mut cache,
                iteration as u64,
                screen,
                area.width,
                area.height,
                None,
            );
            black_box(&cache.buffer);
        }
        let rasterize_elapsed = start.elapsed() / ITERATIONS as u32;

        let start = Instant::now();
        for _ in 0..ITERATIONS {
            blit_buffer(black_box(&cache.buffer), &mut frame_buffer, area);
        }
        let blit_elapsed = start.elapsed() / ITERATIONS as u32;

        eprintln!("120x40 pane, {ITERATIONS} iterations each:");
        eprintln!("  spans + Paragraph (old): {legacy_elapsed:?}");
        eprintln!("  direct rasterize (new):  {rasterize_elapsed:?}");
        eprintln!("  blit to frame:           {blit_elapsed:?}");
        eprintln!(
            "  full frame old/new:      {:?} -> {:?}",
            legacy_elapsed + blit_elapsed,
            rasterize_elapsed + blit_elapsed
        );
        black_box((frame_buffer, legacy));
    }

    #[test]
    fn copy_mode_renders_at_one_cell_without_overflow() {
        let backend = ratatui::backend::TestBackend::new(1, 1);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        let mode = crate::copy_mode::CopyMode::new(0, vec!["\u{6771}".into()], 1, 1);
        let view = mode.view(1, 1);

        terminal
            .draw(|frame| {
                render_copy_mode(frame, Rect::new(0, 0, 1, 1), &view, &GridPalette::default());
            })
            .expect("render copy mode");
    }

    #[test]
    fn workspace_assistant_renders_director_status_and_input() {
        let view = WorkspaceAssistantView {
            grid_title: "Frontend".into(),
            input: "brief me".into(),
            cursor_chars: 8,
            messages: Vec::new(),
            busy: false,
            configured: true,
            pane_count: 8,
            goal: None,
            scroll_from_bottom: 0,
        };
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render_workspace_assistant(frame, frame.area(), &view, &GridPalette::default());
            })
            .expect("render assistant");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("BashBot Director"));
        assert!(rendered.contains("Frontend"));
        assert!(rendered.contains("8 panes"));
        assert!(rendered.contains("you › brief me"));
    }

    #[test]
    fn assistant_transcript_keeps_latest_wrapped_lines() {
        let view = WorkspaceAssistantView {
            grid_title: "Backend".into(),
            input: String::new(),
            cursor_chars: 0,
            messages: vec![
                crate::app::AssistantMessageView {
                    role: AssistantMessageRole::User,
                    text: "brief every grid please".into(),
                },
                crate::app::AssistantMessageView {
                    role: AssistantMessageRole::BashBot,
                    text: "Frontend tests pass and backend is waiting for review.".into(),
                },
            ],
            busy: false,
            configured: true,
            pane_count: 8,
            goal: None,
            scroll_from_bottom: 0,
        };
        let lines = assistant_transcript_lines(&view, 24, 3, 0, &GridPalette::default());
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(lines.len(), 3);
        assert!(text.contains("backend"));
        assert!(text.contains("is waiting"));
        assert!(text.contains("for"));
        assert!(text.contains("review."));
    }

    #[test]
    fn assistant_text_wraps_unicode_without_byte_slicing() {
        assert_eq!(
            wrap_text("東京東京 ready", 3),
            vec!["東京東", "京", "rea", "dy"]
        );
    }

    fn pane_frame(state: PaneState) -> PaneFrame {
        PaneFrame {
            number: 1,
            label: "api".into(),
            summary: "reviewing the latest changes".into(),
            usage: Some("5h 80% left".into()),
            state,
            focused: false,
            selected: false,
            logging: false,
            compact: false,
        }
    }

    fn header_text(view: &PaneFrame, width: u16) -> String {
        let backend = TestBackend::new(width, 3);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render_pane_frame(
                    frame,
                    Rect::new(0, 0, width, 3),
                    view,
                    &GridPalette::default(),
                );
            })
            .expect("render pane frame");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .take(width as usize)
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn a_live_pane_header_shows_its_number_name_and_activity() {
        let header = header_text(&pane_frame(PaneState::Live), 80);

        assert!(header.contains(" 1 api"), "header: {header}");
        assert!(header.contains("reviewing the latest"), "header: {header}");
        // A pane doing its job has no state worth naming.
        for badge in ["idle", "asleep", "exited", "needs you"] {
            assert!(!header.contains(badge), "header: {header}");
        }
    }

    /// An agent that has gone quiet is asking for the user. In a grid full of
    /// agents that is the single most useful thing on screen, so it has to be
    /// stated in words rather than left to a one-character marker.
    #[test]
    fn a_waiting_agent_says_so_in_its_header() {
        let header = header_text(&pane_frame(PaneState::Waiting), 80);

        assert!(header.contains("needs you"), "header: {header}");
    }

    #[test]
    fn every_resting_state_names_itself() {
        for (state, badge) in [
            (PaneState::Idle, "idle"),
            (PaneState::Sleeping, "asleep"),
            (PaneState::Exited, "exited"),
        ] {
            let header = header_text(&pane_frame(state), 80);
            assert!(header.contains(badge), "{state:?} header: {header}");
        }
    }

    /// The name is the only way to tell one pane from another, so it is the last
    /// thing a narrow header gives up — before the summary, and before usage.
    #[test]
    fn a_narrow_header_keeps_the_name_and_drops_the_extras() {
        let header = header_text(&pane_frame(PaneState::Idle), 26);

        assert!(header.contains("api"), "header: {header}");
        assert!(!header.contains("5h 80% left"), "header: {header}");
        assert_eq!(header.chars().count(), 26);
    }

    #[test]
    fn compact_headers_keep_the_summary_but_drop_usage() {
        let mut view = pane_frame(PaneState::Live);
        view.compact = true;
        let header = header_text(&view, 80);

        assert!(header.contains("api"), "header: {header}");
        assert!(!header.contains("5h 80% left"), "header: {header}");
    }

    /// Panes overlap by a cell so their borders merge. If a pane ever drew
    /// outside the rect it was handed, it would erase its neighbour's content
    /// rather than just share a line with it.
    #[test]
    fn a_pane_frame_draws_only_inside_its_own_rect() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render_pane_frame(
                    frame,
                    Rect::new(0, 0, 10, 4),
                    &pane_frame(PaneState::Live),
                    &GridPalette::default(),
                );
            })
            .expect("render pane frame");

        let buffer = terminal.backend().buffer();
        for y in 0..6 {
            for x in 0..20 {
                if x < 10 && y < 4 {
                    continue;
                }
                assert_eq!(buffer[(x, y)].symbol(), " ", "cell ({x}, {y}) was painted");
            }
        }
    }

    /// The header is drawn as two titles, one from each end. They must never
    /// reach far enough to write over each other, at any width a pane can have.
    #[test]
    fn header_titles_never_collide_at_any_width() {
        for state in [
            PaneState::Live,
            PaneState::Waiting,
            PaneState::Idle,
            PaneState::Sleeping,
            PaneState::Exited,
        ] {
            let mut view = pane_frame(state);
            view.number = 100;
            view.label = "a-rather-long-pane-name".into();
            view.logging = true;

            for width in 8..=80u16 {
                let budget = width - 2;
                let trailing = pane_header_trailing(&view, &GridPalette::default(), width);
                let trailing_width = trailing.width() as u16;
                let fits =
                    trailing_width > 0 && pane_number_width(&view) + trailing_width <= budget;
                let leading = pane_header_leading(
                    &view,
                    &GridPalette::default(),
                    if fits {
                        budget - trailing_width
                    } else {
                        budget
                    },
                );

                let used = leading.width() as u16 + if fits { trailing_width } else { 0 };
                assert!(
                    used <= budget,
                    "{state:?} at width {width}: header needs {used} of {budget}"
                );
            }
        }
    }

    #[test]
    fn a_pane_frame_reports_the_area_left_for_its_terminal() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut inner = Rect::default();
        terminal
            .draw(|frame| {
                inner = render_pane_frame(
                    frame,
                    Rect::new(2, 1, 30, 6),
                    &pane_frame(PaneState::Live),
                    &GridPalette::default(),
                );
            })
            .expect("render pane frame");

        assert_eq!(inner, Rect::new(3, 2, 28, 4));
    }

    #[test]
    fn pane_activity_lines_show_latest_output_at_wide_and_narrow_widths() {
        let view = PaneSettingsView {
            index: 1,
            label: "api".into(),
            folder: "gridbash".into(),
            worktree: Some("feat/activity-summary".into()),
            history_summary: "all focused tests passed".into(),
            history_notice: None,
            focused: true,
            selected: false,
            sleeping: false,
            exited: false,
            auth_kind: None,
            auth_options: Vec::new(),
            auth_cursor: 0,
            selected_target: PaneSettingsTarget::Reload,
            goal: None,
            manager_configured: false,
        };
        let line_text = |line: &Line<'_>| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };

        let wide = pane_settings_lines(&view, 80, &GridPalette::default())
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(wide.contains("RECENT ACTIVITY"));
        assert!(wide.contains("summary  all focused tests passed"));
        assert!(wide.contains("Deactivate pane"));
        assert!(!wide.contains("run the focused tests"));

        let narrow = pane_settings_lines(&view, 30, &GridPalette::default())
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(narrow.contains("latest: all focused tests"));
        assert!(narrow.contains("Deactivate pane"));
    }

    #[test]
    fn pane_settings_action_buttons_use_expected_rows() {
        assert_eq!(
            pane_settings_rename_rect(Rect::new(5, 10, 40, 12), false),
            Some(Rect::new(5, 16, 40, 1))
        );
        assert_eq!(
            pane_settings_reload_rect(Rect::new(5, 10, 40, 12), false),
            Some(Rect::new(5, 17, 40, 1))
        );
        assert_eq!(
            pane_settings_rename_rect(Rect::new(5, 10, 20, 5), true),
            Some(Rect::new(5, 13, 20, 1))
        );
        assert_eq!(
            pane_settings_reload_rect(Rect::new(5, 10, 20, 5), true),
            Some(Rect::new(5, 14, 20, 1))
        );
        assert_eq!(
            pane_settings_rename_rect(Rect::new(5, 10, 40, 6), false),
            None
        );
        assert_eq!(
            pane_settings_reload_rect(Rect::new(5, 10, 40, 6), false),
            None
        );
        assert_eq!(
            pane_settings_sleep_rect(Rect::new(5, 10, 40, 14), false, false),
            Some(Rect::new(5, 19, 40, 1))
        );
        assert_eq!(
            pane_settings_deactivate_rect(Rect::new(5, 10, 40, 14), false, false),
            Some(Rect::new(5, 20, 40, 1))
        );
        assert_eq!(
            pane_settings_goal_rect(Rect::new(5, 10, 40, 14), false, true),
            Some(Rect::new(5, 22, 40, 1))
        );
        assert_eq!(
            pane_settings_stop_goal_rect(Rect::new(5, 10, 40, 14), false, true),
            Some(Rect::new(5, 23, 40, 1))
        );
        assert_eq!(
            pane_settings_stop_goal_rect(Rect::new(5, 10, 40, 14), false, false),
            None
        );
    }

    #[test]
    fn selected_pane_setting_has_a_focus_marker_and_active_background() {
        let selected = pane_settings_action_line("[ Refresh activity ]", 40, Color::Yellow, true);
        assert!(
            selected.spans[0]
                .content
                .contains("> [ Refresh activity ] <")
        );
        assert_eq!(selected.spans[0].style.fg, Some(SETTINGS_TEXT));
        assert_eq!(selected.spans[0].style.bg, Some(SETTINGS_ROW_ACTIVE));
        assert!(
            selected.spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );

        let unselected =
            pane_settings_action_line("[ Refresh activity ]", 40, Color::Yellow, false);
        assert!(!unselected.spans[0].content.contains("> ["));
        assert_eq!(unselected.spans[0].style.bg, Some(Color::Yellow));
    }

    #[test]
    fn pane_settings_command_bar_describes_arrow_navigation() {
        let text = pane_settings_command_bar(100, true)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("Up/Down"));
        assert!(text.contains("Enter/Space"));
        assert!(text.contains("Left/Right"));

        let no_auth = pane_settings_command_bar(100, false)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!no_auth.contains("Left/Right"));
    }

    #[test]
    fn auth_command_bar_distinguishes_pane_assignment_and_new_pane_policy() {
        let text = auth_command_bar(100)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(text.contains("Enter"));
        assert!(text.contains("assign"));
        assert!(text.contains("default"));
        assert!(text.contains("policy"));
    }

    #[test]
    fn pane_settings_render_one_selected_row_at_compact_and_wide_widths() {
        let view = PaneSettingsView {
            index: 0,
            label: "1".into(),
            folder: "gridbash".into(),
            worktree: None,
            history_summary: "Assistant: ready".into(),
            history_notice: None,
            focused: true,
            selected: false,
            sleeping: false,
            exited: false,
            auth_kind: None,
            auth_options: Vec::new(),
            auth_cursor: 0,
            selected_target: PaneSettingsTarget::Reload,
            goal: None,
            manager_configured: false,
        };

        for width in [32, 80] {
            let lines = pane_settings_lines(&view, width, &GridPalette::default());
            let active = lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .filter(|span| span.style.bg == Some(SETTINGS_ROW_ACTIVE))
                .collect::<Vec<_>>();

            assert_eq!(active.len(), 1, "width {width}");
            assert!(
                active[0].content.contains("Refresh activity"),
                "width {width}"
            );
        }
    }

    #[test]
    fn background_job_rows_show_state_context_and_stop_confirmation() {
        let job = BackgroundJobView {
            id: 4,
            label: "auth fix".into(),
            agent: "Codex".into(),
            source_tab: "Grid 2".into(),
            folder: "gridbash".into(),
            worktree: Some("feat/auth".into()),
            summary: "running focused tests".into(),
            state: BackgroundJobState::Working,
        };
        let normal = background_job_line(&job, true, false, 100);
        let normal_text = normal
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(normal_text.contains("auth fix"));
        assert!(normal_text.contains("working"));
        assert!(normal_text.contains("Grid 2 | gridbash"));
        assert!(normal_text.contains("feat"));
        assert!(normal_text.contains("running focused tests"));

        let pending = background_job_line(&job, true, true, 100);
        let pending_text = pending
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(pending_text.contains("stop?"));
        assert!(pending_text.contains("Delete again"));
    }

    #[test]
    fn background_job_command_bar_stays_useful_when_narrow() {
        let text = background_jobs_command_bar(40)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("Enter"));
        assert!(text.contains("restart"));
        assert!(text.contains("Esc"));
    }

    #[test]
    fn truncates_non_ascii_text_without_slicing_inside_a_character() {
        assert_eq!(truncate_text("alpha beta", 8), "alpha...");
        assert_eq!(
            truncate_text("codex says 東京 ready", 15),
            "codex says 東..."
        );
    }

    /// Focus is the answer to "where do my keystrokes go", so it outranks every
    /// state a pane arrived at on its own — including an agent asking for help,
    /// which still gets to say so in words.
    #[test]
    fn focus_and_selection_outrank_a_panes_own_state() {
        let palette = GridPalette::default();
        let mut view = pane_frame(PaneState::Waiting);

        assert_eq!(
            pane_border_style(&view, &palette),
            Style::default().fg(LINE)
        );

        view.selected = true;
        assert_eq!(
            pane_border_style(&view, &palette),
            Style::default()
                .fg(palette.selected())
                .add_modifier(Modifier::BOLD)
        );

        view.focused = true;
        assert_eq!(
            pane_border_style(&view, &palette),
            Style::default()
                .fg(palette.focus())
                .add_modifier(Modifier::BOLD)
        );
        assert!(header_text(&view, 80).contains("needs you"));
    }

    /// A resting pane must stay quiet. Borders that shout at the user from every
    /// idle pane are how a grid stops being readable at nine panes.
    #[test]
    fn a_resting_pane_uses_a_quiet_border() {
        let palette = GridPalette::default();

        for state in [PaneState::Live, PaneState::Idle] {
            assert_eq!(
                pane_border_style(&pane_frame(state), &palette),
                Style::default().fg(LINE),
                "{state:?} must not draw attention"
            );
        }
        assert_eq!(
            pane_border_style(&pane_frame(PaneState::Sleeping), &palette),
            Style::default().fg(LINE_SOFT)
        );
    }

    /// The seam is the whole point of overlapping the rects. If ratatui's border
    /// merging ever stops resolving the overlap, panes go back to showing two
    /// parallel lines between every pair — so assert on the junction glyph
    /// rather than trusting the layout arithmetic alone.
    #[test]
    fn adjacent_panes_merge_into_a_single_dividing_line() {
        let backend = TestBackend::new(21, 5);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let palette = GridPalette::default();

        terminal
            .draw(|frame| {
                let mut left = pane_frame(PaneState::Live);
                left.summary = String::new();
                left.usage = None;
                let mut right = left.clone();
                right.number = 2;
                // Overlapping by one column is what `weighted_grid_rects` hands
                // the renderer for two side-by-side panes.
                render_pane_frame(frame, Rect::new(0, 0, 11, 5), &left, &palette);
                render_pane_frame(frame, Rect::new(10, 0, 11, 5), &right, &palette);
            })
            .expect("render two panes");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(10, 0)].symbol(), "┬", "top junction");
        assert_eq!(buffer[(10, 2)].symbol(), "│", "shared divider");
        assert_eq!(buffer[(10, 4)].symbol(), "┴", "bottom junction");
    }

    /// Cache buffers live between frames, so a cell keeps whatever it was last
    /// given. Patching a `Style` onto one only ever adds modifiers, which would
    /// leave text bold forever once anything nearby had been bold.
    #[test]
    fn rasterizing_clears_attributes_that_no_longer_apply() {
        let mut parser = vt100::Parser::new(1, 4, 100);
        let mut cache = PaneRenderCache::default();

        parser.process(b"\x1b[1;31mbold");
        refresh_screen_cache(&mut cache, 1, parser.screen(), 4, 1, None);
        assert!(cache.buffer[(0, 0)].modifier.contains(Modifier::BOLD));

        parser.process(b"\r\x1b[0mflat");
        refresh_screen_cache(&mut cache, 2, parser.screen(), 4, 1, None);
        assert!(
            !cache.buffer[(0, 0)].modifier.contains(Modifier::BOLD),
            "bold outlived the output that set it"
        );
        assert_eq!(cache.buffer[(0, 0)].symbol(), "f");
    }

    /// Ratatui's diff assumes a double-width symbol is followed by a blank and
    /// skips that column. Leaving anything else there prints over the second
    /// half of the glyph.
    #[test]
    fn wide_characters_leave_their_second_column_blank() {
        let mut parser = vt100::Parser::new(1, 4, 100);
        parser.process("東京".as_bytes());
        let mut cache = PaneRenderCache::default();

        refresh_screen_cache(&mut cache, 1, parser.screen(), 4, 1, None);

        assert_eq!(cache.buffer[(0, 0)].symbol(), "東");
        assert_eq!(cache.buffer[(1, 0)].symbol(), " ");
        assert_eq!(cache.buffer[(2, 0)].symbol(), "京");
        assert_eq!(cache.buffer[(3, 0)].symbol(), " ");
    }

    /// A wide glyph in the last column has nowhere to put its second half, and
    /// the cell to its right belongs to the next pane's border.
    #[test]
    fn a_wide_character_clipped_by_the_pane_edge_is_dropped() {
        let mut parser = vt100::Parser::new(1, 4, 100);
        parser.process("a東".as_bytes());
        let mut cache = PaneRenderCache::default();

        refresh_screen_cache(&mut cache, 1, parser.screen(), 2, 1, None);

        assert_eq!(cache.buffer[(0, 0)].symbol(), "a");
        assert_eq!(cache.buffer[(1, 0)].symbol(), " ");
    }

    #[test]
    fn pane_render_cache_reuses_revision_and_invalidates_on_output() {
        let mut parser = vt100::Parser::new(2, 10, 100);
        parser.process(b"hello");
        let mut cache = PaneRenderCache::default();

        refresh_screen_cache(&mut cache, 1, parser.screen(), 10, 2, None);
        let first = cache.buffer.clone();
        parser.process(b" world");
        refresh_screen_cache(&mut cache, 1, parser.screen(), 10, 2, None);
        assert_eq!(cache.buffer, first);

        refresh_screen_cache(&mut cache, 2, parser.screen(), 10, 2, None);
        assert_ne!(cache.buffer, first);
    }

    #[test]
    fn pane_render_cache_keys_selection_and_dimensions() {
        let mut parser = vt100::Parser::new(2, 10, 100);
        parser.process(b"hello");
        let mut cache = PaneRenderCache::default();
        refresh_screen_cache(&mut cache, 1, parser.screen(), 10, 2, None);
        let plain = cache.buffer.clone();
        let selection = Some(PaneSelection {
            start_row: 0,
            start_column: 0,
            end_row: 0,
            end_column: 4,
        });
        refresh_screen_cache(&mut cache, 1, parser.screen(), 10, 2, selection);
        assert_ne!(cache.buffer, plain);
        refresh_screen_cache(&mut cache, 1, parser.screen(), 5, 2, selection);
        assert_eq!(cache.buffer.area, Rect::new(0, 0, 5, 2));
    }

    #[test]
    fn cached_screen_buffer_blits_at_the_pane_offset() {
        let mut source = Buffer::empty(Rect::new(0, 0, 2, 2));
        source[(0, 0)].set_symbol("A");
        source[(1, 0)].set_symbol("B");
        source[(0, 1)].set_symbol("C");
        source[(1, 1)].set_symbol("D");
        let mut target = Buffer::empty(Rect::new(0, 0, 6, 4));

        blit_buffer(&source, &mut target, Rect::new(3, 1, 2, 2));

        assert_eq!(target[(3, 1)].symbol(), "A");
        assert_eq!(target[(4, 1)].symbol(), "B");
        assert_eq!(target[(3, 2)].symbol(), "C");
        assert_eq!(target[(4, 2)].symbol(), "D");
        assert_eq!(target[(2, 1)].symbol(), " ");
    }

    #[test]
    fn blitting_a_pane_rect_wider_than_the_frame_clamps_instead_of_panicking() {
        // A rect measured before the terminal shrank: the release build used to
        // panic here in Buffer::index_of and take the whole process down.
        let mut source = Buffer::empty(Rect::new(0, 0, 4, 3));
        for x in 0..4 {
            for y in 0..3 {
                source[(x, y)].set_symbol("S");
            }
        }
        let mut target = Buffer::empty(Rect::new(0, 0, 3, 2));

        blit_buffer(&source, &mut target, Rect::new(2, 1, 4, 3));

        assert_eq!(target[(2, 1)].symbol(), "S");
        assert_eq!(target[(0, 0)].symbol(), " ");
    }

    #[test]
    fn blitting_a_pane_rect_fully_outside_the_frame_is_a_no_op() {
        let mut source = Buffer::empty(Rect::new(0, 0, 2, 2));
        source[(0, 0)].set_symbol("S");
        let mut target = Buffer::empty(Rect::new(0, 0, 4, 4));
        let untouched = target.clone();

        blit_buffer(&source, &mut target, Rect::new(9, 9, 2, 2));

        assert_eq!(target, untouched);
    }
}
