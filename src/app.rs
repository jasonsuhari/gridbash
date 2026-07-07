use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::{self, Stdout},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio::sync::mpsc;

use crate::{
    cli::{Cli, GridMode},
    composer::Composer,
    config::Config,
    layout::{GridLayout, GridSize, PaneId},
    orchestrator::{
        GroupColor, GroupId, MAX_GROUPS, SendBlock, SendTargets, extract_send_blocks, group_color,
        group_label, manager_pane_id,
    },
    profiles::{Profile, find_profile},
    pty::{PtyEvent, PtyPane},
    setup::{LaunchPlan, PaneLaunchSpec},
    ui, vibe,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const WORKER_RELAY_IDLE: Duration = Duration::from_millis(900);
const WORKER_RELAY_MAX_BYTES: usize = 6000;

pub struct App {
    config: Config,
    manager_profile_name: Option<String>,
    launch_plan: Option<LaunchPlan>,
    layout: GridLayout,
    grid_area: Rect,
    panes: Vec<PtyPane>,
    focus: usize,
    selected: BTreeSet<usize>,
    groups: Vec<AgentGroup>,
    next_group_id: usize,
    prompt: Option<PromptState>,
    rects: Vec<Rect>,
    broadcast: bool,
    settings: SettingsState,
    status: String,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
    event_rx: mpsc::UnboundedReceiver<PtyEvent>,
    last_activity_decay: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyOutcome {
    Continue,
    Render,
    Quit,
}

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub selected: bool,
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
}

#[derive(Debug, Clone)]
struct SettingsState {
    open: bool,
    cursor: usize,
    compact_titles: bool,
    activity_badges: bool,
    confirm_quit: bool,
    pane_density: i32,
    scrollback: i32,
    refresh_ms: i32,
    accent_index: usize,
}

struct AgentGroup {
    id: GroupId,
    palette_index: usize,
    label: char,
    workers: BTreeSet<usize>,
    manager: PtyPane,
    manager_buffer: String,
    relay_buffers: BTreeMap<usize, String>,
    last_worker_output: Option<Instant>,
    status: String,
}

struct PromptState {
    group_id: GroupId,
    input: String,
}

#[derive(Debug, Clone, Copy)]
pub struct PaneGroupView {
    pub label: char,
    pub color: GroupColor,
}

#[derive(Debug, Clone)]
pub struct PromptView {
    pub label: char,
    pub color: GroupColor,
    pub input: String,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            cursor: 0,
            compact_titles: false,
            activity_badges: true,
            confirm_quit: false,
            pane_density: 2,
            scrollback: 10_000,
            refresh_ms: 16,
            accent_index: 0,
        }
    }
}

