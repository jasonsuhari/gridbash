use std::path::PathBuf;

use clap::{Parser, ValueEnum};

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

    /// Launch each pane in its own managed git worktree.
    #[arg(long)]
    pub worktrees: bool,

    /// Prefix for managed worktree folders and branches.
    #[arg(long, default_value = "gridbash")]
    pub worktree_prefix: String,

    /// Load a custom config TOML file.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Select a named theme.
    #[arg(long, default_value = "dark")]
    pub theme: String,

    /// Compatibility flag. GridBash no longer captures mouse input by default.
    #[arg(long, hide = true)]
    pub no_mouse: bool,

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
