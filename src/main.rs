mod app;
mod auth;
mod cli;
mod composer;
mod config;
mod control;
mod image_preview;
mod layout;
mod onboarding;
mod orchestrator;
mod profiles;
mod pty;
mod session;
mod setup;
mod ui;
mod usage;
mod vibe;
mod voice;
mod worktrees;

use anyhow::Result;
use clap::Parser;

use crate::{
    app::App,
    cli::{Cli, Command},
    config::Config,
    onboarding::OnboardingResult,
    profiles::{available_profiles, find_profile},
    session::select_resume_session,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.mcp {
        return control::run_mcp_server();
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
        for (name, available) in available_profiles(&config) {
            let state = if available { "available" } else { "missing" };
            println!("{name}\t{state}");
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
