use crate::commands::{kernels, run};
use crate::notebook::{Cell, Notebook};
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
            "instructions": "Read, edit, and explicitly execute Jupyter notebooks inside the configured workspace root. Execution runs local notebook code and should only be requested for trusted notebooks."
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
        let cells: Vec<Value> = indices
            .into_iter()
            .map(|index| json!({"index": index + 1, "cell": nb.cells[index]}))
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
        run::run(
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
        )?;
        let nb = Notebook::from_file(path_str(&path)?)?;
        let indices = selection::resolve(expression, nb.len())?;
        let cells: Vec<Value> = indices.into_iter().filter(|&i| nb.cells[i].cell_type == "code")
            .map(|index| json!({"index": index + 1, "execution_count": nb.cells[index].execution_count, "outputs": nb.cells[index].outputs})).collect();
        Ok(json!({"status": "ok", "path": relative_display(&self.root, &path), "cells": cells}))
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
                json!({"path": string_prop("Workspace-relative .ipynb path"), "selection": string_prop("Cell selection; default all")}),
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
            "Execute trusted notebook code through a Jupyter kernel and save outputs",
            mutation_schema(
                &["path"],
                json!({
                    "path": string_prop("Workspace-relative .ipynb path"), "selection": string_prop("Cell selection; default all"),
                    "kernel": string_prop("Kernelspec name or discovered kernel ID"), "interpreter": string_prop("Python kernel interpreter"),
                    "driver_python": string_prop("Explicit nbclient driver Python"), "timeout": integer_prop("Per-cell seconds; -1 disables"),
                    "overall_timeout": integer_prop("Overall execution seconds"), "allow_errors": bool_prop("Continue after cell errors"),
                    "include_prior": bool_prop("Execute prior code cells as context")
                }),
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
}
