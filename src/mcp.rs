use crate::commands::{kernels, run, session};
use crate::error::AppExit;
use crate::notebook::{Cell, Notebook};
use crate::output_limit;
use crate::selection;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Component, Path, PathBuf};

const PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

pub fn serve(root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root)
        .with_context(|| format!("Cannot access MCP workspace root '{}'", root.display()))?;
    let server = McpServer { root };
    let stdin = std::io::stdin();
    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());

    for line in stdin.lock().lines() {
        let line = line?;
        if line.len() > MAX_MESSAGE_BYTES {
            bail!("MCP request exceeds the 4 MiB message limit");
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                write_message(
                    &mut stdout,
                    &rpc_error(Value::Null, -32700, &error.to_string()),
                )?;
                continue;
            }
        };
        if let Some(response) = server.handle(&request) {
            write_message(&mut stdout, &response)?;
        }
    }
    Ok(())
}

fn write_message(writer: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

struct McpServer {
    root: PathBuf,
}

impl McpServer {
    fn handle(&self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str);
        id.as_ref()?; // Notifications do not receive responses.
        let id = id.unwrap_or(Value::Null);
        let Some(method) = method else {
            return Some(rpc_error(id, -32600, "Invalid JSON-RPC request"));
        };
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => self.call_tool(&params),
            "resources/list" => self.list_resources(),
            "resources/templates/list" => Ok(json!({"resourceTemplates": [resource_template()]})),
            "resources/read" => self.read_resource(&params),
            _ => return Some(rpc_error(id, -32601, &format!("Unknown method '{method}'"))),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => rpc_error(id, -32602, &format!("{error:#}")),
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let supported = ["2024-11-05", "2025-03-26", "2025-06-18", PROTOCOL_VERSION];
        let version = requested
            .filter(|v| supported.contains(v))
            .unwrap_or(PROTOCOL_VERSION);
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {}, "resources": {}},
            "serverInfo": {"name": "nbedit-mcp", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Read, edit, and explicitly execute Jupyter notebooks inside the configured workspace root. Execution runs local notebook code and should only be requested for trusted notebooks. notebook_run_cells is stateless by default (a fresh kernel per call); pass 'session' to run against a persistent kernel started with notebook_session_start instead, which keeps variables and imports alive across calls until notebook_session_stop. Treat an active session as accumulating arbitrary code and state between calls, under the same trust bar as execution itself."
        })
    }

    fn call_tool(&self, params: &Value) -> Result<Value> {
        let name = required_str(params, "name")?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let outcome = match name {
            "notebook_info" => self.notebook_info(&arguments),
            "notebook_read" => self.notebook_read(&arguments),
            "notebook_create_cell" => self.notebook_create_cell(&arguments),
            "notebook_edit_cell" => self.notebook_edit_cell(&arguments),
            "notebook_delete_cells" => self.notebook_delete_cells(&arguments),
            "notebook_clear_outputs" => self.notebook_clear_outputs(&arguments),
            "notebook_list_kernels" => self.notebook_list_kernels(&arguments),
            "notebook_run_cells" => self.notebook_run_cells(&arguments),
            "notebook_session_start" => self.notebook_session_start(&arguments),
            "notebook_session_list" => self.notebook_session_list(&arguments),
            "notebook_session_stop" => self.notebook_session_stop(&arguments),
            _ => bail!("Unknown tool '{name}'"),
        };
        Ok(match outcome {
            Ok(value) => tool_result(value, false),
            Err(error) => tool_result(json!({"error": format!("{error:#}")}), true),
        })
    }

    fn notebook_info(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let nb = Notebook::from_file(path_str(&path)?)?;
        let code = nb.cells.iter().filter(|c| c.cell_type == "code").count();
        let markdown = nb
            .cells
            .iter()
            .filter(|c| c.cell_type == "markdown")
            .count();
        let raw = nb.cells.iter().filter(|c| c.cell_type == "raw").count();
        Ok(json!({
            "path": relative_display(&self.root, &path), "nbformat": nb.nbformat,
            "nbformat_minor": nb.nbformat_minor, "cells": nb.len(),
            "cell_types": {"code": code, "markdown": markdown, "raw": raw},
            "kernel": {"name": nb.kernel_spec_name(), "display_name": nb.kernel_name(), "language": nb.language()}
        }))
    }

    fn notebook_read(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let nb = Notebook::from_file(path_str(&path)?)?;
        let expression = args
            .get("selection")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let indices = selection::resolve(expression, nb.len())?;
        let max_lines = output_line_limit(args);
        let include_source = args
            .get("include_source")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let inclusion = output_inclusion(args);
        let cells: Vec<Value> = indices
            .into_iter()
            .map(|index| {
                let raw_outputs = &nb.cells[index].outputs;
                let mut cell = serde_json::to_value(&nb.cells[index])
                    .unwrap_or_else(|_| json!(nb.cells[index]));
                if let Some(obj) = cell.as_object_mut() {
                    if should_include_outputs(inclusion, raw_outputs) {
                        if let Some(outputs) = obj.get("outputs").and_then(Value::as_array).cloned()
                        {
                            obj.insert(
                                "outputs".into(),
                                Value::Array(output_limit::limit_outputs(&outputs, max_lines)),
                            );
                        }
                    } else {
                        obj.remove("outputs");
                    }
                    if !include_source {
                        obj.remove("source");
                    }
                }
                json!({"index": index + 1, "cell": cell})
            })
            .collect();
        Ok(json!({"path": relative_display(&self.root, &path), "cells": cells}))
    }

    fn notebook_create_cell(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let mut nb = Notebook::from_file(path_str(&path)?)?;
        let cell_type = args
            .get("cell_type")
            .and_then(Value::as_str)
            .unwrap_or("code");
        if !matches!(cell_type, "code" | "markdown" | "raw") {
            bail!("cell_type must be code, markdown, or raw");
        }
        let mut cell = Cell::new(cell_type);
        cell.set_source(
            args.get("source")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        );
        let at = optional_usize(args, "at")?.unwrap_or(nb.len() + 1);
        if at == 0 || at > nb.len() + 1 {
            bail!("at is outside the valid range 1..={}", nb.len() + 1);
        }
        nb.cells.insert(at - 1, cell);
        nb.ensure_cell_ids();
        nb.save(path_str(&path)?, backup(args))?;
        Ok(
            json!({"path": relative_display(&self.root, &path), "created_cell": at, "cells": nb.len()}),
        )
    }

    fn notebook_edit_cell(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let mut nb = Notebook::from_file(path_str(&path)?)?;
        let index = required_usize(args, "index")?;
        if index == 0 || index > nb.len() {
            bail!("Cell {index} is outside the valid range 1..={}", nb.len());
        }
        let cell = &mut nb.cells[index - 1];
        if let Some(source) = args.get("source").and_then(Value::as_str) {
            cell.set_source(source.to_owned());
        }
        if let Some(cell_type) = args.get("cell_type").and_then(Value::as_str) {
            if !matches!(cell_type, "code" | "markdown" | "raw") {
                bail!("cell_type must be code, markdown, or raw");
            }
            cell.cell_type = cell_type.to_owned();
            if cell_type != "code" {
                cell.outputs.clear();
                cell.execution_count = None;
            }
        }
        nb.ensure_cell_ids();
        nb.save(path_str(&path)?, backup(args))?;
        Ok(json!({"path": relative_display(&self.root, &path), "updated_cell": index}))
    }

    fn notebook_delete_cells(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let mut nb = Notebook::from_file(path_str(&path)?)?;
        let expression = required_str(args, "selection")?;
        let mut indices = selection::resolve(expression, nb.len())?;
        let deleted: Vec<usize> = indices.iter().map(|i| i + 1).collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for index in indices {
            nb.cells.remove(index);
        }
        nb.save(path_str(&path)?, backup(args))?;
        Ok(
            json!({"path": relative_display(&self.root, &path), "deleted_cells": deleted, "cells": nb.len()}),
        )
    }

    fn notebook_clear_outputs(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let mut nb = Notebook::from_file(path_str(&path)?)?;
        let expression = args
            .get("selection")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let indices = selection::resolve(expression, nb.len())?;
        let mut cleared = Vec::new();
        for index in indices {
            if nb.cells[index].cell_type == "code" {
                nb.cells[index].outputs.clear();
                nb.cells[index].execution_count = Some(Value::Null);
                cleared.push(index + 1);
            }
        }
        nb.save(path_str(&path)?, backup(args))?;
        Ok(json!({"path": relative_display(&self.root, &path), "cleared_cells": cleared}))
    }

    fn notebook_list_kernels(&self, args: &Value) -> Result<Value> {
        let workspace = match args.get("path").and_then(Value::as_str) {
            Some(_) => self
                .notebook_path(args)?
                .parent()
                .unwrap_or(&self.root)
                .to_path_buf(),
            None => self.root.clone(),
        };
        let candidates = kernels::discover(
            &workspace,
            args.get("driver_python").and_then(Value::as_str),
        )?;
        Ok(json!({"kernels": candidates}))
    }

    fn notebook_run_cells(&self, args: &Value) -> Result<Value> {
        let path = self.notebook_path(args)?;
        let expression = args
            .get("selection")
            .and_then(Value::as_str)
            .unwrap_or("all");
        // Captured rather than propagated with `?`: a cell raising an error is the
        // single most common outcome an agent needs outputs for (the traceback),
        // so this must not short-circuit before the cells below are built.
        let run_result = run::run(
            path_str(&path)?,
            expression,
            args.get("timeout").and_then(Value::as_i64).unwrap_or(-1),
            args.get("kernel").and_then(Value::as_str),
            args.get("interpreter").and_then(Value::as_str),
            args.get("driver_python").and_then(Value::as_str),
            args.get("allow_errors")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            args.get("include_prior")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            args.get("startup_timeout")
                .and_then(Value::as_u64)
                .unwrap_or(60),
            args.get("iopub_timeout")
                .and_then(Value::as_u64)
                .unwrap_or(4),
            true,
            args.get("overall_timeout").and_then(Value::as_u64),
            None,
            &[],
            false,
            false,
            backup(args),
            true,
            args.get("session").and_then(Value::as_str),
            args.get("create_session")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        let nb = Notebook::from_file(path_str(&path)?)?;
        let indices = selection::resolve(expression, nb.len())?;
        let max_lines = output_line_limit(args);
        let inclusion = output_inclusion(args);
        let mut failed_cell = None;
        let cells: Vec<Value> = indices
            .into_iter()
            .filter(|&i| nb.cells[i].cell_type == "code")
            .map(|index| {
                let outputs = &nb.cells[index].outputs;
                if failed_cell.is_none() && output_limit::has_error(outputs) {
                    failed_cell = Some(index + 1);
                }
                let mut entry = json!({
                    "index": index + 1,
                    "execution_count": nb.cells[index].execution_count,
                });
                if should_include_outputs(inclusion, outputs) {
                    entry["outputs"] =
                        Value::Array(output_limit::limit_outputs(outputs, max_lines));
                }
                entry
            })
            .collect();
        let (status, message) = run_status(&run_result);
        let mut result = json!({
            "status": status, "path": relative_display(&self.root, &path), "cells": cells,
            "session": args.get("session").and_then(Value::as_str),
        });
        if let Some(index) = failed_cell {
            result["failed_cell"] = json!(index);
        }
        if let Some(message) = message {
            result["message"] = json!(message);
        }
        Ok(result)
    }

    fn notebook_session_start(&self, args: &Value) -> Result<Value> {
        let notebook_path = match args.get("path").and_then(Value::as_str) {
            Some(_) => Some(self.notebook_path(args)?),
            None => None,
        };
        // Never inherit the MCP server process's ambient working directory:
        // default the kernel's cwd to the notebook's directory, or the
        // configured workspace root, so it always stays inside --root.
        let cwd = notebook_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        let record = session::start_and_get(
            args.get("name").and_then(Value::as_str),
            args.get("kernel").and_then(Value::as_str),
            args.get("interpreter").and_then(Value::as_str),
            args.get("driver_python").and_then(Value::as_str),
            notebook_path.as_deref(),
            Some(path_str(&cwd)?),
            &[],
            args.get("startup_timeout")
                .and_then(Value::as_u64)
                .unwrap_or(60),
        )?;
        Ok(json!({
            "id": record.id, "name": record.name, "kernel": record.kernel_label,
            "display_name": record.display_name, "language": record.language, "pid": record.pid,
        }))
    }

    fn notebook_session_list(&self, _args: &Value) -> Result<Value> {
        let rows = session::list_records()?;
        let sessions: Vec<Value> = rows
            .iter()
            .map(|(record, alive)| {
                json!({
                    "id": record.id, "name": record.name, "kernel": record.kernel_label,
                    "display_name": record.display_name, "language": record.language,
                    "pid": record.pid, "cwd": record.cwd, "created_at": record.created_at,
                    "alive": alive,
                })
            })
            .collect();
        Ok(json!({"sessions": sessions}))
    }

    fn notebook_session_stop(&self, args: &Value) -> Result<Value> {
        let name = required_str(args, "name")?;
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        session::stop(name, force)?;
        Ok(json!({"status": "ok", "name": name}))
    }

    fn notebook_path(&self, args: &Value) -> Result<PathBuf> {
        let raw = required_str(args, "path")?;
        resolve_existing_notebook(&self.root, raw)
    }

    fn list_resources(&self) -> Result<Value> {
        let mut paths = Vec::new();
        collect_notebooks(&self.root, &self.root, 0, &mut paths)?;
        let resources: Vec<Value> = paths.into_iter().map(|path| {
            let relative = relative_display(&self.root, &path);
            json!({"uri": notebook_uri(&relative), "name": relative, "mimeType": "application/x-ipynb+json"})
        }).collect();
        Ok(json!({"resources": resources}))
    }

    fn read_resource(&self, params: &Value) -> Result<Value> {
        let uri = required_str(params, "uri")?;
        let relative = parse_notebook_uri(uri)?;
        let path = resolve_existing_notebook(&self.root, &relative)?;
        let text = std::fs::read_to_string(&path)?;
        Ok(
            json!({"contents": [{"uri": uri, "mimeType": "application/x-ipynb+json", "text": text}]}),
        )
    }
}

