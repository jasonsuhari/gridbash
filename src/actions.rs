use crate::keybindings::Action;

pub fn palette_actions() -> &'static [Action] {
    Action::all()
}

pub fn palette_label(action: Action) -> &'static str {
    action.description()
}

fn search_keywords(action: Action) -> &'static str {
    match action {
        Action::CommandPalette => "commands actions search",
        Action::EditGoal | Action::StopGoal => "manager orchestrate coordinate",
        Action::SleepPanes => "wake pause resume",
        Action::ZoomPane => "maximize fullscreen restore",
        Action::RestartPanes => "exited dead",
        Action::PreviousPanes => "history list switch",
        Action::CloseGrid => "remove delete grid tab workspace terminate",
        Action::Ports => "localhost server listener process pid terminate",
        Action::VoiceInput => "microphone speech",
        Action::AuthProfiles => "accounts credentials profiles",
        Action::CaptureOutput | Action::ToggleOutputLogging => "save terminal logs",
        Action::CopyMode => "scrollback clipboard search",
        Action::BackgroundPanes | Action::BackgroundJobs => "agents pool stash restore swap",
        Action::BashBot => "assistant brief delegate coordinate",
        _ => "",
    }
}

pub fn fuzzy_match_score(query: &str, action: Action) -> Option<usize> {
    let mut query = query.chars().flat_map(char::to_lowercase).peekable();
    if query.peek().is_none() {
        return Some(0);
    }

    let haystack = format!(
        "{} {} {}",
        palette_label(action),
        action.name(),
        search_keywords(action)
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

    #[test]
    fn fuzzy_matching_accepts_labels_ids_and_keywords() {
        assert!(fuzzy_match_score("rn pane", Action::RenamePane).is_some());
        assert!(fuzzy_match_score("orchestrate", Action::EditGoal).is_some());
        assert!(fuzzy_match_score("tab next", Action::NextTab).is_some());
        assert!(fuzzy_match_score("delete grid", Action::CloseGrid).is_some());
        assert!(fuzzy_match_score("unrelated", Action::NextTab).is_none());
    }
}
