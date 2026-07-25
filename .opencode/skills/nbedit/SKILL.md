---
name: nbedit
description: Use when the user asks about nbedit, how to read/edit/create/delete/move/search/replace cells or lines in a Jupyter notebook from the CLI, or how to use notebook-editor. Covers all nbedit subcommands (read, create, edit, delete, move, info, search, replace), line-level operations (--lines, --insert-after, --insert-before, --delete-lines), regex search and replace with capture groups, cell selection syntax, global flags, and build/install instructions.
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
| `--lines <EXPR>` | Print only specific lines within each cell       |

```sh
nbedit read analysis.ipynb 3
nbedit read analysis.ipynb 1,3,5
nbedit read analysis.ipynb 2-6
nbedit read analysis.ipynb all
nbedit read analysis.ipynb all --type markdown
nbedit read analysis.ipynb 1 --show-outputs
nbedit read analysis.ipynb 2 --json

# Lines 2 to 5 of cell 3
nbedit read analysis.ipynb 3 --lines 2-5

# Lines 1 and 7 of cell 1
nbedit read analysis.ipynb 1 --lines 1,7
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

| Option           | Description                              |
|------------------|------------------------------------------|
| `--source <S>`   | Replace source with inline text          |
| `--file <PATH>`  | Replace source with file contents        |
| `--editor`       | Open the cell in `$EDITOR` (default: vi) |
| `--type`         | Change the cell type                     |
| `--lines <EXPR>` | Replace only specific lines within the cell |
| `--insert-after <N>` | Insert content after line N          |
| `--insert-before <N>` | Insert content before line N        |
| `--delete-lines <EXPR>` | Delete specific lines within the cell |

**`--lines` is a block replace.** All lines from the first to the last selected index are
removed and replaced with the full replacement content. The cell can grow or shrink.

```
--lines 3           removes line 3, inserts all replacement lines there
--lines 2-4         removes lines 2, 3, 4, inserts all replacement lines at position 2
--lines 1,5         removes lines 1 through 5 (the whole span), inserts replacement
```

**`--insert-before`/`--insert-after` bounds:** N must be 1..=cell_line_count.
To append to the end of a cell use `--insert-after <last_line_number>`.

**Multi-line `--source` in the shell:** use `$'line1\nline2'` (bash/zsh) or `--file`.
A double-quoted `"line1\nline2"` passes a literal backslash-n, not a newline.

**Safe workflow:**
```sh
# 1. See the cell with line numbers
nbedit read nb.ipynb 4
# 2. Apply change
nbedit edit nb.ipynb 4 --lines 3 --source $'new line A\nnew line B'
# 3. Verify
nbedit read nb.ipynb 4
```

```sh
nbedit edit analysis.ipynb 4 --source "x = 42"
nbedit edit analysis.ipynb 4 --file updated.py
nbedit edit analysis.ipynb 4 --editor
nbedit edit analysis.ipynb 4 --type markdown

# Replace line 3 of cell 4 with two lines (cell grows by 1)
nbedit edit analysis.ipynb 4 --lines 3 --source $'x = 99\ny = 0'

# Replace the block at lines 2-4 with two lines (cell shrinks by 1)
nbedit edit analysis.ipynb 1 --lines 2-4 --source $'# rewritten\n# second line'

# Insert after line 2
nbedit edit analysis.ipynb 4 --insert-after 2 --source "import json"

# Insert before line 1 (prepend)
nbedit edit analysis.ipynb 4 --insert-before 1 --source "# header"

# Insert from file after line 5
nbedit edit analysis.ipynb 4 --insert-after 5 --file snippet.py

# Delete lines 3 and 5
nbedit edit analysis.ipynb 4 --delete-lines 3,5

# Delete a range of lines
nbedit edit analysis.ipynb 4 --delete-lines 2-4
```

---

### `clear` — clear outputs and reset execution counts

```sh
nbedit clear <NOTEBOOK> <SELECTION> [--dry-run]
```

Clears `outputs` and resets `execution_count` to `null` on all code cells in the
selection. Markdown and raw cells are skipped. Use before committing notebooks to avoid
storing large output blobs in version control.

```sh
# Clear all cells
nbedit clear analysis.ipynb all

