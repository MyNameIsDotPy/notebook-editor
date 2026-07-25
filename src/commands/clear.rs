use anyhow::Result;
use crate::notebook::{Cell, Notebook};
use crate::selection;

pub fn run(
    notebook: &str,
    selection: &str,
    dry_run: bool,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;

    let code_indices: Vec<usize> = indices
        .into_iter()
        .filter(|&i| nb.cells[i].cell_type == "code")
        .collect();

    if dry_run {
        for &i in &code_indices {
            let outputs = nb.cells[i].outputs.len();
            eprintln!(
                "Would clear cell {} ({} output(s))",
                i + 1,
                outputs,
            );
        }
        eprintln!("{} code cell(s) would be cleared (dry run)", code_indices.len());
        return Ok(());
    }

    for &i in &code_indices {
        nb.cells[i].outputs.clear();
        nb.cells[i].execution_count = Some(serde_json::Value::Null);
    }

    nb.save(notebook, backup)?;

    if !quiet {
        eprintln!("Cleared {} code cell(s)", code_indices.len());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn code_with_output(src: &str) -> Cell {
        let mut c = Cell::new("code");
        c.set_source(src.to_string());
        c.outputs.push(serde_json::json!({"output_type": "stream", "text": "ok"}));
        c.execution_count = Some(Value::Number(1.into()));
        c
    }

    #[test]
    fn clears_outputs_and_execution_count() {
        let (_dir, path) = write_nb(&make_nb(vec![code_with_output("x = 1")]));
        run(&path, "1", false, false, true).unwrap();
        let nb = Notebook::from_file(&path).unwrap();
        assert!(nb.cells[0].outputs.is_empty());
        assert_eq!(nb.cells[0].execution_count, Some(Value::Null));
    }

    #[test]
    fn skips_markdown_cells() {
        let mut md = Cell::new("markdown");
        md.set_source("# title".to_string());
        let (_dir, path) = write_nb(&make_nb(vec![md]));
        // should succeed without error even though no code cells are touched
        run(&path, "1", false, false, true).unwrap();
    }

    #[test]
    fn dry_run_does_not_modify_file() {
        let nb = make_nb(vec![code_with_output("x = 1")]);
        let json_before = serde_json::to_string_pretty(&nb).unwrap();
        let (_dir, path) = write_nb(&nb);
        run(&path, "1", true, false, true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), json_before);
    }
}