fn tool_result(value: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({"content": [{"type": "text", "text": text}], "structuredContent": value, "isError": is_error})
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "notebook_info",
            "Summarize a notebook and its configured kernel",
            schema(
                &["path"],
                json!({"path": string_prop("Workspace-relative .ipynb path")}),
            ),
        ),
        tool(
            "notebook_read",
            "Read selected notebook cells as structured JSON",
            schema(
                &["path"],
                json!({
                    "path": string_prop("Workspace-relative .ipynb path"), "selection": string_prop("Cell selection; default all"),
                    "include_source": bool_prop("Include each cell's source; default true"),
                    "include_outputs": output_inclusion_prop("Include each code cell's outputs: true (default), false, or \"on_error\" to include only for cells whose outputs contain an error"),
                    "output_lines": integer_prop("Max lines kept per output field before truncating; default 100"),
                    "full_output": bool_prop("Return outputs in full, without truncation or binary omission")
                }),
            ),
        ),
        tool(
            "notebook_create_cell",
            "Create a notebook cell",
            mutation_schema(
                &["path"],
                json!({"path": string_prop("Workspace-relative .ipynb path"), "cell_type": enum_prop(&["code", "markdown", "raw"]), "source": string_prop("Cell source"), "at": integer_prop("1-based insertion position")}),
            ),
        ),
        tool(
            "notebook_edit_cell",
            "Replace a cell's source or type",
            mutation_schema(
                &["path", "index"],
                json!({"path": string_prop("Workspace-relative .ipynb path"), "index": integer_prop("1-based cell index"), "source": string_prop("Replacement source"), "cell_type": enum_prop(&["code", "markdown", "raw"])}),
            ),
        ),
        tool(
            "notebook_delete_cells",
            "Delete selected cells",
            mutation_schema(
                &["path", "selection"],
                json!({"path": string_prop("Workspace-relative .ipynb path"), "selection": string_prop("Cell selection")}),
            ),
        ),
        tool(
            "notebook_clear_outputs",
            "Clear outputs and execution counts",
            mutation_schema(
                &["path"],
                json!({"path": string_prop("Workspace-relative .ipynb path"), "selection": string_prop("Cell selection; default all")}),
            ),
        ),
        tool(
            "notebook_list_kernels",
            "Discover local kernels and Python environments",
            schema(
                &[],
                json!({"path": string_prop("Optional notebook used as workspace context"), "driver_python": string_prop("Optional driver Python path")}),
            ),
        ),
        tool(
            "notebook_run_cells",
            "Execute trusted notebook code through a Jupyter kernel and save outputs. Always returns 'status' ('ok', 'error', 'missing_dependency', 'overall_timeout', or 'interrupted') plus, on failure, 'message' and 'failed_cell' — outputs (including tracebacks) are returned regardless of status, so check 'failed_cell'/'status' rather than assuming a cell succeeded just because the call didn't throw",
            mutation_schema(
                &["path"],
                json!({
                    "path": string_prop("Workspace-relative .ipynb path"), "selection": string_prop("Cell selection; default all"),
                    "kernel": string_prop("Kernelspec name or discovered kernel ID"), "interpreter": string_prop("Python kernel interpreter"),
                    "driver_python": string_prop("Explicit nbclient driver Python"), "timeout": integer_prop("Per-cell seconds; -1 disables"),
                    "overall_timeout": integer_prop("Overall execution seconds"), "allow_errors": bool_prop("Continue after cell errors"),
                    "include_prior": bool_prop("Execute prior code cells as context"),
                    "session": string_prop("Run against a persistent kernel session (see notebook_session_start) instead of a one-shot kernel"),
                    "create_session": bool_prop("Create the session if it doesn't exist yet; only meaningful together with session"),
                    "include_outputs": output_inclusion_prop("Include each executed cell's outputs in the result: true (default), false, or \"on_error\" to include only for cells whose outputs contain an error"),
                    "output_lines": integer_prop("Max lines kept per output field before truncating; default 100"),
                    "full_output": bool_prop("Return outputs in full, without truncation or binary omission")
                }),
            ),
        ),
        tool(
            "notebook_session_start",
            "Start a persistent kernel session; state (variables, imports) persists across later notebook_run_cells calls that pass the same session name, until notebook_session_stop",
            schema(
                &[],
                json!({
                    "name": string_prop("Session name; a random id is used if omitted"),
                    "path": string_prop("Optional workspace-relative .ipynb path used only to rank kernel candidates"),
                    "kernel": string_prop("Kernelspec name or discovered kernel ID"), "interpreter": string_prop("Python kernel interpreter"),
                    "driver_python": string_prop("Explicit nbclient driver Python"),
                    "startup_timeout": integer_prop("Kernel startup timeout in seconds; default 60")
                }),
            ),
        ),
        tool(
            "notebook_session_list",
            "List known kernel sessions and whether their kernel is still alive",
            schema(&[], json!({})),
        ),
        tool(
            "notebook_session_stop",
            "Stop a kernel session and shut down its kernel",
            schema(
                &["name"],
                json!({"name": string_prop("Session name or id"), "force": bool_prop("Skip the graceful shutdown and kill the kernel process directly")}),
            ),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}
