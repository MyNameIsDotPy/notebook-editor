use crate::commands::{kernels, session, session_client};
use crate::error::AppExit;
use crate::notebook::Notebook;
use crate::selection;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

type EventObserver = Box<dyn Fn(&Value)>;
thread_local! {
    static OBSERVER: std::cell::RefCell<Option<EventObserver>> = std::cell::RefCell::new(None);
    static CANCEL: std::cell::RefCell<Option<std::sync::Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}
pub fn observe_with(cancel: std::sync::Arc<AtomicBool>, observer: impl Fn(&Value) + 'static) {
    CANCEL.with(|c| *c.borrow_mut() = Some(cancel));
    OBSERVER.with(|o| *o.borrow_mut() = Some(Box::new(observer)));
}
fn emit(event: &Value) {
    OBSERVER.with(|o| {
        if let Some(f) = o.borrow().as_ref() {
            f(event);
        }
    });
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static CTRL_C_HANDLER: OnceLock<std::result::Result<(), String>> = OnceLock::new();

#[derive(Debug, Serialize)]
pub struct ExecutionReport {
    pub status: String,
    pub execution_status: String,
    pub kernel: String,
    pub source: String,
    pub requested_cells: Vec<usize>,
    pub executed_cells: Vec<usize>,
    pub failed_cell: Option<usize>,
    pub duration_ms: u128,
    pub outputs_saved: bool,
    pub session: Option<String>,
    pub execution_id: Option<String>,
    pub message: Option<String>,
    pub cells: Vec<CellExecution>,
}

#[derive(Debug, Serialize)]
pub struct CellExecution {
    pub index: usize,
    pub context: bool,
    pub state: String,
    pub execution_count: Value,
    pub outputs: Vec<Value>,
}

impl ExecutionReport {
    pub fn check(&self) -> Result<()> {
        let code = match self.status.as_str() {
            "ok" | "ok_with_errors" | "ready" => return Ok(()),
            "missing_dependency" => 2,
            "overall_timeout" => 124,
            "interrupted" => 130,
            _ => 1,
        };
        Err(AppExit::new(
            code,
            self.message.clone().unwrap_or_else(|| self.status.clone()),
        )
        .into())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    notebook: &str,
    selection: &str,
    timeout: i64,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    allow_errors: bool,
    include_prior: bool,
    startup_timeout: u64,
    iopub_timeout: u64,
    record_timing: bool,
    overall_timeout: Option<u64>,
    cwd: Option<&str>,
    environment: &[String],
    dry_run: bool,
    json: bool,
    backup: bool,
    quiet: bool,
    session: Option<&str>,
    create_session: bool,
) -> Result<()> {
    let report = execute(
        notebook,
        selection,
        timeout,
        kernel,
        interpreter,
        driver_python,
        allow_errors,
        include_prior,
        startup_timeout,
        iopub_timeout,
        record_timing,
        overall_timeout,
        cwd,
        environment,
        dry_run,
        json,
        backup,
        quiet,
        session,
        create_session,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if report.outputs_saved && !quiet {
        eprintln!("Outputs written to {notebook}");
    }
    report.check()
}

#[allow(clippy::too_many_arguments)]
pub fn execute(
    notebook: &str,
    selection: &str,
    timeout: i64,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    allow_errors: bool,
    include_prior: bool,
    startup_timeout: u64,
    iopub_timeout: u64,
    record_timing: bool,
    overall_timeout: Option<u64>,
    cwd: Option<&str>,
    environment: &[String],
    dry_run: bool,
    json: bool,
    backup: bool,
    quiet: bool,
    session: Option<&str>,
    create_session: bool,
) -> Result<ExecutionReport> {
    execute_request(ExecutionRequest {
        notebook,
        selection,
        timeout,
        kernel,
        interpreter,
        driver_python,
        allow_errors,
        include_prior,
        startup_timeout,
        iopub_timeout,
        record_timing,
        overall_timeout,
        cwd,
        environment,
        dry_run,
        json,
        backup,
        quiet,
        session,
        create_session,
    })
}

/// Shared execution options; presentation remains in the CLI/MCP adapters.
#[derive(Debug)]
pub struct ExecutionRequest<'a> {
    pub notebook: &'a str,
    pub selection: &'a str,
    pub timeout: i64,
    pub kernel: Option<&'a str>,
    pub interpreter: Option<&'a str>,
    pub driver_python: Option<&'a str>,
    pub allow_errors: bool,
    pub include_prior: bool,
    pub startup_timeout: u64,
    pub iopub_timeout: u64,
    pub record_timing: bool,
    pub overall_timeout: Option<u64>,
    pub cwd: Option<&'a str>,
    pub environment: &'a [String],
    pub dry_run: bool,
    pub json: bool,
    pub backup: bool,
    pub quiet: bool,
    pub session: Option<&'a str>,
    pub create_session: bool,
}

pub fn execute_request(request: ExecutionRequest<'_>) -> Result<ExecutionReport> {
    let ExecutionRequest {
        notebook,
        selection,
        timeout,
        kernel,
        interpreter,
        driver_python,
        allow_errors,
        include_prior,
        startup_timeout,
        iopub_timeout,
        record_timing,
        overall_timeout,
        cwd,
        environment,
        dry_run,
        json,
        backup,
        quiet,
        session,
        create_session,
    } = request;
    if timeout < -1 {
        bail!("--timeout must be -1 or a non-negative number");
    }
    if include_prior && session.is_some() {
        bail!("--include-prior cannot be used with a session");
    }
    if create_session && session.is_none() {
        bail!("--create-session requires a session");
    }
    let revision = std::fs::read(notebook)?;
    let mut nb: Notebook = serde_json::from_slice(&revision)?;
    nb.ensure_cell_ids();
    let indices = selection::resolve(selection, nb.len())?;
    let code_indices: Vec<usize> = indices
        .into_iter()
        .filter(|&i| nb.cells[i].cell_type == "code")
        .collect();
    if code_indices.is_empty() {
        bail!("No code cells in selection");
    }

    let notebook_path = Path::new(notebook);

    if let Some(session_name) = session {
        if dry_run {
            if !json && !quiet {
                println!("Session: {session_name}");
                println!("Would execute {} code cell(s)", code_indices.len());
            }
            return Ok(ExecutionReport {
                status: "ready".into(),
                execution_status: "ready".into(),
                kernel: String::new(),
                source: String::new(),
                requested_cells: code_indices.iter().map(|i| i + 1).collect(),
                executed_cells: vec![],
                failed_cell: None,
                duration_ms: 0,
                outputs_saved: false,
                session: session.map(str::to_owned),
                execution_id: None,
                message: None,
                cells: vec![],
            });
        }
        return run_session(
            &mut nb,
            notebook,
            notebook_path,
            &code_indices,
            session_name,
            create_session,
            kernel,
            interpreter,
            driver_python,
            cwd,
            environment,
            startup_timeout,
            allow_errors,
            timeout,
            record_timing,
            overall_timeout,
            iopub_timeout,
            json,
            quiet,
            backup,
            &revision,
        );
    }

    let candidate = kernels::resolve(&nb, notebook_path, kernel, interpreter, driver_python)?;
    if dry_run {
        if !json && !quiet {
            println!(
                "Kernel: {} ({:?})",
                candidate.display_name, candidate.source
            );
            if let Some(path) = &candidate.interpreter {
                println!("Interpreter: {}", path.display());
            }
            println!("Would execute {} code cell(s)", code_indices.len());
        }
        return Ok(ExecutionReport {
            status: "ready".into(),
            execution_status: "ready".into(),
            kernel: candidate.execution_label().into(),
            source: format!("{:?}", candidate.source),
            requested_cells: code_indices.iter().map(|i| i + 1).collect(),
            executed_cells: vec![],
            failed_cell: None,
            duration_ms: 0,
            outputs_saved: false,
            session: session.map(str::to_owned),
            execution_id: None,
            message: None,
            cells: vec![],
        });
    }

    let execution_indices: Vec<usize> = if include_prior {
        let last = *code_indices.last().expect("non-empty code selection");
        (0..=last)
            .filter(|&i| nb.cells[i].cell_type == "code")
            .collect()
    } else {
        code_indices.clone()
    };
    let selected_cells: Vec<_> = execution_indices
        .iter()
        .map(|&i| nb.cells[i].clone())
        .collect();
    let mut mini_meta = nb.metadata.clone();
    if let Some(ks) = mini_meta.get_mut("kernelspec") {
        if let Some(name) = &candidate.kernelspec_name {
            ks["name"] = Value::String(name.clone());
        }
    }
    let mini_nb = serde_json::json!({
        "nbformat": nb.nbformat, "nbformat_minor": nb.nbformat_minor,
        "metadata": mini_meta, "cells": selected_cells,
    });

    let tmp_dir = tempfile::tempdir()?;
    let tmp_nb = tmp_dir.path().join("exec.ipynb");
    let report_path = tmp_dir.path().join("report.json");
    std::fs::write(&tmp_nb, serde_json::to_vec_pretty(&mini_nb)?)?;

    let kernel_name = if candidate.kernelspec_name.is_some() {
        candidate.execution_label().to_owned()
    } else {
        kernels::install_synthetic_spec(tmp_dir.path(), "nbedit-env", &candidate)?
    };
    let script = build_script(
        tmp_nb
            .to_str()
            .context("temp notebook path contains non-UTF-8 characters")?,
        report_path
            .to_str()
            .context("temp report path contains non-UTF-8 characters")?,
        timeout,
        &kernel_name,
        allow_errors,
        startup_timeout,
        iopub_timeout,
        record_timing,
        &execution_indices,
        !quiet && !json,
    );
    let driver = find_driver_python(driver_python, &candidate)?;
    let kernel_cwd = cwd
        .map(Path::new)
        .unwrap_or_else(|| kernels::parent_or_current(notebook_path));
    if !kernel_cwd.is_dir() {
        bail!(
            "Working directory '{}' does not exist",
            kernel_cwd.display()
        );
    }
    let env = parse_environment(environment)?;

    if !quiet && !json {
        eprintln!(
            "Executing {} code cell(s) with {}...",
            code_indices.len(),
            candidate.display_name
        );
    }
    let started = Instant::now();
    let mut command = Command::new(&driver);
    command
        .arg("-c")
        .arg(&script)
        .current_dir(kernel_cwd)
        .env("PYTHONUNBUFFERED", "1");
    if candidate.kernelspec_name.is_none() {
        let combined = prepend_search_path(tmp_dir.path(), std::env::var_os("JUPYTER_PATH"));
        command.env("JUPYTER_PATH", combined);
    }
    command.envs(env);
    prepare_interrupt()?;
    let (_status, wall_timed_out, interrupted) =
        wait_for_child(command, overall_timeout, &tmp_dir.path().join("cancel"))
            .with_context(|| format!("Failed to launch '{driver}'"))?;

    let py_report: Value = std::fs::read(&report_path).ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_else(|| serde_json::json!({"status": if wall_timed_out {"overall_timeout"} else if interrupted {"interrupted"} else {"driver_failure"}}));
    let py_status = if wall_timed_out {
        "overall_timeout"
    } else if interrupted {
        "interrupted"
    } else {
        py_report
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("driver_failure")
    };
    let execution_started = py_report
        .get("execution_started")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut outputs_saved = false;
    let states = py_report
        .get("states")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut cell_reports = Vec::new();
    if execution_started {
        let executed: Value = serde_json::from_slice(&std::fs::read(&tmp_nb)?)?;
        let executed_cells = executed
            .get("cells")
            .and_then(Value::as_array)
            .context("Executed notebook missing cells array")?;
        if executed_cells.len() != execution_indices.len() {
            bail!("Cell count mismatch after execution");
        }
        for (slot, &index) in execution_indices.iter().enumerate() {
            let ec = &executed_cells[slot];
            let state = states
                .get(slot)
                .and_then(Value::as_str)
                .unwrap_or("skipped");
            cell_reports.push(CellExecution {
                index: index + 1,
                context: !code_indices.contains(&index),
                state: state.into(),
                execution_count: ec.get("execution_count").cloned().unwrap_or(Value::Null),
                outputs: if state == "skipped" {
                    vec![]
                } else {
                    ec.get("outputs")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                },
            });
        }
        for &nb_idx in &code_indices {
            let slot = execution_indices
                .iter()
                .position(|&i| i == nb_idx)
                .context("Selected cell missing from execution result")?;
            if states
                .get(slot)
                .and_then(Value::as_str)
                .unwrap_or("skipped")
                == "skipped"
            {
                continue;
            }
            let ec = &executed_cells[slot];
            if let Some(outputs) = ec.get("outputs").and_then(Value::as_array) {
                nb.cells[nb_idx].outputs = outputs.clone();
            }
            nb.cells[nb_idx].execution_count =
                Some(ec.get("execution_count").cloned().unwrap_or(Value::Null));
            if let Some(metadata) = ec.get("metadata") {
                nb.cells[nb_idx].metadata = metadata.clone();
            }
            if let Some(id) = ec.get("id").and_then(Value::as_str) {
                nb.cells[nb_idx].id = Some(id.to_owned());
            }
        }
        outputs_saved = save_checked(&nb, notebook, backup, &revision)?;
    }

    if cell_reports.is_empty() {
        cell_reports = execution_indices
            .iter()
            .map(|&index| CellExecution {
                index: index + 1,
                context: !code_indices.contains(&index),
                state: "skipped".into(),
                execution_count: Value::Null,
                outputs: vec![],
            })
            .collect();
    }
    let failed_slot = py_report
        .get("failed_cell")
        .or_else(|| {
            if wall_timed_out || interrupted {
                py_report.get("active_cell")
            } else {
                None
            }
        })
        .and_then(Value::as_u64)
        .map(|i| i as usize);
    if wall_timed_out || interrupted {
        for cell in &mut cell_reports {
            if cell.state == "running" {
                cell.state = "failed".into();
            }
        }
    }
    let report = ExecutionReport {
        status: py_status.into(),
        execution_status: py_status.into(),
        kernel: candidate.execution_label().into(),
        source: format!("{:?}", candidate.source),
        requested_cells: code_indices.iter().map(|i| i + 1).collect(),
        executed_cells: cell_reports
            .iter()
            .filter(|c| c.state != "skipped" && c.state != "pending")
            .map(|c| c.index)
            .collect(),
        failed_cell: failed_slot.and_then(|slot| execution_indices.get(slot).map(|i| i + 1)),
        duration_ms: started.elapsed().as_millis(),
        outputs_saved,
        session: None,
        execution_id: None,
        message: if !outputs_saved && execution_started {
            Some(
                "Notebook changed during execution; results returned without overwriting it".into(),
            )
        } else {
            py_report
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        },
        cells: cell_reports,
    };
    let mut report = report;
    if !outputs_saved && execution_started {
        report.status = "save_conflict".into();
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn run_session(
    nb: &mut Notebook,
    notebook: &str,
    notebook_path: &Path,
    code_indices: &[usize],
    session_name: &str,
    create_session: bool,
    kernel: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
    cwd: Option<&str>,
    environment: &[String],
    startup_timeout: u64,
    allow_errors: bool,
    timeout: i64,
    record_timing: bool,
    overall_timeout: Option<u64>,
    iopub_timeout: u64,
    json: bool,
    quiet: bool,
    backup: bool,
    revision: &[u8],
) -> Result<ExecutionReport> {
    let creation_cwd = kernels::parent_or_current(notebook_path).to_string_lossy();
    let cwd = cwd.or_else(|| {
        if create_session {
            Some(creation_cwd.as_ref())
        } else {
            None
        }
    });
    let record = session::ensure_for_run(
        session_name,
        create_session,
        kernel,
        interpreter,
        driver_python,
        notebook_path,
        cwd,
        environment,
        startup_timeout,
    )?;

    if !quiet && !json {
        eprintln!(
            "Executing {} code cell(s) with session '{}' ({})...",
            code_indices.len(),
            record.name,
            record.display_name
        );
    }

    let cells: Vec<session_client::CellRequest> = code_indices
        .iter()
        .map(|&idx| session_client::CellRequest {
            // Request IDs must be unique even for invalid notebooks that carry
            // duplicate nbformat cell IDs.
            id: format!("nbedit-session-{idx}"),
            source: nb.cells[idx].source_str(),
            metadata: nb.cells[idx].metadata.clone(),
        })
        .collect();
    let id_to_index: HashMap<String, usize> = cells
        .iter()
        .zip(code_indices)
        .map(|(cell, &idx)| (cell.id.clone(), idx))
        .collect();

    let started = Instant::now();
    let response = session_client::execute_observed(
        &record,
        &cells,
        allow_errors,
        timeout,
        record_timing,
        overall_timeout,
        iopub_timeout,
        |event| {
            emit(event);
            if !quiet && !json {
                if let Some(kind @ ("cell_started" | "cell_completed")) =
                    event.get("type").and_then(Value::as_str)
                {
                    let id = event.get("id").and_then(Value::as_str).unwrap_or("");
                    if let Some(index) = id_to_index.get(id) {
                        eprintln!("[nbedit] cell {}: {}", index + 1, kind);
                    }
                }
            }
        },
    )
    .map_err(|error| {
        let code = if error
            .downcast_ref::<session_client::OverallTimeout>()
            .is_some()
        {
            124
        } else {
            1
        };
        AppExit::new(
            code,
            format!("Session '{}' execution failed: {error:#}", record.name),
        )
    })?;

    let mut outputs_saved = false;
    for result in &response.results {
        if result.state == "skipped" {
            continue;
        }
        if let Some(&nb_idx) = id_to_index.get(result.id.as_str()) {
            nb.cells[nb_idx].outputs = result.outputs.clone();
            nb.cells[nb_idx].execution_count = Some(result.execution_count.clone());
            nb.cells[nb_idx].metadata = result.metadata.clone();
        }
    }
    if !response.results.is_empty() {
        outputs_saved = save_checked(nb, notebook, backup, revision)?;
    }

    let failed_slot = response
        .failed_id
        .as_deref()
        .and_then(|id| id_to_index.get(id).copied());
    let cell_reports: Vec<_> = code_indices
        .iter()
        .map(|&index| {
            let result = response
                .results
                .iter()
                .find(|r| id_to_index.get(&r.id) == Some(&index));
            CellExecution {
                index: index + 1,
                context: false,
                state: result
                    .map(|r| r.state.clone())
                    .unwrap_or_else(|| "skipped".into()),
                execution_count: result
                    .map(|r| r.execution_count.clone())
                    .unwrap_or(Value::Null),
                outputs: result.map(|r| r.outputs.clone()).unwrap_or_default(),
            }
        })
        .collect();
    let report = ExecutionReport {
        status: response.status.clone(),
        execution_status: response.status.clone(),
        kernel: record.kernel_label.clone(),
        source: "Session".into(),
        requested_cells: code_indices.iter().map(|i| i + 1).collect(),
        executed_cells: cell_reports
            .iter()
            .filter(|c| c.state != "skipped")
            .map(|c| c.index)
            .collect(),
        failed_cell: failed_slot.map(|i| i + 1),
        duration_ms: started.elapsed().as_millis(),
        outputs_saved,
        session: Some(record.name.clone()),
        execution_id: response.execution_id.clone(),
        message: response.message.clone(),
        cells: cell_reports,
    };
    let mut report = report;
    if !outputs_saved && !response.results.is_empty() {
        report.status = "save_conflict".into();
        report.message = Some(
            "Notebook changed during execution; retrieve results with the execution ID".into(),
        );
    }
    Ok(report)
}

/// Cooperating executions serialize the compare-and-save operation. External
/// editors are detected by a byte-for-byte revision check immediately before save.
fn save_checked(nb: &Notebook, notebook: &str, backup: bool, revision: &[u8]) -> Result<bool> {
    nb.save_if_unchanged(notebook, backup, revision)
}

pub(crate) fn prepare_interrupt() -> Result<()> {
    INTERRUPTED.store(false, Ordering::SeqCst);
    CTRL_C_HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::SeqCst))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("Cannot install Ctrl-C handler: {e}"))?;
    Ok(())
}
pub(crate) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
        || CANCEL.with(|c| {
            c.borrow()
                .as_ref()
                .is_some_and(|v| v.load(Ordering::SeqCst))
        })
}

