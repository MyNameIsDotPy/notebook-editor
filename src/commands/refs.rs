use crate::notebook::Notebook;
use anyhow::{bail, Result};

pub fn run(notebook: &str, cell_id: &str, json: bool) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    if !nb
        .cells
        .iter()
        .any(|cell| cell.id.as_deref() == Some(cell_id))
    {
        bail!("No cell has ID '{cell_id}'");
    }
    let matches: Vec<_> = nb
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.id.as_deref() != Some(cell_id))
        .filter_map(|(i, cell)| {
            let source = cell.source_str();
            source
                .contains(cell_id)
                .then(|| serde_json::json!({"index": i + 1, "scope": "source", "cell_id": cell.id}))
        })
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&matches)?);
    } else {
        for value in &matches {
            println!("Cell {} source", value["index"]);
        }
    }
    Ok(())
}
