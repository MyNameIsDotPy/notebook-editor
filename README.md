# notebook-editor

A command-line tool written in Rust for editing Jupyter notebooks (`.ipynb`) from the terminal. Manipulate individual cells or ranges without opening a GUI.

> Also available as an **[opencode](https://opencode.ai) skill** and a native **MCP server** — letting AI agents inspect, edit, discover kernels for, and explicitly execute notebooks from a coding session. See [MCP server](#mcp-server) and [Use as an opencode skill](#use-as-an-opencode-skill).

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
| `--lines <EXPR>` | Print only specific lines within each cell (same selection syntax) |

**Reading specific lines of a cell**

```sh
# Lines 2 to 4 of cell 3
nbedit read analysis.ipynb 3 --lines 2-4

# Lines 1, 5 and 7 of cell 1
nbedit read analysis.ipynb 1 --lines 1,5,7
```

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
| `--lines <EXPR>` | Replace only specific lines within the cell |
| `--insert-after <N>` | Insert new content after line N |
| `--insert-before <N>` | Insert new content before line N |
| `--delete-lines <EXPR>` | Delete specific lines within the cell |

**Examples**

```sh
nbedit edit analysis.ipynb 4 --source "x = 42"
nbedit edit analysis.ipynb 4 --file updated.py
nbedit edit analysis.ipynb 4 --editor

# Replace line 3 of cell 4
nbedit edit analysis.ipynb 4 --lines 3 --source "x = 99"

# Replace lines 2 to 4
nbedit edit analysis.ipynb 4 --lines 2-4 --source "# rewritten"

# Insert a new line after line 2
nbedit edit analysis.ipynb 4 --insert-after 2 --source "import json"

# Insert multiple lines before line 1
nbedit edit analysis.ipynb 4 --insert-before 1 --source "# header"

# Insert from a file after line 5
nbedit edit analysis.ipynb 4 --insert-after 5 --file snippet.py

# Delete lines 3 and 5
nbedit edit analysis.ipynb 4 --delete-lines 3,5

# Delete a range of lines
nbedit edit analysis.ipynb 4 --delete-lines 2-4
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

### Search

Search for a pattern across all cell sources. Supports full regular expressions.

```
nbedit search <NOTEBOOK> <PATTERN> [OPTIONS]
```

**Options**

| Option | Description |
|---|---|
| `--type` | Filter by cell type: `code`, `markdown`, `raw` |
| `--show-source` | Print the full source of each matching cell |
| `-i / --ignore-case` | Case-insensitive matching |

**Output format**

Matching lines are prefixed with `>`. Non-matching lines are shown only when `--show-source` is used:

```
[Cell 3 | code]
   1 > import pandas as pd
   3 > import numpy as np
```

**Examples**

```sh
# Plain text search
nbedit search analysis.ipynb "pandas"

# Regex: find all function definitions
nbedit search analysis.ipynb "def \w+\("

# Case-insensitive
nbedit search analysis.ipynb "todo" -i

# Only in code cells, show full source context
nbedit search analysis.ipynb "TODO" --type code --show-source

# Lines starting with a comment
nbedit search analysis.ipynb "^#"
```

Exits with code `1` if no matches are found.

---

### Replace

Find and replace text using regular expressions across one or more cells.

```
nbedit replace <NOTEBOOK> <SELECTION> <PATTERN> <REPLACEMENT>
```

**Options**

| Option | Description |
|---|---|
| `--type` | Filter by cell type: `code`, `markdown`, `raw` |
| `-i / --ignore-case` | Case-insensitive matching |
| `--dry-run` | Preview changes without modifying the file |

The replacement string supports **capture groups** using `$1`, `$2`, etc.

**Examples**

```sh
# Simple text replacement across all cells
nbedit replace analysis.ipynb all "foo" "bar"

# Case-insensitive replacement
nbedit replace analysis.ipynb all "todo" "DONE" -i

# Rename a function using a capture group
nbedit replace analysis.ipynb all "def (\w+)\(" "def new_$1("

# Preview changes before writing
nbedit replace analysis.ipynb all "old_api" "new_api" --dry-run

# Only in code cells
nbedit replace analysis.ipynb 1,3-5 "import numpy" "import numpy as np" --type code
```

Exits with code `1` if no matches are found.

---

### Run

Execute selected code cells through a locally discovered Jupyter kernel and merge outputs, execution counts, and execution metadata back into the notebook.

```sh
nbedit run <NOTEBOOK> <SELECTION> [OPTIONS]
```

Without an override, `nbedit` ranks the notebook's configured kernelspec, workspace environments such as `.venv`, the active virtual/Conda environment, registered kernelspecs, environment-manager installations, and Python on `PATH`.

```sh
# Automatically resolve the best kernel
nbedit run analysis.ipynb all

# Use an unregistered Python environment directly
nbedit run analysis.ipynb all --interpreter .venv/bin/python

# Run setup cells before cell 8, but only update cell 8
nbedit run analysis.ipynb 8 --include-prior

# Inspect resolution without starting a kernel
nbedit run analysis.ipynb all --dry-run
nbedit run analysis.ipynb all --dry-run --json

# Run against a persistent kernel so state survives across invocations
nbedit session start --name demo --interpreter .venv/bin/python
nbedit run analysis.ipynb 1 --session demo
nbedit run analysis.ipynb 2 --session demo
nbedit session stop demo
```

Important options:

| Option | Description |
|---|---|
| `--kernel <ID>` | Discovered candidate ID or registered kernelspec name |
| `--interpreter <PATH>` | Python interpreter that runs notebook code; requires `ipykernel` |
| `--driver-python <PATH>` | Python containing `nbclient` and `nbformat`; `--python` remains an alias |
| `--include-prior` | Execute earlier code cells as context without updating them |
| `--allow-errors` | Continue after cell errors and save all resulting outputs |
| `--timeout <SECONDS>` | Per-cell limit; `-1` disables it |
| `--overall-timeout <SECONDS>` | Wall-clock limit for the entire operation |
| `--startup-timeout <SECONDS>` | Kernel startup limit |
| `--iopub-timeout <SECONDS>` | Kernel output-channel limit |
| `--no-record-timing` | Do not write execution timing metadata |
| `--cwd <PATH>` | Override the kernel working directory |
| `--env KEY=VALUE` | Pass an environment value; repeatable |
| `--json` | Emit a structured execution report |
| `--session <NAME>` | Run against a persistent kernel session instead of a one-shot kernel; see [Session](#session) |
| `--create-session` | With `--session`, create it if it doesn't already exist |

By default every `run` is stateless: a fresh kernel starts, executes the selection, and shuts down within the call. `--session` runs against a long-lived kernel started with `nbedit session start` instead, so variables and imports from one `run` are still there for the next. `--include-prior` and `--session` cannot be combined — with a session, prior state already lives in the kernel.

`--json` reports `status`, `kernel`, `source`, `executed_cells`, `failed_cell`, `duration_ms`, and `outputs_saved`; when `--session` is used, it also includes `session` with the session's name, so scripts can confirm which one ran (including when `--create-session` created it on the fly).

The driver and kernel are separate processes: the driver needs `nbclient` and `nbformat`, while a Python kernel needs `ipykernel`. For Python kernels, `nbedit` automatically prefers the interpreter recorded in the selected kernelspec as the driver when it has the required packages. `--driver-python` is only an override. Registered non-Python kernels use their kernelspec launch command while `nbedit` discovers a suitable Python driver independently.

Exit codes are `0` for completed execution (including allowed errors), `1` for execution or cell failure, `2` for missing driver dependencies, `124` for an overall timeout, and `130` for Ctrl-C. Available partial outputs are saved when the driver can shut down cleanly.

---

### Kernels

Discover registered Jupyter kernels and usable Python environments on this machine.

```
nbedit kernels
```

**Options**

| Option | Description |
|---|---|
| `--json` | Emit normalized candidates and resolution information as JSON |
| `--details` | Show source, interpreter, kernelspec, and resource paths |
| `--check` | Verify that discovered Python interpreters can import `ipykernel` |
| `--notebook <PATH>` | Rank candidates for a particular notebook |
| `--driver-python <PATH>` | Also inspect kernels installed under that Python prefix |

Discovery reads standard Jupyter user/system locations and `JUPYTER_PATH`, then adds workspace `.venv`/`venv`, Pixi and local Conda environments, active `VIRTUAL_ENV`/`CONDA_PREFIX`, Conda/Micromamba, Pyenv, Poetry, Pipenv, and Python on `PATH`. Duplicate interpreters are collapsed to one deterministic candidate.

```sh
nbedit kernels --details
nbedit kernels --notebook analysis.ipynb --check
nbedit kernels --json
```

---

### Session

Manage persistent kernels used by `nbedit run --session`. A session is a small daemon that starts one kernel and keeps it (and its execution state) alive across separate `nbedit` invocations, until explicitly stopped.

```sh
nbedit session start [--name NAME] [--kernel ID] [--interpreter PATH] [--driver-python PATH] [--notebook PATH] [--cwd PATH] [--env KEY=VALUE] [--startup-timeout N] [--json]
nbedit session list [--json]
nbedit session stop <NAME> [--force]
```

```sh
# Start a session and reuse it across two separate runs
nbedit session start --name demo --interpreter .venv/bin/python
nbedit run notebook.ipynb 1 --session demo   # e.g. cell 1: x = 1
nbedit run notebook.ipynb 2 --session demo   # e.g. cell 2: print(x) -> 1

# See what's running, then shut it down
nbedit session list
nbedit session stop demo
```

Sessions never expire on their own — `session list` detects and prunes dead entries from its registry, but a live kernel only stops via `session stop` (graceful shutdown, falling back to killing the process; `--force` skips straight to that). Session registry files live under an OS data directory (e.g. `~/.local/share/nbedit/sessions` on Linux), separate from the notebook and workspace.

---

## Global Flags

| Flag | Description |
|---|---|
| `--backup` | Write a `.bak` copy of the notebook before modifying it |
| `-q / --quiet` | Suppress confirmation messages |
| `-v / --verbose` | Print debug information |

---

## MCP server

`nbedit-mcp` is a native [Model Context Protocol](https://modelcontextprotocol.io/) stdio server for notebook-aware AI clients. It shares the same Rust notebook, kernel-discovery, and execution implementation as the CLI; it does not invoke `nbedit` as a subprocess.

Start it with a workspace boundary:

```sh
nbedit-mcp --root /absolute/path/to/project
```

Example MCP client configuration:

```json
{
  "mcpServers": {
    "notebooks": {
      "command": "/absolute/path/to/nbedit-mcp",
      "args": ["--root", "/absolute/path/to/project"]
    }
  }
}
```

On Windows, use the executable and Windows paths:

```json
{
  "mcpServers": {
    "notebooks": {
      "command": "C:\\Tools\\nbedit-mcp.exe",
      "args": ["--root", "C:\\Users\\me\\project"]
    }
  }
}
```

Available tools:

| Tool | Capability |
|---|---|
| `notebook_info` | Notebook format, cell counts, and configured kernel |
| `notebook_read` | Structured selected-cell reading |
| `notebook_create_cell` | Insert code, markdown, or raw cells |
| `notebook_edit_cell` | Replace cell source or type |
| `notebook_delete_cells` | Delete a cell selection |
| `notebook_clear_outputs` | Clear code-cell outputs and counts |
| `notebook_list_kernels` | Discover kernels and Python environments |
| `notebook_run_cells` | Explicitly execute trusted cells and return outputs |
| `notebook_session_start` | Start a persistent kernel session |
| `notebook_session_list` | List sessions and whether their kernel is alive |
| `notebook_session_stop` | Stop a session and shut down its kernel |

Notebooks are also exposed as `notebook:///{path}` resources. Paths are canonicalized and restricted to `--root`; parent traversal and symlink escapes are rejected. Mutating tools create `.bak` files by default. The server never installs packages automatically. Kernel execution runs local notebook code with the permissions of the MCP server process and should only be enabled for trusted workspaces.

`notebook_run_cells` is stateless by default — a fresh kernel per call. Pass `session` (a name returned by `notebook_session_start`) to run against a persistent kernel instead, so state carries over to later `notebook_run_cells` calls until `notebook_session_stop`. Treat an active session under the same trust bar as execution itself, since it accumulates arbitrary code and state across calls rather than resetting each time.

---

## Installation

### From source

```sh
git clone https://github.com/MyNameIsDotPy/notebook-editor
cd notebook-editor
cargo build --release
# Binary at: ./target/release/nbedit
```

### Cargo

```sh
cargo install --git https://github.com/MyNameIsDotPy/notebook-editor
```

### Pre-built binaries

Download the latest binary for your platform from the [Releases](https://github.com/MyNameIsDotPy/notebook-editor/releases) page:

| File | Platform |
|---|---|
| `nbedit-linux-x86_64` | Linux x86_64 (Ubuntu, Arch, Debian, …) |
| `nbedit-linux-aarch64` | Linux ARM64 |
| `nbedit-windows-x86_64.exe` | Windows x86_64 |
| `nbedit-mcp-linux-x86_64` | MCP server, Linux x86_64 |
| `nbedit-mcp-linux-aarch64` | MCP server, Linux ARM64 |
| `nbedit-mcp-windows-x86_64.exe` | MCP server, Windows x86_64 |

```sh
# Linux example
chmod +x nbedit-linux-x86_64
sudo mv nbedit-linux-x86_64 /usr/local/bin/nbedit
```

---

## File Format

`notebook-editor` reads and writes the [nbformat 4](https://nbformat.readthedocs.io/en/latest/format_description.html) specification (`.ipynb` files). It preserves:

- Kernel and language metadata
- Cell metadata and tags
- Cell IDs, attachments, and extension fields
- Cell outputs and execution counts
- Notebook-level metadata

Only the fields explicitly targeted by a command are mutated.

---

## Architecture

```
src/
  main.rs          -- CLI entry point
  lib.rs           -- Shared library surface
  mcp.rs           -- MCP tools, resources, protocol and path confinement
  bin/
    nbedit-mcp.rs  -- MCP stdio entry point
  cli.rs           -- Argument definitions and subcommand enum
  notebook.rs      -- nbformat model and atomic persistence
  selection.rs     -- Parsing and evaluation of cell selection expressions
  commands/
    kernels.rs         -- Kernel/environment discovery and ranking
    run.rs             -- Driver resolution and one-shot notebook execution
    session.rs         -- `session` subcommand: registry, daemon start/stop
    session_client.rs  -- Session registry format, daemon script, TCP protocol
    ...                -- Notebook reading and mutation commands
  error.rs         -- Application errors and exit codes
```

**Key dependencies**

| Crate | Purpose |
|---|---|
| `clap` | CLI argument parsing |
| `serde` / `serde_json` | JSON serialization of `.ipynb` files |
| `anyhow` | Error handling |
| `tempfile` | Atomic saves and execution/editor files |
| `regex` | Search and replacement |
| `ctrlc` | Graceful execution interruption |

---

## Exit Codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Command, execution, or cell failure; differences/no matches where documented |
| 2 | Invalid CLI syntax or missing run-driver dependencies |
| 124 | Overall execution timeout |
| 130 | Execution interrupted with Ctrl-C |

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

## Use as an opencode skill

`nbedit` ships with an [opencode](https://opencode.ai) skill that teaches AI agents how to use the tool. When the skill is active, the agent can autonomously read, create, edit, delete and move notebook cells as part of a larger coding task — without needing to re-read the docs.

### Install the skill

The skill file is included in this repository at `.opencode/skills/nbedit/SKILL.md`. If you cloned the repo it is already available inside your project.

To make it available **globally** across all your projects, copy it to your opencode global skills directory:

```sh
mkdir -p ~/.config/opencode/skills/nbedit
cp .opencode/skills/nbedit/SKILL.md ~/.config/opencode/skills/nbedit/SKILL.md
```

Or point opencode at this repo's skill directory by adding the following to your `~/.config/opencode/opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "skills": {
    "paths": ["/path/to/notebook-editor/.opencode/skills"]
  }
}
```

### How it works

Once the skill is loaded, opencode will automatically activate it whenever you ask something like:

- *"Read cells 2 to 5 from report.ipynb"*
- *"Delete the last cell"*
- *"Add a markdown cell before cell 3"*
- *"Move cells 4-6 to the end"*
- *"Find the best kernel and run all cells"*
- *"Configure the native MCP server for this workspace"*

The skill documents the CLI, automatic kernel and driver resolution, execution controls, MCP tools/resources, safety boundaries, selection syntax, and source layout—giving the agent enough procedural context to use `nbedit` without guessing.

---

## License

MIT
