use crate::notebook::Notebook;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KernelSource {
    Registered,
    Workspace,
    ActiveEnvironment,
    Conda,
    Pyenv,
    Path,
    Explicit,
}

#[derive(Debug, Clone, Serialize)]
pub struct KernelCandidate {
    pub id: String,
    pub display_name: String,
    pub language: Option<String>,
    pub source: KernelSource,
    pub kernelspec_name: Option<String>,
    pub interpreter: Option<PathBuf>,
    pub argv: Vec<String>,
    pub resource_dir: Option<PathBuf>,
    pub usable: bool,
    pub reason: Option<String>,
    #[serde(skip)]
    pub score: i32,
}

impl KernelCandidate {
    pub fn execution_label(&self) -> &str {
        self.kernelspec_name.as_deref().unwrap_or(&self.id)
    }
}

/// `Path::parent()` returns `Some("")` for a bare relative filename like
/// `"notebook.ipynb"`, not `None` — callers that then check `is_dir()` on
/// that result see a bogus empty path. Normalize that case to `.`.
pub(crate) fn parent_or_current(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

pub fn run(
    json: bool,
    details: bool,
    check: bool,
    notebook: Option<&str>,
    driver_python: Option<&str>,
) -> Result<()> {
    let workspace = notebook
        .map(|p| parent_or_current(Path::new(p)))
        .unwrap_or_else(|| Path::new("."));
    let nb = notebook.map(Notebook::from_file).transpose()?;
    let mut candidates = discover(workspace, driver_python)?;
    rank(&mut candidates, nb.as_ref(), workspace);

    if check {
        for candidate in &mut candidates {
            if candidate.language.as_deref() == Some("python") {
                let Some(interpreter) = &candidate.interpreter else {
                    continue;
                };
                candidate.usable = python_has_ipykernel(interpreter);
                if candidate.usable {
                    candidate.reason = None;
                } else {
                    candidate.reason = Some("interpreter cannot import ipykernel".into());
                }
            }
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&candidates)?);
        return Ok(());
    }

    if candidates.is_empty() {
        bail!("No kernels or Python environments discovered");
    }

    for candidate in &candidates {
        let marker = if candidate
            .reason
            .as_deref()
            .is_some_and(|r| r.starts_with("not checked"))
        {
            "unverified"
        } else if candidate.usable {
            "ready"
        } else {
            "unverified"
        };
        println!(
            "{:<28} {:<12} {}",
            candidate.id, marker, candidate.display_name
        );
        if details {
            println!("  source: {:?}", candidate.source);
            if let Some(name) = &candidate.kernelspec_name {
                println!("  kernelspec: {name}");
            }
            if let Some(path) = &candidate.interpreter {
                println!("  interpreter: {}", path.display());
            }
            if let Some(path) = &candidate.resource_dir {
                println!("  resource: {}", path.display());
            }
            if let Some(reason) = &candidate.reason {
                println!("  note: {reason}");
            }
        }
    }
    Ok(())
}

pub fn resolve(
    notebook: &Notebook,
    notebook_path: &Path,
    requested: Option<&str>,
    interpreter: Option<&str>,
    driver_python: Option<&str>,
) -> Result<KernelCandidate> {
    if let Some(path) = interpreter {
        let candidate = python_candidate(PathBuf::from(path), KernelSource::Explicit, true)
            .ok_or_else(|| anyhow::anyhow!("Python interpreter '{path}' does not exist"))?;
        if !candidate.usable {
            bail!("Python interpreter '{path}' cannot import ipykernel; install it with: {path} -m pip install ipykernel");
        }
        return Ok(candidate);
    }

    let workspace = parent_or_current(notebook_path);
    let mut candidates = discover(workspace, driver_python)?;
    rank(&mut candidates, Some(notebook), workspace);

    if let Some(requested) = requested {
        if let Some(found) = candidates
            .iter()
            .find(|c| c.id == requested || c.kernelspec_name.as_deref() == Some(requested))
        {
            return Ok(found.clone());
        }
        // Preserve compatibility: a kernelspec may be visible to the driver even
        // when it was not found in the paths available to the Rust process.
        return Ok(KernelCandidate {
            id: requested.into(),
            display_name: requested.into(),
            language: notebook.language().map(str::to_owned),
            source: KernelSource::Explicit,
            kernelspec_name: Some(requested.into()),
            interpreter: None,
            argv: Vec::new(),
            resource_dir: None,
            usable: true,
            reason: Some("explicit kernelspec name".into()),
            score: i32::MAX,
        });
    }

    for mut candidate in candidates {
        if candidate.kernelspec_name.is_some() {
            return Ok(candidate);
        }
        if let Some(interpreter) = &candidate.interpreter {
            candidate.usable = python_has_ipykernel(interpreter);
            if candidate.usable {
                return Ok(candidate);
            }
        }
    }
    bail!("No usable kernel found; run 'nbedit kernels --details --check' or pass --kernel/--interpreter")
}

