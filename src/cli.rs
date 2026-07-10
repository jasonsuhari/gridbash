use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(name = "gridbash")]
#[command(about = "Fast, beautiful terminal grids for CLI agents")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

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

    /// Compatibility flag. Leave mouse selection to the host terminal instead of GridBash.
    #[arg(long, hide = true)]
    pub no_mouse: bool,

    /// Enable the local agent control API for child agent tools.
    #[arg(long)]
    pub agent_api: bool,

    /// Localhost port for the agent control API. Use 0 to pick an available port.
    #[arg(long, default_value_t = 0)]
    pub agent_api_port: u16,

    /// Run the GridBash MCP server over stdio.
    #[arg(long)]
    pub mcp: bool,

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

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Find and reopen a saved GridBash session.
    Resume(ResumeArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ResumeArgs {
    /// Session id or unique id prefix to resume.
    pub session: Option<String>,

    /// Print saved sessions and exit.
    #[arg(long)]
    pub list: bool,

    /// Resume the most recently updated session without prompting.
    #[arg(long)]
    pub latest: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resume_subcommand() {
        let cli = Cli::parse_from(["gridbash", "resume", "--latest"]);
        let Some(Command::Resume(args)) = cli.command else {
            panic!("expected resume command");
        };

        assert!(args.latest);
        assert!(cli.grid.is_none());
    }

    #[test]
    fn keeps_grid_positional_launches() {
        let cli = Cli::parse_from(["gridbash", "2x3", "--profile", "git-bash"]);

        assert!(cli.command.is_none());
        assert_eq!(cli.grid.as_deref(), Some("2x3"));
        assert_eq!(cli.profile.as_deref(), Some("git-bash"));
    }
}