fn wait_for_child(
    mut command: Command,
    timeout: Option<u64>,
    cancel_path: &Path,
) -> std::io::Result<(ExitStatus, bool, bool)> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000200);
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    let mut interrupted_at = None;
    let mut wall_timed_out = false;
    let mut previous_report = Vec::new();
    loop {
        if let Some(parent) = cancel_path.parent() {
            if let Ok(bytes) = std::fs::read(parent.join("report.json")) {
                if bytes != previous_report {
                    if let Ok(report) = serde_json::from_slice::<Value>(&bytes) {
                        emit(&serde_json::json!({"type":"checkpoint", "report": report}));
                        previous_report = bytes;
                    }
                }
            }
        }
        if let Some(status) = child.try_wait()? {
            return Ok((
                status,
                wall_timed_out,
                interrupted_at.is_some() && !wall_timed_out,
            ));
        }
        if interrupted() && interrupted_at.is_none() {
            // The terminal delivers Ctrl-C to the whole foreground process group,
            // including the Python driver. Give its finally blocks time to write.
            interrupted_at = Some(Instant::now());
            std::fs::write(cancel_path, "interrupted")?;
        }
        if interrupted_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
            let _ = session::kill_pid(child.id());
            let _ = child.kill();
            return Ok((child.wait()?, wall_timed_out, !wall_timed_out));
        }
        if interrupted_at.is_none()
            && timeout.is_some_and(|seconds| started.elapsed() >= Duration::from_secs(seconds))
        {
            wall_timed_out = true;
            interrupted_at = Some(Instant::now());
            std::fs::write(cancel_path, "overall_timeout")?;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) fn parse_environment(values: &[String]) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    for item in values {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid --env '{item}'; expected KEY=VALUE"))?;
        if key.is_empty() || key.contains('\0') || value.contains('\0') {
            bail!("Invalid --env '{item}'");
        }
        result.insert(key.into(), value.into());
    }
    Ok(result)
}

