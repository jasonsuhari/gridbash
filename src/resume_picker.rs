use std::{
    collections::BTreeSet,
    io::{self, Stdout},
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::{
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

use crate::session::{SessionRecord, process_is_running};

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

        if all_panes(record).any(|pane| pane.host.is_some()) {
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
                "The previous client stopped. GridBash will recover this workspace.".into()
            }
            Self::Detached => "Saved pane hosts are ready to reconnect.".into(),
            Self::Saved => "Recreates terminals from the saved layout and history.".into(),
        }
    }
}

enum PickerAction {
    Continue,
    Select(usize),
    Cancel,
}

struct ResumePicker<'a> {
    sessions: &'a [SessionRecord],
    list_state: ListState,
    page_size: usize,
    notice: Option<String>,
}

pub fn select_session(sessions: &[SessionRecord]) -> Result<Option<SessionRecord>> {
    let mut terminal = setup_terminal()?;
    let mut picker = ResumePicker::new(sessions);
    let result = picker.run(&mut terminal);
    let teardown_result = teardown_terminal(&mut terminal);

    match (result, teardown_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(selection), Ok(())) => Ok(selection),
    }
}

impl<'a> ResumePicker<'a> {
    fn new(sessions: &'a [SessionRecord]) -> Self {
        let mut list_state = ListState::default();
        list_state.select((!sessions.is_empty()).then_some(0));
        Self {
            sessions,
            list_state,
            page_size: 1,
            notice: None,
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
                PickerAction::Select(index) => return Ok(Some(self.sessions[index].clone())),
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        let Some(selected) = self.list_state.selected() else {
            return PickerAction::Cancel;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => PickerAction::Cancel,
            KeyCode::Up | KeyCode::Char('k') => {
                self.select(selected.saturating_sub(1));
                PickerAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select((selected + 1).min(self.sessions.len().saturating_sub(1)));
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
            KeyCode::Enter => {
                if let SessionState::Open(owner_pid) =
                    SessionState::for_record(&self.sessions[selected])
                {
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
        self.notice = None;
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

        let (header_height, detail_height, controls_height) = if inner.height >= 22 {
            (3, 9, 3)
        } else {
            (2, 7, 2)
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
        self.draw_details(frame, chunks[1]);
        self.draw_sessions(frame, chunks[2]);
        self.draw_controls(frame, chunks[3]);
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let selected = self.list_state.selected().unwrap_or(0) + 1;
        let selected_state = self
            .list_state
            .selected()
            .and_then(|index| self.sessions.get(index))
            .map(SessionState::for_record)
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
                    "Saved terminals, layout, and working context.",
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

    fn draw_details(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = panel_block("SELECTED SESSION");
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        let Some(record) = self
            .list_state
            .selected()
            .and_then(|index| self.sessions.get(index))
        else {
            return;
        };

        let state = SessionState::for_record(record);
        let session = &record.session;
        let panes = all_panes(record).count();
        let tabs = session.tabs.len() + 1;
        let folders = compact_labels(all_panes(record).map(|pane| pane.folder_name.as_str()));
        let profiles = compact_labels(all_panes(record).map(|pane| pane.profile_name.as_str()));
        let host_count = all_panes(record).filter(|pane| pane.host.is_some()).count();
        let resume_mode = if host_count > 0 {
            format!(
                "reconnect {host_count} PTY host{}",
                if host_count == 1 { "" } else { "s" }
            )
        } else {
            "recreate from snapshot".into()
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    session_title(record),
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
            detail_row("SESSION ID", session.id.clone()),
            detail_row(
                "WORKSPACE",
                format!(
                    "{}x{} | {panes} pane{} | {tabs} tab{} | {resume_mode}",
                    session.grid.rows,
                    session.grid.columns,
                    if panes == 1 { "" } else { "s" },
                    if tabs == 1 { "" } else { "s" },
                ),
            ),
            detail_row("FOLDERS", folders.unwrap_or_else(|| "Unknown".into())),
            detail_row("PROFILES", profiles.unwrap_or_else(|| "Unknown".into())),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(RAISED_BG)),
            inner,
        );
    }

    fn draw_sessions(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let block = panel_block("RECENT SESSIONS");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.page_size = usize::from((inner.height / 2).max(1));

        let items = self
            .sessions
            .iter()
            .map(|record| {
                let state = SessionState::for_record(record);
                ListItem::new(vec![
                    Line::from(vec![
                        state_badge(state),
                        Span::raw(" "),
                        Span::styled(
                            session_title(record),
                            Style::default().fg(SOFT_GREEN).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("           ", Style::default().fg(MUTED)),
                        Span::styled(record.summary(), Style::default().fg(MUTED)),
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
                keycap("UP/DOWN or J/K"),
                Span::styled(" NAVIGATE   ", Style::default().fg(MUTED)),
                launch_keycap("ENTER"),
                Span::styled(" RESUME   ", Style::default().fg(MUTED)),
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

fn all_panes(record: &SessionRecord) -> impl Iterator<Item = &crate::session::SavedPane> {
    record
        .session
        .panes
        .iter()
        .chain(record.session.tabs.iter().flat_map(|tab| tab.panes.iter()))
        .chain(record.session.background_panes.iter().map(|job| &job.pane))
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
        let _ = disable_raw_mode();
        return Err(error).context("failed to enter alternate screen");
    }
    let backend = CrosstermBackend::new(stdout);
    match Terminal::new(backend) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            let _ = disable_raw_mode();
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen);
            Err(error).context("failed to create resume terminal")
        }
    }
}

fn teardown_terminal(terminal: &mut ResumeTerminal) -> Result<()> {
    disable_raw_mode().context("failed to disable raw terminal mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to restore cursor")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        profiles::Profile,
        session::{SavedGrid, SavedPane, SavedPaneHistory, SavedSession},
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
        let details_at = rendered.find("SELECTED SESSION").expect("details panel");
        let sessions_at = rendered.find("RECENT SESSIONS").expect("sessions panel");
        assert!(
            details_at < sessions_at,
            "details should render above sessions"
        );
        assert!(rendered.contains("gridbash resume"));
        assert!(rendered.contains("Fluent workspace"));
        assert!(rendered.contains("DETACHED"));
        assert!(!rendered.contains("01 /"));
        assert!(!rendered.contains("02 /"));
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.fg == TERMINAL_GREEN)
        );
    }

    fn record(title: &str, running: bool, owner_pid: Option<u32>, host: bool) -> SessionRecord {
        let pane = SavedPane {
            index: 0,
            profile_name: "codex".into(),
            command: Profile {
                command: "codex".into(),
                args: Vec::new(),
                title: Some("Codex".into()),
                agent_kind: None,
            },
            cwd: PathBuf::from("fluent"),
            folder_name: "fluent".into(),
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            history: SavedPaneHistory::default(),
            codex_thread_id: None,
            host: host.then(|| crate::pane_host::PtyHostRef {
                endpoint: "127.0.0.1:12345".into(),
                token: "token".into(),
                codex_sqlite_home: None,
                started_at_ms: None,
            }),
        };
        SessionRecord {
            path: PathBuf::from("session.toml"),
            session: SavedSession {
                version: 1,
                id: "session-id".into(),
                started_at: 1,
                updated_at: 1,
                title: title.into(),
                grid: SavedGrid {
                    rows: 1,
                    columns: 1,
                },
                panes: vec![pane],
                background_panes: Vec::new(),
                tabs: Vec::new(),
                running,
                owner_pid,
                recovered_at: None,
            },
        }
    }
}
