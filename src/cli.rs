use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::config::MouseMode;

#[derive(Debug, Clone, Parser)]
#[command(name = "gridbash")]
#[command(about = "Fast, beautiful terminal grids for CLI agents")]
#[command(version)]
pub struct Cli {
    /// Grid size as rows x columns, for example 2x3.
    pub grid: Option<String>,

    /// Auto-arrange this many panes.
    #[arg(long)]
    pub count: Option<usize>,

    /// Profile to launch in every pane. Overrides GRIDBASH_PROFILE and config defaults.
    #[arg(long)]
    pub profile: Option<String>,

    /// Persist the default profile to the GridBash config file and exit.
    #[arg(long, visible_alias = "set-default")]
    pub set_default_profile: Option<String>,

    /// Working directory for launched panes.
    #[arg(long)]
    pub cwd: Option<PathBuf>,

    /// Load a custom config TOML file.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Select a named theme.
    #[arg(long, default_value = "dark")]
    pub theme: String,

    /// Disable mouse capture.
    #[arg(long)]
    pub no_mouse: bool,

    /// Mouse behavior: select allows host-terminal text selection, control enables pane clicks.
    #[arg(long, value_enum)]
    pub mouse_mode: Option<MouseMode>,

    /// Print detected launch profiles and exit.
    #[arg(long)]
    pub list_profiles: bool,

    /// Initial layout strategy.
    #[arg(long, value_enum, default_value_t = GridMode::Grid)]
    pub layout: GridMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GridMode {
    Grid,
    Auto,
}
