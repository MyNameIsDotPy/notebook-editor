# AGENTS.md

## Project Shape

- This is one Rust 2021 crate (`nbedit`), with two binaries: `src/main.rs` for the CLI and `src/bin/nbedit-mcp.rs` for the stdio MCP server. Keep shared behavior in the library modules exposed by `src/lib.rs` rather than duplicating it between binaries.
- `src/cli.rs` defines Clap arguments; `src/commands/mod.rs` is the CLI dispatch boundary. `src/mcp.rs` implements JSON-RPC tools directly and must retain its canonicalized `--root` workspace confinement.
- Public notebook cell numbers are 1-based. `selection::resolve` returns 0-based indices, so convert only at the CLI/API boundary.

## Notebook Semantics

- Preserve nbformat data not explicitly changed: `Cell.extra` flattens unknown cell fields, and source is normalized to an array of newline-inclusive lines by `Cell::set_source`.
- Mutating paths save through `Notebook::save`, which writes atomically and optionally creates a `.bak`; do not replace it with direct file writes.
- `run` has separate Python requirements: the driver needs `nbclient` and `nbformat`; a Python kernel interpreter needs `ipykernel`. Automatic resolution is intentional, so keep explicit driver/interpreter flags as overrides.
- One-shot `run`'s generated driver script (`build_script` in `src/commands/run.rs`) prints per-cell `[nbedit] cell N (i/total): starting`/`...: done` progress to stderr, gated by `!quiet && !json` (never pollutes `--json` stdout), via `nbclient`'s `on_cell_start`/`on_cell_complete` hooks assigned post-construction inside a bare `try/except` — verified against real `nbclient` 0.11.0 source before writing (`run_hook()` just calls `hook(**kwargs)`, awaiting only if the result is awaitable, so a plain sync function works), and the `except` means an older `nbclient` lacking those `Callable` traitlets silently loses the progress lines instead of breaking execution. `run --session`'s daemon RPC (`session_client.rs`) has no equivalent — it's one blocking request/response, not a per-cell stream; that would need a real protocol change, not a copy of this hook.
- One-shot `run` execution is stateless. Persistent state is only via `session`; it is incompatible with `--include-prior`.

## Verification

- CI uses stable Rust and runs `cargo build --verbose` then `cargo test --verbose` on Ubuntu and Windows.
- Run focused tests by their module path, for example `cargo test selection::tests::test_range`.
- The repo-local nbedit skill also requires `cargo clippy --all-targets -- -D warnings` for Rust changes.

## Releases

- Tag pushes matching `v*.*.*` build Linux x86_64, Linux aarch64 (with `cross`), and Windows x86_64 binaries. Keep both `nbedit` and `nbedit-mcp` buildable for supported targets.
