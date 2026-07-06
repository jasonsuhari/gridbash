use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow};

use crate::{
    layout::GridSize,
    profiles::{LaunchCommand, Profile},
};

#[derive(Debug, Clone)]
pub struct LaunchSelection {
    pub folders: Vec<LaunchFolder>,
    pub agents: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchFolder {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub panes: Vec<PaneLaunchSpec>,
    pub grid: GridSize,
}

#[derive(Debug, Clone)]
pub struct PaneLaunchSpec {
    pub profile_name: String,
    pub command: Profile,
    pub cwd: PathBuf,
    pub folder_name: String,
    pub worktree_name: Option<String>,
}

impl LaunchSelection {
    pub fn new(folders: Vec<LaunchFolder>, agents: Vec<String>) -> Self {
        Self { folders, agents }
    }

    pub fn validate(&self) -> Result<()> {
        if self.folders.is_empty() {
            return Err(anyhow!("launch needs at least one folder"));
        }
        if self.agents.is_empty() {
            return Err(anyhow!("launch needs at least one agent"));
        }
        for folder in &self.folders {
            if !folder.path.is_dir() {
                return Err(anyhow!("folder does not exist: {}", folder.path.display()));
            }
        }
        Ok(())
    }

    pub fn launch_plan(&self) -> Result<LaunchPlan> {
        self.validate()?;
        let panes = self
            .agents
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let folder = &self.folders[index % self.folders.len()];
                vibe_pane_spec(agent, folder)
            })
            .collect::<Vec<_>>();
        let grid = GridSize::from_count(panes.len());
        Ok(LaunchPlan { panes, grid })
    }
}

impl LaunchFolder {
    pub fn from_path(path: PathBuf) -> Self {
        let name = folder_display_name(&path);
        Self { name, path }
    }
}

impl LaunchPlan {
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
}

impl PaneLaunchSpec {
    pub fn resolved_command(&self) -> Result<LaunchCommand> {
        self.command.resolved_command()
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

fn vibe_pane_spec(agent: &str, folder: &LaunchFolder) -> PaneLaunchSpec {
    PaneLaunchSpec {
        profile_name: agent.to_string(),
        command: Profile {
            command: "vibe".into(),
            args: vec!["run".into(), agent.into(), "--".into()],
            title: Some(agent.into()),
        },
        cwd: folder.path.clone(),
        folder_name: folder.name.clone(),
        worktree_name: git_worktree_name(&folder.path),
    }
}

pub fn launch_selection_from(
    folders: Vec<PathBuf>,
    agents: Vec<String>,
) -> Result<LaunchSelection> {
    let selection = LaunchSelection::new(
        folders.into_iter().map(LaunchFolder::from_path).collect(),
        agents,
    );
    selection.validate().context("invalid launch selection")?;
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn assigns_agents_round_robin_across_folders() {
        let cwd = env::current_dir().expect("cwd");
        let other = cwd.parent().unwrap_or(&cwd).to_path_buf();
        let selection = LaunchSelection::new(
            vec![
                LaunchFolder {
                    name: "one".into(),
                    path: cwd,
                },
                LaunchFolder {
                    name: "two".into(),
                    path: other,
                },
            ],
            vec!["claude-1".into(), "claude-2".into(), "codex-2".into()],
        );

        let plan = selection.launch_plan().expect("launch plan");
        assert_eq!(plan.panes[0].folder_name, "one");
        assert_eq!(plan.panes[1].folder_name, "two");
        assert_eq!(plan.panes[2].folder_name, "one");
        assert_eq!(plan.grid.count(), 4);
    }

    #[test]
    fn builds_vibe_run_command_for_agent() {
        let cwd = env::current_dir().expect("cwd");
        let selection = LaunchSelection::new(
            vec![LaunchFolder {
                name: "repo".into(),
                path: cwd,
            }],
            vec!["claude-1".into()],
        );

        let plan = selection.launch_plan().expect("launch plan");
        assert_eq!(plan.panes[0].command.command, "vibe");
        assert_eq!(plan.panes[0].command.args, vec!["run", "claude-1", "--"]);
    }
}