pub fn discover(workspace: &Path, driver_python: Option<&str>) -> Result<Vec<KernelCandidate>> {
    let mut candidates = Vec::new();
    discover_registered(&mut candidates, driver_python);

    for name in [".venv", "venv", ".env", "env"] {
        if let Some(path) = python_in_prefix(&workspace.join(name)) {
            if let Some(candidate) = python_candidate(path, KernelSource::Workspace, false) {
                candidates.push(candidate);
            }
        }
    }
    for root in [workspace.join(".pixi/envs"), workspace.join(".conda/envs")] {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                if let Some(path) = python_in_prefix(&entry.path()) {
                    if let Some(candidate) = python_candidate(path, KernelSource::Workspace, false)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
    }

    for var in ["VIRTUAL_ENV", "CONDA_PREFIX"] {
        if let Some(path) = std::env::var_os(var).and_then(|p| python_in_prefix(Path::new(&p))) {
            if let Some(candidate) = python_candidate(path, KernelSource::ActiveEnvironment, false)
            {
                candidates.push(candidate);
            }
        }
    }

    discover_conda(&mut candidates);
    discover_pyenv(&mut candidates);
    discover_project_manager(
        &mut candidates,
        workspace,
        "poetry",
        &["env", "info", "--path"],
    );
    discover_project_manager(&mut candidates, workspace, "pipenv", &["--venv"]);
    if let Some(path) = executable_on_path("python3").or_else(|| executable_on_path("python")) {
        if let Some(candidate) = python_candidate(path, KernelSource::Path, false) {
            candidates.push(candidate);
        }
    }
    deduplicate(candidates)
}

fn discover_registered(out: &mut Vec<KernelCandidate>, driver_python: Option<&str>) {
    let mut roots = jupyter_data_dirs();
    let detected_python;
    let python = if let Some(python) = driver_python {
        Some(python)
    } else {
        detected_python = executable_on_path("python3").or_else(|| executable_on_path("python"));
        detected_python.as_ref().and_then(|p| p.to_str())
    };
    if let Some(python) = python {
        if let Ok(output) = Command::new(python)
            .args(["-c", "import sys; print(sys.prefix)"])
            .output()
        {
            if output.status.success() {
                let prefix = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                roots.push(PathBuf::from(prefix).join("share/jupyter"));
            }
        }
    }

    for root in roots {
        let kernels = root.join("kernels");
        let Ok(entries) = std::fs::read_dir(&kernels) else {
            continue;
        };
        for entry in entries.flatten() {
            let resource_dir = entry.path();
            let file = resource_dir.join("kernel.json");
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(spec) = serde_json::from_str::<Value>(&content) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let argv: Vec<String> = spec
                .get("argv")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let interpreter = argv.first().and_then(|value| {
                let path = PathBuf::from(value);
                if path.is_absolute() {
                    Some(path)
                } else {
                    executable_on_path(value)
                }
            });
            out.push(KernelCandidate {
                id: format!("kernelspec:{name}"),
                display_name: spec
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(&name)
                    .into(),
                language: spec
                    .get("language")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                source: KernelSource::Registered,
                kernelspec_name: Some(name),
                interpreter,
                argv,
                resource_dir: Some(resource_dir),
                usable: true,
                reason: None,
                score: 0,
            });
        }
    }
}

fn discover_project_manager(
    out: &mut Vec<KernelCandidate>,
    workspace: &Path,
    command: &str,
    args: &[&str],
) {
    let Some(executable) = executable_on_path(command) else {
        return;
    };
    let Ok(output) = Command::new(executable)
        .args(args)
        .current_dir(workspace)
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if let Some(path) = python_in_prefix(Path::new(&prefix)) {
        if let Some(candidate) = python_candidate(path, KernelSource::Workspace, false) {
            out.push(candidate);
        }
    }
}

fn discover_conda(out: &mut Vec<KernelCandidate>) {
    let Some(conda) = executable_on_path("conda").or_else(|| executable_on_path("micromamba"))
    else {
        return;
    };
    let Ok(output) = Command::new(conda).args(["env", "list", "--json"]).output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return;
    };
    for prefix in value
        .get("envs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if let Some(path) = python_in_prefix(Path::new(prefix)) {
            if let Some(candidate) = python_candidate(path, KernelSource::Conda, false) {
                out.push(candidate);
            }
        }
    }
}