fn schema(required: &[&str], properties: Value) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}
fn mutation_schema(required: &[&str], mut properties: Value) -> Value {
    properties.as_object_mut().unwrap().insert(
        "backup".into(),
        bool_prop("Write a .bak file; default true"),
    );
    schema(required, properties)
}
fn string_prop(description: &str) -> Value {
    json!({"type": "string", "description": description})
}
fn integer_prop(description: &str) -> Value {
    json!({"type": "integer", "description": description})
}
fn bool_prop(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}
fn output_inclusion_prop(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [{"type": "boolean"}, {"type": "string", "enum": ["on_error"]}]
    })
}
fn enum_prop(values: &[&str]) -> Value {
    json!({"type": "string", "enum": values})
}
fn resource_template() -> Value {
    json!({"uriTemplate": "notebook:///{path}", "name": "Jupyter notebook", "description": "Raw notebook JSON inside the configured workspace", "mimeType": "application/x-ipynb+json"})
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("'{key}' must be a string"))
}
fn required_usize(value: &Value, key: &str) -> Result<usize> {
    optional_usize(value, key)?.with_context(|| format!("'{key}' is required"))
}
fn optional_usize(value: &Value, key: &str) -> Result<Option<usize>> {
    value
        .get(key)
        .map(|v| {
            v.as_u64()
                .map(|n| n as usize)
                .with_context(|| format!("'{key}' must be a non-negative integer"))
        })
        .transpose()
}
fn backup(args: &Value) -> bool {
    args.get("backup").and_then(Value::as_bool).unwrap_or(true)
}
fn output_line_limit(args: &Value) -> Option<usize> {
    if args
        .get("full_output")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    Some(
        args.get("output_lines")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(output_limit::DEFAULT_MAX_LINES),
    )
}