impl SettingsState {
    const ROW_COUNT: usize = 7;
    const ACCENTS: [&'static str; 4] = ["cyan", "yellow", "green", "magenta"];

    fn move_cursor(&mut self, delta: isize) {
        let current = self.cursor as isize;
        self.cursor = (current + delta).clamp(0, Self::ROW_COUNT as isize - 1) as usize;
    }

    fn activate(&mut self) {
        match self.cursor {
            0 => self.compact_titles = !self.compact_titles,
            1 => self.activity_badges = !self.activity_badges,
            2 => self.confirm_quit = !self.confirm_quit,
            6 => self.adjust(1),
            _ => self.adjust(1),
        }
    }

    fn adjust(&mut self, delta: i32) {
        match self.cursor {
            0 => {
                if delta != 0 {
                    self.compact_titles = !self.compact_titles;
                }
            }
            1 => {
                if delta != 0 {
                    self.activity_badges = !self.activity_badges;
                }
            }
            2 => {
                if delta != 0 {
                    self.confirm_quit = !self.confirm_quit;
                }
            }
            3 => self.pane_density = (self.pane_density + delta).clamp(1, 5),
            4 => self.scrollback = (self.scrollback + delta * 1000).clamp(1_000, 50_000),
            5 => self.refresh_ms = (self.refresh_ms + delta * 4).clamp(8, 100),
            6 => {
                let count = Self::ACCENTS.len() as isize;
                self.accent_index =
                    (self.accent_index as isize + delta as isize).rem_euclid(count) as usize;
            }
            _ => {}
        }
    }

    fn rows(&self) -> Vec<SettingsRow> {
        vec![
            self.row(
                0,
                "Compact pane titles",
                switch_value(self.compact_titles),
                "sample switch",
            ),
            self.row(
                1,
                "Activity badges",
                switch_value(self.activity_badges),
                "sample switch",
            ),
            self.row(
                2,
                "Confirm before quit",
                switch_value(self.confirm_quit),
                "sample switch",
            ),
            self.row(
                3,
                "Pane density",
                self.pane_density.to_string(),
                "-/+ sample stepper",
            ),
            self.row(
                4,
                "Scrollback rows",
                self.scrollback.to_string(),
                "-/+ sample stepper",
            ),
            self.row(
                5,
                "Refresh delay",
                format!("{} ms", self.refresh_ms),
                "-/+ sample stepper",
            ),
            self.row(
                6,
                "Accent color",
                Self::ACCENTS[self.accent_index].to_string(),
                "sample choice",
            ),
        ]
    }

    fn row(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        hint: &'static str,
    ) -> SettingsRow {
        SettingsRow {
            selected: self.cursor == index,
            label,
            value,
            hint,
        }
    }
}

fn switch_value(enabled: bool) -> String {
    if enabled { "on".into() } else { "off".into() }
}

impl App {
    pub fn new(cli: Cli, config: Config) -> Result<Self> {
        let launch_plan = resolve_direct_launch_plan(&cli, &config)?;
        let manager_profile_name = resolve_manager_profile_name(&cli, &config);
        let grid = launch_plan
            .as_ref()
            .map(|plan| plan.grid)
            .unwrap_or(GridSize {
                rows: 2,
                columns: 3,
            });
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            config,
            manager_profile_name,
            launch_plan,
            layout: GridLayout::new(grid),
            grid_area: Rect::default(),
            panes: Vec::new(),
            focus: 0,
            selected: BTreeSet::new(),
            groups: Vec::new(),
            next_group_id: 0,
            prompt: None,
            rects: Vec::new(),
            broadcast: false,
            settings: SettingsState::default(),
            status:
                "Alt+arrows move | Alt+s select | Alt+g group/talk | Alt+u ungroup | Alt+o settings"
                    .into(),
            event_tx,
            event_rx,
            last_activity_decay: Instant::now(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.run_in_terminal(&mut terminal);
        teardown_terminal(&mut terminal)?;
        result
    }

    fn run_in_terminal(&mut self, terminal: &mut Tui) -> Result<()> {
        if self.launch_plan.is_none() {
            let current_dir = resolved_current_dir()?;
            let mut composer = Composer::new(current_dir);
            let Some(plan) = composer.run(terminal, &self.config)? else {
                return Ok(());
            };
            self.set_launch_plan(plan);
        }

        self.spawn_initial_panes()?;
        self.sync_initial_pane_sizes(terminal)?;
        self.run_loop(terminal)
    }

    fn set_launch_plan(&mut self, plan: LaunchPlan) {
        self.layout = GridLayout::new(plan.grid);
        self.launch_plan = Some(plan);
    }

    fn spawn_initial_panes(&mut self) -> Result<()> {
        let plan = self
            .launch_plan
            .clone()
            .ok_or_else(|| anyhow!("no launch plan selected"))?;
        self.layout = GridLayout::new(plan.grid);
        self.panes.clear();

        for (index, spec) in plan.panes.iter().enumerate() {
            self.spawn_pane_spec(index, spec, 0)?;
        }

        Ok(())
    }

    fn spawn_pane_spec(
        &mut self,
        index: usize,
        spec: &PaneLaunchSpec,
        generation: u64,
    ) -> Result<()> {
        let launch = spec.resolved_command()?;
        let pane = PtyPane::spawn(
            PaneId(index),
            generation,
            &launch.command,
            &launch.args,
            &spec.cwd,
            self.event_tx.clone(),
        )?;
        self.panes.push(pane);
        Ok(())
    }

    fn run_loop(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut needs_render = true;

        loop {
            needs_render |= self.drain_pty_events();
            needs_render |= self.decay_activity();
            needs_render |= self.relay_worker_output();

            if needs_render {
                terminal.draw(|frame| {
                    let draw_state = ui::draw(frame, self);
                    self.grid_area = draw_state.grid_area;
                    self.rects = draw_state.pane_rects;
                })?;
                self.sync_pane_sizes();
                needs_render = false;
            }

            if event::poll(INPUT_POLL_INTERVAL)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match self.handle_key(key)? {
                            KeyOutcome::Continue => {}
                            KeyOutcome::Render => needs_render = true,
                            KeyOutcome::Quit => break,
                        }
                    }
                    Event::Resize(_, _) => needs_render = true,
                    Event::Paste(text) if self.prompt.is_some() => {
                        if let Some(prompt) = &mut self.prompt {
                            prompt.input.push_str(&text);
                            needs_render = true;
                        }
                    }
                    Event::Paste(text) if !self.settings.open => {
                        self.route_input(text.as_bytes())?;
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn drain_pty_events(&mut self) -> bool {
        let mut changed = false;
        let mut dispatches = Vec::new();

        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                PtyEvent::Output {
                    pane,
                    generation,
                    bytes,
                } => {
                    if let Some(index) = self.visible_pane_index(pane, generation) {
                        let target = &mut self.panes[index];
                        target.process_output(&bytes);
                        self.capture_worker_output(index, &bytes);
                        changed = true;
                    } else if self.process_manager_output(pane, generation, &bytes, &mut dispatches)
                    {
                        changed = true;
                    }
                }
                PtyEvent::Exited { pane, generation } => {
                    if let Some(index) = self.visible_pane_index(pane, generation) {
                        let target = &mut self.panes[index];
                        if !target.exited {
                            target.exited = true;
                            changed = true;
                        }
                    } else if self.process_manager_exit(pane, generation) {
                        changed = true;
                    }
                }
            }
        }

        for pane in &mut self.panes {
            changed |= pane.poll_exit();
        }
        for group in &mut self.groups {
            let exited_now = group.manager.poll_exit();
            if exited_now {
                group.status = "manager exited".into();
            }
            changed |= exited_now;
        }

        changed |= self.dispatch_manager_commands(dispatches);

        changed
    }

    fn visible_pane_index(&self, pane: PaneId, generation: u64) -> Option<usize> {
        self.panes
            .iter()
            .position(|target| target.id() == pane && target.generation() == generation)
    }

    fn process_manager_output(
        &mut self,
        pane: PaneId,
        generation: u64,
        bytes: &[u8],
        dispatches: &mut Vec<(GroupId, SendBlock)>,
    ) -> bool {
        let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.manager.id() == pane && group.manager.generation() == generation)
        else {
            return false;
        };

        group.manager.process_output(bytes);
        group
            .manager_buffer
            .push_str(&String::from_utf8_lossy(bytes));
        for block in extract_send_blocks(&mut group.manager_buffer) {
            dispatches.push((group.id, block));
        }
        let label = group.label;
        group.status = "manager active".into();
        self.status = format!("group {label}: manager active");
        true
    }

