use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Quit,
    Help,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ToggleSelection,
    ToggleGridSelection,
    SelectAll,
    SleepPanes,
    RestartPanes,
    NextTab,
    NewTab,
    CloseGrid,
    ResizeGrid,
    SwapPanes,
    ZoomPane,
    CommandLine,
    CommandPalette,
    CaptureOutput,
    ToggleOutputLogging,
    VoiceInput,
    Settings,
    PreviousPanes,
    PaneActivity,
    Ports,
    CopyMode,
    BackgroundPanes,
    BackgroundJobs,
    AuthProfiles,
    RenameTab,
    RenamePane,
    AdoptTerminal,
}

const ACTIONS: &[Action] = &[
    Action::FocusLeft,
    Action::FocusRight,
    Action::FocusUp,
    Action::FocusDown,
    Action::ToggleSelection,
    Action::ToggleGridSelection,
    Action::SelectAll,
    Action::NewTab,
    Action::NextTab,
    Action::CloseGrid,
    Action::RenameTab,
    Action::CommandLine,
    Action::CommandPalette,
    Action::CaptureOutput,
    Action::ToggleOutputLogging,
    Action::PaneActivity,
    Action::Ports,
    Action::PreviousPanes,
    Action::CopyMode,
    Action::BackgroundPanes,
    Action::BackgroundJobs,
    Action::AuthProfiles,
    Action::ZoomPane,
    Action::ResizeGrid,
    Action::RenamePane,
    Action::AdoptTerminal,
    Action::RestartPanes,
    Action::SwapPanes,
    Action::SleepPanes,
    Action::Settings,
    Action::VoiceInput,
    Action::Quit,
    Action::Help,
];

