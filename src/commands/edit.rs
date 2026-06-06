use anyhow::{bail, Result};
use crate::notebook::Notebook;
use super::create::resolve_source;

pub fn run(
    notebook: &str,
    index: usize,
    source: Option<String>,
    file: Option<String>,
    use_editor: bool,
    new_type: Option<&str>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;

    if index == 0 || index > nb.len() {
        bail!("Cell index {index} is out of range (notebook has {} cells)", nb.len());
    }
    let idx = index - 1;

    let src: Option<String> = if use_editor {
        Some(open_in_editor(&nb.cells[idx].source_str())?)
    } else if source.is_some() || file.is_some() {
        Some(resolve_source(source, file)?)
    } else {
        None
    };

    if let Some(s) = src {
        nb.cells[idx].set_source(s);
    }

    if let Some(t) = new_type {
        nb.cells[idx].cell_type = t.to_string();
        // Adjust execution_count / outputs based on new type
        if t == "code" {
            if nb.cells[idx].execution_count.is_none() {
                nb.cells[idx].execution_count = Some(serde_json::Value::Null);
            }
        } else {
            nb.cells[idx].execution_count = None;
            nb.cells[idx].outputs.clear();
        }
    }

    nb.save(notebook, backup)?;

    if !quiet {
        eprintln!("Cell {index} updated");
    }

    Ok(())
}

fn open_in_editor(current_source: &str) -> Result<String> {
    use std::io::Write;

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    let mut tmp = tempfile::Builder::new()
        .suffix(".py")
        .tempfile()?;
    tmp.write_all(current_source.as_bytes())?;
    let path = tmp.path().to_owned();
    // Keep the file alive until after the editor exits
    tmp.flush()?;

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to launch editor '{editor}': {e}"))?;

    if !status.success() {
        bail!("Editor exited with non-zero status");
    }

    Ok(std::fs::read_to_string(&path)?)
}
