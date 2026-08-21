use crate::notebook::Notebook;
use crate::selection;
use anyhow::{bail, Result};

pub fn run(notebook: &str, selection: &str, backup: bool, quiet: bool) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;
    if indices.len() < 2 || indices.windows(2).any(|pair| pair[1] != pair[0] + 1) {
        bail!("Merge requires two or more contiguous cells");
    }
    let first = indices[0];
    let kind = nb.cells[first].cell_type.clone();
    if indices
        .iter()
        .any(|&index| nb.cells[index].cell_type != kind)
    {
        bail!("Merge requires cells of the same type");
    }
    let source: String = indices
        .iter()
        .map(|&index| nb.cells[index].source_str())
        .collect();
    nb.cells[first].set_source(source);
    if kind == "code" {
        nb.cells[first].outputs.clear();
        nb.cells[first].execution_count = Some(serde_json::Value::Null);
    }
    for index in indices.into_iter().skip(1).rev() {
        nb.cells.remove(index);
    }
    nb.save(notebook, backup)?;
    if !quiet {
        eprintln!("Merged cells into {}", first + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Cell;
    #[test]
    fn merges_contiguous_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.ipynb");
        let mut a = Cell::new("markdown");
        a.set_source("a\n".into());
        let mut b = Cell::new("markdown");
        b.set_source("b".into());
        let nb = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: serde_json::json!({}),
            cells: vec![a, b],
            extra: Default::default(),
        };
        nb.save(path.to_str().unwrap(), false).unwrap();
        run(path.to_str().unwrap(), "1-2", false, true).unwrap();
        assert_eq!(
            Notebook::from_file(path.to_str().unwrap()).unwrap().cells[0].source_str(),
            "a\nb"
        );
    }
}
