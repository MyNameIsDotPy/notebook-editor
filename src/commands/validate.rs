use crate::notebook::Notebook;
use anyhow::Result;
use std::collections::HashSet;

pub fn run(notebook: &str, json: bool) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    let mut issues = Vec::new();
    let mut ids = HashSet::new();
    for (i, cell) in nb.cells.iter().enumerate() {
        if !matches!(cell.cell_type.as_str(), "code" | "markdown" | "raw") {
            issues.push(format!(
                "Cell {} has unsupported type '{}'",
                i + 1,
                cell.cell_type
            ));
        }
        match &cell.id {
            Some(id) if !ids.insert(id) => issues.push(format!("Cell {} duplicates an ID", i + 1)),
            Some(_) => {}
            None => issues.push(format!("Cell {} has no ID", i + 1)),
        }
    }
    if json {
        println!(
            "{}",
            serde_json::json!({"valid": issues.is_empty(), "issues": issues})
        );
    } else if issues.is_empty() {
        println!(
            "Valid nbformat {}.{} notebook",
            nb.nbformat, nb.nbformat_minor
        );
    } else {
        for issue in &issues {
            eprintln!("{issue}");
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Notebook validation failed"))
    }
}

pub fn repair(notebook: &str, backup: bool, quiet: bool) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let mut used = HashSet::new();
    for cell in &mut nb.cells {
        if cell.id.as_ref().is_some_and(|id| !used.insert(id.clone())) {
            cell.id = None;
        }
        if !cell.metadata.is_object() {
            cell.metadata = serde_json::json!({});
        }
        if cell.cell_type == "code" && cell.execution_count.is_none() {
            cell.execution_count = Some(serde_json::Value::Null);
        }
    }
    nb.ensure_cell_ids();
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Notebook IDs and metadata repaired");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Cell;
    #[test]
    fn repair_replaces_duplicate_ids() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("n.ipynb");
        let mut a = Cell::new("code");
        a.id = Some("same".into());
        let mut b = Cell::new("code");
        b.id = Some("same".into());
        let n = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: serde_json::json!({}),
            cells: vec![a, b],
            extra: Default::default(),
        };
        n.save(p.to_str().unwrap(), false).unwrap();
        repair(p.to_str().unwrap(), false, true).unwrap();
        let n = Notebook::from_file(p.to_str().unwrap()).unwrap();
        assert_ne!(n.cells[0].id, n.cells[1].id);
    }
}