# Clear a specific range
nbedit clear analysis.ipynb 3-7

# Preview what would be cleared
nbedit clear analysis.ipynb all --dry-run
```

---

### `copy` — copy cells between notebooks

```sh
nbedit copy <SRC> <SELECTION> <DST> [--at N]
```

Copies the selected cells from `SRC` into `DST`. `--at N` inserts before position N
(1-based); omit to append.

```sh
# Append cells 2-4 from one notebook to another
nbedit copy analysis.ipynb 2-4 report.ipynb

# Insert cell 1 from src before cell 3 in dst
nbedit copy helpers.ipynb 1 analysis.ipynb --at 3
```

---

### `diff` — cell-level diff between two notebooks

```sh
nbedit diff <A> <B> [--detailed]
```

Compares two notebooks cell by cell, ignoring outputs and metadata. Shows added (`+`),
removed (`-`), and changed (`~`) cells. `--detailed` also shows a line-level diff of
the source within changed cells.

Exits with code `1` if any differences are found, `0` if notebooks are identical.

```sh
nbedit diff original.ipynb modified.ipynb
nbedit diff original.ipynb modified.ipynb --detailed
```

---

### `run` — execute cells and capture outputs

```sh
nbedit run <NOTEBOOK> <SELECTION> [OPTIONS]
```

Executes selected code cells via a Jupyter kernel (using `nbclient`) and writes the
outputs and execution counts back into the notebook. Markdown and raw cells in the
selection are skipped. Requires Python 3 with `nbclient` and `nbformat` installed:

```sh
pip install nbclient nbformat
```

| Option | Default | Description |
|---|---|---|
| `--timeout <N>` | `30` | Per-cell execution timeout in seconds |
| `--kernel <NAME>` | from notebook | Override the kernel name |

```sh
# Execute cell 3
nbedit run analysis.ipynb 3

# Execute all cells with a 60-second timeout
nbedit run analysis.ipynb all --timeout 60

# Execute cells 1-5 using a specific kernel
nbedit run analysis.ipynb 1-5 --kernel python3
```

Exits with code `1` if any cell raised an exception (outputs are still saved).
Exits with code `2` if `nbclient`/`nbformat` are not installed.

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

### `replace` — find and replace with regex

```sh
nbedit replace <NOTEBOOK> <SELECTION> <PATTERN> <REPLACEMENT>
```

| Option             | Description                                      |
|--------------------|--------------------------------------------------|
| `--type`           | Filter: `code`, `markdown`, `raw`                |
| `-i/--ignore-case` | Case-insensitive matching                        |
| `--dry-run`        | Preview changes without modifying the file       |

Replacement string supports capture groups: `$1`, `$2`, …

```sh
# Simple replacement in all cells
nbedit replace analysis.ipynb all "foo" "bar"

# Case-insensitive
nbedit replace analysis.ipynb all "todo" "DONE" -i

# Rename a function using capture group
nbedit replace analysis.ipynb all "def (\w+)\(" "def new_$1("

# Preview before writing
nbedit replace analysis.ipynb all "old_api" "new_api" --dry-run

# Only code cells
nbedit replace analysis.ipynb 1,3-5 "import numpy" "import numpy as np" --type code
```

Exits with code `1` if no matches found.

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

| Code | Meaning                                                                 |
|------|-------------------------------------------------------------------------|
| 0    | Success                                                                 |
| 1    | Runtime error (file not found, bad format, out of range) — or no matches found by `search`/`replace` |
| 2    | Bad CLI arguments (clap)                                                |

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
    read.rs        supports --lines
    create.rs
    edit.rs        includes $EDITOR support, --lines for line-level edits
    delete.rs
    move.rs
    info.rs
    search.rs      regex search via the `regex` crate
    replace.rs     regex find & replace with capture group support
```

Key crates: `clap 4` (derive), `serde`/`serde_json`, `anyhow`, `tempfile`, `regex`.

---

## Running tests

```sh
cargo test
```

16 unit tests: 8 in `src/selection.rs`, 4 in `src/commands/search.rs`, 4 in `src/commands/replace.rs`.
