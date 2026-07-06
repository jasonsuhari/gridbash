use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::profiles::Profile;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Defaults::is_empty")]
    pub defaults: Defaults,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
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
        self.profile.is_none()
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
}
