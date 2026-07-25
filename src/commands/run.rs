use anyhow::{bail, Result};
use crate::notebook::Notebook;
use crate::selection;

pub fn run(
    notebook: &str,
    selection: &str,
    timeout: i64,
    kernel: Option<&str>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;

    // Only code cells can be executed; skip others silently
    let code_indices: Vec<usize> = indices.into_iter()
        .filter(|&i| nb.cells[i].cell_type == "code")
        .collect();

    if code_indices.is_empty() {
        bail!("No code cells in selection");
    }

    // Build a minimal notebook with only the selected cells
    let selected_cells: Vec<_> = code_indices.iter().map(|&i| nb.cells[i].clone()).collect();

    let mut mini_meta = nb.metadata.clone();
    if let Some(k) = kernel {
        if let Some(ks) = mini_meta.get_mut("kernelspec") {
            ks["name"] = serde_json::Value::String(k.to_string());
        }
    }

    let mini_nb = serde_json::json!({
        "nbformat": nb.nbformat,
        "nbformat_minor": nb.nbformat_minor,
        "metadata": mini_meta,
        "cells": selected_cells,
    });

    // Write mini-notebook to a temp directory (avoids Windows file-lock conflicts)
    let tmp_dir = tempfile::tempdir()?;
    let tmp_nb = tmp_dir.path().join("exec.ipynb");
    std::fs::write(&tmp_nb, serde_json::to_string_pretty(&mini_nb)?)?;

    let tmp_nb_str = tmp_nb
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp path contains non-UTF-8 characters"))?;

    let kernel_arg = kernel
        .map(|k| format!(", kernel_name={k:?}"))
        .unwrap_or_default();

    // Inline Python: execute the mini-notebook in place via nbclient.
    // CellExecutionError is caught so outputs (including tracebacks) are always
    // written back; we still exit 1 so the caller knows a cell failed.
    let script = format!(
        "import sys\n\
         try:\n\
             import nbformat, nbclient\n\
         except ImportError as e:\n\
             print(f'Missing dependency: {{e}}', file=sys.stderr)\n\
             print('Install with: pip install nbclient nbformat', file=sys.stderr)\n\
             sys.exit(2)\n\
         nb = nbformat.read(open({path:?}), as_version=4)\n\
         client = nbclient.NotebookClient(nb, timeout={timeout}{kernel_arg})\n\
         cell_error = False\n\
         try:\n\
             client.execute()\n\
         except nbclient.exceptions.CellExecutionError as e:\n\
             print(f'Cell raised an error: {{e.ename}}: {{e.evalue}}', file=sys.stderr)\n\
             cell_error = True\n\
         nbformat.write(nb, open({path:?}, 'w'))\n\
         sys.exit(1 if cell_error else 0)\n",
        path = tmp_nb_str,
        timeout = timeout,
        kernel_arg = kernel_arg,
    );

    let python = find_python()?;

    let notebook_dir = std::path::Path::new(notebook)
        .parent()
        .unwrap_or(std::path::Path::new("."));

    if !quiet {
        eprintln!("Executing {} code cell(s)...", code_indices.len());
    }

    let status = std::process::Command::new(&python)
        .arg("-c")
        .arg(&script)
        .current_dir(notebook_dir)
        .env("PYTHONUNBUFFERED", "1")
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to launch '{python}': {e}"))?;

    if status.code() == Some(2) {
        bail!("nbclient/nbformat not installed — run: pip install nbclient nbformat");
    }

    // Read back the executed mini-notebook regardless of cell errors,
    // so we preserve whatever outputs were produced before any failure.
    let executed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&tmp_nb)?)?;

    let executed_cells = executed["cells"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Executed notebook missing cells array"))?;

    if executed_cells.len() != code_indices.len() {
        bail!("Cell count mismatch after execution");
    }

    for (slot, &nb_idx) in code_indices.iter().enumerate() {
        let ec = &executed_cells[slot];
        if let Some(outputs) = ec["outputs"].as_array() {
            nb.cells[nb_idx].outputs = outputs.clone();
        }
        nb.cells[nb_idx].execution_count = Some(ec["execution_count"].clone());
    }

    nb.save(notebook, backup)?;

    if !quiet {
        eprintln!("Outputs written to {notebook}");
    }

    // Propagate cell execution failure after saving
    if !status.success() {
        bail!("One or more cells raised an error (outputs saved)");
    }

    Ok(())
}

fn find_python() -> Result<String> {
    for candidate in ["python3", "python"] {
        if std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    bail!("Python not found — install Python 3 and ensure it is in your PATH")
}
