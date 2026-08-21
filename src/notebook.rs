use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Top-level notebook structure (nbformat 4)
#[derive(Debug, Serialize, Deserialize)]
pub struct Notebook {
    pub nbformat: u32,
    pub nbformat_minor: u32,
    pub metadata: Value,
    pub cells: Vec<Cell>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cell {
    #[serde(default)]
    pub id: Option<String>,
    pub cell_type: String,
    #[serde(default)]
    pub metadata: Value,
    pub source: CellSource,
    #[serde(default)]
    pub execution_count: Option<Value>,
    #[serde(default)]
    pub outputs: Vec<Value>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl serde::Serialize for Cell {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let is_code = self.cell_type == "code";
        let mut map = serializer.serialize_map(None)?;
        if let Some(id) = &self.id {
            map.serialize_entry("id", id)?;
        }
        map.serialize_entry("cell_type", &self.cell_type)?;
        if is_code {
            map.serialize_entry("execution_count", &self.execution_count)?;
        }
        map.serialize_entry("metadata", &self.metadata)?;
        if is_code {
            map.serialize_entry("outputs", &self.outputs)?;
        }
        map.serialize_entry("source", &self.source)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl Cell {
    /// Return the full source as a single string (joining multi-line arrays).
    pub fn source_str(&self) -> String {
        match &self.source {
            CellSource::Lines(lines) => lines.join(""),
            CellSource::Single(s) => s.clone(),
        }
    }

    /// Replace the source, normalising to the nbformat array-of-lines form.
    pub fn set_source(&mut self, src: String) {
        if src.is_empty() {
            self.source = CellSource::Lines(Vec::new());
            return;
        }
        // split_inclusive keeps the \n attached to each line, which matches the
        // nbformat spec (every line except the last ends with \n).
        let lines: Vec<String> = src.split_inclusive('\n').map(|l| l.to_string()).collect();
        self.source = CellSource::Lines(lines);
    }

    /// Create a new blank cell of the given type.
    pub fn new(cell_type: &str) -> Self {
        Cell {
            id: None,
            cell_type: cell_type.to_string(),
            metadata: Value::Object(Default::default()),
            source: CellSource::Lines(Vec::new()),
            execution_count: if cell_type == "code" {
                Some(Value::Null)
            } else {
                None
            },
            outputs: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(src: &str) -> Cell {
        let mut c = Cell::new("code");
        c.set_source(src.to_string());
        c
    }

    #[test]
    fn set_source_empty_yields_empty_array() {
        let mut c = code("");
        c.set_source(String::new());
        assert!(matches!(&c.source, CellSource::Lines(v) if v.is_empty()));
    }

    #[test]
    fn set_source_single_line_no_trailing_newline() {
        let c = code("x = 42");
        assert!(matches!(&c.source, CellSource::Lines(v) if v == &["x = 42"]));
    }

    #[test]
    fn set_source_multi_line_attaches_newlines() {
        let c = code("a\nb\nc");
        assert!(matches!(&c.source, CellSource::Lines(v) if v == &["a\n", "b\n", "c"]));
    }

    #[test]
    fn set_source_trailing_newline() {
        let c = code("a\nb\n");
        assert!(matches!(&c.source, CellSource::Lines(v) if v == &["a\n", "b\n"]));
    }

    #[test]
    fn source_str_roundtrip() {
        let original = "import pandas\ndf = pd.read_csv('data.csv')";
        assert_eq!(code(original).source_str(), original);
    }

    #[test]
    fn ensure_cell_ids_adds_unique_ids_without_replacing_existing_ids() {
        let mut first = code("a = 1");
        first.id = Some("existing".into());
        let mut nb = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: serde_json::json!({}),
            cells: vec![first, code("b = 2"), code("c = 3")],
            extra: BTreeMap::new(),
        };
        nb.ensure_cell_ids();
        let ids: Vec<_> = nb
            .cells
            .iter()
            .map(|cell| cell.id.as_deref().unwrap())
            .collect();
        assert_eq!(ids[0], "existing");
        assert_eq!(
            ids.len(),
            ids.iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }

    #[test]
    fn cell_id_roundtrips_through_json() {
        let mut cell = code("x = 1");
        cell.id = Some("cell-1".into());
        let value = serde_json::to_value(&cell).unwrap();
        assert_eq!(value["id"], "cell-1");
        let decoded: Cell = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.id.as_deref(), Some("cell-1"));
    }

    #[test]
    fn cell_attachments_and_unknown_fields_are_preserved() {
        let value = serde_json::json!({
            "id": "image-cell", "cell_type": "markdown", "metadata": {},
            "source": ["![plot](attachment:plot.png)"],
            "attachments": {"plot.png": {"image/png": "abc"}},
            "custom_extension": {"enabled": true}
        });
        let cell: Cell = serde_json::from_value(value.clone()).unwrap();
        let encoded = serde_json::to_value(cell).unwrap();
        assert_eq!(encoded["attachments"], value["attachments"]);
        assert_eq!(encoded["custom_extension"], value["custom_extension"]);
    }

    #[test]
    fn save_atomically_replaces_notebook_and_can_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.ipynb");
        std::fs::write(&path, "original").unwrap();
        let nb = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: serde_json::json!({}),
            cells: vec![code("x = 1")],
            extra: BTreeMap::new(),
        };
        nb.save(path.to_str().unwrap(), true).unwrap();
        let saved = Notebook::from_file(path.to_str().unwrap()).unwrap();
        assert_eq!(saved.cells[0].source_str(), "x = 1");
        assert_eq!(
            std::fs::read_to_string(format!("{}.bak", path.display())).unwrap(),
            "original"
        );
    }

    #[test]
    fn code_cell_serializes_outputs_and_execution_count() {
        let json = serde_json::to_value(Cell::new("code")).unwrap();
        assert!(json.get("outputs").is_some(), "code cell must have outputs");
        assert!(
            json.get("execution_count").is_some(),
            "code cell must have execution_count"
        );
    }

    #[test]
    fn markdown_cell_omits_outputs_and_execution_count() {
        let json = serde_json::to_value(Cell::new("markdown")).unwrap();
        assert!(
            json.get("outputs").is_none(),
            "markdown must not have outputs"
        );
        assert!(
            json.get("execution_count").is_none(),
            "markdown must not have execution_count"
        );
    }

    #[test]
    fn new_code_cell_source_serializes_as_empty_array() {
        let json = serde_json::to_value(Cell::new("code")).unwrap();
        let src = json.get("source").unwrap();
        assert!(src.is_array());
        assert_eq!(src.as_array().unwrap().len(), 0);
    }
}

/// The nbformat spec allows source to be either a string or an array of strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellSource {
    Lines(Vec<String>),
    Single(String),
}

impl Notebook {
    /// Read a notebook from disk.
    pub fn from_file(path: &str) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Cannot read '{path}'"))?;
        let nb: Notebook =
            serde_json::from_str(&content).with_context(|| format!("Invalid JSON in '{path}'"))?;
        Ok(nb)
    }

