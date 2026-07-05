mod app;
mod cli;
mod config;
mod layout;
mod profiles;
mod pty;
mod ui;

use anyhow::Result;
use clap::Parser;

use crate::{app::App, cli::Cli, config::Config, profiles::available_profiles};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;

    if cli.list_profiles {
        for (name, available) in available_profiles(&config) {
            let state = if available { "available" } else { "missing" };
            println!("{name}\t{state}");
        }
        return Ok(());
    }

    let mut app = App::new(cli, config)?;
    app.run()
}
