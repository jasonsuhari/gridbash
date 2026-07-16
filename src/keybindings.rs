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
    SelectAll,
    SleepPanes,
    RestartPanes,
    NextTab,
    NewTab,
    ResizeGrid,
    SwapPanes,
    ZoomPane,
    CommandLine,
    CommandPalette,
    BashBot,
    CaptureOutput,
    ToggleOutputLogging,
    VoiceInput,
    EditGoal,
    StopGoal,
    Settings,
    PreviousPanes,
    PaneActivity,
    CopyMode,
    AuthProfiles,
    RenameTab,
    RenamePane,
}

const ACTIONS: &[Action] = &[
    Action::FocusLeft,
    Action::FocusRight,
    Action::FocusUp,
    Action::FocusDown,
    Action::ToggleSelection,
    Action::SelectAll,
    Action::NewTab,
    Action::NextTab,
    Action::RenameTab,
    Action::CommandLine,
    Action::CommandPalette,
    Action::BashBot,
    Action::CaptureOutput,
    Action::ToggleOutputLogging,
    Action::PaneActivity,
    Action::PreviousPanes,
    Action::CopyMode,
    Action::AuthProfiles,
    Action::ZoomPane,
    Action::ResizeGrid,
    Action::RenamePane,
    Action::RestartPanes,
    Action::SwapPanes,
    Action::SleepPanes,
    Action::EditGoal,
    Action::StopGoal,
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
            Self::SelectAll => "select-all",
            Self::SleepPanes => "sleep-panes",
            Self::RestartPanes => "restart-panes",
            Self::NextTab => "next-tab",
            Self::NewTab => "new-tab",
            Self::ResizeGrid => "resize-grid",
            Self::SwapPanes => "swap-panes",
            Self::ZoomPane => "zoom-pane",
            Self::CommandLine => "command-line",
            Self::CommandPalette => "command-palette",
            Self::BashBot => "bashbot",
            Self::CaptureOutput => "capture-output",
            Self::ToggleOutputLogging => "toggle-output-logging",
            Self::VoiceInput => "voice-input",
            Self::EditGoal => "edit-goal",
            Self::StopGoal => "stop-goal",
            Self::Settings => "settings",
            Self::PreviousPanes => "previous-panes",
            Self::PaneActivity => "pane-activity",
            Self::CopyMode => "copy-mode",
            Self::AuthProfiles => "auth-profiles",
            Self::RenameTab => "rename-tab",
            Self::RenamePane => "rename-pane",
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
            Self::SelectAll => "select or clear all panes",
            Self::SleepPanes => "sleep or wake panes",
            Self::RestartPanes => "restart exited panes",
            Self::NextTab => "switch to next tab",
            Self::NewTab => "open a new tab",
            Self::ResizeGrid => "resize the grid",
            Self::SwapPanes => "swap selected panes",
            Self::ZoomPane => "zoom or restore focused pane",
            Self::CommandLine => "expand or close command line",
            Self::CommandPalette => "open searchable command palette",
            Self::BashBot => "open or close BashBot workspace assistant",
            Self::CaptureOutput => "capture target pane output",
            Self::ToggleOutputLogging => "start or stop target pane logging",
            Self::VoiceInput => "dictate without submitting",
            Self::EditGoal => "create or edit grid goal",
            Self::StopGoal => "stop grid goal",
            Self::Settings => "open settings and profiles",
            Self::PreviousPanes => "show previous panes",
            Self::PaneActivity => "show focused-pane activity",
            Self::CopyMode => "search and copy pane scrollback",
            Self::AuthProfiles => "manage and assign auth profiles",
            Self::RenameTab => "rename current tab",
            Self::RenamePane => "rename focused pane",
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
            Self::SelectAll => "alt+a",
            Self::SleepPanes => "alt+z",
            Self::RestartPanes => "alt+shift+t",
            Self::NextTab => "alt+t",
            Self::NewTab => "alt+n",
            Self::ResizeGrid => "alt+l",
            Self::SwapPanes => "alt+x",
            Self::ZoomPane => "alt+f",
            Self::CommandLine => "alt+c",
            Self::CommandPalette => "alt+k",
            Self::BashBot => "alt+d",
            Self::CaptureOutput => "alt+shift+c",
            Self::ToggleOutputLogging => "alt+shift+l",
            Self::VoiceInput => "alt+shift+v",
            Self::EditGoal => "alt+g",
            Self::StopGoal => "alt+u",
            Self::Settings => "alt+o",
            Self::PreviousPanes => "alt+shift+p",
            Self::PaneActivity => "alt+p",
            Self::CopyMode => "alt+b",
            Self::AuthProfiles => "alt+shift+a",
            Self::RenameTab => "alt+shift+r",
            Self::RenamePane => "alt+r",
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
        value if value.len() == 1 => Ok(ShortcutKey::Char(
            value.chars().next().expect("one-character shortcut"),
        )),
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

#[derive(Debug, Clone)]
pub struct KeyBindings {
    bindings: BTreeMap<Action, Shortcut>,
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

        for (name, chord) in overrides {
            let normalized = name.trim().to_ascii_lowercase();
            let action = Action::from_name(&normalized).ok_or_else(|| {
                anyhow!(
                    "unknown [keys] action '{name}'; supported actions: {}",
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

        Ok(Self { bindings })
    }

    pub fn action_for(&self, event: &KeyEvent) -> Option<Action> {
        ACTIONS.iter().copied().find(|action| {
            self.bindings
                .get(action)
                .is_some_and(|shortcut| shortcut.matches(event))
        })
    }

    pub fn help_entries(&self) -> Vec<(String, &'static str)> {
        ACTIONS
            .iter()
            .copied()
            .map(|action| {
                let shortcut = self.bindings[&action];
                let label = match action {
                    Action::Quit if shortcut != fallback_quit_shortcut() => {
                        format!("{} / Alt+Q", shortcut.label())
                    }
                    Action::Help => format!("{} / F1", shortcut.label()),
                    _ => shortcut.label(),
                };
                (label, action.description())
            })
            .collect()
    }

    pub fn label_for(&self, action: Action) -> String {
        self.bindings[&action].label()
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
}
