use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::profiles::Profile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VibeProfile {
    pub name: String,
    pub ready: bool,
    pub status: String,
}

#[allow(dead_code)]
pub fn load_profiles() -> Result<Vec<VibeProfile>> {
    let launch = Profile {
        command: "vibe".into(),
        args: vec!["profiles".into()],
        title: None,
    }
    .resolved_command()
    .context("vibe is required for GridBash orchestration, but it was not found")?;

    let output = Command::new(&launch.command)
        .args(&launch.args)
        .output()
        .with_context(|| format!("failed to run {}", launch.command.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("vibe profiles failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let profiles = parse_profiles(&stdout);
    if profiles.is_empty() {
        return Err(anyhow!("vibe did not report any profiles"));
    }

    Ok(profiles)
}

pub fn parse_profiles(raw: &str) -> Vec<VibeProfile> {
    raw.lines()
        .filter_map(parse_profile_line)
        .collect::<Vec<_>>()
}

fn parse_profile_line(line: &str) -> Option<VibeProfile> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("Profiles in ")
        || looks_like_path(trimmed)
        || !line.chars().next().is_some_and(char::is_whitespace)
    {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let name = parts.next()?.to_string();
    let status = parts.collect::<Vec<_>>().join(" ");
    if status.is_empty() {
        return None;
    }

    Some(VibeProfile {
        name,
        ready: status.contains("auth files present"),
        status,
    })
}

fn looks_like_path(value: &str) -> bool {
    value.contains(":\\") || value.starts_with('/') || value.starts_with("~/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vibe_profiles_output() {
        let raw = r#"
Profiles in C:\Users\Jason\.claude-profiles:
  claude-1           auth files present
  C:\Users\Jason\.claude-profiles\claude-1
  codex-1            not logged in
  C:\Users\Jason\.claude-profiles\codex-1
"#;

        let profiles = parse_profiles(raw);
        assert_eq!(
            profiles,
            vec![
                VibeProfile {
                    name: "claude-1".into(),
                    ready: true,
                    status: "auth files present".into(),
                },
                VibeProfile {
                    name: "codex-1".into(),
                    ready: false,
                    status: "not logged in".into(),
                },
            ]
        );
    }
}