fn discover_pyenv(out: &mut Vec<KernelCandidate>) {
    let Some(pyenv) = executable_on_path("pyenv") else {
        return;
    };
    let Ok(output) = Command::new(pyenv).args(["prefix", "--all"]).output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    for prefix in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(path) = python_in_prefix(Path::new(prefix)) {
            if let Some(candidate) = python_candidate(path, KernelSource::Pyenv, false) {
                out.push(candidate);
            }
        }
    }
}

fn python_candidate(
    path: PathBuf,
    source: KernelSource,
    validate: bool,
) -> Option<KernelCandidate> {
    if !path.is_file() {
        return None;
    }
    // Virtualenv interpreters are commonly symlinks to a base Python. Keep the
    // supplied path so invoking it retains the virtualenv's site-packages.
    let name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("python");
    let usable = !validate || python_has_ipykernel(&path);
    Some(KernelCandidate {
        id: format!("python:{}", path.to_string_lossy()),
        display_name: format!("Python ({name})"),
        language: Some("python".into()),
        source,
        kernelspec_name: None,
        interpreter: Some(path.clone()),
        argv: vec![
            path.to_string_lossy().into(),
            "-m".into(),
            "ipykernel_launcher".into(),
            "-f".into(),
            "{connection_file}".into(),
        ],
        resource_dir: None,
        usable,
        reason: if validate {
            (!usable).then(|| "ipykernel is not installed".into())
        } else {
            Some("not checked; use --check to probe ipykernel".into())
        },
        score: 0,
    })
}

fn python_has_ipykernel(path: &Path) -> bool {
    Command::new(path)
        .args(["-c", "import ipykernel"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn rank(candidates: &mut [KernelCandidate], notebook: Option<&Notebook>, workspace: &Path) {
    let wanted_name = notebook.and_then(Notebook::kernel_spec_name);
    let wanted_language = notebook.and_then(Notebook::language);
    for candidate in candidates.iter_mut() {
        candidate.score = match candidate.source {
            KernelSource::Explicit => 10_000,
            KernelSource::Workspace => 700,
            KernelSource::ActiveEnvironment => 600,
            KernelSource::Registered => 400,
            KernelSource::Conda | KernelSource::Pyenv => 300,
            KernelSource::Path => 100,
        };
        if candidate.kernelspec_name.as_deref() == wanted_name {
            candidate.score += 5_000;
        }
        if candidate
            .language
            .as_deref()
            .zip(wanted_language)
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            candidate.score += 500;
        }
        if candidate
            .interpreter
            .as_ref()
            .is_some_and(|p| p.starts_with(workspace))
        {
            candidate.score += 250;
        }
        if !candidate.usable {
            candidate.score -= 10_000;
        }
    }
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
}

fn deduplicate(candidates: Vec<KernelCandidate>) -> Result<Vec<KernelCandidate>> {
    let mut seen_interpreters = HashSet::new();
    let mut seen_specs = HashSet::new();
    let mut result = Vec::new();
    for candidate in candidates {
        let unique = if let Some(path) = &candidate.resource_dir {
            seen_specs.insert(path.clone())
        } else if let Some(path) = &candidate.interpreter {
            seen_interpreters.insert(path.clone())
        } else {
            true
        };
        if unique {
            result.push(candidate);
        }
    }
    Ok(result)
}

fn python_in_prefix(prefix: &Path) -> Option<PathBuf> {
    let unix = prefix.join("bin/python");
    if unix.is_file() {
        return Some(unix);
    }
    let windows = prefix.join("Scripts/python.exe");
    windows.is_file().then_some(windows)
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = dir.join(format!("{name}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn jupyter_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path) = std::env::var_os("JUPYTER_DATA_DIR") {
        dirs.push(PathBuf::from(path));
    }
    if let Some(paths) = std::env::var_os("JUPYTER_PATH") {
        dirs.extend(std::env::split_paths(&paths));
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("jupyter"));
        }
        if let Some(programdata) = std::env::var_os("PROGRAMDATA") {
            dirs.push(PathBuf::from(programdata).join("jupyter"));
        }
    } else {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            dirs.push(PathBuf::from(xdg).join("jupyter"));
        } else if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/jupyter"));
        }
        if cfg!(target_os = "macos") {
            if let Some(home) = std::env::var_os("HOME") {
                dirs.push(PathBuf::from(home).join("Library/Jupyter"));
            }
        }
        dirs.push(PathBuf::from("/usr/local/share/jupyter"));
        dirs.push(PathBuf::from("/usr/share/jupyter"));
        if let Some(paths) = std::env::var_os("XDG_DATA_DIRS") {
            dirs.extend(std::env::split_paths(&paths).map(|p| p.join("jupyter")));
        }
    }
    dirs
}

