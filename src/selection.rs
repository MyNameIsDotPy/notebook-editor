use anyhow::{bail, Result};

/// Resolve a selection expression against `total` cells and return a sorted,
/// deduplicated list of 0-based indices.
///
/// Supported syntax (1-based, as seen by the user):
///   `all`       → every cell
///   `last`      → last cell
///   `3`         → single cell
///   `1,3,5`     → individual cells
///   `2-6`       → inclusive range
///   `1,3-5,8`   → mix
pub fn resolve(expr: &str, total: usize) -> Result<Vec<usize>> {
    if total == 0 {
        bail!("The notebook has no cells");
    }

    let expr = expr.trim();

    if expr.eq_ignore_ascii_case("all") {
        return Ok((0..total).collect());
    }
    if expr.eq_ignore_ascii_case("last") {
        return Ok(vec![total - 1]);
    }

    let mut indices: Vec<usize> = Vec::new();

    for part in expr.split(',') {
        let part = part.trim();
        if part.contains('-') {
            // Range: e.g. "2-6"
            let mut iter = part.splitn(2, '-');
            let start = parse_index(iter.next().unwrap().trim(), total)?;
            let end = parse_index(iter.next().unwrap().trim(), total)?;
            if start > end {
                bail!("Range start {s} is greater than end {e}", s = start + 1, e = end + 1);
            }
            indices.extend(start..=end);
        } else {
            indices.push(parse_index(part, total)?);
        }
    }

    // Sort and deduplicate
    indices.sort_unstable();
    indices.dedup();

    Ok(indices)
}

/// Parse a 1-based user index into a 0-based internal index.
fn parse_index(s: &str, total: usize) -> Result<usize> {
    let n: usize = s
        .parse()
        .map_err(|_| anyhow::anyhow!("'{s}' is not a valid cell index"))?;
    if n == 0 || n > total {
        bail!("Cell index {n} is out of range (notebook has {total} cells)");
    }
    Ok(n - 1)
}

/// Parse a destination expression for the `move` command.
/// Returns a 0-based index.
pub fn resolve_single(expr: &str, total: usize) -> Result<usize> {
    let expr = expr.trim();
    if expr.eq_ignore_ascii_case("last") {
        if total == 0 {
            bail!("The notebook has no cells");
        }
        return Ok(total - 1);
    }
    parse_index(expr, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all() {
        assert_eq!(resolve("all", 5).unwrap(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_last() {
        assert_eq!(resolve("last", 5).unwrap(), vec![4]);
    }

    #[test]
    fn test_single() {
        assert_eq!(resolve("3", 5).unwrap(), vec![2]);
    }

    #[test]
    fn test_list() {
        assert_eq!(resolve("1,3,5", 5).unwrap(), vec![0, 2, 4]);
    }

    #[test]
    fn test_range() {
        assert_eq!(resolve("2-4", 5).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn test_mixed() {
        assert_eq!(resolve("1,3-5,8", 10).unwrap(), vec![0, 2, 3, 4, 7]);
    }

    #[test]
    fn test_dedup() {
        assert_eq!(resolve("1,1,2", 5).unwrap(), vec![0, 1]);
    }

    #[test]
    fn test_out_of_range() {
        assert!(resolve("10", 5).is_err());
    }
}
