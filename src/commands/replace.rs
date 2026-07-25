use anyhow::{Context, Result};
use regex::Regex;
use crate::notebook::Notebook;
use crate::selection;

pub fn run(
    notebook: &str,
    sel: &str,
    pattern: &str,
    replacement: &str,
    type_filter: Option<&str>,
    ignore_case: bool,
    dry_run: bool,
    backup: bool,
    quiet: bool,
) -> Result<()> {
    let re = build_regex(pattern, ignore_case)
        .with_context(|| format!("Invalid regex pattern: '{pattern}'"))?;

    let mut nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(sel, nb.len())?;

    let mut total_replacements = 0usize;
    let mut cells_changed = 0usize;

    for idx in &indices {
        let cell = &nb.cells[*idx];

        if let Some(t) = type_filter {
            if cell.cell_type != t {
                continue;
            }
        }

        let source = cell.source_str();
        let new_source = re.replace_all(&source, replacement).into_owned();

        if new_source == source {
            continue;
        }

        // Count replacements in this cell
        let count = re.find_iter(&source).count();
        total_replacements += count;
        cells_changed += 1;

        if dry_run {
            println!("[Cell {} | {}] {} replacement(s)", idx + 1, cell.cell_type, count);
            // Show a diff-like preview
            for (line_num, (old, new)) in source.lines().zip(new_source.lines()).enumerate() {
                if old != new {
                    println!("  line {:>3} - {old}", line_num + 1);
                    println!("  line {:>3} + {new}", line_num + 1);
                }
            }
        } else {
            nb.cells[*idx].set_source(new_source);
        }
    }

    if dry_run {
        if total_replacements == 0 {
            eprintln!("No matches found for '{pattern}'");
            std::process::exit(1);
        }
        eprintln!("{total_replacements} replacement(s) in {cells_changed} cell(s) (dry run — no changes written)");
        return Ok(());
    }

    if total_replacements == 0 {
        eprintln!("No matches found for '{pattern}'");
        std::process::exit(1);
    }

    nb.save(notebook, backup)?;

    if !quiet {
        eprintln!("{total_replacements} replacement(s) in {cells_changed} cell(s)");
    }

    Ok(())
}

fn build_regex(pattern: &str, ignore_case: bool) -> Result<Regex> {
    let re = if ignore_case {
        Regex::new(&format!("(?i){pattern}"))
    } else {
        Regex::new(pattern)
    };
    re.map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_replace() {
        let re = build_regex("foo", false).unwrap();
        let result = re.replace_all("foo bar foo", "baz").into_owned();
        assert_eq!(result, "baz bar baz");
    }

    #[test]
    fn test_capture_group() {
        let re = build_regex(r"def (\w+)\(", false).unwrap();
        let result = re.replace_all("def my_func(x):", "fn $1(").into_owned();
        assert_eq!(result, "fn my_func(x):");
    }

    #[test]
    fn test_ignore_case_replace() {
        let re = build_regex("TODO", true).unwrap();
        let result = re.replace_all("# todo: fix this", "DONE").into_owned();
        assert_eq!(result, "# DONE: fix this");
    }

    #[test]
    fn test_invalid_regex() {
        assert!(build_regex("[bad", false).is_err());
    }
}
