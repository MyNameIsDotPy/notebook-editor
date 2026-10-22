use super::create::resolve_source;
use crate::{
    notebook::Notebook,
    structural::{self, Operation},
};
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
    run_options(
        notebook,
        index,
        source,
        file,
        use_editor,
        new_type,
        lines_expr,
        insert_after.map(|n| n.to_string()).as_deref(),
        insert_before.map(|n| n.to_string()).as_deref(),
        delete_lines,
        false,
        false,
        None,
        None,
        backup,
        quiet,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_options(
    notebook: &str,
    index: usize,
    source: Option<String>,
    file: Option<String>,
    use_editor: bool,
    new_type: Option<&str>,
    lines_expr: Option<&str>,
    insert_after: Option<&str>,
    insert_before: Option<&str>,
    delete_lines: Option<&str>,
    dry_run: bool,
    python_syntax: bool,
    python: Option<&str>,
    rollback: Option<&str>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = if let Some(path) = rollback {
        Notebook::from_file(path)?
    } else {
        Notebook::from_file(notebook)?
    };
    if rollback.is_none() {
        if index == 0 || index > nb.len() {
            bail!(
                "Cell index {index} is out of range (notebook has {} cells)",
                nb.len()
            );
        }
        let content = if use_editor {
            Some(open_in_editor(&nb.cells[index - 1].source_str())?)
        } else if source.is_some()
            || file.is_some()
            || insert_before.is_some()
            || insert_after.is_some()
            || lines_expr.is_some()
        {
            Some(resolve_source(source, file)?)
        } else {
            None
        };
        let operation = if let Some(lines) = delete_lines {
            Some(Operation::Delete {
                cell: index,
                lines: lines.into(),
            })
        } else if let Some(lines) = insert_before {
            Some(Operation::InsertBefore {
                cell: index,
                lines: lines.into(),
                source: content.unwrap(),
            })
        } else if let Some(lines) = insert_after {
            Some(Operation::InsertAfter {
                cell: index,
                lines: lines.into(),
                source: content.unwrap(),
            })
        } else {
            content.map(|source| Operation::Replace {
                cell: index,
                lines: lines_expr.map(str::to_owned),
                source,
            })
        };
        if let Some(op) = operation {
            structural::apply(&mut nb, &op, python)?;
        }
        if let Some(t) = new_type {
            let cell = &mut nb.cells[index - 1];
            cell.cell_type = t.into();
            if t == "code" {
                if cell.execution_count.is_none() {
                    cell.execution_count = Some(serde_json::Value::Null);
                }
            } else {
                cell.execution_count = None;
                cell.outputs.clear();
            }
        }
    }
    if python_syntax {
        structural::check(&nb, python)?;
    }
    if dry_run {
        println!("{}", serde_json::to_string_pretty(&nb)?);
    } else {
        nb.save(notebook, backup)?;
    }
    if !quiet {
        eprintln!(
            "{}",
            if dry_run {
                "Preview only; notebook unchanged"
            } else {
                "Notebook updated"
            }
        );
    }
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
