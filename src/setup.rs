use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AgentKind,
    layout::GridSize,
    profiles::{AGENT_PROFILE_NAMES, LaunchCommand, Profile},
    worktrees::{ManagedWorktreeOptions, ensure_pane_worktrees},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSetup {
    #[serde(default)]
    pub folders: Vec<SetupFolder>,
    #[serde(default)]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupFolder {
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
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub folder_name: String,
    pub worktree_name: Option<String>,
    pub auth_name: Option<String>,
    pub auth_kind: Option<AgentKind>,
    pub auth_dir: Option<PathBuf>,
}

impl SavedSetup {
    pub fn new(folders: Vec<SetupFolder>, agents: Vec<String>) -> Self {
        Self { folders, agents }
    }

    pub fn validate(&self) -> Result<()> {
        if self.folders.is_empty() {
            return Err(anyhow!("setup needs at least one folder"));
        }
        if self.agents.is_empty() {
            return Err(anyhow!("setup needs at least one agent"));
        }
        for folder in &self.folders {
            if !folder.path.is_dir() {
                return Err(anyhow!("folder does not exist: {}", folder.path.display()));
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
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

impl SetupFolder {
    #[allow(dead_code)]
    pub fn from_path(path: PathBuf) -> Self {
        let name = folder_display_name(&path);
        Self { name, path }
    }
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
                env: BTreeMap::new(),
                cwd: cwd.clone(),
                folder_name: folder_name.clone(),
                worktree_name: worktree_name.clone(),
                auth_name: None,
                auth_kind: None,
                auth_dir: None,
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
                env: BTreeMap::new(),
                cwd: worktree.cwd,
                folder_name: worktree.folder_name,
                worktree_name: Some(worktree.branch_name),
                auth_name: None,
                auth_kind: None,
                auth_dir: None,
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

#[allow(dead_code)]
pub fn sanitize_setup_name(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    (!normalized.is_empty()).then_some(normalized)
}

#[allow(dead_code)]
fn vibe_pane_spec(agent: &str, folder: &SetupFolder) -> PaneLaunchSpec {
    PaneLaunchSpec {
        profile_name: agent.to_string(),
        command: Profile {
            command: "vibe".into(),
            args: vec!["run".into(), agent.into(), "--".into()],
            title: Some(agent.into()),
            agent_kind: None,
        },
        env: BTreeMap::new(),
        cwd: folder.path.clone(),
        folder_name: folder.name.clone(),
        worktree_name: git_worktree_name(&folder.path),
        auth_name: None,
        auth_kind: None,
        auth_dir: None,
    }
}

#[allow(dead_code)]
pub fn setup_from_selection(folders: Vec<PathBuf>, agents: Vec<String>) -> Result<SavedSetup> {
    let setup = SavedSetup::new(
        folders.into_iter().map(SetupFolder::from_path).collect(),
        agents,
    );
    setup.validate().context("invalid setup")?;
    Ok(setup)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn assigns_agents_round_robin_across_folders() {
        let cwd = env::current_dir().expect("cwd");
        let other = cwd.parent().unwrap_or(&cwd).to_path_buf();
        let setup = SavedSetup::new(
            vec![
                SetupFolder {
                    name: "one".into(),
                    path: cwd,
                },
                SetupFolder {
                    name: "two".into(),
                    path: other,
                },
            ],
            vec!["claude-1".into(), "claude-2".into(), "codex-2".into()],
        );

        let plan = setup.launch_plan().expect("launch plan");
        assert_eq!(plan.panes[0].folder_name, "one");
        assert_eq!(plan.panes[1].folder_name, "two");
        assert_eq!(plan.panes[2].folder_name, "one");
        assert_eq!(plan.grid.count(), 4);
    }

    #[test]
    fn builds_vibe_run_command_for_agent() {
        let cwd = env::current_dir().expect("cwd");
        let setup = SavedSetup::new(
            vec![SetupFolder {
                name: "repo".into(),
                path: cwd,
            }],
            vec!["claude-1".into()],
        );

        let plan = setup.launch_plan().expect("launch plan");
        assert_eq!(plan.panes[0].command.command, "vibe");
        assert_eq!(plan.panes[0].command.args, vec!["run", "claude-1", "--"]);
    }

    #[test]
    fn detects_vibe_agent_panes() {
        let cwd = env::current_dir().expect("cwd");
        let spec = PaneLaunchSpec {
            profile_name: "claude-1".into(),
            command: Profile {
                command: "vibe".into(),
                args: vec!["run".into(), "claude-1".into(), "--".into()],
                title: Some("claude-1".into()),
                agent_kind: None,
            },
            env: BTreeMap::new(),
            cwd,
            folder_name: "repo".into(),
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            auth_dir: None,
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
                agent_kind: Some(AgentKind::Codex),
            },
            env: BTreeMap::new(),
            cwd,
            folder_name: "repo".into(),
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            auth_dir: None,
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
                agent_kind: None,
            },
            env: BTreeMap::new(),
            cwd,
            folder_name: "repo".into(),
            worktree_name: None,
            auth_name: None,
            auth_kind: None,
            auth_dir: None,
        };

        assert_eq!(spec.agent_label(), None);
    }

    #[test]
    fn sanitizes_setup_names() {
        assert_eq!(
            sanitize_setup_name("Client Stack!"),
            Some("client-stack".into())
        );
        assert_eq!(sanitize_setup_name("   "), None);
    }
}
