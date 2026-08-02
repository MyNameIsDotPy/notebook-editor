use crate::notebook::{Cell, Notebook};
use anyhow::Result;

pub fn run(a: &str, b: &str, detailed: bool) -> Result<()> {
    let nb_a = Notebook::from_file(a)?;
    let nb_b = Notebook::from_file(b)?;

    let cells_a = &nb_a.cells;
    let cells_b = &nb_b.cells;

    // Myers-style LCS diff on cell source+type as the key.
    // We keep it simple: compute the edit script via the standard DP table.
    let ops = diff_cells(cells_a, cells_b);

    let mut changed = 0usize;
    let mut idx_a = 0usize; // 1-based display counter for a
    let mut idx_b = 0usize; // 1-based display counter for b

    for op in &ops {
        match op {
            Op::Keep(i, j) => {
                idx_a += 1;
                idx_b += 1;
                let _ = (i, j);
            }
            Op::Remove(i) => {
                idx_a += 1;
                let cell = &cells_a[*i];
                println!("- [Cell {idx_a} | {}]", cell.cell_type);
                print_source_prefixed(cell, "- ");
                changed += 1;
            }
            Op::Add(j) => {
                idx_b += 1;
                let cell = &cells_b[*j];
                println!("+ [Cell {idx_b} | {}]", cell.cell_type);
                print_source_prefixed(cell, "+ ");
                changed += 1;
            }
            Op::Change(i, j) => {
                idx_a += 1;
                idx_b += 1;
                let ca = &cells_a[*i];
                let cb = &cells_b[*j];
                println!("~ [Cell {idx_a}→{idx_b} | {}]", cb.cell_type);
                if detailed {
                    print_line_diff(&ca.source_str(), &cb.source_str());
                } else {
                    print_source_prefixed(cb, "  ");
                }
                changed += 1;
            }
        }
    }

    if changed == 0 {
        eprintln!("Notebooks are identical (source content)");
    } else {
        eprintln!("{changed} cell(s) differ");
        std::process::exit(1);
    }

    Ok(())
}

// ── Diff primitives ──────────────────────────────────────────────────────────

enum Op {
    Keep(usize, usize),
    Remove(usize),
    Add(usize),
    Change(usize, usize),
}

fn cell_key(c: &Cell) -> String {
    format!("{}|{}", c.cell_type, c.source_str())
}

fn diff_cells(a: &[Cell], b: &[Cell]) -> Vec<Op> {
    let m = a.len();
    let n = b.len();

    // DP table: lcs[i][j] = LCS length for a[..i] and b[..j]
    let mut lcs = vec![vec![0usize; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if cell_key(&a[i - 1]) == cell_key(&b[j - 1]) {
                lcs[i][j] = lcs[i - 1][j - 1] + 1;
            } else {
                lcs[i][j] = lcs[i - 1][j].max(lcs[i][j - 1]);
            }
        }
    }

    // Backtrack
    let mut ops = Vec::new();
    let mut i = m;
    let mut j = n;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && cell_key(&a[i - 1]) == cell_key(&b[j - 1]) {
            ops.push(Op::Keep(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if i > 0 && j > 0 && lcs[i - 1][j] == lcs[i][j - 1] {
            // Same position, different content → Change
            ops.push(Op::Change(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || lcs[i][j - 1] >= lcs[i - 1][j]) {
            ops.push(Op::Add(j - 1));
            j -= 1;
        } else {
            ops.push(Op::Remove(i - 1));
            i -= 1;
        }
    }

    ops.reverse();
    ops
}

fn print_source_prefixed(cell: &Cell, prefix: &str) {
    let src = cell.source_str();
    for line in src.lines() {
        println!("{prefix}{line}");
    }
    if !src.is_empty() {
        println!();
    }
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Cell;

    fn code(src: &str) -> Cell {
        let mut c = Cell::new("code");
        c.set_source(src.to_string());
        c
    }

    #[test]
    fn identical_cells_are_all_keeps() {
        let cells = vec![code("x = 1"), code("y = 2")];
        let ops = diff_cells(&cells, &cells.clone());
        assert!(ops.iter().all(|op| matches!(op, Op::Keep(_, _))));
    }

    #[test]
    fn added_cell_detected() {
        let ops = diff_cells(&[code("x = 1")], &[code("x = 1"), code("y = 2")]);
        assert_eq!(ops.iter().filter(|op| matches!(op, Op::Add(_))).count(), 1);
    }

    #[test]
    fn removed_cell_detected() {
        let ops = diff_cells(&[code("x = 1"), code("y = 2")], &[code("x = 1")]);
        assert_eq!(
            ops.iter().filter(|op| matches!(op, Op::Remove(_))).count(),
            1
        );
    }

    #[test]
    fn changed_cell_detected() {
        let ops = diff_cells(&[code("x = 1")], &[code("x = 99")]);
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Op::Change(_, _)))
                .count(),
            1
        );
    }

    #[test]
    fn kept_cells_are_not_reported_as_changed() {
        let shared = code("shared");
        let ops = diff_cells(
            &[shared.clone(), code("old")],
            &[shared.clone(), code("new")],
        );
        assert_eq!(
            ops.iter().filter(|op| matches!(op, Op::Keep(_, _))).count(),
            1
        );
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Op::Change(_, _)))
                .count(),
            1
        );
    }
}

// Line-level unified-style diff for the --detailed flag
fn print_line_diff(src_a: &str, src_b: &str) {
    let lines_a: Vec<&str> = src_a.lines().collect();
    let lines_b: Vec<&str> = src_b.lines().collect();

    let m = lines_a.len();
    let n = lines_b.len();
    let mut lcs = vec![vec![0usize; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            lcs[i][j] = if lines_a[i - 1] == lines_b[j - 1] {
                lcs[i - 1][j - 1] + 1
            } else {
                lcs[i - 1][j].max(lcs[i][j - 1])
            };
        }
    }

    let mut line_ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && lines_a[i - 1] == lines_b[j - 1] {
            line_ops.push((' ', lines_a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || lcs[i][j - 1] >= lcs[i - 1][j]) {
            line_ops.push(('+', lines_b[j - 1]));
            j -= 1;
        } else {
            line_ops.push(('-', lines_a[i - 1]));
            i -= 1;
        }
    }
    line_ops.reverse();

    for (mark, line) in line_ops {
        println!("  {mark} {line}");
    }
    println!();
}
