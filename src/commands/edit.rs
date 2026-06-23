use anyhow::{bail, Result};
use crate::notebook::Notebook;
use crate::selection;
use super::create::resolve_source;

pub fn run(
    notebook: &str,
    index: usize,
    source: Option<String>,
    file: Option<String>,
    use_editor: bool,
    new_type: Option<&str>,
    lines_expr: Option<&str>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;

    if index == 0 || index > nb.len() {
        bail!("Cell index {index} is out of range (notebook has {} cells)", nb.len());
    }
    let idx = index - 1;

    if let Some(expr) = lines_expr {
        // Line-level edit: replace only the specified lines
        let new_content = resolve_source(source, file)?;
        let current = nb.cells[idx].source_str();
        let mut all_lines: Vec<String> = current.lines().map(|l| l.to_string()).collect();

        if all_lines.is_empty() {
            bail!("Cell {index} is empty, cannot target lines");
        }

        let line_indices = selection::resolve(expr, all_lines.len())?;
        let replacement_lines: Vec<&str> = new_content.lines().collect();

        if line_indices.len() == 1 {
            // Single line: replace with potentially multiple lines
            let pos = line_indices[0];
            all_lines.splice(pos..=pos, replacement_lines.iter().map(|s| s.to_string()));
        } else {
            // Multi-line selection: replace each targeted line with the corresponding
            // replacement line (cycling if fewer replacements than targets)
            for (i, &li) in line_indices.iter().enumerate() {
                let replacement = replacement_lines
                    .get(i)
                    .copied()
                    .unwrap_or("");
                all_lines[li] = replacement.to_string();
            }
        }

        // Re-join preserving trailing newline if original had one
        let mut new_src = all_lines.join("\n");
        if current.ends_with('\n') {
            new_src.push('\n');
        }
        nb.cells[idx].set_source(new_src);
    } else {
        // Full-cell edit
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
