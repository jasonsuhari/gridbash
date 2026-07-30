use std::{
    collections::BTreeSet,
    io::{self, Stdout, Write},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::session::{SavedTab, SessionRecord, delete_saved_session, process_is_running};

type ResumeTerminal = Terminal<CrosstermBackend<Stdout>>;

const CANVAS_BG: Color = Color::Rgb(0, 5, 2);
const PANEL_BG: Color = Color::Rgb(1, 10, 5);
const RAISED_BG: Color = Color::Rgb(2, 16, 8);
const SELECTED_BG: Color = Color::Rgb(5, 35, 18);
const HAIRLINE: Color = Color::Rgb(18, 76, 40);
const MUTED: Color = Color::Rgb(72, 128, 88);
const DIM_GREEN: Color = Color::Rgb(50, 176, 92);
const TERMINAL_GREEN: Color = Color::Rgb(91, 255, 139);
const SOFT_GREEN: Color = Color::Rgb(159, 255, 183);

/// Marks a pane whose agent conversation comes back with it.
const RESUMABLE_MARK: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Open(u32),
    Interrupted,
    Detached,
    Saved,
}

impl SessionState {
    fn for_record(record: &SessionRecord) -> Self {
        let session = &record.session;
        if session.running {
            if let Some(owner_pid) = session.owner_pid
                && process_is_running(owner_pid)
            {
                return Self::Open(owner_pid);
            }
            return Self::Interrupted;
        }

        if session.all_panes().any(|pane| pane.host.is_some()) {
            Self::Detached
        } else {
            Self::Saved
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Open(_) => "OPEN",
            Self::Interrupted => "RECOVER",
            Self::Detached => "DETACHED",
            Self::Saved => "SAVED",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Open(_) => DIM_GREEN,
            Self::Interrupted => TERMINAL_GREEN,
            Self::Detached => SOFT_GREEN,
            Self::Saved => MUTED,
        }
    }

