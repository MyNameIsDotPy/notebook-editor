# notebook-editor

A command-line tool written in Rust for editing Jupyter notebooks (`.ipynb`) from the terminal. Manipulate individual cells or ranges without opening a GUI.

---

## Overview

`notebook-editor` treats a notebook as an ordered list of cells. Every command targets cells by **index** (1-based) or by a **range/set expression**. The tool reads and writes standard `.ipynb` JSON files and preserves all metadata it does not explicitly modify.

---

## Cell Selection Syntax

| Expression | Meaning |
|---|---|
| `3` | Cell 3 |
| `1,3,5` | Cells 1, 3 and 5 |
| `2-6` | Cells 2 through 6 (inclusive) |
| `1,3-5,8` | Mix of individual indices and ranges |
| `last` | Last cell in the notebook |
| `all` | Every cell |

---

## Commands

### Read

Print the source of one or more cells to stdout.

```
nbedit read <NOTEBOOK> <SELECTION>
```

**Examples**

```sh
nbedit read analysis.ipynb 3
nbedit read analysis.ipynb 1,3,5
nbedit read analysis.ipynb 2-6
nbedit read analysis.ipynb all
```

**Output format**

Each cell is printed with a header line followed by its source:

```
[Cell 3 | code]
import pandas as pd
df = pd.read_csv("data.csv")

[Cell 5 | markdown]
## Results
```

Optional flags:

| Flag | Description |
|---|---|
| `--type` | Filter by cell type: `code`, `markdown`, `raw` |
| `--show-outputs` | Also print cell outputs (code cells only) |
| `--json` | Emit the full cell JSON instead of plain source |

---

### Create

Append a new cell at the end of the notebook, or insert it at a specific position.

```
nbedit create <NOTEBOOK> [OPTIONS]
```

**Options**

| Option | Default | Description |
|---|---|---|
| `--type <TYPE>` | `code` | Cell type: `code`, `markdown`, `raw` |
| `--at <INDEX>` | end | Insert before this index (shifts subsequent cells down) |
| `--source <TEXT>` | — | Inline source string |
| `--file <PATH>` | — | Read source from a file |

**Examples**

```sh
# Append a code cell with inline source
nbedit create analysis.ipynb --source "print('hello')"

# Insert a markdown cell before cell 3
nbedit create analysis.ipynb --type markdown --at 3 --source "## New section"

# Create a code cell from a file
nbedit create analysis.ipynb --file snippet.py
```

---

### Edit

Replace the source of an existing cell.

```
nbedit edit <NOTEBOOK> <INDEX> [OPTIONS]
```

The index must be a single cell (not a range).

**Options**

| Option | Description |
|---|---|
| `--source <TEXT>` | Replace source with inline text |
| `--file <PATH>` | Replace source with file contents |
| `--editor` | Open the cell in `$EDITOR` (default: `vi`) |
| `--type <TYPE>` | Change the cell type |

**Examples**

```sh
nbedit edit analysis.ipynb 4 --source "x = 42"
nbedit edit analysis.ipynb 4 --file updated.py
nbedit edit analysis.ipynb 4 --editor
```

---

### Delete

Remove one or more cells.

```
nbedit delete <NOTEBOOK> <SELECTION>
```

**Examples**

```sh
nbedit delete analysis.ipynb 5
nbedit delete analysis.ipynb 2,4,7
nbedit delete analysis.ipynb 3-6
```

A `--dry-run` flag prints which cells would be deleted without modifying the file.

---

### Move

Reorder cells by moving a selection to a new position.

```
nbedit move <NOTEBOOK> <SELECTION> --to <INDEX>
```

**Examples**

```sh
# Move cell 7 to position 2
nbedit move analysis.ipynb 7 --to 2

# Move a range to the end
nbedit move analysis.ipynb 3-5 --to last
```

---

### Info

Print metadata and a summary of cells.

```
nbedit info <NOTEBOOK>
```

**Output example**

```
Notebook:  analysis.ipynb
Kernel:    python3 (Python 3.11.0)
Format:    nbformat 4.5
Cells:     12  (9 code, 2 markdown, 1 raw)

 #   type       lines  outputs
 1   code           3        0
 2   markdown       5        -
 3   code          12        2
...
```

---

## Global Flags

| Flag | Description |
|---|---|
| `--backup` | Write a `.bak` copy of the notebook before modifying it |
| `--no-backup` | Disable automatic backup (default behavior) |
| `--pretty` | Pretty-print the output JSON with 1-space indent (default: 1) |
| `-q / --quiet` | Suppress confirmation messages |
| `-v / --verbose` | Print debug information |

---

## Installation

### From source

```sh
git clone https://github.com/youruser/notebook-editor
cd notebook-editor
cargo build --release
# Binary at: ./target/release/nbedit
```

### Cargo

```sh
cargo install notebook-editor
```

---

## File Format

`notebook-editor` reads and writes the [nbformat 4](https://nbformat.readthedocs.io/en/latest/format_description.html) specification (`.ipynb` files). It preserves:

- Kernel and language metadata
- Cell metadata and tags
- Cell outputs and execution counts
- Notebook-level metadata

Only the fields explicitly targeted by a command are mutated.

---

## Architecture

```
src/
  main.rs          -- CLI entry point (clap)
  cli.rs           -- Argument definitions and subcommand enum
  notebook.rs      -- Notebook and Cell data model (serde)
  selection.rs     -- Parsing and evaluation of cell selection expressions
  commands/
    read.rs
    create.rs
    edit.rs
    delete.rs
    move.rs
    info.rs
  error.rs         -- Unified error type
```

**Key dependencies**

| Crate | Purpose |
|---|---|
| `clap` | CLI argument parsing |
| `serde` / `serde_json` | JSON serialization of `.ipynb` files |
| `anyhow` | Error handling |
| `tempfile` | Temporary files for `--editor` mode |

---

## Exit Codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Usage error (bad arguments) |
| 2 | File not found or unreadable |
| 3 | Invalid notebook format |
| 4 | Cell index out of range |

---

## Examples

```sh
# Print all markdown cells
nbedit read report.ipynb all --type markdown

# Delete the last cell and make a backup first
nbedit delete report.ipynb last --backup

# Replace cell 2 interactively in $EDITOR
nbedit edit report.ipynb 2 --editor

# Insert a separator markdown cell between cells 4 and 5
nbedit create report.ipynb --at 5 --type markdown --source "---"

# Show notebook summary
nbedit info report.ipynb
```

---

## License

MIT
