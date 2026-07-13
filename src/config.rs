use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{auth::AuthConfig, profiles::Profile};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Defaults::is_empty")]
    pub defaults: Defaults,
    #[serde(default, skip_serializing_if = "TodoSettings::is_empty")]
    pub todos: TodoSettings,
    #[serde(default, skip_serializing_if = "AuthConfig::is_empty")]
    pub auth: AuthConfig,
    #[serde(default, skip_serializing_if = "ManagerConfig::is_empty")]
    pub manager: ManagerConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[serde(default = "ManagerConfig::default_endpoint")]
    pub endpoint: String,
    #[serde(default = "ManagerConfig::default_model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            endpoint: Self::default_endpoint(),
            model: Self::default_model(),
            api_key: String::new(),
        }
    }
}

impl ManagerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.endpoint.trim().is_empty() {
            return Err(anyhow!(
                "set the manager API endpoint in Settings > Manager"
            ));
        }
        if self.model.trim().is_empty() {
            return Err(anyhow!("set the manager model in Settings > Manager"));
        }
        if self.api_key.trim().is_empty() {
            return Err(anyhow!("set the manager API key in Settings > Manager"));
        }
        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        self.validate().is_ok()
    }

    fn default_endpoint() -> String {
        "https://api.openai.com/v1/chat/completions".into()
    }

    fn default_model() -> String {
        "gpt-4o-mini".into()
    }

    fn is_empty(&self) -> bool {
        self.endpoint == Self::default_endpoint()
            && self.model == Self::default_model()
            && self.api_key.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "PaneProcessPriority::is_default")]
    pub pane_priority: PaneProcessPriority,
    #[serde(default, skip_serializing_if = "PaneWorkloadPolicy::is_default")]
    pub pane_workload: PaneWorkloadPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaneProcessPriority {
    Normal,
    #[default]
    BelowNormal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaneWorkloadPolicy {
    #[default]
    Adaptive,
    Unrestricted,
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
        self.profile.is_none() && self.pane_priority.is_default() && self.pane_workload.is_default()
    }
}

impl PaneProcessPriority {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl PaneWorkloadPolicy {
    fn is_default(&self) -> bool {
        *self == Self::default()
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
        assert_eq!(
            config.defaults.pane_priority,
            PaneProcessPriority::BelowNormal
        );
        assert_eq!(config.defaults.pane_workload, PaneWorkloadPolicy::Adaptive);
    }

    #[test]
    fn parses_normal_pane_process_priority() {
        let config: Config = toml::from_str(
            r#"
            [defaults]
            pane_priority = "normal"
            "#,
        )
        .expect("parse config");

        assert_eq!(config.defaults.pane_priority, PaneProcessPriority::Normal);
        let serialized = toml::to_string(&config).expect("serialize config");
        assert!(serialized.contains("pane_priority = \"normal\""));
    }

    #[test]
    fn omits_default_pane_process_priority_from_saved_config() {
        let serialized = toml::to_string(&Config::default()).expect("serialize config");

        assert!(!serialized.contains("pane_priority"));
        assert!(!serialized.contains("pane_workload"));
    }

    #[test]
    fn parses_unrestricted_pane_workload_policy() {
        let config: Config = toml::from_str(
            r#"
            [defaults]
            pane_workload = "unrestricted"
            "#,
        )
        .expect("parse config");

        assert_eq!(
            config.defaults.pane_workload,
            PaneWorkloadPolicy::Unrestricted
        );
        let serialized = toml::to_string(&config).expect("serialize config");
        assert!(serialized.contains("pane_workload = \"unrestricted\""));
    }

    #[test]
    fn parses_auth_defaults() {
        let config: Config = toml::from_str(
            r#"
            [auth]
            home = "C:\\Users\\Jason\\.gridbash-auth"
            auto_cycle = true
            usage_status = true

            [auth.defaults]
            claude = "claude-1"
            codex = "codex-2"
            "#,
        )
        .expect("parse config");

        assert_eq!(config.auth.defaults.claude.as_deref(), Some("claude-1"));
        assert_eq!(config.auth.defaults.codex.as_deref(), Some("codex-2"));
        assert!(config.auth.auto_cycle);
        assert_eq!(config.auth.usage_status, Some(true));
    }

    #[test]
    fn auth_auto_cycle_defaults_to_manual() {
        let config: Config = toml::from_str("[auth]\n").expect("parse config");

        assert!(!config.auth.auto_cycle);
    }

    #[test]
    fn ignores_legacy_setups_table() {
        let config: Config = toml::from_str(
            r#"
            [defaults]
            profile = "powershell"

            [setups.sample]
            agents = ["claude-1", "codex-2"]

            [[setups.sample.folders]]
            name = "gridbash"
            path = "C:\\Users\\Jason\\Documents\\GitHub\\gridbash"
            "#,
        )
        .expect("parse config");

        assert_eq!(config.defaults.profile.as_deref(), Some("powershell"));
        assert!(config.profiles.is_empty());
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

    #[test]
    fn parses_and_round_trips_manager_api_settings() {
        let config: Config = toml::from_str(
            r#"
            [manager]
            endpoint = "https://example.test/v1/chat/completions"
            model = "local-model"
            api_key = "secret"
            "#,
        )
        .expect("parse config");

        assert!(config.manager.is_configured());
        assert_eq!(config.manager.model, "local-model");
        let serialized = toml::to_string(&config).expect("serialize config");
        assert!(serialized.contains("[manager]"));
        assert!(serialized.contains("api_key = \"secret\""));
    }
}
