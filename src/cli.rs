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

    /// Profile to launch in every pane.
    #[arg(long, default_value = "git-bash")]
    pub profile: String,

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
