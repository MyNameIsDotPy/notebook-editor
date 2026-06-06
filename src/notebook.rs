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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    pub cell_type: String,
    #[serde(default)]
    pub metadata: Value,
    /// Source lines. The spec allows both `Vec<String>` and a plain `String`;
    /// we normalise to `Vec<String>` on load and back on save.
    pub source: CellSource,
    /// Present only on code cells
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_count: Option<Value>,
    /// Present only on code cells
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Value>,
}

impl Cell {
    /// Return the full source as a single string (joining multi-line arrays).
    pub fn source_str(&self) -> String {
        match &self.source {
            CellSource::Lines(lines) => lines.join(""),
            CellSource::Single(s) => s.clone(),
        }
    }

    /// Replace the source, storing it as a single string.
    pub fn set_source(&mut self, src: String) {
        self.source = CellSource::Single(src);
    }

    /// Create a new blank cell of the given type.
    pub fn new(cell_type: &str) -> Self {
        Cell {
            cell_type: cell_type.to_string(),
            metadata: Value::Object(Default::default()),
            source: CellSource::Single(String::new()),
            execution_count: if cell_type == "code" {
                Some(Value::Null)
            } else {
                None
            },
            outputs: Vec::new(),
        }
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
