use crate::notebook::Notebook;
use anyhow::{bail, Result};
use regex::Regex;

pub fn run(
    notebook: &str,
    pattern: &str,
    scope: &str,
    ignore_case: bool,
    json: bool,
) -> Result<()> {
    let pattern = if ignore_case {
        format!("(?i){pattern}")
    } else {
        pattern.to_owned()
    };
    let re = Regex::new(&pattern)?;
    if !matches!(scope, "outputs" | "cell-metadata" | "notebook-metadata") {
        bail!("scope must be outputs, cell-metadata, or notebook-metadata");
    }
    let nb = Notebook::from_file(notebook)?;
    let mut matches = Vec::new();
    if scope == "notebook-metadata" {
        let text = serde_json::to_string(&nb.metadata)?;
        if re.is_match(&text) {
            matches.push(serde_json::json!({"scope": scope, "value": text}));
        }
    } else {
        for (index, cell) in nb.cells.iter().enumerate() {
            let value = if scope == "outputs" {
                serde_json::to_string(&cell.outputs)?
            } else {
                serde_json::to_string(&cell.metadata)?
            };
            if re.is_match(&value) {
                matches.push(serde_json::json!({"index": index + 1, "scope": scope, "cell_id": cell.id, "value": value}));
            }
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&matches)?);
    } else {
        for item in &matches {
            println!("{}: {}", item["index"], item["value"]);
        }
    }
    if matches.is_empty() {
        bail!("No matches found for '{pattern}'");
    }
    Ok(())
}
