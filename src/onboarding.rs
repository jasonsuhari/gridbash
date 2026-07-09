use std::{
    env, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
};

use crate::{
    cli::Cli,
    config::Config,
    profiles::{Profile, resolve_executable, terminal_profiles},
};

type OnboardingTerminal = Terminal<CrosstermBackend<io::Stdout>>;

const LOAD_DURATION: Duration = Duration::from_millis(900);
const TICK_RATE: Duration = Duration::from_millis(50);
const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
const MASCOT_ART: [&str; 7] = [
    "    .--------.",
    "   /  [] []  \\",
    "  |    >_    |",
    "  |  \\____/  |",
    "   \\__====__/",
    "     /|GB|\\",
    "    /_|__|_\\",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingResult {
    Continue,
    Quit,
}

#[derive(Debug, Clone)]
struct TerminalChoice {
    name: String,
    title: String,
    command: String,
    executable: PathBuf,
}

pub fn should_run(cli: &Cli, config: &Config) -> bool {
    cli.command.is_none()
        && cli.profile.is_none()
        && env::var_os("GRIDBASH_PROFILE").is_none()
        && config.defaults.profile.is_none()
}

pub fn run(config: &mut Config, config_path: Option<&Path>) -> Result<OnboardingResult> {
    let choices = detected_terminal_choices(config);
    if choices.is_empty() {
        return Err(anyhow!(
            "no terminal profiles detected; install Git Bash, PowerShell, or cmd"
        ));
    }

    let Some(profile) = run_picker(&choices)? else {
        return Ok(OnboardingResult::Quit);
    };

    config.set_default_profile(profile);
    config.save(config_path)?;
    Ok(OnboardingResult::Continue)
}

fn detected_terminal_choices(config: &Config) -> Vec<TerminalChoice> {
    terminal_profiles(config)
        .into_iter()
        .filter_map(|(name, profile)| terminal_choice(name, profile))
        .collect()
}

fn terminal_choice(name: String, profile: Profile) -> Option<TerminalChoice> {
    let executable = resolve_executable(&profile.command)?;
    Some(TerminalChoice {
        title: profile.display_name(&name),
        command: format_command(&profile),
        executable,
        name,
    })
}

fn format_command(profile: &Profile) -> String {
    std::iter::once(profile.command.clone())
        .chain(profile.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_picker(choices: &[TerminalChoice]) -> Result<Option<String>> {
    let mut terminal = setup_terminal()?;
    let result = run_picker_loop(&mut terminal, choices);
    let cleanup = teardown_terminal(&mut terminal);
    cleanup?;
    result
}

fn setup_terminal() -> Result<OnboardingTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(Into::into)
}

fn teardown_terminal(terminal: &mut OnboardingTerminal) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), Show, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_picker_loop(
    terminal: &mut OnboardingTerminal,
    choices: &[TerminalChoice],
) -> Result<Option<String>> {
    let started = Instant::now();
    let mut selected = 0usize;

    loop {
        let elapsed = started.elapsed();
        let loading = elapsed < LOAD_DURATION;

        terminal.draw(|frame| {
            if loading {
                draw_loading(frame, elapsed, choices.len());
            } else {
                draw_picker(frame, choices, selected);
            }
        })?;

        if !event::poll(TICK_RATE)? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        if key.kind != KeyEventKind::Press {
            continue;
        }

        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            return Ok(None);
        }

        if loading {
            continue;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                selected = selected.checked_sub(1).unwrap_or(choices.len() - 1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1) % choices.len();
            }
            KeyCode::Enter => return Ok(Some(choices[selected].name.clone())),
            _ => {}
        }
    }
}

fn draw_loading(frame: &mut Frame<'_>, elapsed: Duration, detected_count: usize) {
    let area = frame.area();
    frame.render_widget(background(), area);

    let panel = centered_rect(area, 88, 13);
    let block = setup_block(" first run ");
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    if let Some((mascot_area, content_area)) = mascot_layout(inner) {
        render_mascot(frame, mascot_area);
        render_loading_content(frame, content_area, elapsed, detected_count, false);
    } else {
        render_loading_content(frame, inner, elapsed, detected_count, true);
    }
}

