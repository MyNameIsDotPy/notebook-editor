//! Registry and wire protocol for persistent kernel sessions.
//!
//! A session is a small asyncio daemon (see `build_daemon_script`) that keeps a
//! single `nbclient.NotebookClient` and its kernel alive for the life of the
//! process, listening on a loopback TCP port. `nbedit run --session <name>`
//! sends batches of cell sources to it instead of spawning a fresh kernel, so
//! state (variables, imports) persists across separate `nbedit` invocations.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct OverallTimeout;

impl std::fmt::Display for OverallTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Overall execution timeout expired")
    }
}

impl std::error::Error for OverallTimeout {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    pub kernel_label: String,
    pub display_name: String,
    pub language: Option<String>,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub cwd: String,
    pub created_at: u64,
}

/// Directory holding one JSON file per session, e.g.
/// `~/Library/Application Support/nbedit/sessions` on macOS,
/// `~/.local/share/nbedit/sessions` on Linux, `%LOCALAPPDATA%\nbedit\sessions`
/// on Windows. Mirrors the manual XDG-style resolution already used for
/// Jupyter data directories in `kernels.rs`.
pub fn registry_dir() -> Result<PathBuf> {
    let dir = data_dir()?.join("nbedit").join("sessions");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create session directory '{}'", dir.display()))?;
    Ok(dir)
}

fn data_dir() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support"))
            .context("Cannot determine session data directory; HOME is not set")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .context("Cannot determine session data directory; LOCALAPPDATA is not set")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .context("Cannot determine session data directory; set XDG_DATA_HOME or HOME")
    }
}

/// `std` has no direct CSPRNG, but `RandomState`'s per-instance keys are
/// OS-seeded, so hashing nothing with a fresh instance yields an
/// unpredictable value. Good enough for session ids and auth tokens, which
/// only need to resist casual local guessing over a loopback port.
fn random_hex(words: usize) -> String {
    (0..words.max(1))
        .map(|_| format!("{:016x}", RandomState::new().build_hasher().finish()))
        .collect()
}

pub fn new_id() -> String {
    random_hex(1)
}

pub fn new_token() -> String {
    random_hex(4)
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn record_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

pub fn all(dir: &Path) -> Result<Vec<SessionRecord>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Cannot read session directory '{}'", dir.display()))
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(record) = serde_json::from_str::<SessionRecord>(&content) {
                out.push(record);
            }
        }
    }
    out.sort_by_key(|record| record.created_at);
    Ok(out)
}

pub fn find(dir: &Path, name_or_id: &str) -> Result<Option<SessionRecord>> {
    Ok(all(dir)?
        .into_iter()
        .find(|record| record.id == name_or_id || record.name == name_or_id))
}

pub fn save(dir: &Path, record: &SessionRecord) -> Result<()> {
    let path = record_path(dir, &record.id);
    let json = serde_json::to_vec_pretty(record)?;
    std::fs::write(&path, json)
        .with_context(|| format!("Cannot write session record '{}'", path.display()))
}

pub fn remove(dir: &Path, id: &str) -> Result<()> {
    let path = record_path(dir, id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("Cannot remove session record '{}'", path.display()))
        }
    }
}

pub fn is_alive(record: &SessionRecord) -> bool {
    ping(record, Duration::from_millis(800)).is_ok()
}

pub fn ping(record: &SessionRecord, timeout: Duration) -> Result<()> {
    let response = request(record, serde_json::json!({"cmd": "ping"}), timeout)?;
    if response.get("status").and_then(Value::as_str) == Some("ok") {
        Ok(())
    } else {
        bail!("Session '{}' did not respond to ping", record.name)
    }
}

pub fn shutdown(record: &SessionRecord, timeout: Duration) -> Result<()> {
    let response = request(record, serde_json::json!({"cmd": "shutdown"}), timeout)?;
    if response.get("status").and_then(Value::as_str) == Some("ok") {
        Ok(())
    } else {
        let message = response
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        bail!("Session '{}' refused shutdown: {message}", record.name)
    }
}

