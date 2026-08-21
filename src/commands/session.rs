//! `nbedit session start|list|stop`: manage persistent kernel daemons.
//!
//! See `session_client` for the on-disk registry format and the wire protocol
//! used to talk to a running session's daemon.

use crate::commands::session_client::{self, SessionRecord};
use crate::commands::{kernels, run};
use crate::notebook::Notebook;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Start a session and return its record without printing anything; used by
/// both the CLI `start` command and the MCP `notebook_session_start` tool.
#[allow(clippy::too_many_arguments)]
pub fn start_and_get(
    name: Option<&str>,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    notebook: Option<&Path>,
    cwd: Option<&str>,
    environment: &[String],
    startup_timeout: u64,
) -> Result<SessionRecord> {
    let dir = session_client::registry_dir()?;
    spawn_and_register(
        &dir,
        name,
        kernel,
        interpreter,
        driver_python,
        notebook,
        cwd,
        environment,
        startup_timeout,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn start(
    name: Option<&str>,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    notebook: Option<&str>,
    cwd: Option<&str>,
    environment: &[String],
    startup_timeout: u64,
    json: bool,
) -> Result<()> {
    let record = start_and_get(
        name,
        kernel,
        interpreter,
        driver_python,
        notebook.map(Path::new),
        cwd,
        environment,
        startup_timeout,
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "id": record.id, "name": record.name, "kernel": record.kernel_label,
                "display_name": record.display_name, "pid": record.pid, "port": record.port,
            }))?
        );
    } else {
        println!(
            "Session '{}' started ({}, pid {})",
            record.name, record.display_name, record.pid
        );
    }
    Ok(())
}

/// List known sessions, pruning stale (dead) registry entries as a side
/// effect; used by both the CLI `list` command and the MCP
/// `notebook_session_list` tool.
pub fn list_records() -> Result<Vec<(SessionRecord, bool)>> {
    let dir = session_client::registry_dir()?;
    let mut rows = Vec::new();
    for record in session_client::all(&dir)? {
        let alive = session_client::is_alive(&record);
        if !alive {
            let _ = session_client::remove(&dir, &record.id);
        }
        rows.push((record, alive));
    }
    Ok(rows)
}

