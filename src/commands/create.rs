use crate::notebook::{Cell, Notebook};
use anyhow::{bail, Result};

pub fn run(
    notebook: &str,
    cell_type: &str,
    at: Option<usize>,
    source: Option<String>,
    file: Option<String>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;

    let src = resolve_source(source, file)?;

    let mut cell = Cell::new(cell_type);
    cell.set_source(src);

    let insert_pos = match at {
        Some(n) => {
            if n == 0 || n > nb.len() + 1 {
                bail!("--at {n} is out of range (notebook has {} cells)", nb.len());
            }
            n - 1 // convert to 0-based
        }
        None => nb.len(), // append
    };

    nb.cells.insert(insert_pos, cell);
    nb.save(notebook, backup)?;

    if !quiet {
        let display_pos = insert_pos + 1;
        eprintln!("Created {cell_type} cell at position {display_pos}");
    }

    Ok(())
}

pub fn resolve_source(source: Option<String>, file: Option<String>) -> Result<String> {
    match (source, file) {
        (Some(s), _) => Ok(s),
        (_, Some(f)) => Ok(std::fs::read_to_string(&f)
            .map_err(|e| anyhow::anyhow!("Cannot read source file '{f}': {e}"))?),
        (None, None) => Ok(String::new()),
    }
}