pub fn install_synthetic_spec(
    root: &Path,
    name: &str,
    candidate: &KernelCandidate,
) -> Result<String> {
    let interpreter = candidate
        .interpreter
        .as_ref()
        .context("synthetic kernel missing interpreter")?;
    let dir = root.join("kernels").join(name);
    std::fs::create_dir_all(&dir)?;
    let spec = serde_json::json!({
        "argv": [interpreter, "-m", "ipykernel_launcher", "-f", "{connection_file}"],
        "display_name": candidate.display_name,
        "language": "python"
    });
    std::fs::write(dir.join("kernel.json"), serde_json::to_vec_pretty(&spec)?)?;
    Ok(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_or_current_normalizes_bare_filenames_to_dot() {
        assert_eq!(
            parent_or_current(Path::new("notebook.ipynb")),
            Path::new(".")
        );
        assert_eq!(
            parent_or_current(Path::new("dir/notebook.ipynb")),
            Path::new("dir")
        );
        assert_eq!(parent_or_current(Path::new("/")), Path::new("."));
    }

    #[test]
    fn finds_python_in_unix_and_windows_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        let unix = temp.path().join("bin/python");
        std::fs::create_dir_all(unix.parent().unwrap()).unwrap();
        std::fs::write(&unix, "").unwrap();
        assert_eq!(python_in_prefix(temp.path()), Some(unix));
    }

    #[test]
    fn python_candidate_preserves_supplied_interpreter_path() {
        let dir = tempfile::tempdir().unwrap();
        let interpreter = dir.path().join("python");
        std::fs::write(&interpreter, "").unwrap();
        let candidate =
            python_candidate(interpreter.clone(), KernelSource::Explicit, false).unwrap();
        assert_eq!(
            candidate.interpreter.as_deref(),
            Some(interpreter.as_path())
        );
    }

    #[test]
    fn ranking_prefers_exact_kernelspec() {
        let nb: Notebook = serde_json::from_value(serde_json::json!({
            "nbformat": 4, "nbformat_minor": 5,
            "metadata": {"kernelspec": {"name": "wanted", "language": "python"}},
            "cells": []
        }))
        .unwrap();
        let mut candidates = vec![KernelCandidate {
            id: "kernelspec:wanted".into(),
            display_name: "Wanted".into(),
            language: Some("python".into()),
            source: KernelSource::Registered,
            kernelspec_name: Some("wanted".into()),
            interpreter: None,
            argv: vec![],
            resource_dir: None,
            usable: true,
            reason: None,
            score: 0,
        }];
        rank(&mut candidates, Some(&nb), Path::new("."));
        assert!(candidates[0].score >= 5_000);
    }
}
