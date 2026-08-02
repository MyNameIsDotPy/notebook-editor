use super::create::resolve_source;
use crate::notebook::Notebook;
use crate::selection;
use anyhow::{bail, Result};

#[allow(clippy::too_many_arguments)]
pub fn run(
    notebook: &str,
    index: usize,
    source: Option<String>,
    file: Option<String>,
    use_editor: bool,
    new_type: Option<&str>,
    lines_expr: Option<&str>,
    insert_after: Option<usize>,
    insert_before: Option<usize>,
    delete_lines: Option<&str>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;

    if index == 0 || index > nb.len() {
        bail!(
            "Cell index {index} is out of range (notebook has {} cells)",
            nb.len()
        );
    }
    let idx = index - 1;

    if let Some(expr) = delete_lines {
        // ── Delete specific lines within the cell ──────────────────────────
        let current = nb.cells[idx].source_str();
        let all_lines: Vec<&str> = current.lines().collect();
        if all_lines.is_empty() {
            bail!("Cell {index} is empty, cannot delete lines");
        }
        let to_delete = selection::resolve(expr, all_lines.len())?;
        let new_src: Vec<&str> = all_lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !to_delete.contains(i))
            .map(|(_, l)| *l)
            .collect();
        let mut joined = new_src.join("\n");
        if current.ends_with('\n') && !joined.is_empty() {
            joined.push('\n');
        }
        nb.cells[idx].set_source(joined);

        if !quiet {
            eprintln!("Deleted {} line(s) from cell {index}", to_delete.len());
        }
    } else if let Some(after) = insert_after {
        // ── Insert lines after a given line number ─────────────────────────
        let new_content = resolve_source(source, file)?;
        let current = nb.cells[idx].source_str();
        let mut all_lines: Vec<String> = current.lines().map(|l| l.to_string()).collect();

        if after == 0 || after > all_lines.len() {
            bail!(
                "--insert-after {after} is out of range (cell {index} has {} lines)",
                all_lines.len()
            );
        }

        let insert_lines: Vec<String> = new_content.lines().map(|l| l.to_string()).collect();
        for (offset, line) in insert_lines.into_iter().enumerate() {
            all_lines.insert(after + offset, line);
        }

        let mut new_src = all_lines.join("\n");
        if current.ends_with('\n') {
            new_src.push('\n');
        }
        nb.cells[idx].set_source(new_src);

        if !quiet {
            eprintln!("Inserted line(s) after line {after} in cell {index}");
        }
    } else if let Some(before) = insert_before {
        // ── Insert lines before a given line number ────────────────────────
        let new_content = resolve_source(source, file)?;
        let current = nb.cells[idx].source_str();
        let mut all_lines: Vec<String> = current.lines().map(|l| l.to_string()).collect();

        if before == 0 || before > all_lines.len() {
            bail!(
                "--insert-before {before} is out of range (cell {index} has {} lines)",
                all_lines.len()
            );
        }

        let insert_lines: Vec<String> = new_content.lines().map(|l| l.to_string()).collect();
        for (offset, line) in insert_lines.into_iter().enumerate() {
            all_lines.insert(before - 1 + offset, line);
        }

        let mut new_src = all_lines.join("\n");
        if current.ends_with('\n') {
            new_src.push('\n');
        }
        nb.cells[idx].set_source(new_src);

        if !quiet {
            eprintln!("Inserted line(s) before line {before} in cell {index}");
        }
    } else if let Some(expr) = lines_expr {
        // ── Replace specific lines ─────────────────────────────────────────
        let new_content = resolve_source(source, file)?;
        let current = nb.cells[idx].source_str();
        let mut all_lines: Vec<String> = current.lines().map(|l| l.to_string()).collect();

        if all_lines.is_empty() {
            bail!("Cell {index} is empty, cannot target lines");
        }

        let line_indices = selection::resolve(expr, all_lines.len())?;
        let replacement_lines: Vec<&str> = new_content.lines().collect();

        let first = line_indices[0];
        let last = *line_indices.last().unwrap();
        all_lines.splice(
            first..=last,
            replacement_lines.iter().map(|s| s.to_string()),
        );

        let mut new_src = all_lines.join("\n");
        if current.ends_with('\n') {
            new_src.push('\n');
        }
        nb.cells[idx].set_source(new_src);

        if !quiet {
            eprintln!("Cell {index} lines updated");
        }
    } else {
        // ── Full-cell replace ──────────────────────────────────────────────
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

        if !quiet {
            eprintln!("Cell {index} updated");
        }
    }

    if let Some(t) = new_type {
        nb.cells[idx].cell_type = t.to_string();
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
    Ok(())
}

fn open_in_editor(current_source: &str) -> Result<String> {
    use std::io::Write;

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    let mut tmp = tempfile::Builder::new().suffix(".py").tempfile()?;
    tmp.write_all(current_source.as_bytes())?;
    let path = tmp.path().to_owned();
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