#[derive(Clone, Copy)]
enum OutputInclusion {
    All,
    None,
    OnError,
}

/// `include_outputs` accepts `true`/`false` (default `true`) or the string
/// `"on_error"` to only include outputs for cells that raised an error.
fn output_inclusion(args: &Value) -> OutputInclusion {
    match args.get("include_outputs") {
        Some(Value::Bool(false)) => OutputInclusion::None,
        Some(Value::String(s)) if s == "on_error" => OutputInclusion::OnError,
        _ => OutputInclusion::All,
    }
}

fn should_include_outputs(inclusion: OutputInclusion, outputs: &[Value]) -> bool {
    match inclusion {
        OutputInclusion::All => true,
        OutputInclusion::None => false,
        OutputInclusion::OnError => output_limit::has_error(outputs),
    }
}

/// Maps a `run::run` result to a status label and, on failure, a message —
/// without discarding it, so the caller can still report cell outputs
/// (notably tracebacks) alongside a non-"ok" status instead of losing them.
fn run_status(result: &Result<()>) -> (&'static str, Option<String>) {
    match result {
        Ok(()) => ("ok", None),
        Err(error) => {
            let label = match error.downcast_ref::<AppExit>().map(|e| e.code) {
                Some(2) => "missing_dependency",
                Some(124) => "overall_timeout",
                Some(130) => "interrupted",
                _ => "error",
            };
            (label, Some(format!("{error:#}")))
        }
    }
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .context("Notebook path contains non-UTF-8 characters")
}
fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn resolve_existing_notebook(root: &Path, raw: &str) -> Result<PathBuf> {
    let supplied = Path::new(raw);
    if supplied
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        bail!("Parent-directory traversal is not allowed");
    }
    let joined = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        root.join(supplied)
    };
    let path = std::fs::canonicalize(&joined)
        .with_context(|| format!("Cannot access notebook '{}'", joined.display()))?;
    if !path.starts_with(root) {
        bail!("Notebook is outside the configured workspace root");
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("ipynb"))
        != Some(true)
    {
        bail!("Only .ipynb files are allowed");
    }
    Ok(path)
}

