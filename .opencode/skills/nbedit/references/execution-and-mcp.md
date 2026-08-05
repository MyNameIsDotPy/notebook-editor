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

For Python, prefer the selected kernelspec interpreter as driver when it imports
`nbclient` and `nbformat`, then probe PATH. Use `--driver-python` only to override.
Non-Python kernels require a separate Python driver. Capture kernel streams into cell
outputs. Save available partial outputs after clean failures or interruption.

Exit codes: `0` success, `1` execution failure, `2` missing driver dependencies, `124`
overall timeout, `130` Ctrl-C.

## Kernel discovery

`nbedit kernels` discovers standard kernelspecs; workspace `.venv`, `venv`, Pixi, and
local Conda environments; active `VIRTUAL_ENV`/`CONDA_PREFIX`; Conda/Micromamba, Pyenv,
Poetry, Pipenv; and PATH Python.

- Use `--details` for paths and sources.
- Use `--check` to probe `ipykernel`.
- Use `--notebook <PATH>` for notebook-aware ranking.
- Use `--json` for normalized candidates.

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

Read raw notebooks through `notebook:///{path}`. Restrict access to `--root`; traversal
and symlink escapes are rejected. Mutations back up by default. Never install packages
automatically. Treat `notebook_run_cells` as explicit local code execution and use it
only for trusted notebooks. Kernel sessions are stateless between calls.
