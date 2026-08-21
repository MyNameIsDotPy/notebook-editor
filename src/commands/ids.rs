use crate::notebook::Notebook;
use anyhow::Result;
use std::collections::HashSet;

pub fn run(notebook: &str, json: bool) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    let mut seen = HashSet::new();
    let rows: Vec<_> = nb.cells.iter().enumerate().map(|(i, cell)| {
        let duplicate = cell.id.as_ref().is_some_and(|id| !seen.insert(id.clone()));
        serde_json::json!({"index": i + 1, "type": cell.cell_type, "id": cell.id, "duplicate": duplicate})
    }).collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for row in rows {
            println!(
                "{}\t{}\t{}",
                row["index"],
                row["type"],
                row["id"].as_str().unwrap_or("<missing>")
            );
        }
    }
    Ok(())
}
