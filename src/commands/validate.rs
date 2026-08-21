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
