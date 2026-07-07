use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{profiles::Profile, setup::SavedSetup};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Defaults::is_empty")]
    pub defaults: Defaults,
    #[serde(default, skip_serializing_if = "TodoSettings::is_empty")]
    pub todos: TodoSettings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub setups: BTreeMap<String, SavedSetup>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoSettings {
    #[serde(
        default = "TodoSettings::default_enabled",
        skip_serializing_if = "TodoSettings::is_enabled"
    )]
    pub enabled: bool,
    #[serde(
        default = "TodoSettings::default_idle_seconds",
        skip_serializing_if = "TodoSettings::is_default_idle_seconds"
    )]
    pub idle_seconds: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
}

impl Default for TodoSettings {
    fn default() -> Self {
        Self {
            enabled: Self::default_enabled(),
            idle_seconds: Self::default_idle_seconds(),
            prompts: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let default_path = Self::default_path();
        let Some(path) = path.or(default_path.as_deref()) else {
            return Ok(Self::default());
        };

        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("failed to parse config {}", path.display()))
    }

    pub fn save(&self, path: Option<&Path>) -> Result<PathBuf> {
        let path = Self::write_path(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }

        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&path, raw)
            .with_context(|| format!("failed to write config {}", path.display()))?;
        Ok(path)
    }

    pub fn set_default_profile(&mut self, profile: impl Into<String>) {
        self.defaults.profile = Some(profile.into());
    }

    #[allow(dead_code)]
    pub fn save_setup(&mut self, name: impl Into<String>, setup: SavedSetup) {
        self.setups.insert(name.into(), setup);
    }

    pub fn default_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "GridBash").map(|dirs| dirs.config_dir().join("config.toml"))
    }

    fn write_path(path: Option<&Path>) -> Result<PathBuf> {
        path.map(Path::to_path_buf)
            .or_else(Self::default_path)
            .ok_or_else(|| anyhow!("failed to resolve GridBash config directory"))
    }
}

impl Defaults {
    fn is_empty(&self) -> bool {
        self.profile.is_none()
    }
}

impl TodoSettings {
    pub fn default_enabled() -> bool {
        true
    }

    pub fn default_idle_seconds() -> u64 {
        90
    }

    pub fn normalized_prompts(&self) -> Vec<String> {
        self.prompts
            .iter()
            .map(|prompt| prompt.trim())
            .filter(|prompt| !prompt.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.enabled == Self::default_enabled()
            && self.idle_seconds == Self::default_idle_seconds()
            && self.prompts.is_empty()
    }

    fn is_enabled(value: &bool) -> bool {
        *value
    }

    fn is_default_idle_seconds(value: &u64) -> bool {
        *value == Self::default_idle_seconds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults_table() {
        let config: Config = toml::from_str(
            r#"
            [defaults]
            profile = "powershell"
            "#,
        )
        .expect("parse config");

        assert_eq!(config.defaults.profile.as_deref(), Some("powershell"));
    }

    #[test]
    fn parses_named_setups() {
        let config: Config = toml::from_str(
            r#"
            [setups.sample]
            agents = ["claude-1", "codex-2"]

            [[setups.sample.folders]]
            name = "gridbash"
            path = "C:\\Users\\Jason\\Documents\\GitHub\\gridbash"
            "#,
        )
        .expect("parse config");

        let setup = config.setups.get("sample").expect("sample setup");
        assert_eq!(setup.agents, vec!["claude-1", "codex-2"]);
        assert_eq!(setup.folders[0].name, "gridbash");
    }

    #[test]
    fn parses_todo_settings() {
        let config: Config = toml::from_str(
            r#"
            [todos]
            enabled = false
            idle_seconds = 45
            prompts = ["Review the diff", "Run tests"]
            "#,
        )
        .expect("parse config");

        assert!(!config.todos.enabled);
        assert_eq!(config.todos.idle_seconds, 45);
        assert_eq!(
            config.todos.normalized_prompts(),
            vec!["Review the diff".to_string(), "Run tests".to_string()]
        );
    }
}
