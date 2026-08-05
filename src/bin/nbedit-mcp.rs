use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "nbedit-mcp",
    about = "MCP server for Jupyter notebooks",
    version
)]
struct Args {
    /// Restrict notebook access to this workspace root
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    nbedit::mcp::serve(&args.root)
}
