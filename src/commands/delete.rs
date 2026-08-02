use crate::notebook::Notebook;
use crate::selection;
use anyhow::Result;

pub fn run(notebook: &str, sel: &str, dry_run: bool, backup: bool, quiet: bool) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let mut indices = selection::resolve(sel, nb.len())?;

    if dry_run {
        for idx in &indices {
            println!(
                "Would delete cell {} ({})",
                idx + 1,
                nb.cells[*idx].cell_type
            );
        }
        return Ok(());
    }

    // Remove in reverse order to keep indices valid
    indices.sort_unstable();
    for idx in indices.iter().rev() {
        nb.cells.remove(*idx);
    }

    nb.save(notebook, backup)?;

    if !quiet {
        eprintln!("Deleted {} cell(s)", indices.len());
    }

    Ok(())
}