impl Action {
    pub fn all() -> &'static [Self] {
        ACTIONS
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Help => "help",
            Self::FocusLeft => "focus-left",
            Self::FocusRight => "focus-right",
            Self::FocusUp => "focus-up",
            Self::FocusDown => "focus-down",
            Self::ToggleSelection => "toggle-selection",
            Self::ToggleGridSelection => "toggle-grid-selection",
            Self::SelectAll => "select-all",
            Self::SleepPanes => "sleep-panes",
            Self::RestartPanes => "restart-panes",
            Self::NextTab => "next-tab",
            Self::NewTab => "new-tab",
            Self::CloseGrid => "close-grid",
            Self::ResizeGrid => "resize-grid",
            Self::SwapPanes => "swap-panes",
            Self::ZoomPane => "zoom-pane",
            Self::CommandLine => "command-line",
            Self::CommandPalette => "command-palette",
            Self::CaptureOutput => "capture-output",
            Self::ToggleOutputLogging => "toggle-output-logging",
            Self::VoiceInput => "voice-input",
            Self::Settings => "settings",
            Self::PreviousPanes => "previous-panes",
            Self::PaneActivity => "pane-activity",
            Self::Ports => "ports",
            Self::CopyMode => "copy-mode",
            Self::BackgroundPanes => "background-panes",
            Self::BackgroundJobs => "background-jobs",
            Self::AuthProfiles => "auth-profiles",
            Self::RenameTab => "rename-tab",
            Self::RenamePane => "rename-pane",
            Self::AdoptTerminal => "adopt-terminal",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Quit => "quit GridBash",
            Self::Help => "open or close help",
            Self::FocusLeft => "focus pane to the left",
            Self::FocusRight => "focus pane to the right",
            Self::FocusUp => "focus pane above",
            Self::FocusDown => "focus pane below",
            Self::ToggleSelection => "toggle pane selection",
            Self::ToggleGridSelection => "toggle current grid selection",
            Self::SelectAll => "select or clear all panes",
            Self::SleepPanes => "sleep or wake panes",
            Self::RestartPanes => "restart exited panes",
            Self::NextTab => "switch to next tab",
            Self::NewTab => "open a new tab",
            Self::CloseGrid => "close current grid",
            Self::ResizeGrid => "resize the grid",
            Self::SwapPanes => "swap selected panes or grids",
            Self::ZoomPane => "zoom or restore focused pane",
            Self::CommandLine => "open or close BashBot Director command center",
            Self::CommandPalette => "open searchable command palette",
            Self::CaptureOutput => "capture target pane output",
            Self::ToggleOutputLogging => "start or stop target pane logging",
            Self::VoiceInput => "dictate without submitting",
            Self::Settings => "open settings and profiles",
            Self::PreviousPanes => "show previous panes",
            Self::PaneActivity => "show focused-pane activity",
            Self::Ports => "show ports used by coding agents",
            Self::CopyMode => "search and copy pane scrollback",
            Self::BackgroundPanes => "background selected or focused panes",
            Self::BackgroundJobs => "show background agents",
            Self::AuthProfiles => "manage and assign auth profiles",
            Self::RenameTab => "rename current tab",
            Self::RenamePane => "rename focused pane",
            Self::AdoptTerminal => "open a pane where an outside Git Bash window is",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        ACTIONS.iter().copied().find(|action| action.name() == name)
    }

    fn default_chord(self) -> &'static str {
        match self {
            Self::Quit => "alt+q",
            Self::Help => "alt+h",
            Self::FocusLeft => "alt+left",
            Self::FocusRight => "alt+right",
            Self::FocusUp => "alt+up",
            Self::FocusDown => "alt+down",
            Self::ToggleSelection => "alt+s",
            Self::ToggleGridSelection => "alt+shift+s",
            Self::SelectAll => "alt+a",
            Self::SleepPanes => "alt+z",
            Self::RestartPanes => "alt+shift+t",
            Self::NextTab => "alt+t",
            Self::NewTab => "alt+n",
            Self::CloseGrid => "alt+w",
            Self::ResizeGrid => "alt+l",
            Self::SwapPanes => "alt+x",
            Self::ZoomPane => "alt+f",
            Self::CommandLine => "alt+c",
            Self::CommandPalette => "alt+k",
            Self::CaptureOutput => "alt+shift+c",
            Self::ToggleOutputLogging => "alt+shift+l",
            Self::VoiceInput => "alt+shift+v",
            Self::Settings => "alt+o",
            Self::PreviousPanes => "alt+shift+p",
            Self::PaneActivity => "alt+p",
            Self::Ports => "ctrl+alt+p",
            Self::CopyMode => "alt+b",
            Self::BackgroundPanes => "alt+shift+b",
            Self::BackgroundJobs => "ctrl+alt+b",
            Self::AuthProfiles => "alt+shift+a",
            Self::RenameTab => "alt+shift+r",
            Self::RenamePane => "alt+r",
            Self::AdoptTerminal => "ctrl+alt+t",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ShortcutKey {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Shortcut {
    control: bool,
    alt: bool,
    shift: bool,
    key: ShortcutKey,
}

impl Shortcut {
    fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            bail!("shortcut cannot be empty");
        }

        let mut parts = normalized.split('+').collect::<Vec<_>>();
        let key = parts.pop().ok_or_else(|| anyhow!("shortcut needs a key"))?;
        let mut shortcut = Self {
            control: false,
            alt: false,
            shift: false,
            key: parse_key(key)?,
        };

        for modifier in parts {
            let slot = match modifier {
                "ctrl" | "control" => &mut shortcut.control,
                "alt" => &mut shortcut.alt,
                "shift" => &mut shortcut.shift,
                "" => bail!("shortcut contains an empty '+' segment"),
                other => bail!("unknown shortcut modifier '{other}'"),
            };
            if *slot {
                bail!("shortcut repeats modifier '{modifier}'");
            }
            *slot = true;
        }

        if !shortcut.control
            && !shortcut.alt
            && !shortcut.shift
            && !matches!(shortcut.key, ShortcutKey::Function(_))
        {
            bail!("unmodified characters and navigation keys belong to the terminal");
        }
        if shortcut == fallback_help_shortcut() {
            bail!("F1 is reserved as the help recovery key");
        }

        Ok(shortcut)
    }

    fn matches(self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if self.control != modifiers.contains(KeyModifiers::CONTROL)
            || self.alt != modifiers.contains(KeyModifiers::ALT)
            || self.shift != modifiers.contains(KeyModifiers::SHIFT)
        {
            return false;
        }

        match (self.key, event.code) {
            (ShortcutKey::Char(expected), KeyCode::Char(actual)) => {
                expected == actual.to_ascii_lowercase()
            }
            (ShortcutKey::Left, KeyCode::Left)
            | (ShortcutKey::Right, KeyCode::Right)
            | (ShortcutKey::Up, KeyCode::Up)
            | (ShortcutKey::Down, KeyCode::Down) => true,
            (ShortcutKey::Function(expected), KeyCode::F(actual)) => expected == actual,
            _ => false,
        }
    }

    fn label(self) -> String {
        let mut parts = Vec::new();
        if self.control {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        parts.push(match self.key {
            ShortcutKey::Char(ch) => ch.to_ascii_uppercase().to_string(),
            ShortcutKey::Left => "Left".into(),
            ShortcutKey::Right => "Right".into(),
            ShortcutKey::Up => "Up".into(),
            ShortcutKey::Down => "Down".into(),
            ShortcutKey::Function(number) => format!("F{number}"),
        });
        parts.join("+")
    }
}

fn parse_key(value: &str) -> Result<ShortcutKey> {
    match value {
        "left" => Ok(ShortcutKey::Left),
        "right" => Ok(ShortcutKey::Right),
        "up" => Ok(ShortcutKey::Up),
        "down" => Ok(ShortcutKey::Down),
        value if value.len() == 1 => value
            .chars()
            .next()
            .map(ShortcutKey::Char)
            .ok_or_else(|| anyhow!("unknown shortcut key '{value}'")),
        value if value.starts_with('f') => {
            let number = value[1..]
                .parse::<u8>()
                .map_err(|_| anyhow!("unknown shortcut key '{value}'"))?;
            if (1..=12).contains(&number) {
                Ok(ShortcutKey::Function(number))
            } else {
                bail!("function key must be between F1 and F12");
            }
        }
        other => bail!("unknown shortcut key '{other}'"),
    }
}

/// The name the `[keys]` table uses for the leader key.
const LEADER_KEY: &str = "leader";

/// The leader shortcut a platform starts with.
///
/// macOS terminals spend the Option key on character composition — Option+C is
/// `ç`, not Alt+C — so out of the box a Mac cannot press a single GridBash
/// shortcut. A leader key gives every one of them back without asking the user
/// to reconfigure their terminal before the app is usable. Everywhere else Alt
/// arrives intact, and taking a Ctrl key away from the panes would cost more
/// than it returns, so there is no leader unless one is configured.
#[cfg(target_os = "macos")]
const DEFAULT_LEADER: Option<&str> = Some("ctrl+g");
#[cfg(not(target_os = "macos"))]
const DEFAULT_LEADER: Option<&str> = None;

/// Values that turn the leader off in config.
const LEADER_DISABLED: [&str; 4] = ["", "off", "none", "false"];

#[derive(Debug, Clone)]
pub struct KeyBindings {
    bindings: BTreeMap<Action, Shortcut>,
    leader: Option<Shortcut>,
}

impl KeyBindings {
    pub fn from_overrides(overrides: &BTreeMap<String, String>) -> Result<Self> {
        let mut bindings = ACTIONS
            .iter()
            .copied()
            .map(|action| {
                Shortcut::parse(action.default_chord()).map(|shortcut| (action, shortcut))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut leader = DEFAULT_LEADER.map(Shortcut::parse).transpose()?;

        for (name, chord) in overrides {
            let normalized = name.trim().to_ascii_lowercase();
            if matches!(normalized.as_str(), "bashbot" | "edit-goal" | "stop-goal") {
                bail!(
                    "[keys] action '{name}' was removed; use 'command-line' and the Alt+C BashBot Director command center"
                );
            }
            if normalized == LEADER_KEY {
                let requested = chord.trim().to_ascii_lowercase();
                leader = if LEADER_DISABLED.contains(&requested.as_str()) {
                    None
                } else {
                    Some(Shortcut::parse(chord).map_err(|error| {
                        anyhow!("invalid [keys].leader shortcut '{chord}': {error}")
                    })?)
                };
                continue;
            }
            let action = Action::from_name(&normalized).ok_or_else(|| {
                anyhow!(
                    "unknown [keys] action '{name}'; supported actions: {LEADER_KEY}, {}",
                    ACTIONS
                        .iter()
                        .map(|action| action.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            let shortcut = Shortcut::parse(chord)
                .map_err(|error| anyhow!("invalid [keys].{name} shortcut '{chord}': {error}"))?;
            if shortcut == fallback_quit_shortcut() && action != Action::Quit {
                bail!("Alt+Q is reserved as the quit recovery key");
            }
            bindings.insert(action, shortcut);
        }

        let mut seen = BTreeMap::new();
        for (action, shortcut) in &bindings {
            if let Some(previous) = seen.insert(*shortcut, *action) {
                bail!(
                    "shortcut {} is assigned to both '{}' and '{}'",
                    shortcut.label(),
                    previous.name(),
                    action.name()
                );
            }
        }

        // A leader that is also a shortcut would swallow that shortcut, since
        // the leader is read before anything else.
        if let Some(leader) = leader
            && let Some(action) = seen.get(&leader)
        {
            bail!(
                "shortcut {} is the leader key and is also assigned to '{}'",
                leader.label(),
                action.name()
            );
        }

        Ok(Self { bindings, leader })
    }

    /// Whether this event is the leader key itself.
    pub fn is_leader(&self, event: &KeyEvent) -> bool {
        self.leader.is_some_and(|shortcut| shortcut.matches(event))
    }

    /// How the leader key is written for the user, when there is one.
    pub fn leader_label(&self) -> Option<String> {
        self.leader.map(Shortcut::label)
    }

    pub fn action_for(&self, event: &KeyEvent) -> Option<Action> {
        ACTIONS.iter().copied().find(|action| {
            self.bindings
                .get(action)
                .is_some_and(|shortcut| shortcut.matches(event))
        })
    }

    pub fn help_entries(&self) -> Vec<(String, &'static str)> {
        self.leader_label()
            .map(|label| (label, "leader: press it, then a shortcut key without Alt"))
            .into_iter()
            .chain(ACTIONS.iter().copied().map(|action| {
                // `from_overrides` seeds every action, but indexing a map that
                // is one action short would panic instead of degrading.
                let label = match self.shortcut_for(action) {
                    None => "unbound".to_string(),
                    Some(shortcut) => match action {
                        Action::Quit if shortcut != fallback_quit_shortcut() => {
                            format!("{} / Alt+Q", shortcut.label())
                        }
                        Action::Help => format!("{} / F1", shortcut.label()),
                        _ => shortcut.label(),
                    },
                };
                (label, action.description())
            }))
            .collect()
    }

    pub fn label_for(&self, action: Action) -> String {
        self.shortcut_for(action)
            .map_or_else(|| "unbound".to_string(), Shortcut::label)
    }

    fn shortcut_for(&self, action: Action) -> Option<Shortcut> {
        self.bindings
            .get(&action)
            .copied()
            .or_else(|| Shortcut::parse(action.default_chord()).ok())
    }
}

pub fn is_quit_recovery(event: &KeyEvent) -> bool {
    fallback_quit_shortcut().matches(event)
}

pub fn is_help_recovery(event: &KeyEvent) -> bool {
    fallback_help_shortcut().matches(event)
}

fn fallback_quit_shortcut() -> Shortcut {
    Shortcut {
        control: false,
        alt: true,
        shift: false,
        key: ShortcutKey::Char('q'),
    }
}

/// The same key press with Alt held.
///
/// The leader stands in for Alt, so the key after it is matched as though Alt
/// had been held: leader then `C` runs Alt+C, and leader then `Ctrl+P` runs
/// Ctrl+Alt+P.
pub fn with_leader_alt(event: &KeyEvent) -> KeyEvent {
    let mut event = *event;
    event.modifiers |= KeyModifiers::ALT;
    // A terminal that reports a capital letter without also reporting Shift
    // would otherwise reach none of the Alt+Shift bindings through the leader.
    if matches!(event.code, KeyCode::Char(ch) if ch.is_ascii_uppercase()) {
        event.modifiers |= KeyModifiers::SHIFT;
    }
    event
}

fn fallback_help_shortcut() -> Shortcut {
    Shortcut {
        control: false,
        alt: false,
        shift: false,
        key: ShortcutKey::Function(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn overrides(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    /// The help overlay and every shortcut label used to index the binding map
    /// directly, which panics for an action the map is missing.
    #[test]
    fn labels_survive_a_binding_map_that_is_missing_an_action() {
        let mut bindings = KeyBindings::from_overrides(&overrides(&[])).expect("default bindings");
        bindings.bindings.clear();

        assert_eq!(
            bindings.label_for(Action::ZoomPane),
            Shortcut::parse(Action::ZoomPane.default_chord())
                .expect("default chord")
                .label(),
            "a missing binding falls back to the action's default chord"
        );
        let leader_rows = usize::from(bindings.leader_label().is_some());
        assert_eq!(bindings.help_entries().len(), ACTIONS.len() + leader_rows);
        assert!(
            bindings
                .action_for(&KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT))
                .is_none()
        );
    }

    #[test]
    fn normalizes_and_dispatches_custom_shortcuts() {
        let bindings = KeyBindings::from_overrides(&overrides(&[("zoom-pane", "Ctrl+Shift+K")]))
            .expect("custom bindings");

        assert_eq!(
            bindings.action_for(&KeyEvent::new(
                KeyCode::Char('K'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(Action::ZoomPane)
        );
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)),
            None
        );
    }

    #[test]
    fn rejects_duplicate_and_unknown_bindings() {
        let duplicate = KeyBindings::from_overrides(&overrides(&[("zoom-pane", "alt+l")]))
            .expect_err("duplicate binding");
        assert!(duplicate.to_string().contains("resize-grid"));

        let unknown = KeyBindings::from_overrides(&overrides(&[("warp-pane", "alt+w")]))
            .expect_err("unknown action");
        assert!(unknown.to_string().contains("unknown [keys] action"));
    }

    #[test]
    fn removed_director_shortcuts_have_a_clear_migration_error() {
        for action in ["bashbot", "edit-goal", "stop-goal"] {
            let error = KeyBindings::from_overrides(&overrides(&[(action, "alt+d")]))
                .expect_err("removed action");
            assert!(error.to_string().contains("Alt+C BashBot Director"));
        }
    }

    #[test]
    fn former_director_shortcuts_are_free_by_default() {
        let bindings = KeyBindings::from_overrides(&BTreeMap::new()).expect("default bindings");
        for key in ['d', 'g', 'u'] {
            assert_eq!(
                bindings.action_for(&KeyEvent::new(KeyCode::Char(key), KeyModifiers::ALT)),
                None
            );
        }
    }

    #[test]
    fn preserves_terminal_input_and_recovery_keys() {
        let plain = KeyBindings::from_overrides(&overrides(&[("zoom-pane", "k")]))
            .expect_err("plain terminal key");
        assert!(plain.to_string().contains("belong to the terminal"));

        let f1 = KeyBindings::from_overrides(&overrides(&[("zoom-pane", "f1")]))
            .expect_err("help recovery key");
        assert!(f1.to_string().contains("F1 is reserved"));

        let alt_q =
            KeyBindings::from_overrides(&overrides(&[("quit", "ctrl+q"), ("zoom-pane", "alt+q")]))
                .expect_err("quit recovery key");
        assert!(alt_q.to_string().contains("Alt+Q is reserved"));
    }

    /// The whole point of the leader: a Mac that never sends Alt can still
    /// reach every shortcut.
    #[test]
    fn the_leader_stands_in_for_alt() {
        let bindings =
            KeyBindings::from_overrides(&overrides(&[("leader", "ctrl+g")])).expect("leader");

        assert!(bindings.is_leader(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)));
        assert_eq!(bindings.leader_label().as_deref(), Some("Ctrl+G"));

        // Plain letters reach the Alt bindings.
        assert_eq!(
            bindings.action_for(&with_leader_alt(&KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::NONE
            ))),
            Some(Action::CommandLine)
        );
        // Arrows too, which are the shortcuts most likely to be reached for.
        assert_eq!(
            bindings.action_for(&with_leader_alt(&KeyEvent::new(
                KeyCode::Left,
                KeyModifiers::NONE
            ))),
            Some(Action::FocusLeft)
        );
        // Ctrl+Alt chords need only their Ctrl half after the leader.
        assert_eq!(
            bindings.action_for(&with_leader_alt(&KeyEvent::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL
            ))),
            Some(Action::Ports)
        );
        // An unbound key resolves to nothing rather than to a near miss.
        assert_eq!(
            bindings.action_for(&with_leader_alt(&KeyEvent::new(
                KeyCode::Char('9'),
                KeyModifiers::NONE
            ))),
            None
        );
    }

    /// Alt+Shift bindings are reachable through the leader whether or not the
    /// terminal reports Shift alongside the capital letter.
    #[test]
    fn the_leader_reaches_shifted_shortcuts_either_way() {
        let bindings =
            KeyBindings::from_overrides(&overrides(&[("leader", "ctrl+g")])).expect("leader");

        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            assert_eq!(
                bindings.action_for(&with_leader_alt(&KeyEvent::new(
                    KeyCode::Char('C'),
                    modifiers
                ))),
                Some(Action::CaptureOutput),
                "{modifiers:?}"
            );
        }
    }

    #[test]
    fn the_leader_can_be_turned_off_and_cannot_shadow_a_shortcut() {
        for value in ["off", "none", "false", ""] {
            let bindings = KeyBindings::from_overrides(&overrides(&[("leader", value)]))
                .expect("leader override");
            assert_eq!(bindings.leader_label(), None, "{value}");
            assert!(!bindings.is_leader(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)));
        }

        let clash = KeyBindings::from_overrides(&overrides(&[
            ("leader", "ctrl+g"),
            ("zoom-pane", "ctrl+g"),
        ]))
        .expect_err("leader that shadows a shortcut");
        assert!(clash.to_string().contains("leader key"));

        let invalid = KeyBindings::from_overrides(&overrides(&[("leader", "ctrl+shift+nope")]))
            .expect_err("invalid leader");
        assert!(invalid.to_string().contains("[keys].leader"));
    }

    /// Whatever the platform default is, it must parse and must not collide
    /// with a shipped binding.
    #[test]
    fn the_platform_default_leader_is_usable() {
        let bindings = KeyBindings::from_overrides(&BTreeMap::new()).expect("default bindings");
        assert_eq!(
            bindings.leader_label(),
            DEFAULT_LEADER.map(|chord| {
                Shortcut::parse(chord)
                    .expect("default leader parses")
                    .label()
            })
        );
        assert_eq!(bindings.leader_label().is_some(), cfg!(target_os = "macos"));
    }

    #[test]
    fn help_entries_show_effective_and_recovery_bindings() {
        let bindings = KeyBindings::from_overrides(&overrides(&[
            ("help", "ctrl+shift+h"),
            ("quit", "ctrl+q"),
        ]))
        .expect("custom recovery actions");
        let entries = bindings.help_entries();

        assert!(entries.iter().any(|entry| entry.0 == "Ctrl+Shift+H / F1"));
        assert!(entries.iter().any(|entry| entry.0 == "Ctrl+Q / Alt+Q"));
    }

    #[test]
    fn action_names_are_unique() {
        let names = ACTIONS
            .iter()
            .map(|action| action.name())
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), ACTIONS.len());
    }

    #[test]
    fn grid_selection_has_a_pane_selection_companion_shortcut() {
        let bindings = KeyBindings::from_overrides(&BTreeMap::new()).expect("default bindings");
        assert_eq!(
            bindings.action_for(&KeyEvent::new(
                KeyCode::Char('S'),
                KeyModifiers::ALT | KeyModifiers::SHIFT,
            )),
            Some(Action::ToggleGridSelection)
        );
    }

    #[test]
    fn close_grid_has_a_modeless_default_shortcut() {
        let bindings = KeyBindings::from_overrides(&BTreeMap::new()).expect("default bindings");
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT)),
            Some(Action::CloseGrid)
        );
    }

    #[test]
    fn agent_ports_has_a_modeless_default_shortcut() {
        let bindings = KeyBindings::from_overrides(&BTreeMap::new()).expect("default bindings");
        assert_eq!(
            bindings.action_for(&KeyEvent::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            )),
            Some(Action::Ports)
        );
    }
}
