# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build --release        # build the nbedit binary (output: target/release/nbedit)
cargo build                  # debug build (output: target/debug/nbedit)
cargo test                   # run all tests
cargo test selection         # run tests in a specific module (e.g. selection::tests)
cargo test test_range        # run a single test by name
cargo clippy                 # lint
cargo fmt                    # format
```

## Architecture

`notebook-editor` is a Rust CLI (`nbedit`) that edits Jupyter `.ipynb` files in place. The entry point is `src/main.rs` — it parses the CLI with `clap` and delegates to `commands::dispatch`.

**Data flow for every mutating command:**
1. `Notebook::from_file` deserializes the `.ipynb` JSON via `serde_json`
2. The command function mutates `nb.cells` in memory
3. `Notebook::save` serializes back with `serde_json::to_string_pretty`

**Key modules:**

| File | Role |
|---|---|
| `src/cli.rs` | All clap argument structs and the `Command` enum — one variant per subcommand |
| `src/notebook.rs` | `Notebook` and `Cell` types; `CellSource` handles the nbformat duality (source can be a `String` or `Vec<String>`) |
| `src/selection.rs` | Parses cell-selection expressions (`all`, `last`, `3`, `1,3-5`) into sorted 0-based index vecs; also used for line-level selection within cells |
| `src/commands/` | One file per subcommand; each exports a `run(…)` function called from `commands::dispatch` in `mod.rs` |
| `src/error.rs` | Unified error type (thin wrapper; most code uses `anyhow` directly) |

**Index convention:** User-facing indices are 1-based; `selection::resolve` converts them to 0-based before returning. All internal code works with 0-based indices.

**`CellSource` invariant:** `Notebook::from_file` may yield either variant. After any mutation, `Cell::set_source` always stores `CellSource::Lines`. Do not assume a particular variant when reading; use `Cell::source_str()`.

**Adding a new subcommand:** add a variant to `cli::Command` in `cli.rs`, add a `src/commands/<name>.rs` with a `pub fn run(…)` function, register it in `src/commands/mod.rs` dispatch match, and update `README.md`.

## Execution architecture

Both binaries share `commands::run::ExecutionRequest` and `ExecutionReport`.
Reports distinguish selected/context cells, actual execution, skipped cells,
current-run outputs, precise failures, and save conflicts. Save results through
the revision-checked path and `Notebook::save`; never infer current failures
from old notebook outputs.

`session_client.rs` embeds `session_daemon.py` (loopback protocol v2). One worker
serializes execution while independent connections handle submit/result/cancel,
status/interrupt/restart/configure/shutdown. Preserve bounded event/history
retention, disk-backed completed outputs, cancellation grace periods, and explicit
unhealthy-state recovery. `execution_jobs.rs` owns MCP background handles and
cursors; handles last for that server instance.

Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, and, after building,
`python -m unittest discover -s tests -p test_execution.py -v` using Python with
nbclient, nbformat, and ipykernel.
