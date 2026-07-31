use anyhow::{bail, Result};
use crate::notebook::Notebook;
use crate::selection;

pub fn run(
    notebook: &str,
    selection: &str,
    timeout: i64,
    kernel: Option<&str>,
    python: Option<&str>,
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

    let script = build_script(tmp_nb_str, timeout, kernel);

    let python = find_python(python)?;

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

/// Resolves the Python interpreter to drive execution with. `override_path`
/// (the `--python` flag) takes precedence over PATH-based auto-detection —
/// useful when the desired interpreter isn't first on PATH and the user
/// can't or doesn't want to reorder it (e.g. locked-down machines).
pub(crate) fn find_python(override_path: Option<&str>) -> Result<String> {
    if let Some(path) = override_path {
        return Ok(path.to_string());
    }

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
    bail!("Python not found — install Python 3 and ensure it is in your PATH, or pass --python <path>")
}

// Inline Python: execute the mini-notebook in place via nbclient.
// CellExecutionError is caught so outputs (including tracebacks) are always
// written back; we still exit 1 so the caller knows a cell failed.
//
// Built as a joined Vec of lines rather than a `\`-continued string literal:
// Rust strips ALL leading whitespace from a source line following a `\`
// continuation, which silently discards Python indentation and produces
// invalid syntax (`try:` with no indented body).
fn build_script(path: &str, timeout: i64, kernel: Option<&str>) -> String {
    let kernel_arg = kernel
        .map(|k| format!(", kernel_name={k:?}"))
        .unwrap_or_default();
    let path_repr = format!("{path:?}");

    let lines: Vec<String> = vec![
        "import sys".to_string(),
        "try:".to_string(),
        "    import nbformat, nbclient".to_string(),
        "except ImportError as e:".to_string(),
        "    print(f'Missing dependency: {e}', file=sys.stderr)".to_string(),
        "    print('Install with: pip install nbclient nbformat', file=sys.stderr)".to_string(),
        "    sys.exit(2)".to_string(),
        format!("nb = nbformat.read(open({path_repr}), as_version=4)"),
        format!("client = nbclient.NotebookClient(nb, timeout={timeout}{kernel_arg})"),
        "cell_error = False".to_string(),
        "try:".to_string(),
        "    client.execute()".to_string(),
        "except nbclient.exceptions.CellExecutionError as e:".to_string(),
        "    print(f'Cell raised an error: {e.ename}: {e.evalue}', file=sys.stderr)".to_string(),
        "    cell_error = True".to_string(),
        format!("nbformat.write(nb, open({path_repr}, 'w'))"),
        "sys.exit(1 if cell_error else 0)".to_string(),
    ];

    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_block_header_is_followed_by_an_indented_line() {
        // Regression test for the original bug: a `\`-continued Rust string
        // literal silently stripped Python indentation, turning `try:` into
        // a statement with no body (IndentationError at runtime).
        let script = build_script("nb.ipynb", 60, Some("python3"));
        let lines: Vec<&str> = script.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_end().ends_with(':') {
                let next = lines
                    .get(i + 1)
                    .unwrap_or_else(|| panic!("line {i} ({line:?}) opens a block but has no following line"));
                assert!(
                    next.starts_with("    "),
                    "line {i} ({line:?}) opens a block but next line {next:?} is not indented"
                );
            }
        }
    }

    #[test]
    fn omits_kernel_arg_when_not_specified() {
        let script = build_script("nb.ipynb", 60, None);
        assert!(script.contains("NotebookClient(nb, timeout=60)"));
        assert!(!script.contains("kernel_name"));
    }

    #[test]
    fn includes_kernel_arg_when_specified() {
        let script = build_script("nb.ipynb", 60, Some("python3119"));
        assert!(script.contains(r#"kernel_name="python3119""#));
    }

    #[test]
    fn embeds_windows_path_as_escaped_python_string_literal() {
        let script = build_script(r"C:\tmp\exec.ipynb", 60, None);
        assert!(script.contains(r#"open("C:\\tmp\\exec.ipynb")"#));
    }

    #[test]
    fn generated_script_is_syntactically_valid_python() {
        // Directly catches the class of bug this was written for: compile
        // (don't execute) the generated script with the system Python.
        let Ok(python) = find_python(None) else {
            eprintln!("skipping: no python interpreter found on PATH");
            return;
        };
        let script = build_script("nb.ipynb", 60, Some("python3"));
        let output = std::process::Command::new(&python)
            .arg("-c")
            .arg(format!("compile({script:?}, '<test>', 'exec')"))
            .output()
            .expect("failed to invoke python");
        assert!(
            output.status.success(),
            "generated script failed to compile:\n{}\n---\n{}",
            String::from_utf8_lossy(&output.stderr),
            script
        );
    }

    #[test]
    fn find_python_prefers_override_path() {
        let resolved = find_python(Some(r"C:\custom\python.exe")).unwrap();
        assert_eq!(resolved, r"C:\custom\python.exe");
    }
}