fn render_loading_content(
    frame: &mut Frame<'_>,
    area: Rect,
    elapsed: Duration,
    detected_count: usize,
    include_compact_mascot: bool,
) {
    let spinner = SPINNER[((elapsed.as_millis() / 110) as usize) % SPINNER.len()];
    let ratio = (elapsed.as_secs_f64() / LOAD_DURATION.as_secs_f64()).clamp(0.0, 1.0);
    let mut lines = Vec::new();
    if include_compact_mascot {
        lines.extend(compact_mascot_lines());
    }
    lines.extend([
        Line::from(Span::styled(
            "GridBash setup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(spinner, Style::default().fg(Color::Yellow)),
            Span::raw(" detecting available terminals"),
        ]),
        Line::from(format!("found {detected_count} terminal profile(s)")),
    ]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(setup_panel_style()),
        chunks[0],
    );

    frame.render_widget(
        Gauge::default()
            .ratio(ratio)
            .gauge_style(Style::default().fg(Color::Cyan))
            .label(""),
        inset_x(chunks[2], 2),
    );
}

fn draw_picker(frame: &mut Frame<'_>, choices: &[TerminalChoice], selected: usize) {
    let area = frame.area();
    frame.render_widget(background(), area);

    let height = (choices.len() as u16)
        .saturating_mul(2)
        .saturating_add(13)
        .min(area.height.max(1));
    let panel = centered_rect(area, 88, height);
    let block = setup_block(" terminal setup ");
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    if let Some((mascot_area, content_area)) = mascot_layout(inner) {
        render_mascot(frame, mascot_area);
        render_picker_content(frame, content_area, choices, selected, false);
    } else {
        render_picker_content(frame, inner, choices, selected, true);
    }
}

fn render_picker_content(
    frame: &mut Frame<'_>,
    area: Rect,
    choices: &[TerminalChoice],
    selected: usize,
    include_compact_mascot: bool,
) {
    let mut lines = Vec::new();
    if include_compact_mascot {
        lines.extend(compact_mascot_lines());
    }

    lines.extend([
        Line::from(Span::styled(
            "Choose your default terminal",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "GridBash will save this to your config and use it for new panes.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
    ]);

    for (index, choice) in choices.iter().enumerate() {
        let is_selected = index == selected;
        let marker = if is_selected { ">" } else { " " };
        let row_style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, row_style),
            Span::raw(" "),
            Span::styled(choice.title.as_str(), row_style),
            Span::raw("  "),
            Span::styled(
                format!("({})", choice.name),
                Style::default().fg(Color::Gray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                choice.command.as_str(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                choice.executable.display().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Up/Down move | Enter choose | q quit",
        Style::default().fg(Color::Gray),
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(setup_panel_style()),
        area,
    );
}

fn render_mascot(frame: &mut Frame<'_>, area: Rect) {
    let mut lines = Vec::with_capacity(MASCOT_ART.len() + 3);
    lines.push(Line::from(Span::styled(
        "BashBot",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "setup sidekick",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));
    lines.extend(
        MASCOT_ART
            .iter()
            .map(|line| Line::from(Span::styled(*line, Style::default().fg(Color::LightCyan)))),
    );

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(setup_panel_style()),
        area,
    );
}

fn compact_mascot_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                "BashBot",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" [>_]", Style::default().fg(Color::LightCyan)),
            Span::styled(" ready to grid", Style::default().fg(Color::Gray)),
        ]),
        Line::from(""),
    ]
}

fn mascot_layout(area: Rect) -> Option<(Rect, Rect)> {
    if area.width < 74 || area.height < 10 {
        return None;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22),
            Constraint::Length(2),
            Constraint::Min(40),
        ])
        .split(area);

    Some((chunks[0], chunks[2]))
}

fn background() -> Paragraph<'static> {
    Paragraph::new("").style(Style::default().bg(Color::Rgb(11, 15, 20)))
}

fn setup_panel_style() -> Style {
    Style::default().bg(Color::Rgb(11, 15, 20))
}

fn setup_block(title: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan))
        .style(setup_panel_style())
}

fn inset_x(area: Rect, x: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(x),
        y: area.y,
        width: area.width.saturating_sub(x.saturating_mul(2)),
        height: area.height,
    }
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
