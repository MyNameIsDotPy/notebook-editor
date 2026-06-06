---
name: nbedit
description: Use when the user asks about nbedit, how to read/edit/create/delete/move cells in a Jupyter notebook from the CLI, or how to use notebook-editor. Covers all nbedit subcommands (read, create, edit, delete, move, info), cell selection syntax, flags, and build/install instructions.
---

# nbedit — notebook-editor skill

`nbedit` is a Rust CLI tool for editing Jupyter `.ipynb` notebooks from the terminal without a GUI. The binary is built from this project.

---

## Build & install

```sh
# Dev build (fast, unoptimised)
cargo build
./target/debug/nbedit --help

# Release build
cargo build --release
./target/release/nbedit --help

# Install globally as `nbedit`
cargo install --path .
nbedit --help
```

Run without installing using `cargo run -- <args>`:

```sh
cargo run -- info notebook.ipynb
```

---

## Cell selection syntax

All commands that accept a `<SELECTION>` support:

| Expression    | Meaning                          |
|---------------|----------------------------------|
| `3`           | Cell 3                           |
| `1,3,5`       | Cells 1, 3 and 5                 |
| `2-6`         | Cells 2 through 6 (inclusive)    |
| `1,3-5,8`     | Mix of indices and ranges        |
| `last`        | Last cell in the notebook        |
| `all`         | Every cell                       |

Indices are **1-based**.

---

## Commands

### `info` — notebook summary

```sh
nbedit info <NOTEBOOK>
```

Prints kernel, nbformat version, total cells and a per-cell table (type, lines, outputs).

```sh
nbedit info analysis.ipynb
```

---

### `read` — print cell source

```sh
nbedit read <NOTEBOOK> <SELECTION> [OPTIONS]
```

| Option           | Description                                      |
|------------------|--------------------------------------------------|
| `--type`         | Filter: `code`, `markdown`, `raw`                |
| `--show-outputs` | Also print cell outputs (code cells only)        |
| `--json`         | Emit the full cell JSON instead of plain source  |

```sh
nbedit read analysis.ipynb 3
nbedit read analysis.ipynb 1,3,5
nbedit read analysis.ipynb 2-6
nbedit read analysis.ipynb all
nbedit read analysis.ipynb all --type markdown
nbedit read analysis.ipynb 1 --show-outputs
nbedit read analysis.ipynb 2 --json
```

---

### `create` — add a new cell

```sh
nbedit create <NOTEBOOK> [OPTIONS]
```

| Option          | Default | Description                                  |
|-----------------|---------|----------------------------------------------|
| `--type`        | `code`  | `code`, `markdown`, `raw`                    |
| `--at <INDEX>`  | end     | Insert before this index (shifts cells down) |
| `--source <S>`  | —       | Inline source string                         |
| `--file <PATH>` | —       | Read source from a file                      |

```sh
# Append a code cell
nbedit create analysis.ipynb --source "print('hello')"

# Insert a markdown cell before cell 3
nbedit create analysis.ipynb --type markdown --at 3 --source "## New section"

# Create from a file
nbedit create analysis.ipynb --file snippet.py
```

---

### `edit` — replace source of a cell

```sh
nbedit edit <NOTEBOOK> <INDEX> [OPTIONS]
```

`<INDEX>` must be a single cell number.

| Option          | Description                              |
|-----------------|------------------------------------------|
| `--source <S>`  | Replace source with inline text          |
| `--file <PATH>` | Replace source with file contents        |
| `--editor`      | Open the cell in `$EDITOR` (default: vi) |
| `--type`        | Change the cell type                     |

```sh
nbedit edit analysis.ipynb 4 --source "x = 42"
nbedit edit analysis.ipynb 4 --file updated.py
nbedit edit analysis.ipynb 4 --editor
nbedit edit analysis.ipynb 4 --type markdown
```

---

### `delete` — remove cells

```sh
nbedit delete <NOTEBOOK> <SELECTION> [--dry-run]
```

`--dry-run` prints which cells would be deleted without touching the file.

```sh
nbedit delete analysis.ipynb 5
nbedit delete analysis.ipynb 2,4,7
nbedit delete analysis.ipynb 3-6
nbedit delete analysis.ipynb last --dry-run
```

---

### `move` — reorder cells

```sh
nbedit move <NOTEBOOK> <SELECTION> --to <INDEX|last>
```

```sh
# Move cell 7 to position 2
nbedit move analysis.ipynb 7 --to 2

# Move a range to the end
nbedit move analysis.ipynb 3-5 --to last
```

---

### `search` — search with regex

```sh
nbedit search <NOTEBOOK> <PATTERN> [OPTIONS]
```

| Option            | Description                                           |
|-------------------|-------------------------------------------------------|
| `--type`          | Filter: `code`, `markdown`, `raw`                     |
| `--show-source`   | Print full source of each matching cell               |
| `-i/--ignore-case`| Case-insensitive matching                             |

Matching lines are prefixed with `>`. Exits with code `1` if no matches found.

```sh
# Plain text
nbedit search analysis.ipynb "pandas"

# Regex: all function definitions
nbedit search analysis.ipynb "def \w+\("

# Case-insensitive
nbedit search analysis.ipynb "todo" -i

# Only code cells, full source context
nbedit search analysis.ipynb "TODO" --type code --show-source

# Lines starting with a comment
nbedit search analysis.ipynb "^#"
```

---

## Global flags

| Flag        | Description                                         |
|-------------|-----------------------------------------------------|
| `--backup`  | Write a `.bak` copy before modifying                |
| `-q`        | Suppress confirmation messages                      |
| `-v`        | Print debug information                             |

```sh
nbedit delete report.ipynb last --backup
```

---

## Exit codes

| Code | Meaning                    |
|------|----------------------------|
| 0    | Success                    |
| 1    | Usage error                |
| 2    | File not found             |
| 3    | Invalid notebook format    |
| 4    | Cell index out of range    |

---

## Source layout

```
src/
  main.rs          entry point
  cli.rs           clap argument definitions
  notebook.rs      Notebook/Cell structs + serde (nbformat 4)
  selection.rs     selection expression parser (unit-tested)
  error.rs         NbError enum
  commands/
    mod.rs         dispatch()
    read.rs
    create.rs
    edit.rs        includes $EDITOR support via tempfile
    delete.rs
    move.rs
    info.rs
    search.rs      regex search via the `regex` crate
```

Key crates: `clap 4` (derive), `serde`/`serde_json`, `anyhow`, `tempfile`, `regex`.

---

## Running tests

```sh
cargo test
```

12 unit tests: 8 in `src/selection.rs` (selection expressions) and 4 in `src/commands/search.rs` (regex patterns).
