//! Grids remembered by name.
//!
//! A grid name used to be a tab label and nothing more, so the same workspace
//! was re-described from scratch every time: same shape, same folder, typed
//! again. Launching one now records what the name meant — how many rows and
//! columns, and which directory — and typing that name again recalls it.
//!
//! The store is deliberately small and disposable. It is a convenience, not
//! part of a session: losing it costs the user one re-entry, so every failure
//! to read or write it is swallowed rather than reported.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// How many named grids are kept. Past this the least recently launched are
/// dropped, so the list stays short enough to scan.
const MAX_PRESETS: usize = 64;

/// What one remembered name meant the last time it was launched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPreset {
    pub name: String,
    pub rows: usize,
    pub columns: usize,
    pub path: PathBuf,
    /// Milliseconds since the epoch, used only to order and prune the list.
    #[serde(default)]
    pub used_at: u64,
}

/// Every remembered grid, most recently launched first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GridPresets {
    #[serde(default)]
    presets: Vec<GridPreset>,
}

impl GridPresets {
    /// Reads the store, treating anything unreadable as empty.
    ///
    /// A corrupt or half-written file must not stop a grid from being launched,
    /// and there is nothing in here the user cannot retype.
    pub fn load() -> Self {
        let Ok(path) = presets_path() else {
            return Self::default();
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let mut presets: Self = serde_json::from_str(&contents).unwrap_or_default();
        presets.sort_and_prune();
        presets
    }

    /// The grid saved under this name, matched the way a user would expect:
    /// ignoring case and surrounding space.
    pub fn get(&self, name: &str) -> Option<&GridPreset> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.presets
            .iter()
            .find(|preset| preset.name.trim().eq_ignore_ascii_case(name))
    }

    /// Remembered names that start with what has been typed, most recent first.
    ///
    /// An empty prefix offers everything, so the step opens as a list of what
    /// is already there rather than a blank field.
    pub fn matching(&self, prefix: &str) -> Vec<&GridPreset> {
        let prefix = prefix.trim().to_ascii_lowercase();
        self.presets
            .iter()
            .filter(|preset| {
                prefix.is_empty() || preset.name.to_ascii_lowercase().starts_with(&prefix)
            })
            .collect()
    }

    /// Records what a name meant, replacing any earlier meaning of it.
    pub fn remember(&mut self, name: &str, rows: usize, columns: usize, path: &Path) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.presets
            .retain(|preset| !preset.name.trim().eq_ignore_ascii_case(name));
        self.presets.push(GridPreset {
            name: name.to_string(),
            rows,
            columns,
            path: path.to_path_buf(),
            used_at: now_millis(),
        });
        self.sort_and_prune();
    }

    pub fn relocate(&mut self, name: &str, path: &Path) -> bool {
        let name = name.trim();
        let Some(preset) = self
            .presets
            .iter_mut()
            .find(|preset| preset.name.trim().eq_ignore_ascii_case(name))
        else {
            return false;
        };
        if preset.path == path {
            return false;
        }
        preset.path = path.to_path_buf();
        true
    }

    /// Writes the store, reporting nothing if it cannot be written.
    pub fn save(&self) {
        let _ = self.try_save();
    }

    fn try_save(&self) -> Result<()> {
        let path = presets_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn sort_and_prune(&mut self) {
        // Newest first, and a stable tiebreak so two grids launched in the same
        // millisecond do not swap places between runs.
        self.presets.sort_by(|left, right| {
            right
                .used_at
                .cmp(&left.used_at)
                .then(left.name.cmp(&right.name))
        });
        self.presets.truncate(MAX_PRESETS);
    }
}

