use crate::notebook::Notebook;
use crate::selection;
use anyhow::{bail, Result};

pub fn run(
    src: &str,
    selection: &str,
    dst: &str,
    at: Option<usize>,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let src_nb = Notebook::from_file(src)?;
    let indices = selection::resolve(selection, src_nb.len())?;
    let cells: Vec<_> = indices.iter().map(|&i| src_nb.cells[i].clone()).collect();

    let mut dst_nb = Notebook::from_file(dst)?;

    let insert_pos = match at {
        Some(n) => {
            if n == 0 || n > dst_nb.len() + 1 {
                bail!(
                    "--at {n} is out of range (destination has {} cells)",
                    dst_nb.len()
                );
            }
            n - 1
        }
        None => dst_nb.len(),
    };

    // Insert in order; each successive cell goes one position further
    for (offset, cell) in cells.iter().enumerate() {
        dst_nb.cells.insert(insert_pos + offset, cell.clone());
    }

    dst_nb.save(dst, backup)?;

    if !quiet {
        eprintln!(
            "Copied {} cell(s) from {src} into {dst} at position {}",
            cells.len(),
            insert_pos + 1,
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::{Cell, Notebook};
    use serde_json::Value;

    fn make_nb(cells: Vec<Cell>) -> Notebook {
        Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: Value::Object(Default::default()),
            cells,
        }
    }

    fn write_nb(nb: &Notebook) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nb.ipynb");
        std::fs::write(&path, serde_json::to_string_pretty(nb).unwrap()).unwrap();
        (dir, path.to_str().unwrap().to_string())
    }

    fn code(src: &str) -> Cell {
        let mut c = Cell::new("code");
        c.set_source(src.to_string());
        c
    }

    #[test]
    fn copy_appends_when_no_at() {
        let (_sd, src) = write_nb(&make_nb(vec![code("x = 1"), code("y = 2")]));
        let (_dd, dst) = write_nb(&make_nb(vec![code("z = 3")]));
        run(&src, "1-2", &dst, None, false, true).unwrap();
        let nb = Notebook::from_file(&dst).unwrap();
        assert_eq!(nb.cells.len(), 3);
        assert_eq!(nb.cells[1].source_str(), "x = 1");
        assert_eq!(nb.cells[2].source_str(), "y = 2");
    }

    #[test]
    fn copy_with_at_inserts_at_position() {
        let (_sd, src) = write_nb(&make_nb(vec![code("inserted")]));
        let (_dd, dst) = write_nb(&make_nb(vec![code("first"), code("second")]));
        run(&src, "1", &dst, Some(2), false, true).unwrap();
        let nb = Notebook::from_file(&dst).unwrap();
        assert_eq!(nb.cells.len(), 3);
        assert_eq!(nb.cells[1].source_str(), "inserted");
        assert_eq!(nb.cells[2].source_str(), "second");
    }

    #[test]
    fn copy_preserves_source_cell_count() {
        let (_sd, src) = write_nb(&make_nb(vec![code("a"), code("b"), code("c")]));
        let (_dd, dst) = write_nb(&make_nb(vec![]));
        run(&src, "all", &dst, None, false, true).unwrap();
        let src_nb = Notebook::from_file(&src).unwrap();
        assert_eq!(
            src_nb.cells.len(),
            3,
            "source notebook must not be modified"
        );
    }
}
