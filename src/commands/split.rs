use crate::notebook::Notebook;
use anyhow::{bail, Result};

pub fn run(notebook: &str, index: usize, at_line: usize, backup: bool, quiet: bool) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    if index == 0 || index > nb.len() {
        bail!("Cell {index} is outside the valid range");
    }
    let cell = &mut nb.cells[index - 1];
    let source = cell.source_str();
    let lines: Vec<&str> = source.lines().collect();
    if at_line == 0 || at_line >= lines.len() {
        bail!("--at-line must split between existing lines");
    }
    let trailing = source.ends_with('\n');
    let first = lines[..at_line].join("\n");
    let mut second = lines[at_line..].join("\n");
    if trailing {
        second.push('\n');
    }
    cell.set_source(first);
    if cell.cell_type == "code" {
        cell.outputs.clear();
        cell.execution_count = Some(serde_json::Value::Null);
    }
    let mut sibling = cell.clone();
    sibling.id = None;
    sibling.set_source(second);
    nb.cells.insert(index, sibling);
    nb.ensure_cell_ids();
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Split cell {index}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Cell;
    #[test]
    fn splits_cell_and_assigns_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.ipynb");
        let mut c = Cell::new("code");
        c.id = Some("a".into());
        c.set_source("one\ntwo".into());
        let nb = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: serde_json::json!({}),
            cells: vec![c],
            extra: Default::default(),
        };
        nb.save(path.to_str().unwrap(), false).unwrap();
        run(path.to_str().unwrap(), 1, 1, false, true).unwrap();
        let nb = Notebook::from_file(path.to_str().unwrap()).unwrap();
        assert_eq!(nb.cells.len(), 2);
        assert_eq!(nb.cells[1].source_str(), "two");
        assert!(nb.cells[1].id.is_some());
    }
}
