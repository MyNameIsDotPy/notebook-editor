use nbedit::{
    notebook::{Cell, Notebook},
    structural::{self, Operation},
};
use serde_json::json;

fn notebook(source: &str) -> Notebook {
    let mut cell = Cell::new("code");
    cell.id = Some("stable-id".into());
    cell.set_source(source.into());
    cell.extra.insert("extension".into(), json!({"keep":true}));
    cell.outputs
        .push(json!({"output_type":"stream","name":"stdout","text":["previous\n"]}));
    Notebook {
        nbformat: 4,
        nbformat_minor: 5,
        metadata: json!({}),
        cells: vec![cell],
        extra: Default::default(),
    }
}

#[test]
fn wrapping_preserves_string_values_comments_and_continuations() {
    let source = "# comment\r\ns = '''first\r\n  second\r\nlast'''\r\nx = (1 +\r\n     2)\r\n\r\ny = 1 + \\\r\n    2\r\n";
    let mut nb = notebook(source);
    let original = serde_json::to_value(&nb.cells[0]).unwrap();
    structural::apply(
        &mut nb,
        &Operation::Wrap {
            cell: 1,
            description: Some("Example".into()),
            input: Some("None".into()),
            output: Some("Values".into()),
            logger_info: Some("start\n'quoted'".into()),
            logger_error: Some("failed".into()),
        },
        None,
    )
    .unwrap();
    let wrapped = nb.cells[0].source_str();
    assert!(wrapped.contains("    # comment\r\n"));
    assert!(wrapped.contains("'''first\r\n  second\r\nlast'''"));
    assert!(wrapped.contains("exc_info=True"));
    assert!(structural::syntax_issues(&nb, None).unwrap().is_empty());
    let comparison = structural::python(
        json!({"mode":"syntax", "cells":[{"cell":1,"source":wrapped}]}),
        None,
    )
    .unwrap();
    assert_eq!(comparison, json!([]));
    let saved = serde_json::to_value(&nb.cells[0]).unwrap();
    for key in ["id", "metadata", "outputs", "extension", "execution_count"] {
        assert_eq!(original[key], saved[key]);
    }
}

#[test]
fn lines_are_counted_within_cell_and_errors_say_lines() {
    let mut nb = notebook("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n");
    structural::apply(
        &mut nb,
        &Operation::Replace {
            cell: 1,
            lines: Some("8-last".into()),
            source: "replacement".into(),
        },
        None,
    )
    .unwrap();
    assert_eq!(
        nb.cells[0].source_str(),
        "a\nb\nc\nd\ne\nf\ng\nreplacement\n"
    );
    let error = structural::apply(
        &mut nb,
        &Operation::Replace {
            cell: 1,
            lines: Some("20".into()),
            source: "x".into(),
        },
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("cell has 8 lines"), "{error}");
    assert!(!error.contains("notebook"));
}

#[test]
fn insertion_uses_range_boundaries_and_delete_keeps_unselected_lines() {
    let mut nb = notebook("a\nb\nc\nd");
    structural::apply(
        &mut nb,
        &Operation::InsertAfter {
            cell: 1,
            lines: "2-3".into(),
            source: "x\ny".into(),
        },
        None,
    )
    .unwrap();
    assert_eq!(nb.cells[0].source_str(), "a\nb\nc\nx\ny\nd");
    structural::apply(
        &mut nb,
        &Operation::Delete {
            cell: 1,
            lines: "2,4".into(),
        },
        None,
    )
    .unwrap();
    assert_eq!(nb.cells[0].source_str(), "a\nc\ny\nd");
}

#[test]
fn syntax_reports_cell_and_line_without_running_code() {
    let nb = notebook("try:\nprint('do not execute')\n");
    let issues = structural::syntax_issues(&nb, None).unwrap();
    assert_eq!(issues[0]["cell"], 1);
    assert_eq!(issues[0]["line"], 2);
}

#[test]
fn transactions_validate_final_state_and_never_write_on_failure_or_preview() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("n.ipynb");
    let path = path.to_str().unwrap();
    notebook("x = 1\n").save(path, false).unwrap();
    let before = std::fs::read(path).unwrap();
    let bad = Operation::Replace {
        cell: 1,
        lines: None,
        source: "try:\n".into(),
    };
    assert!(structural::transaction(path, &[bad], true, false, None).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(!std::path::Path::new(&format!("{path}.bak")).exists());
    let operations = vec![
        Operation::Replace {
            cell: 1,
            lines: None,
            source: "try:\n".into(),
        },
        Operation::InsertAfter {
            cell: 1,
            lines: "1".into(),
            source: "    x = 2\nexcept Exception:\n    raise\n".into(),
        },
    ];
    structural::transaction(path, &operations, true, true, None).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before);
    structural::transaction(path, &operations, true, false, None).unwrap();
    assert_eq!(std::fs::read(format!("{path}.bak")).unwrap(), before);
    nbedit::commands::edit::run_options(
        path,
        1,
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        false,
        false,
        None,
        Some(&format!("{path}.bak")),
        false,
        true,
    )
    .unwrap();
    assert_eq!(
        Notebook::from_file(path).unwrap().cells[0].source_str(),
        "x = 1\n"
    );
}

#[test]
fn lint_uses_ast_so_strings_and_comments_do_not_fake_statements() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("n.ipynb");
    let rules = dir.path().join("rules.yaml");
    notebook("# try: logger.info('fake')\ns = 'logger.error except raise'\nprint(s)\n")
        .save(path.to_str().unwrap(), false)
        .unwrap();
    std::fs::write(&rules,"required: [try, logger.info, except, logger.error, raise]\nforbidden_patterns: ['print\\(']\n").unwrap();
    let issues = structural::lint(path.to_str().unwrap(), rules.to_str().unwrap(), None).unwrap();
    assert_eq!(issues.len(), 6);
    assert_eq!(issues[5]["line"], 3);
}

#[test]
fn indent_preserves_multiline_fstrings_and_blank_lines() {
    let mut nb = notebook("s = f'''first\n  {1 + 2}\nlast'''\n\nx = (\n    1\n)\n");
    structural::apply(
        &mut nb,
        &Operation::Indent {
            cell: 1,
            lines: "1-last".into(),
            spaces: 4,
        },
        None,
    )
    .unwrap();
    assert_eq!(
        nb.cells[0].source_str(),
        "    s = f'''first\n  {1 + 2}\nlast'''\n\n    x = (\n        1\n    )\n"
    );
}

#[test]
fn cli_edit_line_range_is_independent_of_notebook_cell_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("n.ipynb");
    notebook("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n")
        .save(path.to_str().unwrap(), false)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_nbedit"))
        .args([
            "edit",
            path.to_str().unwrap(),
            "1",
            "--lines",
            "8-last",
            "--source",
            "replacement",
            "--backup",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let preview: Notebook = serde_json::from_slice(&result.stdout).unwrap();
    assert!(preview.cells[0].source_str().ends_with("g\nreplacement\n"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!dir.path().join("n.ipynb.bak").exists());
}
