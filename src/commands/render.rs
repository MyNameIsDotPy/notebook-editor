use anyhow::{bail, Context, Result};
use std::process::Command;

pub fn run(notebook: &str, output: &str, force: bool, driver_python: Option<&str>) -> Result<()> {
    let python = driver_python.unwrap_or("python3");
    let script = "import sys\nfrom nbconvert import HTMLExporter\nfrom nbformat import read\nnb = read(open(sys.argv[1], encoding='utf-8'), as_version=4)\nbody, _ = HTMLExporter().from_notebook_node(nb)\nopen(sys.argv[2], 'w', encoding='utf-8').write(body)\n";
    if !force && std::path::Path::new(output).exists() {
        bail!("Export file already exists; pass --force to replace it");
    }
    let status = Command::new(python)
        .args(["-c", script, notebook, output])
        .status()
        .with_context(|| format!("Cannot launch '{python}'"))?;
    if !status.success() {
        bail!("Rendering failed; install nbconvert with: {python} -m pip install nbconvert");
    }
    Ok(())
}