#[derive(Serialize)]
pub struct CellRequest {
    pub id: String,
    pub source: String,
    pub metadata: Value,
}

#[derive(Debug, Deserialize)]
pub struct CellResult {
    pub id: String,
    pub execution_count: Value,
    #[serde(default)]
    pub outputs: Vec<Value>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteResponse {
    pub status: String,
    #[serde(default)]
    pub results: Vec<CellResult>,
    #[serde(default)]
    pub failed_id: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

pub fn execute(
    record: &SessionRecord,
    cells: &[CellRequest],
    allow_errors: bool,
    timeout: i64,
    record_timing: bool,
    overall_timeout: Option<u64>,
    iopub_timeout: u64,
) -> Result<ExecuteResponse> {
    let payload = serde_json::json!({
        "cmd": "execute",
        "cells": cells,
        "allow_errors": allow_errors,
        "timeout": timeout,
        "record_timing": record_timing,
        "iopub_timeout": iopub_timeout,
    });
    let response = match overall_timeout {
        Some(seconds) => request_with_deadline(
            record,
            payload,
            Instant::now() + Duration::from_secs(seconds),
        )?,
        None => request_with_read_timeout(record, payload, None)?,
    };
    serde_json::from_value(response).context("Malformed response from session daemon")
}

fn request_with_deadline(
    record: &SessionRecord,
    payload: Value,
    deadline: Instant,
) -> Result<Value> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(OverallTimeout)?;
    let addr: SocketAddr = format!("127.0.0.1:{}", record.port)
        .parse()
        .context("Invalid session address")?;
    let mut stream = TcpStream::connect_timeout(&addr, remaining).with_context(|| {
        format!(
            "Cannot connect to session '{}' on port {} (is it still running? try 'nbedit session list')",
            record.name, record.port
        )
    })?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(OverallTimeout)?;
    stream.set_read_timeout(Some(remaining))?;
    stream.set_write_timeout(Some(remaining))?;
    send_and_read(record, &mut stream, payload).map_err(|error| {
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
        {
            OverallTimeout.into()
        } else {
            error
        }
    })
}

fn request(record: &SessionRecord, payload: Value, timeout: Duration) -> Result<Value> {
    request_with_read_timeout(record, payload, Some(timeout))
}

fn request_with_read_timeout(
    record: &SessionRecord,
    payload: Value,
    read_timeout: Option<Duration>,
) -> Result<Value> {
    let addr: SocketAddr = format!("127.0.0.1:{}", record.port)
        .parse()
        .context("Invalid session address")?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).with_context(
        || {
            format!(
                "Cannot connect to session '{}' on port {} (is it still running? try 'nbedit session list')",
                record.name, record.port
            )
        },
    )?;
    stream.set_read_timeout(read_timeout)?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    send_and_read(record, &mut stream, payload)
}

fn send_and_read(
    record: &SessionRecord,
    stream: &mut TcpStream,
    mut payload: Value,
) -> Result<Value> {
    payload
        .as_object_mut()
        .expect("request payload is always a JSON object")
        .insert("token".into(), Value::String(record.token.clone()));
    let mut line = serde_json::to_vec(&payload)?;
    line.push(b'\n');
    stream
        .write_all(&line)
        .with_context(|| format!("Cannot send request to session '{}'", record.name))?;
    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .with_context(|| format!("Cannot read response from session '{}'", record.name))?;
    if response_line.trim().is_empty() {
        bail!(
            "Session '{}' closed the connection without responding",
            record.name
        );
    }
    serde_json::from_str(&response_line).context("Invalid JSON response from session daemon")
}

