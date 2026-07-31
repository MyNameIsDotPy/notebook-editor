use anyhow::{bail, Result};
use crate::commands::run::find_python;

pub fn run(json: bool, python: Option<&str>) -> Result<()> {
    let python = find_python(python)?;

    let mut cmd = std::process::Command::new(&python);
    cmd.args(["-m", "jupyter", "kernelspec", "list"]);
    if json {
        cmd.arg("--json");
    }

    let status = cmd
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to launch '{python}': {e}"))?;

    if !status.success() {
        bail!("jupyter kernelspec list failed — is Jupyter installed? (pip install jupyter)");
    }

    Ok(())
}
