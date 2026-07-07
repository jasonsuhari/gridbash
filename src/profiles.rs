use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub command: PathBuf,
    pub args: Vec<String>,
}

pub const TERMINAL_PROFILE_NAMES: &[&str] = &["git-bash", "pwsh", "powershell", "cmd"];

impl Profile {
    pub fn resolved_command(&self) -> Result<LaunchCommand> {
        let exe = resolve_executable(&self.command)
            .ok_or_else(|| anyhow!("profile command not found on PATH: {}", self.command))?;
        Ok(wrap_for_windows_script(exe, &self.args))
    }

    pub fn display_name(&self, key: &str) -> String {
        self.title.clone().unwrap_or_else(|| key.to_string())
    }
}

fn wrap_for_windows_script(command: PathBuf, args: &[String]) -> LaunchCommand {
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
            let available = resolve_executable(&profile.command).is_some();
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
    profiles.insert(
        "git-bash".into(),
        Profile {
            command: "bash".into(),
            args: vec!["--login".into(), "-i".into()],
            title: Some("Git Bash".into()),
        },
    );
    profiles.insert(
        "pwsh".into(),
        Profile {
            command: "pwsh".into(),
            args: vec!["-NoLogo".into()],
            title: Some("PowerShell 7".into()),
        },
    );
    profiles.insert(
        "powershell".into(),
        Profile {
            command: "powershell".into(),
            args: vec!["-NoLogo".into()],
            title: Some("PowerShell".into()),
        },
    );
    profiles.insert(
        "cmd".into(),
        Profile {
            command: "cmd".into(),
            args: vec![],
            title: Some("cmd".into()),
        },
    );

    for agent in [
        "codex", "claude", "gemini", "opencode", "aider", "amp", "goose", "copilot", "cursor",
    ] {
        profiles.insert(
            agent.into(),
            Profile {
                command: agent.into(),
                args: vec![],
                title: Some(agent.into()),
            },
        );
    }

    profiles
}

pub fn resolve_executable(command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.is_absolute() || command_path.components().count() > 1 {
        return command_path.exists().then(|| command_path.to_path_buf());
    }

    let path = env::var_os("PATH")?;
    let pathext: Vec<String> = env::var_os("PATHEXT")
        .map(|value| {
            env::split_paths(&value)
                .filter_map(|p| p.to_str().map(|s| s.trim_start_matches('.').to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["exe".into(), "cmd".into(), "bat".into(), "ps1".into()]);

    let has_extension = Path::new(command).extension().is_some();

    for dir in env::split_paths(&path) {
        if has_extension {
            let direct = dir.join(command);
            if direct.is_file() {
                return Some(direct);
            }
        } else {
            for ext in &pathext {
                let candidate = dir.join(format!("{command}.{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }

            let direct = dir.join(command);
            if direct.is_file() {
                return Some(direct);
            }
        }
    }

    None
}
