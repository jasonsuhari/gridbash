use std::{collections::BTreeSet, io::Stdout, path::PathBuf, time::Duration};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    setup::{LaunchPlan, LaunchSelection, launch_selection_from},
    vibe::{self, VibeProfile},
};

type ComposerTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct Composer {
    stage: Stage,
    vibe_profiles: Vec<VibeProfile>,
    vibe_error: Option<String>,
    folders: Vec<PathBuf>,
    folder_cursor: usize,
    path_input: String,
    agent_cursor: usize,
    selected_agents: BTreeSet<usize>,
    preview_selection: Option<LaunchSelection>,
    status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Folders,
    PathInput,
    Agents,
    Preview,
}

enum ComposerEvent {
    Continue,
    Launch(LaunchPlan),
    Quit,
}

impl Composer {
    pub fn new(current_dir: PathBuf) -> Self {
        let (vibe_profiles, vibe_error) = match vibe::load_profiles() {
            Ok(profiles) => (profiles, None),
            Err(error) => (Vec::new(), Some(format!("{error:#}"))),
        };
        let selected_agents = default_selected_agents(&vibe_profiles);

        Self {
            stage: Stage::Folders,
            vibe_profiles,
            vibe_error,
            folders: vec![current_dir],
            folder_cursor: 0,
            path_input: String::new(),
            agent_cursor: 0,
            selected_agents,
            preview_selection: None,
            status: "Folders: Enter continues | a adds | d removes".into(),
        }
    }

    pub fn run(&mut self, terminal: &mut ComposerTerminal) -> Result<Option<LaunchPlan>> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(Duration::from_millis(50))? {
                continue;
            }

