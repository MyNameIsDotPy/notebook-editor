use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Top-level notebook structure (nbformat 4)
#[derive(Debug, Serialize, Deserialize)]
pub struct Notebook {
    pub nbformat: u32,
    pub nbformat_minor: u32,
    pub metadata: Value,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cell {
    pub cell_type: String,
    #[serde(default)]
    pub metadata: Value,
    pub source: CellSource,
    #[serde(default)]
    pub execution_count: Option<Value>,
    #[serde(default)]
    pub outputs: Vec<Value>,
}

impl serde::Serialize for Cell {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let is_code = self.cell_type == "code";
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("cell_type", &self.cell_type)?;
        if is_code {
            map.serialize_entry("execution_count", &self.execution_count)?;
        }
        map.serialize_entry("metadata", &self.metadata)?;
        if is_code {
            map.serialize_entry("outputs", &self.outputs)?;
        }
        map.serialize_entry("source", &self.source)?;
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
            cell_type: cell_type.to_string(),
            metadata: Value::Object(Default::default()),
            source: CellSource::Lines(Vec::new()),
            execution_count: if cell_type == "code" {
                Some(Value::Null)
            } else {
                None
            },
            outputs: Vec::new(),
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
    fn code_cell_serializes_outputs_and_execution_count() {
        let json = serde_json::to_value(Cell::new("code")).unwrap();
        assert!(json.get("outputs").is_some(), "code cell must have outputs");
        assert!(json.get("execution_count").is_some(), "code cell must have execution_count");
    }

    #[test]
    fn markdown_cell_omits_outputs_and_execution_count() {
        let json = serde_json::to_value(Cell::new("markdown")).unwrap();
        assert!(json.get("outputs").is_none(), "markdown must not have outputs");
        assert!(json.get("execution_count").is_none(), "markdown must not have execution_count");
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
            std::fs::copy(path, &bak)
                .with_context(|| format!("Cannot write backup '{bak}'"))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json).with_context(|| format!("Cannot write '{path}'"))?;
        Ok(())
    }

    /// Total number of cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Kernel display name from metadata, if present.
    pub fn kernel_name(&self) -> Option<&str> {
        self.metadata
            .get("kernelspec")
            .and_then(|k| k.get("display_name"))
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
