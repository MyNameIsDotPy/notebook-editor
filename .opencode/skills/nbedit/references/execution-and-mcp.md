# Execution, kernel discovery, and MCP

## CLI execution

```sh
nbedit run <NOTEBOOK> <SELECTION> [OPTIONS]
```

Execute selected code cells and merge outputs, counts, IDs, and timing metadata. The
driver requires `nbclient` and `nbformat`; a directly selected Python kernel requires
`ipykernel`.

| Option | Default | Purpose |
|---|---|---|
| `--timeout <N>` | `-1` | Per-cell timeout; `-1` disables |
| `--kernel <ID>` | automatic | Kernelspec or discovered candidate |
| `--interpreter <PATH>` | automatic | Unregistered Python kernel environment |
| `--driver-python <PATH>` | automatic | Explicit nbclient driver; `--python` is an alias |
| `--include-prior` | off | Run prior code as context; update selected cells only |
| `--allow-errors` | off | Continue after errors |
| `--startup-timeout <N>` | `60` | Kernel startup timeout |
| `--iopub-timeout <N>` | `4` | Output-channel timeout |
| `--overall-timeout <N>` | none | Whole-operation limit |
| `--no-record-timing` | off | Disable timing metadata |
| `--cwd <PATH>` | notebook dir | Kernel working directory |
| `--env KEY=VALUE` | none | Kernel environment; repeatable |
| `--dry-run` | off | Resolve without executing |
| `--json` | off | Structured report |
| `--session <NAME>` | none | Run against a persistent kernel session instead of a one-shot kernel |
| `--create-session` | off | With `--session`, create it if it doesn't already exist |

For Python, prefer the selected kernelspec interpreter as driver when it imports
`nbclient` and `nbformat`, then probe PATH. Use `--driver-python` only to override.
Non-Python kernels require a separate Python driver. Capture kernel streams into cell
outputs. Save available partial outputs after clean failures or interruption.

Exit codes: `0` success, `1` execution failure, `2` missing driver dependencies, `124`
overall timeout, `130` Ctrl-C.

`--json` reports `status`, `kernel`, `source`, `executed_cells`, `failed_cell`,
`duration_ms`, `outputs_saved`, and — only when `--session` was used —
`session` with the session's name (present whether it was reused or created
via `--create-session`).

By default each `run` is stateless: a fresh kernel starts, executes, and shuts
down within the call. Pass `--session <name>` to run against a persistent
kernel instead, so variables and imports carry over to the next `run` call.
`--include-prior` and `--session` are mutually exclusive (prior state already
lives in the kernel).

## Kernel discovery

`nbedit kernels` discovers standard kernelspecs; workspace `.venv`, `venv`, Pixi, and
local Conda environments; active `VIRTUAL_ENV`/`CONDA_PREFIX`; Conda/Micromamba, Pyenv,
Poetry, Pipenv; and PATH Python.

- Use `--details` for paths and sources.
- Use `--check` to probe `ipykernel`.
- Use `--notebook <PATH>` for notebook-aware ranking.
- Use `--json` for normalized candidates.

## Sessions

```sh
nbedit session start [--name NAME] [--kernel ID] [--interpreter PATH] [--cwd PATH] [--env KEY=VALUE] [--startup-timeout N]
nbedit session list [--json]
nbedit session stop <NAME> [--force]
```

A session is a small daemon process that starts one kernel and keeps it (and
its `nbclient` execution state) alive until explicitly stopped, listening on a
loopback TCP port with a per-session auth token. Registry files live under an
OS-specific data directory (e.g. `~/.local/share/nbedit/sessions` on Linux,
`~/Library/Application Support/nbedit/sessions` on macOS,
`%LOCALAPPDATA%\nbedit\sessions` on Windows).

Reference a session from `run` with `--session <name>`:

- If the session exists and is alive, cells execute against it — outputs merge
  back into the notebook exactly like a normal run, but variables/imports from
  earlier calls are visible.
- If it doesn't exist (or its process died), `run` fails unless
  `--create-session` is also passed, in which case it's created on the fly
  using the same `--kernel`/`--interpreter`/`--driver-python` resolution as a
  normal run.

Sessions never expire on their own; `session list` detects and prunes dead
entries from the registry, but a live kernel is only stopped by
`session stop` (graceful shutdown over TCP, falling back to killing the PID;
`--force` skips straight to killing the PID). Ctrl-C during a
`run --session` call interrupts the CLI but does not interrupt the kernel —
the cell keeps running in the daemon.

## MCP server

Configure the client to launch the stdio server; do not start it separately:

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

Tools:

- `notebook_info`, `notebook_read`
- `notebook_create_cell`, `notebook_edit_cell`, `notebook_delete_cells`
- `notebook_clear_outputs`
- `notebook_list_kernels`
- `notebook_run_cells`
- `notebook_session_start`, `notebook_session_list`, `notebook_session_stop`

Read raw notebooks through `notebook:///{path}`. Restrict access to `--root`; traversal
and symlink escapes are rejected. Mutations back up by default. Never install packages
automatically. Treat `notebook_run_cells` as explicit local code execution and use it
only for trusted notebooks.

`notebook_run_cells` is stateless by default: a fresh kernel per call, gone
when the call returns. Pass `session` (a name from `notebook_session_start`)
to run against a persistent kernel instead — state then accumulates across
calls until `notebook_session_stop`, so hold an active session to the same
trust bar as execution itself. Kernel working directory for
`notebook_session_start` always defaults to the notebook's directory (or the
workspace root), never the MCP server process's own working directory.
