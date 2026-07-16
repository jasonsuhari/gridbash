mod app;
mod auth;
mod cli;
mod codex_sqlite;
mod composer;
mod config;
mod control;
mod control_discovery;
mod image_preview;
mod layout;
mod manager;
mod onboarding;
mod output_capture;
mod process_priority;
mod profiles;
mod pty;
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
    onboarding::OnboardingResult,
    profiles::{find_profile, profile_diagnostics},
    session::select_resume_session,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.mcp {
        return control::run_mcp_server();
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

        let mut app = App::resume(config, record, !cli.no_mouse)?;
        return app.run();
    }

    if onboarding::should_run(&cli, &config)
        && onboarding::run(&mut config, cli.config.as_deref())? == OnboardingResult::Quit
    {
        return Ok(());
    }

    let mut app = App::new(cli, config)?;
    app.run()
}