            let event = event::read()?;
            let result = match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key)?,
                Event::Paste(text) => self.handle_paste(text),
                _ => ComposerEvent::Continue,
            };

            match result {
                ComposerEvent::Continue => {}
                ComposerEvent::Launch(plan) => return Ok(Some(plan)),
                ComposerEvent::Quit => return Ok(None),
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
        if matches!(key.code, KeyCode::Char('q')) && self.stage != Stage::PathInput {
            return Ok(ComposerEvent::Quit);
        }

        match self.stage {
            Stage::Folders => self.handle_folders_key(key),
            Stage::PathInput => self.handle_path_input_key(key),
            Stage::Agents => self.handle_agents_key(key),
            Stage::Preview => self.handle_preview_key(key),
        }
    }

    fn handle_paste(&mut self, text: String) -> ComposerEvent {
        if self.stage == Stage::PathInput {
            self.path_input.push_str(text.trim());
        }
        ComposerEvent::Continue
    }

    fn handle_folders_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
        if let Some(error) = &self.vibe_error {
            self.status = format!("vibe is required before launch: {error}");
            return Ok(ComposerEvent::Continue);
        }

        match key.code {
            KeyCode::Up => self.folder_cursor = self.folder_cursor.saturating_sub(1),
            KeyCode::Down => {
                self.folder_cursor = (self.folder_cursor + 1).min(self.folders.len());
            }
            KeyCode::Char('a') => {
                self.path_input.clear();
                self.stage = Stage::PathInput;
                self.status = "Paste or type a folder path, then Enter".into();
            }
            KeyCode::Char('d') => {
                if self.folders.len() > 1 && self.folder_cursor < self.folders.len() {
                    self.folders.remove(self.folder_cursor);
                    self.folder_cursor = self.folder_cursor.min(self.folders.len() - 1);
                }
            }
            KeyCode::Enter | KeyCode::Right => {
                if self.folder_cursor >= self.folders.len() {
                    self.path_input.clear();
                    self.stage = Stage::PathInput;
                } else {
                    self.stage = Stage::Agents;
                    self.status = "Agents: Space toggles | Enter previews".into();
                }
            }
            KeyCode::Esc => self.status = "Press q to quit".into(),
            _ => {}
        }

        Ok(ComposerEvent::Continue)
    }

    fn handle_path_input_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
        match key.code {
            KeyCode::Enter => {
                let candidate = PathBuf::from(self.path_input.trim());
                if let Ok(path) = candidate.canonicalize() {
                    if path.is_dir() {
                        if !self.folders.iter().any(|folder| folder == &path) {
                            self.folders.push(path);
                        }
                        self.folder_cursor = self.folders.len().saturating_sub(1);
                        self.path_input.clear();
                        self.stage = Stage::Folders;
                        self.status = "Folder added".into();
                    } else {
                        self.status = "Path exists, but it is not a folder".into();
                    }
                } else {
                    self.status = "Folder path does not exist".into();
                }
            }
            KeyCode::Esc => {
                self.path_input.clear();
                self.stage = Stage::Folders;
            }
            KeyCode::Backspace => {
                self.path_input.pop();
            }
            KeyCode::Char(ch) => self.path_input.push(ch),
            _ => {}
        }

        Ok(ComposerEvent::Continue)
    }

    fn handle_agents_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
        let max = self.vibe_profiles.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => self.agent_cursor = self.agent_cursor.saturating_sub(1),
            KeyCode::Down => self.agent_cursor = (self.agent_cursor + 1).min(max),
            KeyCode::Char(' ') => self.toggle_agent(self.agent_cursor),
            KeyCode::Char('a') => self.select_all_ready_agents(),
            KeyCode::Enter | KeyCode::Right => {
                let agents = self.selected_agent_names();
                if agents.is_empty() {
                    self.status = "Select at least one logged-in vibe profile".into();
                } else {
                    self.preview_selection =
                        Some(launch_selection_from(self.folders.clone(), agents)?);
                    self.stage = Stage::Preview;
                    self.status = "Enter launches | Esc returns".into();
                }
            }
            KeyCode::Esc => self.stage = Stage::Folders,
            _ => {}
        }

        Ok(ComposerEvent::Continue)
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
        match key.code {
            KeyCode::Enter => self.launch_preview(),
            KeyCode::Esc => {
                self.stage = Stage::Agents;
                Ok(ComposerEvent::Continue)
            }
            _ => Ok(ComposerEvent::Continue),
        }
    }

    fn launch_preview(&mut self) -> Result<ComposerEvent> {
        let Some(preview) = &self.preview_selection else {
            self.status = "No launch preview selected".into();
            return Ok(ComposerEvent::Continue);
        };

        match preview.launch_plan() {
            Ok(plan) => Ok(ComposerEvent::Launch(plan)),
            Err(error) => {
                self.status = format!("{error:#}");
                Ok(ComposerEvent::Continue)
            }
        }
    }

    fn toggle_agent(&mut self, index: usize) {
        let Some(profile) = self.vibe_profiles.get(index) else {
            return;
        };
        if !profile.ready {
            self.status = format!("{} is not logged in", profile.name);
            return;
        }

        if !self.selected_agents.insert(index) {
            self.selected_agents.remove(&index);
        }
    }

    fn select_all_ready_agents(&mut self) {
        self.selected_agents = self
            .vibe_profiles
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| profile.ready.then_some(index))
            .collect();
        self.status = format!("Selected {} ready agents", self.selected_agents.len());
    }

    fn selected_agent_names(&self) -> Vec<String> {
        self.selected_agents
            .iter()
            .filter_map(|index| self.vibe_profiles.get(*index))
            .filter(|profile| profile.ready)
            .map(|profile| profile.name.clone())
            .collect()
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(header(), chunks[0]);
        match self.stage {
            Stage::Folders => self.draw_folders(frame, chunks[1]),
            Stage::PathInput => self.draw_path_input(frame, chunks[1]),
            Stage::Agents => self.draw_agents(frame, chunks[1]),
            Stage::Preview => self.draw_preview(frame, chunks[1]),
        }
        frame.render_widget(status_bar(&self.status), chunks[2]);
    }

    fn draw_folders(&self, frame: &mut Frame<'_>, area: Rect) {
        if let Some(error) = &self.vibe_error {
            let lines = vec![
                Line::from(Span::styled(
                    "vibe is required for orchestration",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(error.clone()),
                Line::from(""),
                Line::from("Install or repair vibe, then reopen GridBash."),
                Line::from("Press q to quit."),
            ];
            frame.render_widget(panel("Launch", lines), area);
            return;
        }

        let mut lines = vec![Line::from("Choose where agents should work.")];
        lines.push(Line::from(""));
        for (index, folder) in self.folders.iter().enumerate() {
            lines.push(selectable_line(
                self.folder_cursor == index,
                &format!("{}  {}", index + 1, folder.display()),
            ));
        }
        lines.push(selectable_line(
            self.folder_cursor >= self.folders.len(),
            "+ Add folder",
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Enter continues on a folder row. Use a to add, d to remove.",
        ));

        frame.render_widget(panel("Folders", lines), area);
    }

    fn draw_path_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let lines = vec![
            Line::from("Paste or type a folder path."),
            Line::from(""),
            Line::from(vec![
                Span::styled("> ", Style::default().fg(Color::Cyan)),
                Span::raw(self.path_input.clone()),
            ]),
            Line::from(""),
            Line::from("Enter adds the folder. Esc returns."),
        ];

        frame.render_widget(panel("Add Folder", lines), area);
    }

    fn draw_agents(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines = vec![Line::from("Choose the vibe profiles to launch.")];
        lines.push(Line::from(""));
        for (index, profile) in self.vibe_profiles.iter().enumerate() {
            let selected = self.selected_agents.contains(&index);
            let marker = if selected { "[x]" } else { "[ ]" };
            let status_style = if profile.ready {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            let prefix = if self.agent_cursor == index {
                "> "
            } else {
                "  "
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, cursor_style(self.agent_cursor == index)),
                Span::raw(format!("{marker} {:<18}", profile.name)),
                Span::styled(profile.status.clone(), status_style),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Space toggles. a selects all ready agents. Enter previews.",
        ));

        frame.render_widget(panel("Agents", lines), area);
    }

    fn draw_preview(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines = Vec::new();
        let Some(preview) = &self.preview_selection else {
            frame.render_widget(
                panel("Preview", vec![Line::from("No launch preview selected")]),
                area,
            );
            return;
        };

        lines.push(Line::from("Launch preview"));
        lines.push(Line::from(""));
        match preview.launch_plan() {
            Ok(plan) => {
                for (index, pane) in plan.panes.iter().enumerate() {
                    lines.push(Line::from(format!(
                        "Pane {:>2}: {:<18} -> {} ({})",
                        index + 1,
                        pane.profile_name,
                        pane.folder_name,
                        pane.cwd.display()
                    )));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(format!(
                    "Grid: {} pane(s), auto layout {}x{}",
                    plan.panes.len(),
                    plan.grid.rows,
                    plan.grid.columns
                )));
            }
            Err(error) => lines.push(Line::from(Span::styled(
                format!("{error:#}"),
                Style::default().fg(Color::Red),
            ))),
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Enter launches. Esc returns."));

        frame.render_widget(panel("Preview", lines), area);
    }
}

fn default_selected_agents(profiles: &[VibeProfile]) -> BTreeSet<usize> {
    profiles
        .iter()
        .enumerate()
        .filter_map(|(index, profile)| profile.ready.then_some(index))
        .take(3)
        .collect()
}

fn header<'a>() -> Paragraph<'a> {
    Paragraph::new(vec![
        Line::from(vec![Span::styled(
            " GridBash ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("Guided agent launch"),
    ])
    .style(Style::default().bg(Color::Rgb(11, 15, 20)))
}

fn status_bar<'a>(status: &str) -> Paragraph<'a> {
    Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(status.to_string(), Style::default().fg(Color::Gray)),
    ]))
    .style(Style::default().bg(Color::Rgb(11, 15, 20)))
}

fn panel<'a>(title: &'a str, lines: Vec<Line<'a>>) -> Paragraph<'a> {
    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} "))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(
            Style::default()
                .fg(Color::Rgb(230, 237, 243))
                .bg(Color::Rgb(11, 15, 20)),
        )
}

fn selectable_line<'a>(selected: bool, text: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(if selected { "> " } else { "  " }, cursor_style(selected)),
        Span::styled(
            text.to_string(),
            if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
    ])
}

fn cursor_style(selected: bool) -> Style {
    if selected {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}
