use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Result;

use crate::{
    layout::GridSize,
    profiles::{AGENT_PROFILE_NAMES, LaunchCommand, Profile},
    worktrees::{ManagedWorktreeOptions, ensure_pane_worktrees},
};

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub panes: Vec<PaneLaunchSpec>,
    pub grid: GridSize,
}

#[derive(Debug, Clone)]
pub struct PaneLaunchSpec {
    #[allow(dead_code)]
    pub profile_name: String,
    pub command: Profile,
    pub cwd: PathBuf,
    pub folder_name: String,
    pub worktree_name: Option<String>,
}

impl LaunchPlan {
    pub fn from_launch_options(
        profile_name: String,
        command: Profile,
        cwd: PathBuf,
        count: usize,
        grid: GridSize,
        worktrees: Option<&ManagedWorktreeOptions>,
    ) -> Result<Self> {
        if let Some(options) = worktrees {
            return Self::managed_worktrees(profile_name, command, cwd, count, grid, options);
        }

        Ok(Self::legacy(profile_name, command, cwd, count, grid))
    }

    pub fn legacy(
        profile_name: String,
        command: Profile,
        cwd: PathBuf,
        count: usize,
        grid: GridSize,
    ) -> Self {
        let folder_name = folder_display_name(&cwd);
        let worktree_name = git_worktree_name(&cwd);
        let panes = (0..count)
            .map(|_| PaneLaunchSpec {
                profile_name: profile_name.clone(),
                command: command.clone(),
                cwd: cwd.clone(),
                folder_name: folder_name.clone(),
                worktree_name: worktree_name.clone(),
            })
            .collect();

        Self { panes, grid }
    }

    fn managed_worktrees(
        profile_name: String,
        command: Profile,
        cwd: PathBuf,
        count: usize,
        grid: GridSize,
        options: &ManagedWorktreeOptions,
    ) -> Result<Self> {
        let panes = ensure_pane_worktrees(&cwd, count, options)?
            .into_iter()
            .map(|worktree| PaneLaunchSpec {
                profile_name: profile_name.clone(),
                command: command.clone(),
                cwd: worktree.cwd,
                folder_name: worktree.folder_name,
                worktree_name: Some(worktree.branch_name),
            })
            .collect();

        Ok(Self { panes, grid })
    }
}

impl PaneLaunchSpec {
    pub fn resolved_command(&self) -> Result<LaunchCommand> {
        self.command.resolved_command()
    }

    pub fn agent_label(&self) -> Option<String> {
        if command_basename(&self.command.command).as_deref() == Some("vibe") {
            return self
                .command
                .args
                .windows(2)
                .find_map(|args| (args[0] == "run").then(|| args[1].clone()))
                .or_else(|| self.command.title.clone())
                .or_else(|| Some(self.profile_name.clone()))
                .map(clean_agent_label);
        }

        if is_agent_like(&self.profile_name)
            || self.command.title.as_deref().is_some_and(is_agent_like)
            || command_basename(&self.command.command)
                .as_deref()
                .is_some_and(is_agent_like)
        {
            return Some(clean_agent_label(
                self.command
                    .title
                    .clone()
                    .unwrap_or_else(|| self.profile_name.clone()),
            ));
        }

        None
    }
}

pub fn folder_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

pub fn git_worktree_name(path: &Path) -> Option<String> {
    run_git(path, &["branch", "--show-current"])
        .or_else(|| run_git(path, &["rev-parse", "--short", "HEAD"]).map(|hash| format!("@{hash}")))
}

fn run_git(path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn command_basename(command: &str) -> Option<String> {
    let path = Path::new(command);
    let file_name = path
        .file_stem()
        .or_else(|| path.file_name())
        .and_then(|value| value.to_str())?;
    Some(file_name.to_ascii_lowercase())
}

fn is_agent_like(value: &str) -> bool {
    let normalized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let parts = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    AGENT_PROFILE_NAMES.iter().any(|agent| {
        parts
            .iter()
            .any(|part| *part == *agent || part.starts_with(agent))
    })
}

fn clean_agent_label(value: String) -> String {
    let trimmed = value.trim().trim_matches('-').to_string();
    if trimmed.is_empty() {
        "vibe".into()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn detects_vibe_agent_panes() {
        let cwd = env::current_dir().expect("cwd");
        let spec = PaneLaunchSpec {
            profile_name: "claude-1".into(),
            command: Profile {
                command: "vibe".into(),
                args: vec!["run".into(), "claude-1".into(), "--".into()],
                title: Some("claude-1".into()),
            },
            cwd,
            folder_name: "repo".into(),
            worktree_name: None,
        };

        assert_eq!(spec.agent_label().as_deref(), Some("claude-1"));
    }

    #[test]
    fn detects_custom_agent_profiles() {
        let cwd = env::current_dir().expect("cwd");
        let spec = PaneLaunchSpec {
            profile_name: "review".into(),
            command: Profile {
                command: "codex".into(),
                args: vec!["--model".into(), "gpt-5.5".into()],
                title: Some("Codex Review".into()),
            },
            cwd,
            folder_name: "repo".into(),
            worktree_name: None,
        };

        assert_eq!(spec.agent_label().as_deref(), Some("Codex Review"));
    }

    #[test]
    fn ignores_plain_terminal_profiles() {
        let cwd = env::current_dir().expect("cwd");
        let spec = PaneLaunchSpec {
            profile_name: "git-bash".into(),
            command: Profile {
                command: "bash".into(),
                args: vec!["--login".into()],
                title: Some("Git Bash".into()),
            },
            cwd,
            folder_name: "repo".into(),
            worktree_name: None,
        };

        assert_eq!(spec.agent_label(), None);
    }
}
