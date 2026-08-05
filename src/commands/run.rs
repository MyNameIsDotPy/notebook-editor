use crate::commands::kernels;
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
use std::time::{Duration, Instant};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
struct ExecutionReport {
    status: String,
    kernel: String,
    source: String,
    executed_cells: Vec<usize>,
    failed_cell: Option<usize>,
    duration_ms: u128,
    outputs_saved: bool,
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
) -> Result<()> {
    if timeout < -1 {
        bail!("--timeout must be -1 or a non-negative number");
    }
    let mut nb = Notebook::from_file(notebook)?;
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
    let candidate = kernels::resolve(&nb, notebook_path, kernel, interpreter, driver_python)?;
    if dry_run {
        let report = serde_json::json!({
            "status": "ready", "kernel": candidate.execution_label(),
            "display_name": candidate.display_name, "source": candidate.source,
            "interpreter": candidate.interpreter, "cells": code_indices.iter().map(|i| i + 1).collect::<Vec<_>>()
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "Kernel: {} ({:?})",
                candidate.display_name, candidate.source
            );
            if let Some(path) = candidate.interpreter {
                println!("Interpreter: {}", path.display());
            }
            println!("Would execute {} code cell(s)", code_indices.len());
        }
        return Ok(());
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
        kernels::install_synthetic_spec(tmp_dir.path(), &candidate)?
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
    );
    let driver = find_driver_python(driver_python, &candidate)?;
    let kernel_cwd = cwd
        .map(Path::new)
        .unwrap_or_else(|| notebook_path.parent().unwrap_or_else(|| Path::new(".")));
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
    INTERRUPTED.store(false, Ordering::SeqCst);
    ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::SeqCst))
        .context("Cannot install Ctrl-C handler")?;
    let (status, wall_timed_out, interrupted) = wait_for_child(command, overall_timeout)
        .with_context(|| format!("Failed to launch '{driver}'"))?;

    let py_report: Value = std::fs::read(&report_path).ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_else(|| serde_json::json!({"status": if wall_timed_out {"overall_timeout"} else if interrupted {"interrupted"} else {"driver_failure"}}));
    let py_status = if interrupted {
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

    if execution_started {
        let executed: Value = serde_json::from_slice(&std::fs::read(&tmp_nb)?)?;
        let executed_cells = executed
            .get("cells")
            .and_then(Value::as_array)
            .context("Executed notebook missing cells array")?;
        if executed_cells.len() != execution_indices.len() {
            bail!("Cell count mismatch after execution");
        }
        for &nb_idx in &code_indices {
            let slot = execution_indices
                .iter()
                .position(|&i| i == nb_idx)
                .context("Selected cell missing from execution result")?;
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
        nb.save(notebook, backup)?;
        outputs_saved = true;
    }

    let failed_slot = py_report
        .get("failed_cell")
        .and_then(Value::as_u64)
        .map(|i| i as usize);
    let report = ExecutionReport {
        status: py_status.into(),
        kernel: candidate.execution_label().into(),
        source: format!("{:?}", candidate.source),
        executed_cells: code_indices.iter().map(|i| i + 1).collect(),
        failed_cell: failed_slot.and_then(|slot| execution_indices.get(slot).map(|i| i + 1)),
        duration_ms: started.elapsed().as_millis(),
        outputs_saved,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if outputs_saved && !quiet {
        eprintln!("Outputs written to {notebook}");
    }

    match py_status {
        "ok" | "ok_with_errors" => Ok(()),
        "missing_dependency" => Err(AppExit::new(
            2,
            py_report
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("nbclient/nbformat not installed"),
        )
        .into()),
        "overall_timeout" => Err(AppExit::new(
            124,
            "Overall execution timeout expired; the kernel process was terminated",
        )
        .into()),
        "interrupted" => {
            Err(AppExit::new(130, "Execution interrupted (available outputs saved)").into())
        }
        "cell_error" => Err(AppExit::new(
            1,
            "One or more cells raised an error (available outputs saved)",
        )
        .into()),
        "cell_timeout" => {
            Err(AppExit::new(1, "Cell execution timed out (available outputs saved)").into())
        }
        "kernel_error" => Err(AppExit::new(
            1,
            py_report
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Kernel execution failed"),
        )
        .into()),
        _ if !status.success() => Err(AppExit::new(
            1,
            py_report
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Execution driver failed"),
        )
        .into()),
        _ => Ok(()),
    }
}

fn wait_for_child(
    mut command: Command,
    timeout: Option<u64>,
) -> std::io::Result<(ExitStatus, bool, bool)> {
    let mut child = command.spawn()?;
    let started = Instant::now();
    let mut interrupted_at = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false, interrupted_at.is_some()));
        }
        if INTERRUPTED.load(Ordering::SeqCst) && interrupted_at.is_none() {
            // The terminal delivers Ctrl-C to the whole foreground process group,
            // including the Python driver. Give its finally blocks time to write.
            interrupted_at = Some(Instant::now());
        }
        if interrupted_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
            child.kill()?;
            return Ok((child.wait()?, false, true));
        }
        if timeout.is_some_and(|seconds| started.elapsed() >= Duration::from_secs(seconds)) {
            child.kill()?;
            return Ok((child.wait()?, true, false));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn parse_environment(values: &[String]) -> Result<HashMap<String, String>> {
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

fn prepend_search_path(first: &Path, existing: Option<std::ffi::OsString>) -> std::ffi::OsString {
    let mut paths = vec![first.to_path_buf()];
    if let Some(existing) = existing {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap_or_else(|_| first.as_os_str().to_owned())
}

fn find_driver_python(
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
    let lines = vec![
        "import json, sys, traceback".to_string(),
        "if sys.platform == 'win32':".to_string(),
        "    import asyncio".to_string(),
        "    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())".to_string(),
        format!("report_path = {report_repr}"),
        "result = {'status': 'driver_failure', 'execution_started': False}".to_string(),
        "nb = None".to_string(),
        "try:".to_string(),
        "    import nbformat, nbclient".to_string(),
        "except ImportError as e:".to_string(),
        "    result = {'status': 'missing_dependency', 'execution_started': False, 'message': str(e)}".to_string(),
        "else:".to_string(),
        "    try:".to_string(),
        format!("        nb = nbformat.read(open({path_repr}, encoding='utf-8'), as_version=4)"),
        format!("        client = nbclient.NotebookClient(nb, timeout={timeout}, kernel_name={kernel_repr}, allow_errors={allow_errors}, startup_timeout={startup_timeout}, iopub_timeout={iopub_timeout}, record_timing={record_timing})"),
        "        result['execution_started'] = True".to_string(),
        "        client.execute()".to_string(),
        "        failures = [(i, o) for i, c in enumerate(nb.cells) for o in c.get('outputs', []) if o.get('output_type') == 'error']".to_string(),
        "        result = {'status': 'ok_with_errors' if failures else 'ok', 'execution_started': True}".to_string(),
        "        if failures: result['failed_cell'] = failures[0][0]".to_string(),
        "    except BaseException as e:".to_string(),
        "        name = type(e).__name__".to_string(),
        "        status = 'cell_timeout' if 'Timeout' in name else ('cell_error' if name == 'CellExecutionError' else 'kernel_error')".to_string(),
        "        result = {'status': status, 'execution_started': result.get('execution_started', False), 'message': f'{name}: {e}'}".to_string(),
        "        for i, cell in enumerate(nb.cells if nb else []):".to_string(),
        "            if any(o.get('output_type') == 'error' for o in cell.get('outputs', [])): result['failed_cell'] = i; break".to_string(),
        "    finally:".to_string(),
        "        if nb is not None:".to_string(),
        format!("            nbformat.write(nb, open({path_repr}, 'w', encoding='utf-8'))"),
        "finally:".to_string(),
        "    with open(report_path, 'w', encoding='utf-8') as f: json.dump(result, f)".to_string(),
        "sys.exit(0 if result['status'] in ('ok', 'ok_with_errors') else (2 if result['status'] == 'missing_dependency' else 1))".to_string(),
    ];
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_script_has_finally_persistence() {
        let script = build_script("nb.ipynb", "report.json", 60, "python3", false, 60, 4, true);
        assert!(script.contains("finally:\n        if nb is not None:"));
        assert!(script.contains("json.dump(result, f)"));
    }

    #[test]
    fn no_limit_maps_to_python_none() {
        let script = build_script("nb.ipynb", "report.json", -1, "python3", false, 60, 4, true);
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
        let script = build_script("nb.ipynb", "report.json", 60, "python3", false, 60, 4, true);
        let output = Command::new(python)
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
    fn parses_environment_values() {
        let parsed = parse_environment(&["A=1".into(), "B=x=y".into()]).unwrap();
        assert_eq!(parsed["B"], "x=y");
        assert!(parse_environment(&["BROKEN".into()]).is_err());
    }

    #[test]
    fn windows_driver_uses_selector_event_loop_policy() {
        let script = build_script("nb.ipynb", "report.json", 60, "python3", false, 60, 4, true);
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
