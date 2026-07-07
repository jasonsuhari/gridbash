mod app;
mod cli;
mod composer;
mod config;
mod layout;
mod onboarding;
mod profiles;
mod pty;
mod setup;
mod ui;
mod vibe;

use anyhow::Result;
use clap::Parser;

use crate::{
    app::App,
    cli::Cli,
    config::Config,
    onboarding::OnboardingResult,
    profiles::{available_profiles, find_profile},
};

fn main() -> Result<()> {
    let cli = Cli::parse();
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

    if onboarding::should_run(&cli, &config)
        && onboarding::run(&mut config, cli.config.as_deref())? == OnboardingResult::Quit
    {
        return Ok(());
    }

    let mut app = App::new(cli, config)?;
    app.run()
}
