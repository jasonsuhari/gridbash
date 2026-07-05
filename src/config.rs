use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::profiles::Profile;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
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

    pub fn default_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "GridBash").map(|dirs| dirs.config_dir().join("config.toml"))
    }
}
