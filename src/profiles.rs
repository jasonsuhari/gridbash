use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{auth::AgentKind, config::Config};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<AgentKind>,
}

#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub command: PathBuf,
    pub args: Vec<String>,
}

#[cfg(windows)]
const TERMINAL_PROFILE_NAMES: &[&str] = &["git-bash", "pwsh", "powershell", "cmd"];
#[cfg(not(windows))]
const TERMINAL_PROFILE_NAMES: &[&str] = &["zsh", "bash", "fish", "sh", "pwsh"];
pub const AGENT_PROFILE_NAMES: &[&str] = &[
    "codex", "claude", "gemini", "opencode", "aider", "amp", "goose", "copilot", "cursor",
];

impl Profile {
    pub fn resolved_command(&self) -> Result<LaunchCommand> {
        let exe = self
            .resolved_executable()
            .ok_or_else(|| anyhow!("profile command not found on PATH: {}", self.command))?;
        Ok(wrap_launch_command(exe, &self.args))
    }

    pub fn display_name(&self, key: &str) -> String {
        self.title.clone().unwrap_or_else(|| key.to_string())
    }

    fn resolved_executable(&self) -> Option<PathBuf> {
        self.native_agent_executable()
            .or_else(|| resolve_executable(&self.command))
    }

    fn native_agent_executable(&self) -> Option<PathBuf> {
        let kind = self.agent_kind?;
        (self.command == kind.default_command())
            .then(|| crate::auth::resolve_agent_executable(kind))?
    }
}

#[cfg(windows)]
fn wrap_launch_command(command: PathBuf, args: &[String]) -> LaunchCommand {
    let extension = command
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    match extension.as_deref() {
        Some("cmd" | "bat") => LaunchCommand {
            command: PathBuf::from("cmd.exe"),
            args: vec![
                "/d".into(),
                "/s".into(),
                "/c".into(),
                quote_cmd_command(&command, args),
            ],
        },
        Some("ps1") => LaunchCommand {
            command: PathBuf::from("powershell.exe"),
            args: [
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                command.to_string_lossy().to_string(),
            ]
            .into_iter()
            .chain(args.iter().cloned())
            .collect(),
        },
        _ => LaunchCommand {
            command,
            args: args.to_vec(),
        },
    }
}

#[cfg(not(windows))]
fn wrap_launch_command(command: PathBuf, args: &[String]) -> LaunchCommand {
    LaunchCommand {
        command,
        args: args.to_vec(),
    }
}

#[cfg(windows)]
fn quote_cmd_command(command: &Path, args: &[String]) -> String {
    std::iter::once(command.to_string_lossy().to_string())
        .chain(args.iter().cloned())
        .map(|arg| {
            if arg.contains([' ', '\t', '"', '&', '|', '<', '>', '^']) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn default_profile_name() -> &'static str {
    #[cfg(windows)]
    {
        "git-bash"
    }
    #[cfg(target_os = "macos")]
    {
        "zsh"
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        "bash"
    }
}

pub fn all_profiles(config: &Config) -> BTreeMap<String, Profile> {
    let mut profiles = builtin_profiles();
    profiles.extend(config.profiles.clone());
    profiles
}

pub fn find_profile(config: &Config, name: &str) -> Result<Profile> {
    let profiles = all_profiles(config);
    profiles
        .get(name)
        .cloned()
        .with_context(|| format!("unknown profile '{name}'"))
}

pub fn available_profiles(config: &Config) -> Vec<(String, bool)> {
    all_profiles(config)
        .into_iter()
        .map(|(name, profile)| {
            let available = profile.resolved_executable().is_some();
            (name, available)
        })
        .collect()
}

pub fn terminal_profiles(config: &Config) -> Vec<(String, Profile)> {
    let profiles = all_profiles(config);
    TERMINAL_PROFILE_NAMES
        .iter()
        .filter_map(|name| {
            profiles
                .get(*name)
                .cloned()
                .map(|profile| ((*name).to_string(), profile))
        })
        .collect()
}

fn builtin_profiles() -> BTreeMap<String, Profile> {
    let mut profiles = BTreeMap::new();
    insert_terminal_profiles(&mut profiles);

    for agent in AGENT_PROFILE_NAMES {
        profiles.insert(
            (*agent).into(),
            Profile {
                command: (*agent).into(),
                args: vec![],
                title: Some((*agent).into()),
                agent_kind: builtin_agent_kind(agent),
            },
        );
    }

    profiles
}

#[cfg(windows)]
fn insert_terminal_profiles(profiles: &mut BTreeMap<String, Profile>) {
    profiles.insert(
        "git-bash".into(),
        Profile {
            command: "bash".into(),
            args: vec!["--login".into(), "-i".into()],
            title: Some("Git Bash".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "pwsh".into(),
        Profile {
            command: "pwsh".into(),
            args: vec!["-NoLogo".into()],
            title: Some("PowerShell 7".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "powershell".into(),
        Profile {
            command: "powershell".into(),
            args: vec!["-NoLogo".into()],
            title: Some("PowerShell".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "cmd".into(),
        Profile {
            command: "cmd".into(),
            args: vec![],
            title: Some("cmd".into()),
            agent_kind: None,
        },
    );
}

#[cfg(not(windows))]
fn insert_terminal_profiles(profiles: &mut BTreeMap<String, Profile>) {
    profiles.insert(
        "zsh".into(),
        Profile {
            command: "zsh".into(),
            args: vec!["-i".into()],
            title: Some("Z shell".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "bash".into(),
        Profile {
            command: "bash".into(),
            args: vec!["--login".into(), "-i".into()],
            title: Some("Bash".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "sh".into(),
        Profile {
            command: "sh".into(),
            args: vec!["-i".into()],
            title: Some("POSIX shell".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "fish".into(),
        Profile {
            command: "fish".into(),
            args: vec!["--interactive".into()],
            title: Some("fish".into()),
            agent_kind: None,
        },
    );
    profiles.insert(
        "pwsh".into(),
        Profile {
            command: "pwsh".into(),
            args: vec!["-NoLogo".into()],
            title: Some("PowerShell 7".into()),
            agent_kind: None,
        },
    );
}

fn builtin_agent_kind(agent: &str) -> Option<AgentKind> {
    match agent {
        "codex" => Some(AgentKind::Codex),
        "claude" => Some(AgentKind::Claude),
        _ => None,
    }
}

pub fn resolve_executable(command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.is_absolute() || command_path.components().count() > 1 {
        return is_executable_file(command_path).then(|| command_path.to_path_buf());
    }

    let path = env::var_os("PATH")?;
    #[cfg(windows)]
    let pathext: Vec<String> = env::var_os("PATHEXT")
        .map(|value| {
            env::split_paths(&value)
                .filter_map(|p| p.to_str().map(|s| s.trim_start_matches('.').to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["exe".into(), "cmd".into(), "bat".into(), "ps1".into()]);

    #[cfg(windows)]
    let has_extension = Path::new(command).extension().is_some();

    for dir in env::split_paths(&path) {
        #[cfg(windows)]
        if has_extension {
            let direct = dir.join(command);
            if is_executable_file(&direct) {
                return Some(direct);
            }
        } else {
            for ext in &pathext {
                let candidate = dir.join(format!("{command}.{ext}"));
                if is_executable_file(&candidate) {
                    return Some(candidate);
                }
            }

            let direct = dir.join(command);
            if is_executable_file(&direct) {
                return Some(direct);
            }
        }

        #[cfg(not(windows))]
        {
            let direct = dir.join(command);
            if is_executable_file(&direct) {
                return Some(direct);
            }
        }
    }

    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}
