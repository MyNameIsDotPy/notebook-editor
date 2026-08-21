use crate::notebook::Notebook;
use crate::selection;
use anyhow::{bail, Result};

pub fn run(
    notebook: &str,
    selection: &str,
    at: Option<usize>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;
    let mut cells: Vec<_> = indices.into_iter().map(|i| nb.cells[i].clone()).collect();
    for cell in &mut cells {
        cell.id = None;
    }
    let pos = at.unwrap_or(nb.len() + 1);
    if pos == 0 || pos > nb.len() + 1 {
        bail!("--at {pos} is out of range");
    }
    for (offset, cell) in cells.into_iter().enumerate() {
        nb.cells.insert(pos - 1 + offset, cell);
    }
    nb.ensure_cell_ids();
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Duplicated cells at position {pos}");
    }
    Ok(())
}