    fn process_manager_exit(&mut self, pane: PaneId, generation: u64) -> bool {
        let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.manager.id() == pane && group.manager.generation() == generation)
        else {
            return false;
        };

        if group.manager.exited {
            return false;
        }

        let label = group.label;
        group.manager.exited = true;
        group.status = "manager exited".into();
        self.status = format!("group {label}: manager exited");
        true
    }

    fn capture_worker_output(&mut self, pane_index: usize, bytes: &[u8]) {
        let output = String::from_utf8_lossy(bytes);
        if output.trim().is_empty() {
            return;
        }

        for group in &mut self.groups {
            if !group.workers.contains(&pane_index) {
                continue;
            }

            let buffer = group.relay_buffers.entry(pane_index).or_default();
            buffer.push_str(&output);
            trim_relay_buffer(buffer);
            group.last_worker_output = Some(Instant::now());
        }
    }

    fn dispatch_manager_commands(&mut self, dispatches: Vec<(GroupId, SendBlock)>) -> bool {
        let mut changed = false;

        for (group_id, block) in dispatches {
            let Some(group_index) = self.groups.iter().position(|group| group.id == group_id)
            else {
                continue;
            };
            let workers = self.groups[group_index].workers.clone();
            let targets = match block.targets {
                SendTargets::All => workers,
                SendTargets::Panes(panes) => panes
                    .into_iter()
                    .filter_map(|pane_number| pane_number.checked_sub(1))
                    .filter(|pane_index| workers.contains(pane_index))
                    .collect::<BTreeSet<_>>(),
            };

            if targets.is_empty() {
                self.groups[group_index].status = "manager send had no valid targets".into();
                self.status = format!(
                    "group {} send skipped: no valid targets",
                    self.groups[group_index].label
                );
                changed = true;
                continue;
            }

            let bytes = paste_and_enter_bytes(&block.message);
            let mut sent = 0_usize;
            for pane_index in targets {
                if let Some(pane) = self.panes.get(pane_index)
                    && pane.write(&bytes).is_ok()
                {
                    sent += 1;
                }
            }

            let label = self.groups[group_index].label;
            self.groups[group_index].status = format!("sent to {sent} worker(s)");
            self.status = format!("group {label} manager sent to {sent} worker(s)");
            changed = true;
        }

        changed
    }

