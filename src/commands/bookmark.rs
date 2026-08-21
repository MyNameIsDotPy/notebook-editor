use crate::notebook::Notebook;
use anyhow::{bail, Result};
use serde_json::{Map, Value};

pub fn set(notebook: &str, name: &str, index: usize, backup: bool, quiet: bool) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    if index == 0 || index > nb.len() {
        bail!("Cell {index} is outside the valid range");
    }
    nb.ensure_cell_ids();
    let id = nb.cells[index - 1].id.clone().unwrap();
    let metadata = nb
        .metadata
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Notebook metadata must be an object"))?;
    let nbedit = metadata
        .entry("nbedit")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("metadata.nbedit must be an object"))?;
    let bookmarks = nbedit
        .entry("bookmarks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("metadata.nbedit.bookmarks must be an object"))?;
    bookmarks.insert(name.into(), Value::String(id));
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Bookmark '{name}' set");
    }
    Ok(())
}
pub fn list(notebook: &str, json: bool) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    let bookmarks = nb
        .metadata
        .pointer("/nbedit/bookmarks")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    if json {
        println!("{}", serde_json::to_string_pretty(&bookmarks)?)
    } else if let Some(values) = bookmarks.as_object() {
        for (name, id) in values {
            println!("{name}\t{id}");
        }
    }
    Ok(())
}
pub fn remove(notebook: &str, name: &str, backup: bool, quiet: bool) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let bookmarks = nb
        .metadata
        .pointer_mut("/nbedit/bookmarks")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("No bookmarks"))?;
    if bookmarks.remove(name).is_none() {
        bail!("No bookmark named '{name}'");
    }
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Bookmark '{name}' removed");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Cell;
    #[test]
    fn stores_id() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("n.ipynb");
        let n = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: Value::Object(Map::new()),
            cells: vec![Cell::new("code")],
            extra: Default::default(),
        };
        n.save(p.to_str().unwrap(), false).unwrap();
        set(p.to_str().unwrap(), "start", 1, false, true).unwrap();
        assert!(Notebook::from_file(p.to_str().unwrap())
            .unwrap()
            .metadata
            .pointer("/nbedit/bookmarks/start")
            .is_some());
    }
}