    /// Write the notebook back to disk, optionally creating a .bak first.
    pub fn save(&self, path: &str, backup: bool) -> Result<()> {
        if backup && Path::new(path).exists() {
            let bak = format!("{path}.bak");
            std::fs::copy(path, &bak).with_context(|| format!("Cannot write backup '{bak}'"))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let target = Path::new(path);
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("Cannot create temporary file beside '{path}'"))?;
        use std::io::Write;
        temp.write_all(json.as_bytes())
            .with_context(|| format!("Cannot write temporary notebook for '{path}'"))?;
        temp.as_file()
            .sync_all()
            .with_context(|| format!("Cannot flush temporary notebook for '{path}'"))?;
        temp.persist(target)
            .map_err(|e| e.error)
            .with_context(|| format!("Cannot replace '{path}'"))?;
        Ok(())
    }

    /// Total number of cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Add stable, nbformat-compatible IDs to legacy cells that do not have one.
    pub fn ensure_cell_ids(&mut self) {
        use std::collections::HashSet;
        let mut used: HashSet<String> = self.cells.iter().filter_map(|c| c.id.clone()).collect();
        for (index, cell) in self.cells.iter_mut().enumerate() {
            if cell.id.is_some() {
                continue;
            }
            let base = format!("nbedit-{}", index + 1);
            let mut id = base.clone();
            let mut suffix = 2;
            while used.contains(&id) {
                id = format!("{base}-{suffix}");
                suffix += 1;
            }
            used.insert(id.clone());
            cell.id = Some(id);
        }
    }

    /// Kernel display name from metadata, if present.
    pub fn kernel_name(&self) -> Option<&str> {
        self.metadata
            .get("kernelspec")
            .and_then(|k| k.get("display_name"))
            .and_then(|v| v.as_str())
    }

    /// Kernel identifier used to resolve a registered kernelspec.
    pub fn kernel_spec_name(&self) -> Option<&str> {
        self.metadata
            .get("kernelspec")
            .and_then(|k| k.get("name"))
            .and_then(|v| v.as_str())
    }

    /// Language name from metadata, if present.
    pub fn language(&self) -> Option<&str> {
        self.metadata
            .get("kernelspec")
            .and_then(|k| k.get("language"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                self.metadata
                    .get("language_info")
                    .and_then(|l| l.get("name"))
                    .and_then(|v| v.as_str())
            })
    }
}
