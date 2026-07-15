use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenCommandPalette,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    TogglePaneSelection,
    ToggleSelectAll,
    ToggleSleep,
    ToggleZoom,
    RestartExited,
    NewTab,
    NextTab,
    RenameTab,
    ResizeGrid,
    SwapSelected,
    ToggleCommandLine,
    TogglePaneSummary,
    TogglePreviousPanes,
    RenamePane,
    ToggleVoiceInput,
    EditGridGoal,
    StopGridGoal,
    OpenSettings,
    OpenHelp,
    Quit,
}

impl Action {
    pub const PALETTE: [Self; 24] = [
        Self::FocusLeft,
        Self::FocusRight,
        Self::FocusUp,
        Self::FocusDown,
        Self::TogglePaneSelection,
        Self::ToggleSelectAll,
        Self::ToggleSleep,
        Self::ToggleZoom,
        Self::RestartExited,
        Self::NewTab,
        Self::NextTab,
        Self::RenameTab,
        Self::ResizeGrid,
        Self::SwapSelected,
        Self::ToggleCommandLine,
        Self::TogglePaneSummary,
        Self::TogglePreviousPanes,
        Self::RenamePane,
        Self::ToggleVoiceInput,
        Self::EditGridGoal,
        Self::StopGridGoal,
        Self::OpenSettings,
        Self::OpenHelp,
        Self::Quit,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::OpenCommandPalette => "open-command-palette",
            Self::FocusLeft => "focus-left",
            Self::FocusRight => "focus-right",
            Self::FocusUp => "focus-up",
            Self::FocusDown => "focus-down",
            Self::TogglePaneSelection => "toggle-pane-selection",
            Self::ToggleSelectAll => "toggle-select-all",
            Self::ToggleSleep => "toggle-sleep",
            Self::ToggleZoom => "toggle-zoom",
            Self::RestartExited => "restart-exited",
            Self::NewTab => "new-tab",
            Self::NextTab => "next-tab",
            Self::RenameTab => "rename-tab",
            Self::ResizeGrid => "resize-grid",
            Self::SwapSelected => "swap-selected",
            Self::ToggleCommandLine => "toggle-command-line",
            Self::TogglePaneSummary => "toggle-pane-summary",
            Self::TogglePreviousPanes => "toggle-previous-panes",
            Self::RenamePane => "rename-pane",
            Self::ToggleVoiceInput => "toggle-voice-input",
            Self::EditGridGoal => "edit-grid-goal",
            Self::StopGridGoal => "stop-grid-goal",
            Self::OpenSettings => "open-settings",
            Self::OpenHelp => "open-help",
            Self::Quit => "quit",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenCommandPalette => "Grid: Open command palette",
            Self::FocusLeft => "Pane: Focus left",
            Self::FocusRight => "Pane: Focus right",
            Self::FocusUp => "Pane: Focus up",
            Self::FocusDown => "Pane: Focus down",
            Self::TogglePaneSelection => "Pane: Toggle selection",
            Self::ToggleSelectAll => "Pane: Select or clear all",
            Self::ToggleSleep => "Pane: Sleep or wake",
            Self::ToggleZoom => "Pane: Toggle focused zoom",
            Self::RestartExited => "Pane: Restart exited targets",
            Self::NewTab => "Tab: New tab",
            Self::NextTab => "Tab: Switch to next tab",
            Self::RenameTab => "Tab: Rename current tab",
            Self::ResizeGrid => "Grid: Resize",
            Self::SwapSelected => "Grid: Swap selected panes",
            Self::ToggleCommandLine => "Grid: Toggle command line",
            Self::TogglePaneSummary => "Pane: Toggle activity summary",
            Self::TogglePreviousPanes => "Pane: Toggle previous panes",
            Self::RenamePane => "Pane: Rename focused pane",
            Self::ToggleVoiceInput => "Input: Toggle voice dictation",
            Self::EditGridGoal => "Manager: Edit grid goal",
            Self::StopGridGoal => "Manager: Stop grid goal",
            Self::OpenSettings => "Grid: Open settings",
            Self::OpenHelp => "Grid: Open help",
            Self::Quit => "Grid: Quit",
        }
    }

    pub fn search_text(self) -> &'static str {
        match self {
            Self::OpenCommandPalette => "grid open command palette actions search",
            Self::FocusLeft => "pane focus left previous move navigation",
            Self::FocusRight => "pane focus right next move navigation",
            Self::FocusUp => "pane focus up move navigation",
            Self::FocusDown => "pane focus down move navigation",
            Self::TogglePaneSelection => "pane toggle select selection target",
            Self::ToggleSelectAll => "pane select clear all selection targets",
            Self::ToggleSleep => "pane sleep wake pause resume targets",
            Self::ToggleZoom => "pane zoom maximize fullscreen focus restore layout",
            Self::RestartExited => "pane restart exited dead targets",
            Self::NewTab => "tab new open grid",
            Self::NextTab => "tab switch next cycle grid",
            Self::RenameTab => "tab rename title name",
            Self::ResizeGrid => "grid resize rows columns layout",
            Self::SwapSelected => "grid swap selected panes reorder",
            Self::ToggleCommandLine => "grid cli command line shell output",
            Self::TogglePaneSummary => "pane activity summary details usage",
            Self::TogglePreviousPanes => "pane previous list switch history",
            Self::RenamePane => "pane rename title name",
            Self::ToggleVoiceInput => "input voice dictate microphone speech",
            Self::EditGridGoal => "manager edit create grid goal orchestrate",
            Self::StopGridGoal => "manager stop cancel grid goal orchestrate",
            Self::OpenSettings => "grid open settings config profiles auth",
            Self::OpenHelp => "grid open help shortcuts controls",
            Self::Quit => "grid quit exit close",
        }
    }

    pub fn default_shortcut(self) -> &'static str {
        match self {
            Self::OpenCommandPalette => "Alt+k",
            Self::FocusLeft => "Alt+Left",
            Self::FocusRight => "Alt+Right",
            Self::FocusUp => "Alt+Up",
            Self::FocusDown => "Alt+Down",
            Self::TogglePaneSelection => "Alt+s",
            Self::ToggleSelectAll => "Alt+a",
            Self::ToggleSleep => "Alt+z",
            Self::ToggleZoom => "Alt+f",
            Self::RestartExited => "Alt+Shift+t",
            Self::NewTab => "Alt+n",
            Self::NextTab => "Alt+t",
            Self::RenameTab => "Alt+Shift+r",
            Self::ResizeGrid => "Alt+l",
            Self::SwapSelected => "Alt+x",
            Self::ToggleCommandLine => "Alt+c",
            Self::TogglePaneSummary => "Alt+p",
            Self::TogglePreviousPanes => "Alt+Shift+p",
            Self::RenamePane => "Alt+r",
            Self::ToggleVoiceInput => "Alt+Shift+v",
            Self::EditGridGoal => "Alt+g",
            Self::StopGridGoal => "Alt+u",
            Self::OpenSettings => "Alt+o",
            Self::OpenHelp => "Alt+h / F1",
            Self::Quit => "Alt+q",
        }
    }

    pub fn from_key(key: &KeyEvent) -> Option<Self> {
        if matches!(key.code, KeyCode::F(1)) {
            return Some(Self::OpenHelp);
        }
        if !key.modifiers.contains(KeyModifiers::ALT)
            || key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return None;
        }

        match key.code {
            KeyCode::Left => Some(Self::FocusLeft),
            KeyCode::Right => Some(Self::FocusRight),
            KeyCode::Up => Some(Self::FocusUp),
            KeyCode::Down => Some(Self::FocusDown),
            KeyCode::Char(ch) => {
                let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
                match (ch.to_ascii_lowercase(), shifted) {
                    ('k', false) => Some(Self::OpenCommandPalette),
                    ('s', false) => Some(Self::TogglePaneSelection),
                    ('a', false) => Some(Self::ToggleSelectAll),
                    ('z', false) => Some(Self::ToggleSleep),
                    ('f', false) => Some(Self::ToggleZoom),
                    ('t', true) => Some(Self::RestartExited),
                    ('t', false) => Some(Self::NextTab),
                    ('n', false) => Some(Self::NewTab),
                    ('l', false) => Some(Self::ResizeGrid),
                    ('x', false) => Some(Self::SwapSelected),
                    ('c', false) => Some(Self::ToggleCommandLine),
                    ('v', true) => Some(Self::ToggleVoiceInput),
                    ('g', false) => Some(Self::EditGridGoal),
                    ('u', false) => Some(Self::StopGridGoal),
                    ('o', false) => Some(Self::OpenSettings),
                    ('p', true) => Some(Self::TogglePreviousPanes),
                    ('p', false) => Some(Self::TogglePaneSummary),
                    ('r', true) => Some(Self::RenameTab),
                    ('r', false) => Some(Self::RenamePane),
                    ('h', false) => Some(Self::OpenHelp),
                    ('q', false) => Some(Self::Quit),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

pub fn fuzzy_match_score(query: &str, action: Action) -> Option<usize> {
    let mut query = query.chars().flat_map(char::to_lowercase).peekable();
    if query.peek().is_none() {
        return Some(0);
    }

    let haystack = format!(
        "{} {} {}",
        action.label(),
        action.id(),
        action.search_text()
    );
    let mut score = 0usize;
    let mut last_match = None;
    for (index, ch) in haystack.chars().flat_map(char::to_lowercase).enumerate() {
        if query.peek().is_some_and(|next| *next == ch) {
            if let Some(previous) = last_match {
                score = score.saturating_add(index.saturating_sub(previous + 1));
            } else {
                score = score.saturating_add(index);
            }
            last_match = Some(index);
            query.next();
            if query.peek().is_none() {
                return Some(score);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    #[test]
    fn maps_existing_modeless_shortcuts_to_actions() {
        assert_eq!(
            Action::from_key(&alt(KeyCode::Left)),
            Some(Action::FocusLeft)
        );
        assert_eq!(
            Action::from_key(&KeyEvent::new(
                KeyCode::Char('T'),
                KeyModifiers::ALT | KeyModifiers::SHIFT,
            )),
            Some(Action::RestartExited)
        );
        assert_eq!(
            Action::from_key(&KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
            Some(Action::OpenHelp)
        );
    }

    #[test]
    fn leaves_plain_and_control_keys_for_the_terminal() {
        assert_eq!(
            Action::from_key(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            Action::from_key(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL,)),
            None
        );
    }

    #[test]
    fn fuzzy_matching_accepts_labels_ids_and_keywords() {
        assert!(fuzzy_match_score("rn pane", Action::RenamePane).is_some());
        assert!(fuzzy_match_score("orchestrate", Action::EditGridGoal).is_some());
        assert!(fuzzy_match_score("tab next", Action::NextTab).is_some());
        assert!(fuzzy_match_score("unrelated", Action::NextTab).is_none());
    }
}
