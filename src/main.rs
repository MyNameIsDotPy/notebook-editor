fn main() {
    use clap::Parser;
    let cli = nbedit::cli::Cli::parse();
    if let Err(error) = nbedit::commands::dispatch(cli) {
        eprintln!("Error: {error:#}");
        let code = error
            .downcast_ref::<nbedit::error::AppExit>()
            .map(|e| e.code)
            .unwrap_or(1);
        std::process::exit(code);
    }
}