    fn description(self) -> String {
        match self {
            Self::Open(owner_pid) => {
                format!("Already attached to a live GridBash client (PID {owner_pid}).")
            }
            Self::Interrupted => {
                "The previous client stopped. Every grid comes back as it was.".into()
            }
            Self::Detached => "Terminals are still running and will reconnect.".into(),
            Self::Saved => {
                "Rebuilds each grid at its saved size, with its panes, names, and agent \
                 conversations."
                    .into()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerAction {
    Continue,
    Select(usize),
    Delete(usize),
    Cancel,
}

/// One pane as the picker draws it: where it sat, what it was called, and
/// whether its agent conversation comes back.
#[derive(Debug, Clone)]
struct PaneCell {
    label: String,
    resumable: bool,
}

/// A saved grid, laid out the way it will be restored.
#[derive(Debug, Clone)]
struct GridPreview {
    title: String,
    rows: usize,
    columns: usize,
    pane_count: usize,
    /// One entry per cell, row by row. `None` is a cell the grid kept empty.
    cells: Vec<Option<PaneCell>>,
    active: bool,
}

impl GridPreview {
    fn dimensions(&self) -> String {
        format!("{}x{}", self.rows, self.columns)
    }
}

/// A saved workspace with the parts that cost real work to compute already
/// resolved, so redrawing never touches the filesystem again.
struct SessionEntry {
    record: SessionRecord,
    title: String,
    grids: Vec<GridPreview>,
    pane_count: usize,
    resumable_count: usize,
    background_count: usize,
    folders: Option<String>,
    profiles: Option<String>,
}

impl SessionEntry {
    fn new(record: SessionRecord) -> Self {
        let (grids, active) = record.session.ordered_grids();
        let grids = grids
            .iter()
            .enumerate()
            .map(|(index, grid)| grid_preview(grid, index == active))
            .collect::<Vec<_>>();
        let pane_count = record.session.all_panes().count();
        let resumable_count = record
            .session
            .all_panes()
            .filter(|pane| pane.resumable_conversation().is_some())
            .count();
        let folders = compact_labels(
            record
                .session
                .all_panes()
                .map(|pane| pane.folder_name.as_str()),
        );
        let profiles = compact_labels(
            record
                .session
                .all_panes()
                .map(|pane| pane.profile_name.as_str()),
        );

        Self {
            title: session_title(&record),
            grids,
            pane_count,
            resumable_count,
            background_count: record.session.background_panes.len(),
            folders,
            profiles,
            record,
        }
    }

    fn state(&self) -> SessionState {
        SessionState::for_record(&self.record)
    }

    /// One line naming every grid with the size it will be rebuilt at.
    fn grid_summary(&self) -> String {
        let shown = self
            .grids
            .iter()
            .take(4)
            .map(|grid| format!("{} {}", grid.title, grid.dimensions()))
            .collect::<Vec<_>>()
            .join(", ");
        let extra = self.grids.len().saturating_sub(4);
        if extra > 0 {
            format!("{shown} +{extra}")
        } else {
            shown
        }
    }
}

/// Lay a saved grid out cell by cell so the picker can show that every pane
/// comes back in the position it held.
fn grid_preview(grid: &SavedTab, active: bool) -> GridPreview {
    let rows = grid.grid.rows.max(1);
    let columns = grid.grid.columns.max(1);
    let mut cells = vec![None; rows.saturating_mul(columns)];
    let mut panes = grid.panes.iter().collect::<Vec<_>>();
    panes.sort_by_key(|pane| pane.index);
    for (position, pane) in panes.iter().enumerate() {
        let Some(cell) = cells.get_mut(position) else {
            break;
        };
        *cell = Some(PaneCell {
            label: pane
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| pane.profile_name.clone()),
            resumable: pane.resumable_conversation().is_some(),
        });
    }

    GridPreview {
        title: if grid.title.trim().is_empty() {
            "Untitled grid".into()
        } else {
            grid.title.clone()
        },
        rows,
        columns,
        pane_count: grid.panes.len(),
        cells,
        active,
    }
}

struct ResumePicker {
    sessions: Vec<SessionEntry>,
    list_state: ListState,
    /// Grid of the selected workspace shown in the map, so every saved grid can
    /// be inspected before resuming.
    grid_cursor: usize,
    page_size: usize,
    notice: Option<String>,
    pending_delete: Option<String>,
}

pub fn select_session(sessions: &[SessionRecord]) -> Result<Option<SessionRecord>> {
    let mut restore_guard = ResumeTerminalRestoreGuard::new();
    let mut terminal = setup_terminal()?;
    let mut picker = ResumePicker::new(sessions);
    let result = picker.run(&mut terminal);
    let teardown_result = teardown_terminal(&mut terminal);

    if teardown_result.is_ok() {
        restore_guard.disarm();
    }
    match (result, teardown_result) {
        (Err(error), Err(cleanup_error)) => Err(error.context(format!(
            "resume picker terminal cleanup also failed: {cleanup_error:#}"
        ))),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(selection), Ok(())) => Ok(selection),
    }
}

impl ResumePicker {
    fn new(sessions: &[SessionRecord]) -> Self {
        let mut list_state = ListState::default();
        list_state.select((!sessions.is_empty()).then_some(0));
        let sessions = sessions
            .iter()
            .cloned()
            .map(SessionEntry::new)
            .collect::<Vec<_>>();
        let grid_cursor = sessions.first().map(active_grid_index).unwrap_or(0);
        Self {
            sessions,
            list_state,
            grid_cursor,
            page_size: 1,
            notice: None,
            pending_delete: None,
        }
    }

    fn run(&mut self, terminal: &mut ResumeTerminal) -> Result<Option<SessionRecord>> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match self.handle_key(key) {
                PickerAction::Continue => {}
                PickerAction::Cancel => return Ok(None),
                PickerAction::Select(index) => {
                    return Ok(Some(self.sessions[index].record.clone()));
                }
                PickerAction::Delete(index) => {
                    let title = self.sessions[index].title.clone();
                    match delete_saved_session(&self.sessions[index].record) {
                        Ok(()) => {
                            self.sessions.remove(index);
                            self.pending_delete = None;
                            if self.sessions.is_empty() {
                                return Ok(None);
                            }
                            self.select(index.min(self.sessions.len().saturating_sub(1)));
                            self.notice = Some(format!("Deleted saved session {title}."));
                        }
                        Err(error) => {
                            self.pending_delete = None;
                            self.notice = Some(format!("Could not delete session: {error:#}"));
                        }
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        let Some(selected) = self.list_state.selected() else {
            return PickerAction::Cancel;
        };
        if self.pending_delete.is_some() && !matches!(key.code, KeyCode::Delete | KeyCode::Esc) {
            self.pending_delete = None;
        }
        match key.code {
            KeyCode::Esc if self.pending_delete.take().is_some() => {
                self.notice = Some("Session deletion canceled.".into());
                PickerAction::Continue
            }
            KeyCode::Esc | KeyCode::Char('q') => PickerAction::Cancel,
            KeyCode::Up | KeyCode::Char('k') => {
                self.select(selected.saturating_sub(1));
                PickerAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select((selected + 1).min(self.sessions.len().saturating_sub(1)));
                PickerAction::Continue
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.cycle_grid(1);
                PickerAction::Continue
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.cycle_grid(-1);
                PickerAction::Continue
            }
            KeyCode::Home => {
                self.select(0);
                PickerAction::Continue
            }
            KeyCode::End => {
                self.select(self.sessions.len().saturating_sub(1));
                PickerAction::Continue
            }
            KeyCode::PageUp => {
                self.select(selected.saturating_sub(self.page_size));
                PickerAction::Continue
            }
            KeyCode::PageDown => {
                self.select((selected + self.page_size).min(self.sessions.len().saturating_sub(1)));
                PickerAction::Continue
            }
            KeyCode::Delete => self.request_delete(selected),
            KeyCode::Enter => {
                if let SessionState::Open(owner_pid) = self.sessions[selected].state() {
                    self.notice = Some(format!(
                        "Session is already open in PID {owner_pid}. Switch to that GridBash window or close it before resuming."
                    ));
                    PickerAction::Continue
                } else {
                    PickerAction::Select(selected)
                }
            }
            _ => PickerAction::Continue,
        }
    }

    fn select(&mut self, index: usize) {
        self.list_state.select(Some(index));
        self.grid_cursor = self.sessions.get(index).map(active_grid_index).unwrap_or(0);
        self.notice = None;
        self.pending_delete = None;
    }

    /// Step through the selected workspace's grids, wrapping at both ends.
    fn cycle_grid(&mut self, delta: isize) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let count = entry.grids.len();
        if count <= 1 {
            return;
        }

        let current = self.grid_cursor.min(count - 1) as isize;
        self.grid_cursor = (current + delta).rem_euclid(count as isize) as usize;
        self.notice = None;
    }

    fn selected_entry(&self) -> Option<&SessionEntry> {
        self.list_state
            .selected()
            .and_then(|index| self.sessions.get(index))
    }

    fn request_delete(&mut self, selected: usize) -> PickerAction {
        let entry = &self.sessions[selected];
        if let SessionState::Open(owner_pid) = entry.state() {
            self.pending_delete = None;
            self.notice = Some(format!(
                "Session is open in PID {owner_pid}. Close that GridBash window before deleting it."
            ));
            return PickerAction::Continue;
        }

        if self.pending_delete.as_deref() == Some(entry.record.session.id.as_str()) {
            return PickerAction::Delete(selected);
        }

        self.pending_delete = Some(entry.record.session.id.clone());
        let detached = entry
            .record
            .session
            .all_panes()
            .any(|pane| pane.host.is_some());
        self.notice = Some(if detached {
            "Press Delete again to stop detached terminals and permanently delete this session."
                .into()
        } else {
            "Press Delete again to permanently delete this saved session.".into()
        });
        PickerAction::Continue
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(CANVAS_BG)),
            area,
        );

        let panel = if area.width >= 84 && area.height >= 20 {
            inset(area, 1, 1)
        } else {
            area
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(Line::from(vec![
                Span::styled(" $ ", Style::default().fg(TERMINAL_GREEN)),
                Span::styled(
                    "gridbash resume",
                    Style::default()
                        .fg(TERMINAL_GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", Style::default()),
            ]))
            .border_style(Style::default().fg(TERMINAL_GREEN))
            .style(Style::default().fg(SOFT_GREEN).bg(PANEL_BG));
        let inner = inset(block.inner(panel), 1, 0);
        frame.render_widget(block, panel);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let (header_height, detail_height, controls_height) = if inner.height >= 24 {
            (3, 11, 3)
        } else {
            (2, 9, 2)
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Length(detail_height),
                Constraint::Min(4),
                Constraint::Length(controls_height),
            ])
            .split(inner);
        self.draw_header(frame, chunks[0]);
        self.draw_selected(frame, chunks[1]);
        self.draw_sessions(frame, chunks[2]);
        self.draw_controls(frame, chunks[3]);
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let selected = self.list_state.selected().unwrap_or(0) + 1;
        let selected_state = self
            .selected_entry()
            .map(SessionEntry::state)
            .unwrap_or(SessionState::Saved);
        let right_width = area.width.min(24);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(right_width)])
            .split(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "SELECT A WORKSPACE TO RESUME",
                    Style::default().fg(SOFT_GREEN).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "Grid sizes, pane positions, names, and agent conversations are restored.",
                    Style::default().fg(MUTED),
                )),
            ])
            .style(Style::default().bg(PANEL_BG)),
            columns[0],
        );

        if columns[1].width > 0 {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(state_badge(selected_state)),
                    Line::from(Span::styled(
                        format!("{selected} of {}", self.sessions.len()),
                        Style::default().fg(MUTED),
                    )),
                ])
                .alignment(Alignment::Right)
                .style(Style::default().bg(PANEL_BG)),
                columns[1],
            );
        }
    }

    /// Details on the left, the selected grid drawn in position on the right.
    fn draw_selected(&self, frame: &mut Frame<'_>, area: Rect) {
        let map_width = if area.width >= 96 {
            area.width / 2
        } else if area.width >= 70 {
            area.width * 2 / 5
        } else {
            0
        };
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(30), Constraint::Length(map_width)])
            .split(area);
        self.draw_details(frame, columns[0]);
        if map_width > 0 {
            self.draw_grid_map(frame, columns[1]);
        }
    }

    fn draw_details(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = panel_block("SELECTED WORKSPACE");
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        let Some(entry) = self.selected_entry() else {
            return;
        };

        let state = entry.state();
        let grid_count = entry.grids.len();
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    entry.title.clone(),
                    Style::default()
                        .fg(TERMINAL_GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                state_badge(state),
            ]),
            Line::from(Span::styled(
                state.description(),
                Style::default().fg(MUTED),
            )),
            detail_row("SESSION", entry.record.session.id.clone()),
            detail_row(
                "GRIDS",
                format!(
                    "{grid_count} | {}",
                    if entry.grids.is_empty() {
                        "none recorded".into()
                    } else {
                        entry.grid_summary()
                    }
                ),
            ),
            detail_row(
                "PANES",
                format!(
                    "{} | {} agent conversation{} resume",
                    entry.pane_count,
                    entry.resumable_count,
                    if entry.resumable_count == 1 { "" } else { "s" },
                ),
            ),
            detail_row(
                "FOLDERS",
                entry.folders.clone().unwrap_or_else(|| "Unknown".into()),
            ),
            detail_row(
                "PROFILES",
                entry.profiles.clone().unwrap_or_else(|| "Unknown".into()),
            ),
        ];
        if entry.background_count > 0 {
            lines.push(detail_row(
                "BACKGROUND",
                format!(
                    "{} pane{} kept out of the grids",
                    entry.background_count,
                    if entry.background_count == 1 { "" } else { "s" },
                ),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(RAISED_BG)),
            inner,
        );
    }

    /// Draw the selected grid as a map, one row of cells per grid row, so the
    /// user can see the arrangement that will come back.
    fn draw_grid_map(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(entry) = self.selected_entry() else {
            frame.render_widget(panel_block("GRID"), area);
            return;
        };
        let index = self.grid_cursor.min(entry.grids.len().saturating_sub(1));
        let Some(grid) = entry.grids.get(index) else {
            frame.render_widget(panel_block("GRID"), area);
            return;
        };

        let heading = format!(
            " {} · {} · {} pane{} ",
            grid.title,
            grid.dimensions(),
            grid.pane_count,
            if grid.pane_count == 1 { "" } else { "s" },
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(Line::from(Span::styled(
                heading,
                Style::default()
                    .fg(TERMINAL_GREEN)
                    .add_modifier(Modifier::BOLD),
            )))
            .border_style(Style::default().fg(HAIRLINE))
            .style(Style::default().bg(RAISED_BG));
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let mut lines = grid_map_lines(grid, inner.width);
        if entry.grids.len() > 1 {
            lines.push(Line::from(Span::styled(
                format!(
                    "grid {} of {}{}",
                    index + 1,
                    entry.grids.len(),
                    if grid.active { " · opens here" } else { "" }
                ),
                Style::default().fg(MUTED),
            )));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(RAISED_BG)),
            inner,
        );
    }

    fn draw_sessions(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let block = panel_block("RECENT WORKSPACES");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.page_size = usize::from((inner.height / 2).max(1));

        let items = self
            .sessions
            .iter()
            .map(|entry| {
                ListItem::new(vec![
                    Line::from(vec![
                        state_badge(entry.state()),
                        Span::raw(" "),
                        Span::styled(
                            entry.title.clone(),
                            Style::default().fg(SOFT_GREEN).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("           ", Style::default().fg(MUTED)),
                        Span::styled(entry.record.summary(), Style::default().fg(MUTED)),
                    ]),
                ])
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .highlight_symbol("> ")
            .highlight_style(Style::default().fg(TERMINAL_GREEN).bg(SELECTED_BG))
            .style(Style::default().bg(RAISED_BG));
        frame.render_stateful_widget(list, inner, &mut self.list_state);
    }

    fn draw_controls(&self, frame: &mut Frame<'_>, area: Rect) {
        let line = if let Some(notice) = &self.notice {
            Line::from(vec![
                Span::styled("[!]", Style::default().fg(TERMINAL_GREEN)),
                Span::raw("  "),
                Span::styled(notice.clone(), Style::default().fg(SOFT_GREEN)),
            ])
        } else {
            Line::from(vec![
                keycap("UP/DOWN"),
                Span::styled(" WORKSPACE   ", Style::default().fg(MUTED)),
                keycap("TAB"),
                Span::styled(" GRID   ", Style::default().fg(MUTED)),
                launch_keycap("ENTER"),
                Span::styled(" RESUME   ", Style::default().fg(MUTED)),
                keycap("DELETE x2"),
                Span::styled(" REMOVE   ", Style::default().fg(MUTED)),
                keycap("Q or ESC"),
                Span::styled(" CANCEL", Style::default().fg(MUTED)),
            ])
        };
        frame.render_widget(
            Paragraph::new(line)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(PANEL_BG)),
            area,
        );
    }
}

fn active_grid_index(entry: &SessionEntry) -> usize {
    entry.grids.iter().position(|grid| grid.active).unwrap_or(0)
}

/// Render a grid as bracketed cells, one line per grid row. Panes whose agent
/// conversation resumes are marked, and empty cells are drawn as empty.
fn grid_map_lines(grid: &GridPreview, width: u16) -> Vec<Line<'static>> {
    let columns = grid.columns.max(1);
    // Each cell spends two characters on its brackets. A grid with many columns
    // gets narrow cells rather than a row that runs off the panel.
    let cell_width = usize::from(width)
        .saturating_div(columns)
        .saturating_sub(2)
        .clamp(1, 18);

    (0..grid.rows.max(1))
        .map(|row| {
            let spans = (0..columns)
                .map(|column| {
                    let position = row.saturating_mul(columns).saturating_add(column);
                    match grid.cells.get(position).and_then(Option::as_ref) {
                        Some(cell) => Span::styled(
                            format!("[{}]", cell_text(cell, position + 1, cell_width)),
                            Style::default().fg(if cell.resumable {
                                TERMINAL_GREEN
                            } else {
                                SOFT_GREEN
                            }),
                        ),
                        None => Span::styled(
                            format!("[{}]", " ".repeat(cell_width)),
                            Style::default().fg(HAIRLINE),
                        ),
                    }
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}

/// A cell's text: its pane number, its name, and a mark when its conversation
/// comes back. Truncated to the space the grid leaves it.
fn cell_text(cell: &PaneCell, number: usize, width: usize) -> String {
    let mark = if cell.resumable { RESUMABLE_MARK } else { "" };
    let prefix = format!("{number}{mark} ");
    let room = width.saturating_sub(prefix.chars().count());
    let mut text = prefix;
    text.extend(cell.label.chars().take(room));
    let padding = width.saturating_sub(text.chars().count());
    text.push_str(&" ".repeat(padding));
    text.chars().take(width).collect()
}

fn session_title(record: &SessionRecord) -> String {
    if record.session.title.trim().is_empty() {
        record
            .session
            .panes
            .first()
            .map(|pane| pane.folder_name.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Untitled workspace".into())
    } else {
        record.session.title.clone()
    }
}

fn compact_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Option<String> {
    let unique = labels
        .filter(|label| !label.is_empty())
        .collect::<BTreeSet<_>>();
    if unique.is_empty() {
        return None;
    }
    let shown = unique.iter().take(3).copied().collect::<Vec<_>>();
    let extra = unique.len().saturating_sub(shown.len());
    let mut label = shown.join(", ");
    if extra > 0 {
        label.push_str(&format!(" +{extra}"));
    }
    Some(label)
}

fn detail_row(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<12}"),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(value, Style::default().fg(SOFT_GREEN)),
    ])
}

fn state_badge(state: SessionState) -> Span<'static> {
    Span::styled(
        format!("[{:<8}]", state.label()),
        Style::default()
            .fg(state.color())
            .add_modifier(Modifier::BOLD),
    )
}

fn panel_block(label: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(Line::from(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(TERMINAL_GREEN)
                .add_modifier(Modifier::BOLD),
        )))
        .border_style(Style::default().fg(HAIRLINE))
        .style(Style::default().bg(RAISED_BG))
}