pub fn list(json: bool) -> Result<()> {
    let rows = list_records()?;

    if json {
        let value: Vec<Value> = rows
            .iter()
            .map(|(record, alive)| {
                json!({
                    "id": record.id, "name": record.name, "kernel": record.kernel_label,
                    "display_name": record.display_name, "language": record.language,
                    "pid": record.pid, "port": record.port, "cwd": record.cwd,
                    "created_at": record.created_at, "alive": alive,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("No sessions");
        return Ok(());
    }
    for (record, alive) in &rows {
        println!(
            "{:<20} {:<6} {:<28} pid={}",
            record.name,
            if *alive { "alive" } else { "dead" },
            record.display_name,
            record.pid
        );
    }
    Ok(())
}

pub fn stop(name: &str, force: bool) -> Result<()> {
    let dir = session_client::registry_dir()?;
    let record =
        session_client::find(&dir, name)?.with_context(|| format!("No session named '{name}'"))?;

    if force {
        kill_pid(record.pid)?;
    } else if let Err(error) = session_client::shutdown(&record, Duration::from_secs(10)) {
        eprintln!(
            "Graceful shutdown failed ({error:#}); killing pid {}",
            record.pid
        );
        kill_pid(record.pid)?;
    }
    session_client::remove(&dir, &record.id)?;
    eprintln!("Session '{name}' stopped");
    Ok(())
}

/// Resolve `--session <name>` for `nbedit run`: reuse a live session, drop a
/// stale registry entry for a dead one, and either create a fresh session or
/// fail depending on `create_if_missing`.
#[allow(clippy::too_many_arguments)]
pub fn ensure_for_run(
    name: &str,
    create_if_missing: bool,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    notebook_path: &Path,
    cwd: Option<&str>,
    environment: &[String],
    startup_timeout: u64,
) -> Result<SessionRecord> {
    let dir = session_client::registry_dir()?;
    if let Some(record) = session_client::find(&dir, name)? {
        if session_client::is_alive(&record) {
            return Ok(record);
        }
        session_client::remove(&dir, &record.id)?;
    }
    if !create_if_missing {
        bail!(
            "Session '{name}' does not exist or is no longer running. Pass --create-session \
             to create it, or run 'nbedit session start --name {name}' first."
        );
    }
    spawn_and_register(
        &dir,
        Some(name),
        kernel,
        interpreter,
        driver_python,
        Some(notebook_path),
        cwd,
        environment,
        startup_timeout,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_and_register(
    dir: &Path,
    name: Option<&str>,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    notebook_for_ranking: Option<&Path>,
    cwd: Option<&str>,
    environment: &[String],
    startup_timeout: u64,
) -> Result<SessionRecord> {
    let id = unique_id(dir)?;
    let name = name.map(str::to_owned).unwrap_or_else(|| id.clone());
    if session_client::find(dir, &name)?.is_some() {
        bail!("A session named '{name}' already exists");
    }

    let session_cwd = resolve_cwd(cwd)?;
    let candidate = resolve_kernel(
        notebook_for_ranking,
        &session_cwd,
        kernel,
        interpreter,
        driver_python,
    )?;
    let driver = run::find_driver_python(driver_python, &candidate)?;

    let kernel_name = if candidate.kernelspec_name.is_some() {
        candidate.execution_label().to_owned()
    } else {
        kernels::install_synthetic_spec(dir, &format!("nbedit-session-{id}"), &candidate)?
    };
    let token = session_client::new_token();
    let script = session_client::build_daemon_script(&kernel_name, &token, startup_timeout);
    let env = run::parse_environment(environment)?;

    let mut command = Command::new(&driver);
    command
        .arg("-c")
        .arg(&script)
        .current_dir(&session_cwd)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if candidate.kernelspec_name.is_none() {
        let combined = run::prepend_search_path(dir, std::env::var_os("JUPYTER_PATH"));
        command.env("JUPYTER_PATH", combined);
    }
    command.envs(env);
    // Detach from the shell's foreground process group so a Ctrl-C aimed at
    // `nbedit session start` (or whatever spawned it) doesn't also reach the
    // daemon that is meant to outlive this command.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to launch '{driver}'"))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        let _ = tx.send(line);
    });
    let wait = Duration::from_secs(startup_timeout) + Duration::from_secs(15);
    let line = rx
        .recv_timeout(wait)
        .map_err(|_| anyhow::anyhow!("Timed out waiting for the session kernel to start"))?;
    if line.trim().is_empty() {
        let mut stderr_output = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut stderr_output);
        }
        let detail = stderr_output.trim();
        bail!(
            "Session kernel exited before starting up{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
    }
    let payload: Value = serde_json::from_str(line.trim())
        .context("Invalid startup response from session daemon")?;
    if payload.get("status").and_then(Value::as_str) != Some("ready") {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        bail!("Failed to start session kernel: {message}");
    }
    let port = payload
        .get("port")
        .and_then(Value::as_u64)
        .context("Missing port in session daemon startup response")? as u16;
    let pid = payload
        .get("pid")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| u64::from(child.id()));

    let record = SessionRecord {
        id: id.clone(),
        name,
        kernel_label: candidate.execution_label().to_owned(),
        display_name: candidate.display_name.clone(),
        language: candidate.language.clone(),
        pid: pid as u32,
        port,
        token,
        cwd: session_cwd.to_string_lossy().into_owned(),
        created_at: session_client::now_unix(),
    };
    session_client::save(dir, &record)?;
    Ok(record)
}

fn resolve_kernel(
    notebook_for_ranking: Option<&Path>,
    workspace_cwd: &Path,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
) -> Result<kernels::KernelCandidate> {
    match notebook_for_ranking {
        Some(path) => {
            let path_str = path
                .to_str()
                .context("Notebook path contains non-UTF-8 characters")?;
            let nb = Notebook::from_file(path_str)?;
            kernels::resolve(&nb, path, kernel, interpreter, driver_python)
        }
        None => {
            let nb = Notebook {
                nbformat: 4,
                nbformat_minor: 5,
                metadata: json!({}),
                cells: Vec::new(),
                extra: Default::default(),
            };
            // `resolve` only reads the parent directory of this path for
            // workspace-scoped kernel discovery; the file itself need not exist.
            let anchor = workspace_cwd.join("session.ipynb");
            kernels::resolve(&nb, &anchor, kernel, interpreter, driver_python)
        }
    }
}

fn resolve_cwd(cwd: Option<&str>) -> Result<PathBuf> {
    let path = match cwd {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir().context("Cannot determine the current directory")?,
    };
    if !path.is_dir() {
        bail!("Working directory '{}' does not exist", path.display());
    }
    Ok(path)
}

fn unique_id(dir: &Path) -> Result<String> {
    for _ in 0..5 {
        let id = session_client::new_id();
        if !dir.join(format!("{id}.json")).exists() {
            return Ok(id);
        }
    }
    bail!("Could not allocate a unique session id")
}

fn kill_pid(pid: u32) -> Result<()> {
    #[cfg(unix)]
    // The daemon owns a separate process group and the kernel is its child.
    // Targeting the group prevents an orphaned kernel after a forced stop.
    let status = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
    #[cfg(windows)]
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("Failed to kill process {pid} (exit status {status})"),
        Err(error) => Err(error).with_context(|| format!("Failed to kill process {pid}")),
    }
}
