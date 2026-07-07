use std::{
    collections::BTreeSet,
    io::Stdout,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
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
    config::Config,
    setup::{LaunchPlan, SavedSetup, sanitize_setup_name, setup_from_selection},
    vibe::{self, VibeProfile},
};

type ComposerTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct Composer {
    stage: Stage,
    vibe_profiles: Vec<VibeProfile>,
    vibe_error: Option<String>,
    saved_names: Vec<String>,
    home_cursor: usize,
    folders: Vec<PathBuf>,
    folder_cursor: usize,
    path_input: String,
    agent_cursor: usize,
    selected_agents: BTreeSet<usize>,
    active_setup: Option<ActiveSetup>,
    name_input: String,
    status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Home,
    Folders,
    PathInput,
    Agents,
    Preview,
    NameInput,
}

#[derive(Debug, Clone)]
struct ActiveSetup {
    name: Option<String>,
    setup: SavedSetup,
    from_saved: bool,
}

enum ComposerEvent {
    Continue,
    Launch(LaunchPlan),
    Quit,
}

impl Composer {
    pub fn new(config: &Config, current_dir: PathBuf) -> Self {
        let (vibe_profiles, vibe_error) = match vibe::load_profiles() {
            Ok(profiles) => (profiles, None),
            Err(error) => (Vec::new(), Some(format!("{error:#}"))),
        };
        let selected_agents = default_selected_agents(&vibe_profiles);
        let saved_names = config.setups.keys().cloned().collect::<Vec<_>>();

        Self {
            stage: Stage::Home,
            vibe_profiles,
            vibe_error,
            saved_names,
            home_cursor: 0,
            folders: vec![current_dir],
            folder_cursor: 0,
            path_input: String::new(),
            agent_cursor: 0,
            selected_agents,
            active_setup: None,
            name_input: String::new(),
            status: "Enter selects | q quits".into(),
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut ComposerTerminal,
        config: &mut Config,
        config_path: Option<&Path>,
    ) -> Result<Option<LaunchPlan>> {
        loop {
            terminal.draw(|frame| self.draw(frame, config))?;

            if !event::poll(Duration::from_millis(50))? {
                continue;
            }

            let event = event::read()?;
            let result = match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    self.handle_key(key, config, config_path)?
                }
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

    fn handle_key(
        &mut self,
        key: KeyEvent,
        config: &mut Config,
        config_path: Option<&Path>,
    ) -> Result<ComposerEvent> {
        if matches!(key.code, KeyCode::Char('q')) && self.stage != Stage::PathInput {
            return Ok(ComposerEvent::Quit);
        }

        match self.stage {
            Stage::Home => self.handle_home_key(key, config),
            Stage::Folders => self.handle_folders_key(key),
            Stage::PathInput => self.handle_path_input_key(key),
            Stage::Agents => self.handle_agents_key(key),
            Stage::Preview => self.handle_preview_key(key),
            Stage::NameInput => self.handle_name_input_key(key, config, config_path),
        }
    }

    fn handle_paste(&mut self, text: String) -> ComposerEvent {
        if self.stage == Stage::PathInput {
            self.path_input.push_str(text.trim());
        }
        ComposerEvent::Continue
    }

    fn handle_home_key(&mut self, key: KeyEvent, config: &Config) -> Result<ComposerEvent> {
        let max = self.saved_names.len();
        match key.code {
            KeyCode::Up => self.home_cursor = self.home_cursor.saturating_sub(1),
            KeyCode::Down => self.home_cursor = (self.home_cursor + 1).min(max),
            KeyCode::Enter => {
                if self.vibe_error.is_some() {
                    self.status = "Install or fix vibe before launching orchestrated agents".into();
                    return Ok(ComposerEvent::Continue);
                }

                if self.home_cursor == 0 {
                    self.stage = Stage::Folders;
                    self.status = "Folders: Enter continues | a adds | d removes".into();
                } else if let Some(name) = self.saved_names.get(self.home_cursor - 1) {
                    if let Some(setup) = config.setups.get(name).cloned() {
                        self.active_setup = Some(ActiveSetup {
                            name: Some(name.clone()),
                            setup,
                            from_saved: true,
                        });
                        self.stage = Stage::Preview;
                        self.status = "Enter launches | Esc returns".into();
                    }
                }
            }
            _ => {}
        }

        Ok(ComposerEvent::Continue)
    }

    fn handle_folders_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
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
            KeyCode::Esc => self.stage = Stage::Home,
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
                    let setup = setup_from_selection(self.folders.clone(), agents)?;
                    self.active_setup = Some(ActiveSetup {
                        name: None,
                        setup,
                        from_saved: false,
                    });
                    self.stage = Stage::Preview;
                    self.status = "Enter launches | s saves and launches | Esc returns".into();
                }
            }
            KeyCode::Esc => self.stage = Stage::Folders,
            _ => {}
        }

        Ok(ComposerEvent::Continue)
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> Result<ComposerEvent> {
        match key.code {
            KeyCode::Enter => self.launch_active_setup(),
            KeyCode::Char('s') => {
                if self
                    .active_setup
                    .as_ref()
                    .is_some_and(|setup| setup.from_saved)
                {
                    self.status = "Saved setup is already named; press Enter to launch".into();
                } else {
                    self.name_input.clear();
                    self.stage = Stage::NameInput;
                    self.status = "Name this setup, then Enter".into();
                }
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Esc => {
                self.stage = if self
                    .active_setup
                    .as_ref()
                    .is_some_and(|setup| setup.from_saved)
                {
                    Stage::Home
                } else {
                    Stage::Agents
                };
                Ok(ComposerEvent::Continue)
            }
            _ => Ok(ComposerEvent::Continue),
        }
    }

    fn handle_name_input_key(
        &mut self,
        key: KeyEvent,
        config: &mut Config,
        config_path: Option<&Path>,
    ) -> Result<ComposerEvent> {
        match key.code {
            KeyCode::Enter => {
                let Some(name) = sanitize_setup_name(&self.name_input) else {
                    self.status = "Use at least one letter or number in the setup name".into();
                    return Ok(ComposerEvent::Continue);
                };
                let Some(active) = &mut self.active_setup else {
                    self.stage = Stage::Home;
                    return Ok(ComposerEvent::Continue);
                };
                active.name = Some(name.clone());
                config.save_setup(name.clone(), active.setup.clone());
                config
                    .save(config_path)
                    .context("failed to save named setup")?;
                if !self.saved_names.iter().any(|existing| existing == &name) {
                    self.saved_names.push(name.clone());
                    self.saved_names.sort();
                }
                self.launch_active_setup()
            }
            KeyCode::Esc => {
                self.name_input.clear();
                self.stage = Stage::Preview;
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Backspace => {
                self.name_input.pop();
                Ok(ComposerEvent::Continue)
            }
            KeyCode::Char(ch) => {
                self.name_input.push(ch);
                Ok(ComposerEvent::Continue)
            }
            _ => Ok(ComposerEvent::Continue),
        }
    }

    fn launch_active_setup(&mut self) -> Result<ComposerEvent> {
        let Some(active) = &self.active_setup else {
            self.status = "No setup selected".into();
            return Ok(ComposerEvent::Continue);
        };

        match active.setup.launch_plan() {
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

    fn draw(&self, frame: &mut Frame<'_>, config: &Config) {
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
            Stage::Home => self.draw_home(frame, chunks[1], config),
            Stage::Folders => self.draw_folders(frame, chunks[1]),
            Stage::PathInput => self.draw_path_input(frame, chunks[1]),
            Stage::Agents => self.draw_agents(frame, chunks[1]),
            Stage::Preview => self.draw_preview(frame, chunks[1]),
            Stage::NameInput => self.draw_name_input(frame, chunks[1]),
        }
        frame.render_widget(status_bar(&self.status), chunks[2]);
    }

    fn draw_home(&self, frame: &mut Frame<'_>, area: Rect, config: &Config) {
        let mut lines = Vec::new();
        if let Some(error) = &self.vibe_error {
            lines.push(Line::from(Span::styled(
                "vibe is required for orchestration",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(error.clone()));
            lines.push(Line::from(""));
            lines.push(Line::from("Install or repair vibe, then reopen GridBash."));
            lines.push(Line::from("Press q to quit."));
            frame.render_widget(panel("Setup", lines), area);
            return;
        }

        lines.push(selectable_line(self.home_cursor == 0, "New setup"));
        for (index, name) in self.saved_names.iter().enumerate() {
            let selected = self.home_cursor == index + 1;
            let summary = config
                .setups
                .get(name)
                .map(setup_summary)
                .unwrap_or_else(|| "saved setup".into());
            lines.push(selectable_line(selected, &format!("{name}  {summary}")));
        }
        if self.saved_names.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from("No named setups yet."));
        }

        frame.render_widget(panel("Choose Setup", lines), area);
    }

    fn draw_folders(&self, frame: &mut Frame<'_>, area: Rect) {
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
        let Some(active) = &self.active_setup else {
            frame.render_widget(
                panel("Preview", vec![Line::from("No setup selected")]),
                area,
            );
            return;
        };

        let title = active.name.as_deref().unwrap_or("Unsaved setup");
        lines.push(Line::from(format!("Setup: {title}")));
        lines.push(Line::from(""));
        match active.setup.launch_plan() {
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
        lines.push(Line::from(
            "Enter launches. s saves and launches. Esc returns.",
        ));

        frame.render_widget(panel("Preview", lines), area);
    }

    fn draw_name_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let lines = vec![
            Line::from("Name this setup for future launches."),
            Line::from(""),
            Line::from(vec![
                Span::styled("> ", Style::default().fg(Color::Cyan)),
                Span::raw(self.name_input.clone()),
            ]),
            Line::from(""),
            Line::from("Names are saved in lowercase kebab-case. Esc returns."),
        ];

        frame.render_widget(panel("Save Setup", lines), area);
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

fn setup_summary(setup: &SavedSetup) -> String {
    format!(
        "{} folder(s), {} agent(s)",
        setup.folders.len(),
        setup.agents.len()
    )
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
        Line::from("Guided agent setup"),
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