fn keycap(label: &'static str) -> Span<'static> {
    Span::styled(
        format!("[{label}]"),
        Style::default()
            .fg(TERMINAL_GREEN)
            .bg(RAISED_BG)
            .add_modifier(Modifier::BOLD),
    )
}

fn launch_keycap(label: &'static str) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(Color::Black)
            .bg(TERMINAL_GREEN)
            .add_modifier(Modifier::BOLD),
    )
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(x.min(area.width)),
        y: area.y.saturating_add(y.min(area.height)),
        width: area.width.saturating_sub(x.saturating_mul(2)),
        height: area.height.saturating_sub(y.saturating_mul(2)),
    }
}

fn setup_terminal() -> Result<ResumeTerminal> {
    enable_raw_mode().context("failed to enable raw terminal mode")?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        let _ = restore_terminal_output(&mut stdout);
        return Err(error).context("failed to enter alternate screen");
    }
    let backend = CrosstermBackend::new(stdout);
    match Terminal::new(backend) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            let mut stdout = io::stdout();
            let _ = restore_terminal_output(&mut stdout);
            Err(error).context("failed to create resume terminal")
        }
    }
}

fn teardown_terminal(terminal: &mut ResumeTerminal) -> Result<()> {
    restore_terminal_output(terminal.backend_mut())
}