fn presets_path() -> Result<PathBuf> {
    ProjectDirs::from("", "", "GridBash")
        .map(|dirs| dirs.data_local_dir().join("grids.json"))
        .ok_or_else(|| anyhow!("failed to resolve the GridBash data directory"))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str, used_at: u64) -> GridPreset {
        GridPreset {
            name: name.into(),
            rows: 2,
            columns: 3,
            path: PathBuf::from("/tmp"),
            used_at,
        }
    }

    /// A name means one thing. Launching it again with a different shape
    /// replaces what it meant rather than leaving two entries to pick between.
    #[test]
    fn remembering_a_name_again_replaces_what_it_meant() {
        let mut presets = GridPresets::default();
        presets.remember("api", 2, 3, Path::new("/tmp/one"));
        presets.remember("api", 1, 4, Path::new("/tmp/two"));

        assert_eq!(presets.presets.len(), 1);
        let recalled = presets.get("api").expect("remembered grid");
        assert_eq!((recalled.rows, recalled.columns), (1, 4));
        assert_eq!(recalled.path, PathBuf::from("/tmp/two"));
    }

    /// Names are matched the way they are typed, not the way they were stored.
    #[test]
    fn names_are_matched_ignoring_case_and_surrounding_space() {
        let mut presets = GridPresets::default();
        presets.remember("  API Work  ", 3, 1, Path::new("/tmp"));

        assert!(presets.get("api work").is_some());
        assert!(presets.get("  API WORK ").is_some());
        assert!(presets.get("api").is_none(), "a prefix is not the name");
        assert_eq!(
            presets.presets[0].name, "API Work",
            "the name is stored trimmed but otherwise as it was typed"
        );
    }

    /// An empty name is not a name, so it is never stored and never recalled.
    #[test]
    fn blank_names_are_not_remembered() {
        let mut presets = GridPresets::default();
        presets.remember("   ", 2, 2, Path::new("/tmp"));
        presets.remember("", 2, 2, Path::new("/tmp"));

        assert!(presets.presets.is_empty());
        assert!(presets.get("").is_none());
    }

    /// The list is offered most recently launched first, because that is the
    /// one most likely to be wanted again.
    #[test]
    fn presets_are_offered_most_recent_first() {
        let mut presets = GridPresets {
            presets: vec![
                preset("old", 10),
                preset("newest", 30),
                preset("middle", 20),
            ],
        };
        presets.sort_and_prune();

        let names = presets
            .presets
            .iter()
            .map(|preset| preset.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["newest", "middle", "old"]);
    }

    /// Typing filters the list down; typing nothing offers all of it.
    #[test]
    fn matching_filters_by_prefix() {
        let mut presets = GridPresets::default();
        presets.remember("api", 2, 2, Path::new("/tmp"));
        presets.remember("apex", 2, 2, Path::new("/tmp"));
        presets.remember("web", 2, 2, Path::new("/tmp"));

        assert_eq!(presets.matching("").len(), 3);
        assert_eq!(presets.matching("ap").len(), 2);
        assert_eq!(presets.matching("AP").len(), 2, "case is ignored");
        assert_eq!(presets.matching("web").len(), 1);
        assert!(presets.matching("nothing").is_empty());
    }

    /// The store stays short enough to scan, dropping what has not been used.
    #[test]
    fn the_store_is_capped_at_the_most_recent_names() {
        let mut presets = GridPresets::default();
        for index in 0..(MAX_PRESETS + 10) {
            presets
                .presets
                .push(preset(&format!("grid-{index:03}"), index as u64));
        }
        presets.sort_and_prune();

        assert_eq!(presets.presets.len(), MAX_PRESETS);
        assert_eq!(
            presets.presets[0].name,
            format!("grid-{:03}", MAX_PRESETS + 9)
        );
        assert!(
            presets.get("grid-000").is_none(),
            "the least recently launched names are the ones dropped"
        );
    }

    /// Nothing in here is worth failing a launch over.
    #[test]
    fn unreadable_stores_load_as_empty() {
        assert!(
            serde_json::from_str::<GridPresets>("not json")
                .unwrap_or_default()
                .presets
                .is_empty()
        );
        assert!(
            serde_json::from_str::<GridPresets>("{}")
                .unwrap_or_default()
                .presets
                .is_empty()
        );
    }
}