pub(crate) fn prepend_search_path(
    first: &Path,
    existing: Option<std::ffi::OsString>,
) -> std::ffi::OsString {
    let mut paths = vec![first.to_path_buf()];
    if let Some(existing) = existing {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap_or_else(|_| first.as_os_str().to_owned())
}

pub(crate) fn find_driver_python(
    override_path: Option<&str>,
    candidate: &kernels::KernelCandidate,
) -> Result<String> {
    select_driver_python(override_path, candidate, python_has_driver_dependencies)
}

fn select_driver_python(
    override_path: Option<&str>,
    candidate: &kernels::KernelCandidate,
    has_dependencies: impl Fn(&str) -> bool,
) -> Result<String> {
    if let Some(path) = override_path {
        return require_driver_dependencies(path, &has_dependencies);
    }

    // A Python kernelspec normally points at the environment that should run
    // the notebook. Prefer it as the nbclient driver too, avoiding a surprising
    // and usually incorrect second Python selected from PATH.
    if candidate
        .language
        .as_deref()
        .is_some_and(|language| language.eq_ignore_ascii_case("python"))
    {
        if let Some(path) = candidate.interpreter.as_ref().and_then(|p| p.to_str()) {
            if has_dependencies(path) {
                return Ok(path.to_owned());
            }
        }
    }

    for candidate in ["python3", "python"] {
        if has_dependencies(candidate) {
            return Ok(candidate.to_string());
        }
    }

    let suggested = candidate
        .interpreter
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<python>".into());
    Err(AppExit::new(
        2,
        format!(
            "No Python driver with nbclient and nbformat was found. Install them for the selected kernel with:\n  \"{suggested}\" -m pip install nbclient nbformat\nor pass --driver-python <path>"
        ),
    )
    .into())
}

fn require_driver_dependencies(
    path: &str,
    has_dependencies: impl Fn(&str) -> bool,
) -> Result<String> {
    if has_dependencies(path) {
        Ok(path.to_owned())
    } else {
        Err(AppExit::new(
            2,
            format!(
                "Driver Python '{path}' cannot import nbclient and nbformat. Install them with:\n  \"{path}\" -m pip install nbclient nbformat"
            ),
        )
        .into())
    }
}

fn python_has_driver_dependencies(path: &str) -> bool {
    Command::new(path)
        .args(["-c", "import nbclient, nbformat"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn build_script(
    path: &str,
    report: &str,
    timeout: i64,
    kernel: &str,
    allow_errors: bool,
    startup_timeout: u64,
    iopub_timeout: u64,
    record_timing: bool,
    execution_indices: &[usize],
    verbose: bool,
) -> String {
    // JSON string literals are valid Python string literals and safely handle
    // quotes, control characters, Windows separators, and Unicode paths.
    let path_repr = serde_json::to_string(path).expect("string serialization cannot fail");
    let report_repr = serde_json::to_string(report).expect("string serialization cannot fail");
    let kernel_repr = serde_json::to_string(kernel).expect("string serialization cannot fail");
    let timeout = if timeout == -1 {
        "None".into()
    } else {
        timeout.to_string()
    };
    let allow_errors = if allow_errors { "True" } else { "False" };
    let record_timing = if record_timing { "True" } else { "False" };
    // 1-based original notebook cell numbers, in the same order as the mini-notebook's cells
    // (see `execution_indices` at the call site) — lets the progress hooks below report the
    // number a human/agent actually sees via `nbedit read`, not nbclient's own 0-based position
    // within this temporary subset.
    let cell_numbers: Vec<usize> = execution_indices.iter().map(|i| i + 1).collect();
    let cell_numbers_repr =
        serde_json::to_string(&cell_numbers).expect("number serialization cannot fail");
    // Progress hooks print to stderr (matching this file's existing eprintln! convention for
    // status/progress text, e.g. "Executing N code cell(s)..." above) so they never mix into a
    // `--json` caller's stdout, and are gated by the same `!quiet && !json` computed at the call
    // site — `verbose=false` omits this block entirely, leaving the generated script identical
    // to before this feature existed. Assigning `on_cell_start`/`on_cell_complete` (rather than
    // passing them as NotebookClient constructor kwargs) and guarding the assignment in a bare
    // `try/except` means an nbclient version too old to have these Callable traitlets just loses
    // the progress lines instead of breaking execution — confirmed against the real nbclient
    // 0.11.0 source (`client.py`): both hooks are called with plain `hook(**kwargs)`, only
    // awaited if the return value is itself awaitable, so a plain sync function is valid.
    let progress_hooks = if verbose {
        vec![
            format!("        _nbedit_cells = {cell_numbers_repr}"),
            "        def _nbedit_cell_start(cell=None, cell_index=None, **_kw):".to_string(),
            "            n = _nbedit_cells[cell_index] if cell_index is not None and 0 <= cell_index < len(_nbedit_cells) else '?'".to_string(),
            "            print(f'[nbedit] cell {n} ({(cell_index or 0) + 1}/{len(_nbedit_cells)}): starting', file=sys.stderr, flush=True)".to_string(),
            "        def _nbedit_cell_complete(cell=None, cell_index=None, **_kw):".to_string(),
            "            n = _nbedit_cells[cell_index] if cell_index is not None and 0 <= cell_index < len(_nbedit_cells) else '?'".to_string(),
            "            print(f'[nbedit] cell {n} ({(cell_index or 0) + 1}/{len(_nbedit_cells)}): done', file=sys.stderr, flush=True)".to_string(),
            "        try:".to_string(),
            "            client.on_cell_start = _nbedit_cell_start".to_string(),
            "            client.on_cell_complete = _nbedit_cell_complete".to_string(),
            "        except Exception:".to_string(),
            "            pass".to_string(),
        ]
    } else {
        vec![]
    };
    let tracking_hooks = vec![
        "        states = ['skipped'] * len(nb.cells)".to_string(),
        "        result['states'] = states".to_string(),
        "        for c in nb.cells: c.outputs = []; c.execution_count = None".to_string(),

        "        def checkpoint():".to_string(),
        "            active = result.get('active_cell')".to_string(),
        "            if active is not None: result['active_outputs'] = nb.cells[active].get('outputs', [])".to_string(),
        format!("            with open({path_repr} + '.next', 'w', encoding='utf-8') as f: nbformat.write(nb, f)"),
        format!("            os.replace({path_repr} + '.next', {path_repr})"),
        "            with open(report_path, 'w', encoding='utf-8') as f: json.dump(result, f)".to_string(),
        "        def track_start(cell=None, cell_index=None, **kw):".to_string(),
        "            states[cell_index] = 'running'".to_string(),
        "            result['active_cell'] = cell_index".to_string(),
        "            cell.outputs = []; cell.execution_count = None".to_string(),
        "            checkpoint()".to_string(),

        "        def track_done(cell=None, cell_index=None, **kw):".to_string(),
        "            states[cell_index] = 'failed' if any(o.get('output_type') == 'error' for o in cell.get('outputs', [])) else 'completed'".to_string(),
        "            checkpoint()".to_string(),
        "        client.on_cell_execute = track_start".to_string(),
        "        client.on_cell_executed = track_done".to_string(),
        "        original_process_message = client.process_message".to_string(),
        "        def process_message(msg, cell, cell_index):".to_string(),
        "            value = original_process_message(msg, cell, cell_index)".to_string(),
        "            checkpoint()".to_string(),
        "            return value".to_string(),
        "        client.process_message = process_message".to_string(),
    ];
    let lines = [vec![
        "import json, sys, traceback, os, asyncio, time".to_string(),
        "if sys.platform == 'win32':".to_string(),
        "    import asyncio".to_string(),
        "    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())".to_string(),
        format!("report_path = {report_repr}"),
        format!("result = {{'status': 'driver_failure', 'execution_started': False, 'cell_numbers': {cell_numbers_repr}}}"),
        "nb = None".to_string(),
        "try:".to_string(),
        "    import nbformat, nbclient".to_string(),
        "except ImportError as e:".to_string(),
        "    result = {'status': 'missing_dependency', 'execution_started': False, 'message': str(e)}".to_string(),
        "else:".to_string(),
        "    try:".to_string(),
        format!("        nb = nbformat.read(open({path_repr}, encoding='utf-8'), as_version=4)"),
        format!("        client = nbclient.NotebookClient(nb, timeout={timeout}, kernel_name={kernel_repr}, allow_errors={allow_errors}, startup_timeout={startup_timeout}, iopub_timeout={iopub_timeout}, record_timing={record_timing})"),
    ], progress_hooks, tracking_hooks, vec![
        "        result['execution_started'] = True".to_string(),
        "        async def execute_with_cancel():".to_string(),
        "            task = asyncio.create_task(client.async_execute())".to_string(),
        format!("            marker = os.path.join(os.path.dirname({report_repr}), 'cancel')"),
        "            try:".to_string(),
        "                while not task.done():".to_string(),
        "                    await asyncio.wait({task}, timeout=0.05)".to_string(),
        "                    if os.path.exists(marker):".to_string(),
        "                        if client.km and client.km.has_kernel: await client.km.interrupt_kernel()".to_string(),
        "                        try: await asyncio.wait_for(asyncio.shield(task), timeout=3)".to_string(),
        "                        except BaseException:".to_string(),
        "                            if client.km and client.km.has_kernel: await client.km.shutdown_kernel(now=True)".to_string(),
        "                        break".to_string(),
        "                if task.done(): await task".to_string(),
        "            finally:".to_string(),
        "                if not task.done():".to_string(),
        "                    task.cancel()".to_string(),
        "                    try: await task".to_string(),
        "                    except BaseException: pass".to_string(),
        "        asyncio.run(execute_with_cancel())".to_string(),
        "        failures = [(i, o) for i, c in enumerate(nb.cells) for o in c.get('outputs', []) if o.get('output_type') == 'error']".to_string(),
        "        result.update(status='ok_with_errors' if failures else 'ok', execution_started=True)".to_string(),
        "        if failures: result['failed_cell'] = failures[0][0]".to_string(),
        "    except BaseException as e:".to_string(),
        "        name = type(e).__name__".to_string(),
        "        status = 'cell_timeout' if 'Timeout' in name else ('cell_error' if name == 'CellExecutionError' else 'kernel_error')".to_string(),
        "        result.update(status=status, message=f'{name}: {e}')".to_string(),
        "        if result.get('active_cell') is not None: result['failed_cell'] = result['active_cell']; result['states'][result['active_cell']] = 'failed'".to_string(),
        "        for i, cell in enumerate(nb.cells if nb else []):".to_string(),
        "            if 'failed_cell' not in result and any(o.get('output_type') == 'error' for o in cell.get('outputs', [])): result['failed_cell'] = i; break".to_string(),
        "    finally:".to_string(),
        "        if nb is not None:".to_string(),
        format!("            nbformat.write(nb, open({path_repr}, 'w', encoding='utf-8'))"),
        "finally:".to_string(),
        "    with open(report_path, 'w', encoding='utf-8') as f: json.dump(result, f)".to_string(),
        "sys.exit(0 if result['status'] in ('ok', 'ok_with_errors') else (2 if result['status'] == 'missing_dependency' else 1))".to_string(),
    ]]
    .concat();
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_notebook_is_not_overwritten_or_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ipynb");
        let original = br#"{"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[]}"#;
        std::fs::write(&path, original).unwrap();
        let nb: Notebook = serde_json::from_slice(original).unwrap();
        std::fs::write(&path, b"external edit").unwrap();
        assert!(!save_checked(&nb, path.to_str().unwrap(), true, original).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
        assert!(!path.with_extension("ipynb.nbedit-lock").exists());
        assert!(!path.with_extension("ipynb.bak").exists());
    }

    #[test]
    fn busy_save_lock_retains_results_without_removing_another_writers_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ipynb");
        let original = br#"{"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[]}"#;
        std::fs::write(&path, original).unwrap();
        let lock = path.with_extension("ipynb.nbedit-lock");
        std::fs::write(&lock, b"busy").unwrap();
        let nb: Notebook = serde_json::from_slice(original).unwrap();
        assert!(!save_checked(&nb, path.to_str().unwrap(), false, original).unwrap());
        assert_eq!(std::fs::read(lock).unwrap(), b"busy");
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn generated_script_has_finally_persistence() {
        let script = build_script(
            "nb.ipynb",
            "report.json",
            60,
            "python3",
            false,
            60,
            4,
            true,
            &[],
            false,
        );
        assert!(script.contains("finally:\n        if nb is not None:"));
        assert!(script.contains("json.dump(result, f)"));
    }

    #[test]
    fn no_limit_maps_to_python_none() {
        let script = build_script(
            "nb.ipynb",
            "report.json",
            -1,
            "python3",
            false,
            60,
            4,
            true,
            &[],
            false,
        );
        assert!(script.contains("timeout=None"));
    }

    #[test]
    fn generated_script_is_syntactically_valid_python() {
        let Some(python) = ["python3", "python"].into_iter().find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        }) else {
            return;
        };
        // verbose=true here so the progress-hook branch (the newest, riskiest code) is
        // exercised by the real interpreter, not just the quiet/json-suppressed path.
        for verbose in [false, true] {
            let script = build_script(
                "nb.ipynb",
                "report.json",
                60,
                "python3",
                false,
                60,
                4,
                true,
                &[2, 4],
                verbose,
            );
            let output = Command::new(python)
                .arg("-c")
                .arg(format!("compile({script:?}, '<test>', 'exec')"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "verbose={verbose}: {}\n{script}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn verbose_emits_progress_hooks_with_one_based_original_cell_numbers() {
        let script = build_script(
            "nb.ipynb",
            "report.json",
            60,
            "python3",
            false,
            60,
            4,
            true,
            &[3, 4],
            true,
        );
        assert!(
            script.contains("_nbedit_cells = [4,5]") || script.contains("_nbedit_cells = [4, 5]")
        );
        assert!(script.contains("client.on_cell_start = _nbedit_cell_start"));
        assert!(script.contains("client.on_cell_complete = _nbedit_cell_complete"));
        // Guarded so an nbclient too old to have these Callable traitlets loses the progress
        // lines instead of breaking execution.
        assert!(script.contains("        try:\n            client.on_cell_start"));
        assert!(script.contains("        except Exception:\n            pass"));
        // Goes to stderr, not stdout, so it never corrupts a --json caller's stdout parsing.
        assert!(script.contains("file=sys.stderr"));
    }

    #[test]
    fn quiet_or_json_omits_progress_hooks_entirely() {
        let script = build_script(
            "nb.ipynb",
            "report.json",
            60,
            "python3",
            false,
            60,
            4,
            true,
            &[0],
            false,
        );
        assert!(!script.contains("client.on_cell_start = _nbedit_cell_start"));
        assert!(!script.contains("_nbedit_cell"));
    }

    #[test]
    fn parses_environment_values() {
        let parsed = parse_environment(&["A=1".into(), "B=x=y".into()]).unwrap();
        assert_eq!(parsed["B"], "x=y");
        assert!(parse_environment(&["BROKEN".into()]).is_err());
    }

    #[test]
    fn windows_driver_uses_selector_event_loop_policy() {
        let script = build_script(
            "nb.ipynb",
            "report.json",
            60,
            "python3",
            false,
            60,
            4,
            true,
            &[],
            false,
        );
        assert!(script.contains("WindowsSelectorEventLoopPolicy"));
        assert!(
            script.find("WindowsSelectorEventLoopPolicy").unwrap()
                < script.find("import nbformat").unwrap()
        );
    }

    #[test]
    fn selected_python_kernel_interpreter_is_preferred_as_driver() {
        let interpreter = if cfg!(windows) {
            r"C:\Python311\python.exe"
        } else {
            "/opt/python311/bin/python"
        };
        let candidate = kernels::KernelCandidate {
            id: "kernelspec:python311".into(),
            display_name: "Python 3.11".into(),
            language: Some("python".into()),
            source: kernels::KernelSource::Registered,
            kernelspec_name: Some("python311".into()),
            interpreter: Some(interpreter.into()),
            argv: vec![],
            resource_dir: None,
            usable: true,
            reason: None,
            score: 0,
        };
        let selected = select_driver_python(None, &candidate, |path| path == interpreter).unwrap();
        assert_eq!(selected, interpreter);
    }
}
