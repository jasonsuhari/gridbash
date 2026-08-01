mod actions;
mod app;
mod auth;
mod cli;
mod codex_sqlite;
mod composer;
mod config;
mod control;
mod control_discovery;
mod copy_mode;
mod diagnostics;
mod image_preview;
mod keybindings;
mod layout;
mod manager;
mod output_capture;
mod pane_host;
mod ports;
mod process_priority;
mod profiles;
mod pty;
mod resume_picker;
mod session;
mod setup;
mod ui;
mod usage;
mod voice;
#[cfg(target_os = "linux")]
mod voice_model;
mod worktrees;

use anyhow::Result;
use clap::Parser;

use crate::{
    app::App,
    cli::{Cli, Command},
    config::Config,
    profiles::{find_profile, profile_diagnostics},
    session::{claim_interrupted_recovery, select_resume_session},
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Installed before anything else can fail so the crash report survives the
    // terminal. App::run chains its terminal-restoring hook onto this one.
    let role = diagnostics_role(&cli);
    diagnostics::install_panic_logger(role);

    let result = run(cli);
    if let Err(error) = &result
        && records_error_exits(role)
    {
        diagnostics::record_error_exit(role, error);
    }
    result
}

/// What this process is, for crash reports.
fn diagnostics_role(cli: &Cli) -> &'static str {
    if cli.pane_host.is_some() {
        return "pane-host";
    }
    if cli.mcp {
        return "mcp";
    }
    match cli.command {
        Some(Command::Agent(_)) => "agent",
        Some(Command::Ctl(_)) => "ctl",
        _ => "tui",
    }
}

/// Whether a non-zero exit from this role is crash evidence.
///
/// A one-shot control command reports its own failure to a terminal the user is
/// still looking at, so naming a pane that no longer exists is an ordinary
/// usage error rather than something to record. Writing those would be actively
/// harmful: reports are pruned to a fixed count, and agent panes address each
/// other by pane number often enough that a stale one would keep evicting real
/// reports. A panic in the same command is still recorded, because that is a
/// defect wherever it happens.
fn records_error_exits(role: &str) -> bool {
    !matches!(role, "agent" | "ctl")
}

fn run(cli: Cli) -> Result<()> {
    if let Some(spec_path) = cli.pane_host.as_deref() {
        return pane_host::run_pane_host(spec_path);
    }

    if cli.mcp {
        return control::run_mcp_server();
    }

    if let Some(Command::Agent(args)) = &cli.command {
        return control::run_agent(args);
    }

    if let Some(Command::Ctl(args)) = &cli.command {
        return control::run_ctl(args);
    }

    let mut config = Config::load(cli.config.as_deref())?;

    if let Some(profile) = cli.set_default_profile.as_deref() {
        find_profile(&config, profile)?;
        config.set_default_profile(profile.to_string());
        let path = config.save(cli.config.as_deref())?;
        println!("default profile\t{profile}");
        println!("config\t{}", path.display());
        return Ok(());
    }

    if cli.list_profiles {
        println!("DEFAULT\tPROFILE\tSTATUS\tSOURCE\tDETAIL");
        for profile in profile_diagnostics(&config) {
            let selected = if profile.selected { "*" } else { "" };
            let source = if profile.custom { "custom" } else { "built-in" };
            let (status, detail) = match profile.executable {
                Some(path) => ("available", path.display().to_string()),
                None => (
                    "missing",
                    format!("command '{}' was not found", profile.command),
                ),
            };
            println!("{selected}\t{}\t{status}\t{source}\t{detail}", profile.name);
        }
        return Ok(());
    }

    if let Some(Command::Resume(args)) = &cli.command {
        let Some(record) = select_resume_session(args)? else {
            return Ok(());
        };

        let mut app = App::resume(
            config,
            record,
            !cli.no_mouse,
            cli.agent_control_enabled(),
            cli.agent_api_port,
        )?;
        return app.run();
    }

    if cli.allows_automatic_recovery()
        && let Some(recovery) = claim_interrupted_recovery()?
    {
        let mut app = App::recover_interrupted(
            config,
            recovery,
            !cli.no_mouse,
            cli.agent_control_enabled(),
            cli.agent_api_port,
        )?;
        return app.run();
    }

    let mut app = App::new(cli, config)?;
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role_for(args: &[&str]) -> &'static str {
        diagnostics_role(&Cli::parse_from(args))
    }

    /// Reports are labelled by what the process actually was, so a control
    /// command's failure is not filed as a crash of the interface.
    #[test]
    fn each_entry_point_reports_under_its_own_role() {
        assert_eq!(role_for(&["gridbash"]), "tui");
        assert_eq!(role_for(&["gridbash", "resume", "--latest"]), "tui");
        assert_eq!(role_for(&["gridbash", "--mcp"]), "mcp");
        assert_eq!(role_for(&["gridbash", "agent", "panes"]), "agent");
        assert_eq!(role_for(&["gridbash", "ctl", "list"]), "ctl");
    }

    /// A mistyped pane number in an agent pane is an ordinary usage error. It
    /// used to write a crash report, and reports are pruned to a fixed count,
    /// so repeating it evicted real evidence.
    #[test]
    fn one_shot_control_commands_do_not_record_error_exits() {
        assert!(!records_error_exits("agent"));
        assert!(!records_error_exits("ctl"));
        assert!(records_error_exits("tui"));
        assert!(records_error_exits("pane-host"));
        assert!(records_error_exits("mcp"));
    }
}
