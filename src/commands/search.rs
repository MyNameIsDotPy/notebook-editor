use crate::notebook::Notebook;
use anyhow::{Context, Result};
use regex::Regex;

pub fn run(
    notebook: &str,
    pattern: &str,
    type_filter: Option<&str>,
    show_source: bool,
    ignore_case: bool,
) -> Result<()> {
    let re = build_regex(pattern, ignore_case)
        .with_context(|| format!("Invalid regex pattern: '{pattern}'"))?;

    let nb = Notebook::from_file(notebook)?;

    let mut total_matches = 0;

    for (idx, cell) in nb.cells.iter().enumerate() {
        let cell_num = idx + 1;

        if let Some(t) = type_filter {
            if cell.cell_type != t {
                continue;
            }
        }

        let source = cell.source_str();
        let mut cell_matches: Vec<(usize, &str)> = Vec::new();

        for (line_num, line) in source.lines().enumerate() {
            if re.is_match(line) {
                cell_matches.push((line_num + 1, line));
            }
        }

        if cell_matches.is_empty() {
            continue;
        }

        total_matches += cell_matches.len();

        // Header
        println!("[Cell {cell_num} | {}]", cell.cell_type);

        if show_source {
            // Print the full source with matching lines highlighted by a marker
            for (line_num, line) in source.lines().enumerate() {
                let lnum = line_num + 1;
                if re.is_match(line) {
                    println!("{lnum:>4} > {line}");
                } else {
                    println!("{lnum:>4}   {line}");
                }
            }
        } else {
            // Print only matching lines
            for (line_num, line) in &cell_matches {
                println!("{line_num:>4} > {line}");
            }
        }

        println!();
    }

    if total_matches == 0 {
        eprintln!("No matches found for '{pattern}'");
        std::process::exit(1);
    } else {
        eprintln!("{total_matches} match(es) found");
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
    fn test_basic_pattern() {
        let re = build_regex("import", false).unwrap();
        assert!(re.is_match("import os"));
        assert!(!re.is_match("print('hello')"));
    }

    #[test]
    fn test_ignore_case() {
        let re = build_regex("import", true).unwrap();
        assert!(re.is_match("IMPORT os"));
    }

    #[test]
    fn test_regex_pattern() {
        let re = build_regex(r"def \w+\(", false).unwrap();
        assert!(re.is_match("def my_function(x):"));
        assert!(!re.is_match("class Foo:"));
    }

    #[test]
    fn test_invalid_regex() {
        assert!(build_regex("[invalid", false).is_err());
    }
}
