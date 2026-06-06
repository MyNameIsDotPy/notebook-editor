use anyhow::Result;

mod cli;
mod commands;
mod error;
mod notebook;
mod selection;

fn main() -> Result<()> {
    use clap::Parser;
    let cli = cli::Cli::parse();
    commands::dispatch(cli)
}
