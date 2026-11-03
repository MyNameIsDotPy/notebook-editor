fn main() {
    // Clap's generated parser for the full command tree needs more than the
    // Windows main thread's default 1 MiB stack in debug builds.
    std::thread::Builder::new()
        .name("nbedit-cli".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(cli_main)
        .expect("Cannot start CLI thread")
        .join()
        .expect("CLI thread panicked");
}

fn cli_main() {
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
