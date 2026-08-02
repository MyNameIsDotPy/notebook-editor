mod cli;
mod commands;
mod error;
mod notebook;
mod selection;

fn main() {
    use clap::Parser;
    let cli = cli::Cli::parse();
    if let Err(error) = commands::dispatch(cli) {
        eprintln!("Error: {error:#}");
        let code = error
            .downcast_ref::<error::AppExit>()
            .map(|e| e.code)
            .unwrap_or(1);
        std::process::exit(code);
    }
}
