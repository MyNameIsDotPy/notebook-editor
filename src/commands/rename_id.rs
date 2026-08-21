use crate::notebook::Notebook;
use anyhow::{bail, Result};

pub fn run(notebook: &str, old: &str, new: &str, backup: bool, quiet: bool) -> Result<()> {
    if new.is_empty()
        || new.len() > 64
        || !new
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("Cell ID must be 1-64 ASCII letters, numbers, '-' or '_'");
    }
    let mut nb = Notebook::from_file(notebook)?;
    if nb.cells.iter().any(|c| c.id.as_deref() == Some(new)) {
        bail!("Cell ID '{new}' already exists");
    }
    let cell = nb
        .cells
        .iter_mut()
        .find(|c| c.id.as_deref() == Some(old))
        .ok_or_else(|| anyhow::anyhow!("No cell has ID '{old}'"))?;
    cell.id = Some(new.into());
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Renamed cell ID");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Cell;
    #[test]
    fn renames_unique_id() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("n.ipynb");
        let mut c = Cell::new("code");
        c.id = Some("old".into());
        let n = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: serde_json::json!({}),
            cells: vec![c],
            extra: Default::default(),
        };
        n.save(p.to_str().unwrap(), false).unwrap();
        run(p.to_str().unwrap(), "old", "new", false, true).unwrap();
        assert_eq!(
            Notebook::from_file(p.to_str().unwrap()).unwrap().cells[0]
                .id
                .as_deref(),
            Some("new")
        );
    }
}