fn collect_notebooks(
    root: &Path,
    directory: &Path,
    depth: usize,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > 4 || out.len() >= 500 {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)?.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_notebooks(root, &path, depth + 1, out)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ipynb"))
        {
            if let Ok(path) = std::fs::canonicalize(path) {
                if path.starts_with(root) {
                    out.push(path);
                }
            }
        }
        if out.len() >= 500 {
            break;
        }
    }
    out.sort();
    Ok(())
}

fn notebook_uri(relative: &str) -> String {
    format!("notebook:///{}", percent_encode(relative))
}
fn parse_notebook_uri(uri: &str) -> Result<String> {
    let encoded = uri
        .strip_prefix("notebook:///")
        .context("Resource URI must start with notebook:///")?;
    percent_decode(encoded)
}
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                bail!("Invalid percent-encoded resource URI");
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])?;
            result
                .push(u8::from_str_radix(hex, 16).context("Invalid percent-encoded resource URI")?);
            index += 3;
        } else {
            result.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(result).context("Resource URI is not valid UTF-8")
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> (tempfile::TempDir, McpServer) {
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer {
            root: std::fs::canonicalize(dir.path()).unwrap(),
        };
        (dir, server)
    }

    fn write_notebook(dir: &Path) {
        std::fs::write(dir.join("test.ipynb"), serde_json::to_vec(&json!({
            "nbformat": 4, "nbformat_minor": 5, "metadata": {},
            "cells": [{"id": "one", "cell_type": "code", "metadata": {}, "source": ["x = 1"], "execution_count": null, "outputs": []}]
        })).unwrap()).unwrap();
    }

    #[test]
    fn initialize_advertises_tools_and_resources() {
        let (_dir, server) = server();
        let response = server.handle(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-11-25"}})).unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tool_calls_read_and_edit_notebook() {
        let (dir, server) = server();
        write_notebook(dir.path());
        let edit = server.call_tool(&json!({"name":"notebook_edit_cell", "arguments":{"path":"test.ipynb", "index":1, "source":"x = 2", "backup":false}})).unwrap();
        assert_eq!(edit["isError"], false);
        let read = server.notebook_read(&json!({"path":"test.ipynb"})).unwrap();
        assert_eq!(read["cells"][0]["cell"]["source"][0], "x = 2");
    }

    #[test]
    fn paths_cannot_escape_workspace() {
        let (_dir, server) = server();
        assert!(resolve_existing_notebook(&server.root, "../outside.ipynb").is_err());
    }

    #[test]
    fn resource_uri_roundtrips_spaces_and_unicode() {
        let path = "reports/my café.ipynb";
        assert_eq!(parse_notebook_uri(&notebook_uri(path)).unwrap(), path);
    }

    fn write_notebook_with_outputs(dir: &Path) {
        std::fs::write(dir.join("out.ipynb"), serde_json::to_vec(&json!({
            "nbformat": 4, "nbformat_minor": 5, "metadata": {},
            "cells": [
                {"id": "ok", "cell_type": "code", "metadata": {}, "source": ["1+1"], "execution_count": 1,
                 "outputs": [{"output_type": "execute_result", "execution_count": 1, "data": {"text/plain": "2"}}]},
                {"id": "bad", "cell_type": "code", "metadata": {}, "source": ["1/0"], "execution_count": 2,
                 "outputs": [{"output_type": "error", "ename": "ZeroDivisionError", "evalue": "division by zero", "traceback": ["Traceback...\n"]}]}
            ]
        })).unwrap()).unwrap();
    }

    #[test]
    fn notebook_read_include_source_false_omits_source() {
        let (dir, server) = server();
        write_notebook(dir.path());
        let read = server
            .notebook_read(&json!({"path": "test.ipynb", "include_source": false}))
            .unwrap();
        assert!(read["cells"][0]["cell"].get("source").is_none());
    }

    #[test]
    fn notebook_read_on_error_only_includes_failing_cell_outputs() {
        let (dir, server) = server();
        write_notebook_with_outputs(dir.path());
        let read = server
            .notebook_read(&json!({"path": "out.ipynb", "include_outputs": "on_error"}))
            .unwrap();
        let cells = read["cells"].as_array().unwrap();
        assert!(cells[0]["cell"].get("outputs").is_none());
        assert_eq!(cells[1]["cell"]["outputs"][0]["output_type"], "error");
    }

    #[test]
    fn run_status_maps_app_exit_codes_without_losing_the_message() {
        assert_eq!(run_status(&Ok(())).0, "ok");

        let missing: Result<()> = Err(AppExit::new(2, "no nbclient").into());
        assert_eq!(run_status(&missing).0, "missing_dependency");

        let timeout: Result<()> = Err(AppExit::new(124, "timed out").into());
        assert_eq!(run_status(&timeout).0, "overall_timeout");

        let interrupted: Result<()> = Err(AppExit::new(130, "ctrl-c").into());
        assert_eq!(run_status(&interrupted).0, "interrupted");

        let cell_error: Result<()> = Err(AppExit::new(1, "cell failed").into());
        let (label, message) = run_status(&cell_error);
        assert_eq!(label, "error");
        assert_eq!(message.unwrap(), "cell failed");
    }

    #[test]
    fn should_include_outputs_respects_inclusion_mode() {
        let clean: Vec<Value> = vec![json!({"output_type": "stream", "text": "ok\n"})];
        let failing: Vec<Value> =
            vec![json!({"output_type": "error", "ename": "E", "evalue": "x", "traceback": []})];
        assert!(should_include_outputs(OutputInclusion::All, &clean));
        assert!(!should_include_outputs(OutputInclusion::None, &failing));
        assert!(!should_include_outputs(OutputInclusion::OnError, &clean));
        assert!(should_include_outputs(OutputInclusion::OnError, &failing));
    }
}
