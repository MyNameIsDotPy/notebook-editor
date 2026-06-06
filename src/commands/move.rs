use anyhow::Result;
use crate::notebook::Notebook;
use crate::selection;

pub fn run(
    notebook: &str,
    sel: &str,
    to_expr: &str,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(sel, nb.len())?;

    // Partition into selected and not-selected
    let (sel_cells, rest): (Vec<_>, Vec<_>) = nb
        .cells
        .drain(..)
        .enumerate()
        .partition(|(i, _)| indices.contains(i));

    let selected: Vec<crate::notebook::Cell> = sel_cells.into_iter().map(|(_, c)| c).collect();
    let mut rest_cells: Vec<crate::notebook::Cell> = rest.into_iter().map(|(_, c)| c).collect();

    // Resolve destination in the *remaining* list
    let dest = selection::resolve_single(to_expr, rest_cells.len() + selected.len())?;
    // Clamp to valid insertion point in rest_cells
    let insert_at = dest.min(rest_cells.len());

    for (offset, cell) in selected.into_iter().enumerate() {
        rest_cells.insert(insert_at + offset, cell);
    }

    nb.cells = rest_cells;
    nb.save(notebook, backup)?;

    if !quiet {
        eprintln!("Moved cells to position {}", dest + 1);
    }

    Ok(())
}
