use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde_json::json;

use crate::{
    auth::AgentKind, layout::PaneId, profiles::Profile, session::is_direct_claude_command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Working,
    Waiting,
}

#[derive(Debug)]
pub struct TurnReport {
    path: PathBuf,
    modified: Option<SystemTime>,
    turn: Option<Turn>,
}

impl TurnReport {
    pub fn turn(&self) -> Option<Turn> {
        self.turn
    }

    pub fn refresh(&mut self) -> bool {
        let modified = fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified.is_none() || modified == self.modified {
            return false;
        }
        self.modified = modified;
        let turn = match fs::read_to_string(&self.path).as_deref() {
            Ok("working") => Some(Turn::Working),
            Ok("waiting") => Some(Turn::Waiting),
            _ => self.turn,
        };
        let changed = turn != self.turn;
        self.turn = turn;
        changed
    }
}

/// Claude fires no hook when the person interrupts a turn, so a quiet screen
/// still has to count as waiting.
pub fn report_claude_turns(command: &mut Profile, pane: PaneId) -> Option<TurnReport> {
    if command.agent_kind != Some(AgentKind::Claude)
        || !is_direct_claude_command(&command.command)
        || command.args.iter().any(|argument| argument == "--settings")
    {
        return None;
    }
    let directory = std::env::temp_dir().join("gridbash-turns");
    fs::create_dir_all(&directory).ok()?;
    let path = directory.join(format!("{}-{}", std::process::id(), pane.0));
    let _ = fs::remove_file(&path);
    command.args.push("--settings".into());
    command.args.push(hook_settings(&path));
    Some(TurnReport {
        path,
        modified: None,
        turn: None,
    })
}

fn hook_settings(report: &Path) -> String {
    let target = shell_quote(report);
    let hook = |turn: &str| json!([{ "hooks": [{ "type": "command", "command": format!("printf {turn} > {target}") }] }]);
    json!({ "hooks": { "UserPromptSubmit": hook("working"), "Stop": hook("waiting") } }).to_string()
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}
