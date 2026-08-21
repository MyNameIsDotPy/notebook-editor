use crate::notebook::Notebook;
use anyhow::{bail, Context, Result};
use std::fs::OpenOptions;
use std::io::Write;

pub fn run(notebook: &str, index: usize, file: &str, force: bool, quiet: bool) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    if index == 0 || index > nb.len() {
        bail!("Cell {index} is outside the valid range 1..={}", nb.len());
    }
    write_source(file, &nb.cells[index - 1].source_str(), force)?;
    if !quiet {
        eprintln!("Exported cell {index} to {file}");
    }
    Ok(())
}

pub fn write_source(path: &str, source: &str, force: bool) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("Cannot write export file '{path}'"))?;
    file.write_all(source.as_bytes())
        .with_context(|| format!("Cannot write export file '{path}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cell.py");
        std::fs::write(&path, "old").unwrap();
        assert!(write_source(path.to_str().unwrap(), "new", false).is_err());
        write_source(path.to_str().unwrap(), "new", true).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "new");
    }
}