fn restore_terminal_output(output: &mut impl Write) -> Result<()> {
    let mut first_error = disable_raw_mode()
        .err()
        .map(|error| anyhow!(error).context("failed to disable raw terminal mode"));
    if let Err(error) = execute!(output, LeaveAlternateScreen)
        && first_error.is_none()
    {
        first_error = Some(anyhow!(error).context("failed to leave alternate screen"));
    }
    if let Err(error) = execute!(output, Show)
        && first_error.is_none()
    {
        first_error = Some(anyhow!(error).context("failed to restore cursor"));
    }
    first_error.map_or(Ok(()), Err)
}

struct ResumeTerminalRestoreGuard {
    armed: bool,
}

impl ResumeTerminalRestoreGuard {
    fn new() -> Self {
        Self { armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ResumeTerminalRestoreGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = restore_terminal_output(&mut io::stdout());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        profiles::Profile,
        session::{SavedGrid, SavedPane, SavedPaneHistory, SavedSession, SavedView},
    };

    #[test]
    fn marks_live_and_interrupted_sessions() {
        let mut record = record("active", true, Some(std::process::id()), false);
        assert_eq!(
            SessionState::for_record(&record),
            SessionState::Open(std::process::id())
        );

        record.session.owner_pid = Some(u32::MAX);
        assert_eq!(SessionState::for_record(&record), SessionState::Interrupted);
    }

    #[test]
    fn marks_detached_sessions_with_saved_hosts() {
        let record = record("detached", false, None, true);
        assert_eq!(SessionState::for_record(&record), SessionState::Detached);
    }

    #[test]
    fn renders_terminal_green_stacked_resume_picker() {
        let sessions = vec![record("Fluent workspace", false, None, true)];
        let mut picker = ResumePicker::new(&sessions);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| picker.draw(frame))
            .expect("draw resume picker");

        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let details_at = rendered.find("SELECTED WORKSPACE").expect("details panel");
        let sessions_at = rendered.find("RECENT WORKSPACES").expect("sessions panel");
        assert!(
            details_at < sessions_at,
            "details should render above sessions"
        );
        assert!(rendered.contains("gridbash resume"));
        assert!(rendered.contains("Fluent workspace"));
        assert!(rendered.contains("DETACHED"));
        assert!(rendered.contains("DELETE x2"));
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.fg == TERMINAL_GREEN)
        );
    }

    /// The picker has to prove the arrangement survives, so it shows each grid's
    /// saved size and draws its panes in the cells they occupied.
    #[test]
    fn shows_saved_grid_dimensions_and_pane_positions() {
        let mut session = record("Fluent workspace", false, None, false).session;
        session.title = "Main".into();
        session.grid = SavedGrid {
            rows: 2,
            columns: 3,
        };
        session.panes = vec![
            named_pane(0, "codex", Some("planner")),
            named_pane(1, "claude", None),
            named_pane(2, "git-bash", None),
            named_pane(3, "codex", None),
        ];
        session.tabs = vec![SavedTab {
            title: "api".into(),
            grid: SavedGrid {
                rows: 1,
                columns: 2,
            },
            view: SavedView::default(),
            panes: vec![named_pane(0, "codex", None), named_pane(1, "codex", None)],
        }];
        let sessions = vec![SessionRecord {
            path: PathBuf::from("session.toml"),
            session,
        }];
        let mut picker = ResumePicker::new(&sessions);
        let backend = TestBackend::new(130, 34);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| picker.draw(frame))
            .expect("draw resume picker");
        let rendered = rendered_text(&terminal);

        // Both grids are named with the size they will be rebuilt at.
        assert!(rendered.contains("Main 2x3"), "{rendered}");
        assert!(rendered.contains("api 1x2"), "{rendered}");
        // The active grid's map shows the user's pane name in the first cell and
        // leaves the two unused cells of the 2x3 grid empty.
        assert!(rendered.contains("Main · 2x3"), "{rendered}");
        assert!(rendered.contains("planner"), "{rendered}");
    }

    /// Tab walks the saved grids so every one can be checked before resuming.
    #[test]
    fn tab_cycles_through_saved_grids() {
        let mut session = record("Fluent workspace", false, None, false).session;
        session.title = "Main".into();
        session.tabs = vec![SavedTab {
            title: "api".into(),
            grid: SavedGrid {
                rows: 1,
                columns: 1,
            },
            view: SavedView::default(),
            panes: vec![named_pane(0, "codex", None)],
        }];
        let sessions = vec![SessionRecord {
            path: PathBuf::from("session.toml"),
            session,
        }];
        let mut picker = ResumePicker::new(&sessions);
        assert_eq!(picker.grid_cursor, 0);

        let tab = KeyEvent::new(KeyCode::Tab, crossterm::event::KeyModifiers::NONE);
        assert_eq!(picker.handle_key(tab), PickerAction::Continue);
        assert_eq!(picker.grid_cursor, 1);
        assert_eq!(picker.handle_key(tab), PickerAction::Continue);
        assert_eq!(
            picker.grid_cursor, 0,
            "cycling wraps back to the first grid"
        );
    }

    #[test]
    fn session_deletion_requires_a_second_delete_press() {
        let sessions = vec![record("Saved workspace", false, None, false)];
        let mut picker = ResumePicker::new(&sessions);
        let delete = KeyEvent::new(KeyCode::Delete, crossterm::event::KeyModifiers::NONE);

        assert_eq!(picker.handle_key(delete), PickerAction::Continue);
        assert!(picker.pending_delete.is_some());
        assert_eq!(picker.handle_key(delete), PickerAction::Delete(0));
    }

    #[test]
    fn open_sessions_cannot_be_deleted_from_the_picker() {
        let sessions = vec![record(
            "Open workspace",
            true,
            Some(std::process::id()),
            false,
        )];
        let mut picker = ResumePicker::new(&sessions);
        let delete = KeyEvent::new(KeyCode::Delete, crossterm::event::KeyModifiers::NONE);

        assert_eq!(picker.handle_key(delete), PickerAction::Continue);
        assert!(picker.pending_delete.is_none());
        assert!(
            picker
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Close that GridBash window"))
        );
    }

    fn rendered_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn named_pane(index: usize, profile_name: &str, name: Option<&str>) -> SavedPane {
        let mut pane = pane(profile_name, false);
        pane.index = index;
        pane.name = name.map(str::to_string);
        pane
    }

    fn pane(profile_name: &str, host: bool) -> SavedPane {
        SavedPane {
            index: 0,
            profile_name: profile_name.into(),
            command: Profile {
                command: profile_name.into(),
                args: Vec::new(),
                title: Some(profile_name.into()),
                agent_kind: None,
            },
            cwd: PathBuf::from("fluent"),
            folder_name: "fluent".into(),
            name: None,
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            sleeping: false,
            history: SavedPaneHistory::default(),
            codex_thread_id: None,
            claude_session_id: None,
            host: host.then(|| crate::pane_host::PtyHostRef {
                endpoint: "127.0.0.1:12345".into(),
                token: "token".into(),
                process_id: None,
                codex_sqlite_home: None,
                started_at_ms: None,
            }),
        }
    }

    fn record(title: &str, running: bool, owner_pid: Option<u32>, host: bool) -> SessionRecord {
        SessionRecord {
            path: PathBuf::from("session.toml"),
            session: SavedSession {
                version: 2,
                id: "session-id".into(),
                started_at: 1,
                updated_at: 1,
                title: title.into(),
                active_tab: 0,
                next_tab_number: 2,
                grid: SavedGrid {
                    rows: 1,
                    columns: 1,
                },
                view: SavedView::default(),
                panes: vec![pane("codex", host)],
                background_panes: Vec::new(),
                tabs: Vec::new(),
                running,
                owner_pid,
                recovered_at: None,
            },
        }
    }
}