    fn relay_worker_output(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;

        for group in &mut self.groups {
            let Some(last_output) = group.last_worker_output else {
                continue;
            };
            if now.duration_since(last_output) < WORKER_RELAY_IDLE || group.relay_buffers.is_empty()
            {
                continue;
            }

            let relay = worker_relay_message(group.label, &group.relay_buffers);
            if group.manager.write(&paste_and_enter_bytes(&relay)).is_ok() {
                group.status = format!("relayed {} worker(s)", group.relay_buffers.len());
                self.status = format!("group {}: worker output relayed", group.label);
                group.relay_buffers.clear();
                group.last_worker_output = None;
                changed = true;
            }
        }

        changed
    }

    fn decay_activity(&mut self) -> bool {
        if self.last_activity_decay.elapsed() < Duration::from_millis(250) {
            return false;
        }

        let changed = self.panes.iter().any(|pane| pane.active);
        for pane in &mut self.panes {
            pane.active = false;
        }
        self.last_activity_decay = Instant::now();
        changed
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }

        if self.settings.open {
            return self.handle_settings_key(key);
        }

        if key.modifiers.contains(KeyModifiers::ALT)
            && let Some(quit) = self.handle_app_key(key)?
        {
            return Ok(if quit {
                KeyOutcome::Quit
            } else {
                KeyOutcome::Render
            });
        }

