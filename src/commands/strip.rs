use crate::notebook::Notebook;
use crate::selection;
use anyhow::{bail, Result};
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
pub fn run(
    notebook: &str,
    selection: &str,
    outputs: bool,
    cell_metadata: bool,
    notebook_metadata: bool,
    dry_run: bool,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    if !outputs && !cell_metadata && !notebook_metadata {
        bail!("Select at least one strip target");
    }
    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;
    if notebook_metadata && selection != "all" {
        bail!("--notebook-metadata requires selection 'all'");
    }
    if dry_run {
        if !quiet {
            eprintln!("Would strip selected notebook data");
        }
        return Ok(());
    }
    for index in indices {
        let cell = &mut nb.cells[index];
        if outputs && cell.cell_type == "code" {
            cell.outputs.clear();
            cell.execution_count = Some(Value::Null);
        }
        if cell_metadata {
            cell.metadata = Value::Object(Default::default());
        }
    }
    if notebook_metadata {
        nb.metadata = Value::Object(Default::default());
    }
    nb.save(notebook, backup)
}