/// Generate the `-c` script for the session daemon: starts one kernel, keeps
/// it alive under a single `nbclient.NotebookClient`, and serves `execute`
/// requests over a loopback TCP socket until told to shut down.
pub fn build_daemon_script(kernel_name: &str, token: &str, startup_timeout: u64) -> String {
    let kernel_repr = serde_json::to_string(kernel_name).expect("string serialization cannot fail");
    let token_repr = serde_json::to_string(token).expect("string serialization cannot fail");
    let lines = vec![
        "import asyncio, json, os, sys".to_string(),
        "if sys.platform == 'win32':".to_string(),
        "    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())".to_string(),
        format!("KERNEL_NAME = {kernel_repr}"),
        format!("TOKEN = {token_repr}"),
        format!("STARTUP_TIMEOUT = {startup_timeout}"),
        String::new(),
        "def announce(payload):".to_string(),
        "    sys.stdout.write(json.dumps(payload) + '\\n')".to_string(),
        "    sys.stdout.flush()".to_string(),
        String::new(),
        "def cell_payload(cell_req, cell):".to_string(),
        "    return {".to_string(),
        "        'id': cell_req.get('id'),".to_string(),
        "        'execution_count': cell.get('execution_count'),".to_string(),
        "        'outputs': cell.get('outputs', []),".to_string(),
        "        'metadata': cell.get('metadata', {}),".to_string(),
        "    }".to_string(),
        String::new(),
        "async def respond(writer, payload):".to_string(),
        "    writer.write((json.dumps(payload) + '\\n').encode())".to_string(),
        "    await writer.drain()".to_string(),
        String::new(),
        "async def main():".to_string(),
        "    try:".to_string(),
        "        import nbformat, nbclient".to_string(),
        "    except ImportError as e:".to_string(),
        "        announce({'status': 'error', 'message': f'missing_dependency: {e}'})".to_string(),
        "        return".to_string(),
        "    nb = nbformat.v4.new_notebook()".to_string(),
        "    client = nbclient.NotebookClient(nb, kernel_name=KERNEL_NAME, startup_timeout=STARTUP_TIMEOUT)".to_string(),
        "    stop_event = asyncio.Event()".to_string(),
        String::new(),
        "    async def handle(reader, writer):".to_string(),
        "        try:".to_string(),
        "            line = await reader.readline()".to_string(),
        "            if not line:".to_string(),
        "                return".to_string(),
        "            req = json.loads(line)".to_string(),
        "            if req.get('token') != TOKEN:".to_string(),
        "                await respond(writer, {'status': 'error', 'message': 'unauthorized'})".to_string(),
        "                return".to_string(),
        "            cmd = req.get('cmd')".to_string(),
        "            if cmd == 'ping':".to_string(),
        "                await respond(writer, {'status': 'ok'})".to_string(),
        "            elif cmd == 'shutdown':".to_string(),
        "                await respond(writer, {'status': 'ok'})".to_string(),
        "                await client.km.shutdown_kernel()".to_string(),
        "                stop_event.set()".to_string(),
        "            elif cmd == 'execute':".to_string(),
        "                client.allow_errors = bool(req.get('allow_errors', False))".to_string(),
        "                client.record_timing = bool(req.get('record_timing', True))".to_string(),
        "                client.timeout = req.get('timeout', -1)".to_string(),
        "                client.iopub_timeout = req.get('iopub_timeout', 4)".to_string(),
        "                results = []".to_string(),
        "                status = 'ok'".to_string(),
        "                failed_id = None".to_string(),
        "                message = None".to_string(),
        "                for cell_req in req.get('cells', []):".to_string(),
        "                    cell = nbformat.v4.new_code_cell(cell_req['source'], metadata=cell_req.get('metadata', {}))".to_string(),
        "                    nb.cells.append(cell)".to_string(),
        "                    index = len(nb.cells) - 1".to_string(),
        "                    try:".to_string(),
        "                        await client.async_execute_cell(cell, index, execution_count=client.code_cells_executed + 1)".to_string(),
        "                    except BaseException as e:".to_string(),
        "                        name = type(e).__name__".to_string(),
        "                        status = 'cell_timeout' if 'Timeout' in name else ('cell_error' if name == 'CellExecutionError' else 'kernel_error')".to_string(),
        "                        message = f'{name}: {e}'".to_string(),
        "                        failed_id = cell_req.get('id')".to_string(),
        "                        results.append(cell_payload(cell_req, cell))".to_string(),
        "                        break".to_string(),
        "                    results.append(cell_payload(cell_req, cell))".to_string(),
        "                if status == 'ok':".to_string(),
        "                    for entry in results:".to_string(),
        "                        if any(o.get('output_type') == 'error' for o in entry['outputs']):".to_string(),
        "                            status = 'ok_with_errors'".to_string(),
        "                            failed_id = entry['id']".to_string(),
        "                            break".to_string(),
        "                await respond(writer, {'status': status, 'results': results, 'failed_id': failed_id, 'message': message})".to_string(),
        "            else:".to_string(),
        "                await respond(writer, {'status': 'error', 'message': f\"unknown command '{cmd}'\"})".to_string(),
        "        except BaseException as e:".to_string(),
        "            try:".to_string(),
        "                await respond(writer, {'status': 'error', 'message': f'{type(e).__name__}: {e}'})".to_string(),
        "            except Exception:".to_string(),
        "                pass".to_string(),
        "        finally:".to_string(),
        "            try:".to_string(),
        "                writer.close()".to_string(),
        "            except Exception:".to_string(),
        "                pass".to_string(),
        String::new(),
        "    try:".to_string(),
        "        async with client.async_setup_kernel(cleanup_kc=False):".to_string(),
        "            server = await asyncio.start_server(handle, host='127.0.0.1', port=0)".to_string(),
        "            port = server.sockets[0].getsockname()[1]".to_string(),
        "            announce({'status': 'ready', 'port': port, 'pid': os.getpid()})".to_string(),
        "            async with server:".to_string(),
        "                await stop_event.wait()".to_string(),
        "    except BaseException as e:".to_string(),
        "        announce({'status': 'error', 'message': f'{type(e).__name__}: {e}'})".to_string(),
        String::new(),
        "asyncio.run(main())".to_string(),
    ];
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_hex_produces_distinct_high_entropy_values() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
        let token = new_token();
        assert_eq!(token.len(), 64);
    }

    #[test]
    fn registry_roundtrips_records() {
        let dir = tempfile::tempdir().unwrap();
        let record = SessionRecord {
            id: "abc123".into(),
            name: "demo".into(),
            kernel_label: "python3".into(),
            display_name: "Python 3".into(),
            language: Some("python".into()),
            pid: 1234,
            port: 5555,
            token: "secret".into(),
            cwd: "/tmp".into(),
            created_at: now_unix(),
        };
        save(dir.path(), &record).unwrap();
        let found = find(dir.path(), "demo").unwrap().unwrap();
        assert_eq!(found.id, "abc123");
        let found_by_id = find(dir.path(), "abc123").unwrap().unwrap();
        assert_eq!(found_by_id.name, "demo");
        assert_eq!(all(dir.path()).unwrap().len(), 1);
        remove(dir.path(), "abc123").unwrap();
        assert!(all(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn find_returns_none_for_unknown_session() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find(dir.path(), "missing").unwrap().is_none());
    }

    #[test]
    fn daemon_script_is_syntactically_valid_python() {
        let Some(python) = ["python3", "python"].into_iter().find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        }) else {
            return;
        };
        let script = build_daemon_script("python3", "tok", 60);
        let output = std::process::Command::new(python)
            .arg("-c")
            .arg(format!("compile({script:?}, '<test>', 'exec')"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{script}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn daemon_script_preserves_metadata_and_honors_iopub_timeout() {
        let script = build_daemon_script("python3", "tok", 60);
        assert!(script.contains("metadata=cell_req.get('metadata', {})"));
        assert!(script.contains("client.iopub_timeout = req.get('iopub_timeout', 4)"));
    }

    #[test]
    fn zero_overall_timeout_fails_before_connecting() {
        let record = SessionRecord {
            id: "id".into(),
            name: "test".into(),
            kernel_label: "python3".into(),
            display_name: "Python 3".into(),
            language: Some("python".into()),
            pid: 0,
            port: 1,
            token: "token".into(),
            cwd: ".".into(),
            created_at: 0,
        };
        let error = execute(&record, &[], false, -1, true, Some(0), 4).unwrap_err();
        assert!(error.downcast_ref::<OverallTimeout>().is_some());
    }
}