        if let Some(bytes) = terminal_key_bytes(key) {
            self.route_input(&bytes)?;
        }
        Ok(KeyOutcome::Continue)
    }

    fn handle_app_key(&mut self, key: KeyEvent) -> Result<Option<bool>> {
        match key.code {
            KeyCode::Char(ch) => self.handle_alt_char(ch),
            KeyCode::Left => {
                self.focus_previous();
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            KeyCode::Right => {
                self.focus_next();
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            KeyCode::Up => {
                self.focus_in_grid(-1);
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            KeyCode::Down => {
                self.focus_in_grid(1);
                self.status = format!("focused pane {}", self.focus + 1);
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn handle_alt_char(&mut self, ch: char) -> Result<Option<bool>> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            'q' => Ok(Some(true)),
            'b' => {
                self.broadcast = !self.broadcast;
                self.status = if self.broadcast {
                    "broadcast selected: on".into()
                } else {
                    "broadcast selected: off".into()
                };
                Ok(Some(false))
            }
            's' => {
                toggle_selection(&mut self.selected, self.focus);
                self.status = format!("selected {} panes", self.selected.len());
                Ok(Some(false))
            }
            'a' => {
                if self.selected.len() == self.panes.len() {
                    self.selected.clear();
                } else {
                    self.selected = (0..self.panes.len()).collect();
                }
                self.status = format!("selected {} panes", self.selected.len());
                Ok(Some(false))
            }
            'o' => {
                self.settings.open = true;
                self.status = "settings open".into();
                Ok(Some(false))
            }
            'g' => {
                if self.selected.is_empty() {
                    self.open_manager_prompt()?;
                } else {
                    self.create_group_from_selection()?;
                }
                Ok(Some(false))
            }
            'u' => {
                self.dissolve_focused_group();
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn create_group_from_selection(&mut self) -> Result<()> {
        if self.groups.len() >= MAX_GROUPS {
            self.status = format!("group limit reached ({MAX_GROUPS})");
            return Ok(());
        }

        let workers = self
            .selected
            .iter()
            .copied()
            .filter(|index| *index < self.panes.len())
            .collect::<BTreeSet<_>>();
        if workers.is_empty() {
            self.status = "select worker panes before grouping".into();
            return Ok(());
        }

        if let Some((pane_index, label)) = self.first_grouped_pane(&workers) {
            self.status = format!("pane {} already belongs to group {label}", pane_index + 1);
            return Ok(());
        }

        let Some(palette_index) = self.next_palette_index() else {
            self.status = format!("group limit reached ({MAX_GROUPS})");
            return Ok(());
        };
        let label = group_label(palette_index);

        let (manager_name, manager_profile) = match self.resolve_manager_profile() {
            Ok(profile) => profile,
            Err(error) => {
                self.status = format!("manager profile unavailable: {error:#}");
                return Ok(());
            }
        };
        let launch = match manager_profile.resolved_command() {
            Ok(launch) => launch,
            Err(error) => {
                self.status = format!("manager profile failed: {error:#}");
                return Ok(());
            }
        };

        let group_id = GroupId(self.next_group_id);
        self.next_group_id += 1;
        let cwd = self.group_cwd(&workers)?;
        let manager = match PtyPane::spawn(
            PaneId(manager_pane_id(group_id)),
            group_id.0 as u64 + 1,
            &launch.command,
            &launch.args,
            &cwd,
            self.event_tx.clone(),
        ) {
            Ok(manager) => manager,
            Err(error) => {
                self.status = format!("manager spawn failed: {error:#}");
                return Ok(());
            }
        };

        let intro = self.manager_intro_message(label, &workers);
        if let Err(error) = manager.write(&paste_and_enter_bytes(&intro)) {
            self.status = format!("manager init failed: {error:#}");
            return Ok(());
        }

        self.groups.push(AgentGroup {
            id: group_id,
            palette_index,
            label,
            workers,
            manager,
            manager_buffer: String::new(),
            relay_buffers: BTreeMap::new(),
            last_worker_output: None,
            status: format!("manager {manager_name} ready"),
        });
        self.selected.clear();
        self.status = format!("group {label} attached to hidden manager {manager_name}");
        Ok(())
    }

    fn open_manager_prompt(&mut self) -> Result<()> {
        let group_id = self
            .group_for_pane(self.focus)
            .or_else(|| (self.groups.len() == 1).then_some(self.groups[0].id));
        let Some(group_id) = group_id else {
            self.status = "select panes to create a group, or focus a grouped pane".into();
            return Ok(());
        };

        let Some(group) = self.groups.iter().find(|group| group.id == group_id) else {
            self.status = "group is no longer available".into();
            return Ok(());
        };

        self.prompt = Some(PromptState {
            group_id,
            input: String::new(),
        });
        self.status = format!("talking to group {} manager", group.label);
        Ok(())
    }

    fn dissolve_focused_group(&mut self) {
        let group_id = self
            .group_for_pane(self.focus)
            .or_else(|| (self.groups.len() == 1).then_some(self.groups[0].id));
        let Some(group_id) = group_id else {
            self.status = "focus a grouped pane to dissolve its group".into();
            return;
        };

        let Some(index) = self.groups.iter().position(|group| group.id == group_id) else {
            self.status = "group is no longer available".into();
            return;
        };

        let label = self.groups[index].label;
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.group_id == group_id)
        {
            self.prompt = None;
        }
        self.groups.remove(index);
        self.status = format!("group {label} dissolved");
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }

        let Some(prompt) = &mut self.prompt else {
            return Ok(KeyOutcome::Continue);
        };

        match key.code {
            KeyCode::Esc => {
                self.prompt = None;
                self.status = "manager prompt closed".into();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Enter => self.send_prompt_to_manager(),
            KeyCode::Backspace => {
                prompt.input.pop();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                prompt.input.clear();
                Ok(KeyOutcome::Render)
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                prompt.input.push(ch);
                Ok(KeyOutcome::Render)
            }
            _ => Ok(KeyOutcome::Continue),
        }
    }

    fn send_prompt_to_manager(&mut self) -> Result<KeyOutcome> {
        let Some(prompt) = self.prompt.take() else {
            return Ok(KeyOutcome::Continue);
        };
        let input = prompt.input.trim();
        if input.is_empty() {
            self.status = "manager prompt skipped".into();
            return Ok(KeyOutcome::Render);
        }

        let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.id == prompt.group_id)
        else {
            self.status = "group is no longer available".into();
            return Ok(KeyOutcome::Render);
        };

        let message = user_manager_message(group.label, input);
        match group.manager.write(&paste_and_enter_bytes(&message)) {
            Ok(()) => {
                group.status = "manager prompted".into();
                self.status = format!("sent prompt to group {} manager", group.label);
            }
            Err(error) => {
                group.status = "manager write failed".into();
                self.status = format!("manager prompt failed: {error:#}");
            }
        }

        Ok(KeyOutcome::Render)
    }

    fn first_grouped_pane(&self, workers: &BTreeSet<usize>) -> Option<(usize, char)> {
        workers.iter().find_map(|pane_index| {
            self.groups
                .iter()
                .find(|group| group.workers.contains(pane_index))
                .map(|group| (*pane_index, group.label))
        })
    }

    fn next_palette_index(&self) -> Option<usize> {
        let used = self
            .groups
            .iter()
            .map(|group| group.palette_index)
            .collect::<BTreeSet<_>>();
        (0..MAX_GROUPS).find(|index| !used.contains(index))
    }

    fn resolve_manager_profile(&self) -> Result<(String, Profile)> {
        let name = self
            .manager_profile_name
            .as_deref()
            .ok_or_else(|| anyhow!("set --manager-profile or [defaults].manager_profile"))?;

        if let Ok(profile) = find_profile(&self.config, name) {
            return Ok((name.to_string(), profile));
        }

        let profiles = vibe::load_profiles()?;
        let profile = vibe::profile_for_name(name, &profiles)
            .ok_or_else(|| anyhow!("vibe profile '{name}' is missing or not ready"))?;
        Ok((name.to_string(), profile))
    }

    fn group_cwd(&self, workers: &BTreeSet<usize>) -> Result<PathBuf> {
        let Some(first_worker) = workers.iter().next() else {
            return resolved_current_dir();
        };
        Ok(self
            .panes
            .get(*first_worker)
            .map(|pane| pane.cwd().to_path_buf())
            .unwrap_or(resolved_current_dir()?))
    }

    fn manager_intro_message(&self, label: char, workers: &BTreeSet<usize>) -> String {
        let worker_lines = workers
            .iter()
            .map(|pane_index| {
                format!(
                    "- pane {}: {}",
                    pane_index + 1,
                    self.worker_label(*pane_index)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "You are the hidden GridBash manager for group {label}.\n\
            Coordinate only these worker panes:\n\
            {worker_lines}\n\n\
            When you need GridBash to send instructions to workers, emit a fenced block whose opening line is three backticks immediately followed by one of these commands:\n\
            gridbash send all\n\
            gridbash send panes 1, 3\n\
            Put only the worker instruction text inside that fence.\n\n\
            I will relay worker output snapshots back to you. Keep routing blocks concise and only target panes in this group."
        )
    }

    fn worker_label(&self, pane_index: usize) -> String {
        let folder = self
            .pane_folder(pane_index)
            .map(str::to_string)
            .unwrap_or_else(|| {
                self.panes
                    .get(pane_index)
                    .map(|pane| pane.cwd().display().to_string())
                    .unwrap_or_else(|| "unknown cwd".into())
            });
        match self.pane_worktree(pane_index) {
            Some(worktree) => format!("{folder} ({worktree})"),
            None => folder,
        }
    }

    fn group_for_pane(&self, pane_index: usize) -> Option<GroupId> {
        self.groups
            .iter()
            .find(|group| group.workers.contains(&pane_index))
            .map(|group| group.id)
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('q')) {
            return Ok(KeyOutcome::Quit);
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O'))
        {
            self.settings.open = false;
            self.status = "settings closed".into();
            return Ok(KeyOutcome::Render);
        }

        let changed = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings.open = false;
                self.status = "settings closed".into();
                true
            }
            KeyCode::Up => {
                self.settings.move_cursor(-1);
                true
            }
            KeyCode::Down => {
                self.settings.move_cursor(1);
                true
            }
            KeyCode::Left | KeyCode::Char('-') => {
                self.settings.adjust(-1);
                true
            }
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => {
                self.settings.adjust(1);
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings.activate();
                true
            }
            _ => false,
        };

        Ok(if changed {
            KeyOutcome::Render
        } else {
            KeyOutcome::Continue
        })
    }

    fn route_input(&mut self, bytes: &[u8]) -> Result<()> {
        let targets = self.input_targets();
        for index in targets {
            self.panes
                .get(index)
                .ok_or_else(|| anyhow!("invalid pane index {index}"))?
                .write(bytes)?;
        }
        Ok(())
    }

    fn input_targets(&self) -> Vec<usize> {
        if self.broadcast && !self.selected.is_empty() {
            self.selected.iter().copied().collect()
        } else {
            vec![self.focus.min(self.panes.len().saturating_sub(1))]
        }
    }

    fn focus_next(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.focus = (self.focus + 1) % self.panes.len();
    }

    fn focus_previous(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.focus = if self.focus == 0 {
            self.panes.len() - 1
        } else {
            self.focus - 1
        };
    }

    fn focus_in_grid(&mut self, row_delta: isize) {
        if self.panes.is_empty() {
            return;
        }

        let columns = self.layout.size().columns;
        let candidate = if row_delta.is_negative() {
            self.focus.saturating_sub(columns)
        } else {
            self.focus.saturating_add(columns)
        };
        if candidate < self.panes.len() {
            self.focus = candidate;
        }
    }

    pub fn pane_rects(&self, area: Rect) -> Vec<Rect> {
        self.layout.rects(area, self.panes.len())
    }

    pub fn panes(&self) -> &[PtyPane] {
        &self.panes
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn selected(&self) -> &BTreeSet<usize> {
        &self.selected
    }

    pub fn broadcast(&self) -> bool {
        self.broadcast
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn settings_open(&self) -> bool {
        self.settings.open
    }

    pub fn settings_rows(&self) -> Vec<SettingsRow> {
        self.settings.rows()
    }

    pub fn pane_group(&self, index: usize) -> Option<PaneGroupView> {
        self.groups
            .iter()
            .find(|group| group.workers.contains(&index))
            .map(|group| PaneGroupView {
                label: group.label,
                color: group_color(group.palette_index),
            })
    }

    pub fn prompt_view(&self) -> Option<PromptView> {
        let prompt = self.prompt.as_ref()?;
        let group = self
            .groups
            .iter()
            .find(|group| group.id == prompt.group_id)?;
        Some(PromptView {
            label: group.label,
            color: group_color(group.palette_index),
            input: prompt.input.clone(),
        })
    }

    pub fn pane_folder(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .map(|pane| pane.folder_name.as_str())
    }

    pub fn pane_worktree(&self, index: usize) -> Option<&str> {
        self.launch_plan
            .as_ref()
            .and_then(|plan| plan.panes.get(index))
            .and_then(|pane| pane.worktree_name.as_deref())
    }

    pub fn sync_pane_sizes(&mut self) {
        for (index, rect) in self.rects.iter().enumerate() {
            let Some(pane) = self.panes.get_mut(index) else {
                continue;
            };

            let rows = rect.height.saturating_sub(2).max(1);
            let cols = rect.width.saturating_sub(2).max(1);
            if let Err(error) = pane.resize(rows, cols) {
                self.status = format!("resize failed: {error:#}");
            }
        }
    }

    fn sync_initial_pane_sizes(&mut self, terminal: &Tui) -> Result<()> {
        let size = terminal.size().context("failed to read terminal size")?;
        self.grid_area = Rect::new(0, 0, size.width, size.height.saturating_sub(1));
        self.rects = self.pane_rects(self.grid_area);
        self.sync_pane_sizes();
        Ok(())
    }
}

fn resolve_grid(cli: &Cli) -> Result<GridSize> {
    if let Some(grid) = &cli.grid {
        return GridSize::parse(grid).with_context(|| format!("invalid grid '{grid}'"));
    }

    if cli.layout == GridMode::Auto {
        return Ok(GridSize::from_count(cli.count.unwrap_or(6)));
    }

    if let Some(count) = cli.count {
        return Ok(GridSize::from_count(count));
    }

    Ok(GridSize {
        rows: 2,
        columns: 3,
    })
}

fn resolve_direct_launch_plan(cli: &Cli, config: &Config) -> Result<Option<LaunchPlan>> {
    if !uses_direct_launch(cli) {
        return Ok(None);
    }

    let grid = resolve_grid(cli)?;
    let profile_name = resolve_profile_name(cli, config);
    let profile = find_profile(config, &profile_name)?;
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or(env::current_dir().context("failed to resolve current directory")?);
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let pane_count = cli.count.unwrap_or_else(|| grid.count()).clamp(1, 100);

    Ok(Some(LaunchPlan::legacy(
        profile_name,
        profile,
        cwd,
        pane_count,
        grid,
    )))
}

fn uses_direct_launch(cli: &Cli) -> bool {
    cli.grid.is_some()
        || cli.count.is_some()
        || cli.profile.is_some()
        || cli.cwd.is_some()
        || cli.layout == GridMode::Auto
}

fn resolve_profile_name(cli: &Cli, config: &Config) -> String {
    cli.profile
        .clone()
        .or_else(|| env::var("GRIDBASH_PROFILE").ok())
        .or_else(|| config.defaults.profile.clone())
        .unwrap_or_else(|| "git-bash".into())
}

fn resolve_manager_profile_name(cli: &Cli, config: &Config) -> Option<String> {
    cli.manager_profile
        .clone()
        .or_else(|| env::var("GRIDBASH_MANAGER_PROFILE").ok())
        .or_else(|| config.defaults.manager_profile.clone())
}

fn resolved_current_dir() -> Result<std::path::PathBuf> {
    let current = env::current_dir().context("failed to resolve current directory")?;
    Ok(current.canonicalize().unwrap_or(current))
}

fn trim_relay_buffer(buffer: &mut String) {
    if buffer.len() > WORKER_RELAY_MAX_BYTES {
        let keep_from = buffer.len().saturating_sub(WORKER_RELAY_MAX_BYTES);
        buffer.drain(..keep_from);
    }
}

fn worker_relay_message(label: char, buffers: &BTreeMap<usize, String>) -> String {
    let mut message = format!("GridBash worker output snapshot for group {label}.");
    for (pane_index, output) in buffers {
        message.push_str(&format!(
            "\n\n[pane {} output]\n{}",
            pane_index + 1,
            output.trim()
        ));
    }
    message
}

fn user_manager_message(label: char, input: &str) -> String {
    format!(
        "User instruction for GridBash group {label}:\n{input}\n\nRoute work to workers with gridbash send blocks when needed."
    )
}

fn paste_and_enter_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 16);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~\r");
    bytes
}

fn toggle_selection(selected: &mut BTreeSet<usize>, index: usize) {
    if !selected.insert(index) {
        selected.remove(&index);
    }
}

fn control_byte(ch: char) -> Option<u8> {
    let lower = ch.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        Some((lower as u8) - b'a' + 1)
    } else {
        match ch {
            '[' => Some(0x1b),
            '\\' => Some(0x1c),
            ']' => Some(0x1d),
            '^' => Some(0x1e),
            '_' => Some(0x1f),
            _ => None,
        }
    }
}

fn terminal_key_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }

    match key.code {
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::F(number) => bytes.extend_from_slice(function_key_sequence(number)?),
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            bytes.push(control_byte(ch)?);
        }
        KeyCode::Char(ch) => {
            let mut buffer = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
        }
        _ => return None,
    }

    Some(bytes)
}

fn function_key_sequence(number: u8) -> Option<&'static [u8]> {
    match number {
        1 => Some(b"\x1bOP"),
        2 => Some(b"\x1bOQ"),
        3 => Some(b"\x1bOR"),
        4 => Some(b"\x1bOS"),
        5 => Some(b"\x1b[15~"),
        6 => Some(b"\x1b[17~"),
        7 => Some(b"\x1b[18~"),
        8 => Some(b"\x1b[19~"),
        9 => Some(b"\x1b[20~"),
        10 => Some(b"\x1b[21~"),
        11 => Some(b"\x1b[23~"),
        12 => Some(b"\x1b[24~"),
        _ => None,
    }
}

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to create terminal")
}

fn teardown_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
