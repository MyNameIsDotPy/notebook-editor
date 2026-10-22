//! Structural editing and validation shared by command frontends.
use crate::{notebook::Notebook, selection};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub fn python(request: Value, interpreter: Option<&str>) -> Result<Value> {
    let candidates: Vec<&str> = interpreter
        .map(|p| vec![p])
        .unwrap_or_else(|| vec!["python", "python3"]);
    for executable in candidates {
        let child = Command::new(executable)
            .args(["-I", "-c", include_str!("structural.py")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) if interpreter.is_none() && error.kind() == std::io::ErrorKind::NotFound => {
                continue
            }
            Err(error) => return Err(error).context("Cannot start Python; use --python"),
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(serde_json::to_string(&request)?.as_bytes())?;
        let output = child.wait_with_output()?;
        let response: Value = serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "Python helper failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
        if !output.status.success() {
            bail!("Python: {}", response["error"]);
        }
        return Ok(response["result"].clone());
    }
    bail!("Python 3 is required; use --python PATH")
}

fn cells(nb: &Notebook) -> Result<Vec<Value>> {
    if nb
        .language()
        .is_some_and(|l| !l.eq_ignore_ascii_case("python"))
    {
        bail!("Python operations require a Python notebook");
    }
    Ok(nb
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| c.cell_type == "code")
        .filter(|(_, c)| {
            c.metadata
                .pointer("/language")
                .and_then(Value::as_str)
                .is_none_or(|l| l.eq_ignore_ascii_case("python"))
        })
        .map(|(i, c)| json!({"cell":i+1,"source":c.source_str()}))
        .collect())
}

pub fn syntax_issues(nb: &Notebook, interpreter: Option<&str>) -> Result<Vec<Value>> {
    Ok(serde_json::from_value(python(
        json!({"mode":"syntax", "cells":cells(nb)?}),
        interpreter,
    )?)?)
}

pub fn check(nb: &Notebook, interpreter: Option<&str>) -> Result<()> {
    let structure = crate::commands::validate::issues(nb);
    let syntax = syntax_issues(nb, interpreter)?;
    if !structure.is_empty() || !syntax.is_empty() {
        bail!(
            "Validation failed: {} {}",
            structure.join("; "),
            serde_json::to_string(&syntax)?
        );
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Operation {
    Wrap {
        cell: usize,
        description: Option<String>,
        input: Option<String>,
        output: Option<String>,
        logger_info: Option<String>,
        logger_error: Option<String>,
    },
    Indent {
        cell: usize,
        lines: String,
        spaces: usize,
    },
    Replace {
        cell: usize,
        lines: Option<String>,
        source: String,
    },
    InsertBefore {
        cell: usize,
        lines: String,
        source: String,
    },
    InsertAfter {
        cell: usize,
        lines: String,
        source: String,
    },
    Delete {
        cell: usize,
        lines: String,
    },
}

pub fn apply(nb: &mut Notebook, op: &Operation, interpreter: Option<&str>) -> Result<()> {
    let value = serde_json::to_value(op)?;
    let index = value["cell"].as_u64().unwrap() as usize;
    if index == 0 || index > nb.len() {
        bail!("Cell {index} is out of range");
    }
    if matches!(op, Operation::Wrap { .. } | Operation::Indent { .. }) {
        cells(nb)?;
        if nb.cells[index - 1].cell_type != "code" {
            bail!("Cell {index} is not code");
        }
    }
    let current = nb.cells[index - 1].source_str();
    let lines: Vec<&str> = current.split_inclusive('\n').collect();
    let result = match op {
        Operation::Wrap { .. } => {
            let mut request = value.clone();
            request["mode"] = json!("wrap");
            request["source"] = json!(current);
            python(request, interpreter)?
                .as_str()
                .context("Invalid Python result")?
                .to_owned()
        }
        Operation::Indent {
            lines: expr,
            spaces,
            ..
        } => {
            let selected = selection::resolve_lines(expr, lines.len())?;
            python(
                json!({"mode":"indent", "source":current,"selected":selected,"spaces":spaces}),
                interpreter,
            )?
            .as_str()
            .context("Invalid Python result")?
            .to_owned()
        }
        Operation::Replace {
            lines: None,
            source,
            ..
        } => source.clone(),
        Operation::Delete { lines: expr, .. } => {
            let selected = selection::resolve_lines(expr, lines.len())?;
            lines
                .iter()
                .enumerate()
                .filter(|(i, _)| !selected.contains(i))
                .map(|(_, line)| *line)
                .collect()
        }
        _ => {
            let expr = value["lines"].as_str().context("Missing lines")?;
            let selected = selection::resolve_lines(expr, lines.len())?;
            if matches!(
                op,
                Operation::InsertBefore { .. } | Operation::InsertAfter { .. }
            ) && selected.windows(2).any(|w| w[1] != w[0] + 1)
            {
                bail!("Expected a contiguous line range");
            }
            let first = selected[0];
            let last = selected[selected.len() - 1];
            let (start, end) = match op {
                Operation::InsertBefore { .. } => (first, first),
                Operation::InsertAfter { .. } => (last + 1, last + 1),
                _ => (first, last + 1),
            };
            let mut source = value["source"].as_str().unwrap_or("").to_owned();
            let mut prefix = lines[..start].concat();
            let suffix = lines[end..].concat();
            if !source.is_empty() && !prefix.is_empty() && !prefix.ends_with('\n') {
                prefix.push('\n');
            }
            if !source.is_empty()
                && (!suffix.is_empty() || current.ends_with('\n'))
                && !source.ends_with('\n')
            {
                source.push('\n');
            }
            format!("{prefix}{source}{suffix}")
        }
    };
    nb.cells[index - 1].set_source(result);
    Ok(())
}

pub fn transaction(
    path: &str,
    operations: &[Operation],
    backup: bool,
    dry_run: bool,
    interpreter: Option<&str>,
) -> Result<()> {
    let mut nb = Notebook::from_file(path)?;
    for op in operations {
        apply(&mut nb, op, interpreter)?;
    }
    check(&nb, interpreter)?;
    if dry_run {
        println!("{}", serde_json::to_string_pretty(&nb)?);
    } else {
        nb.save(path, backup)?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub required: Vec<String>,
    #[serde(default)]
    pub forbidden_patterns: Vec<String>,
}

pub fn lint(path: &str, rules_path: &str, interpreter: Option<&str>) -> Result<Vec<Value>> {
    let rules: Rules = serde_yaml::from_str(&std::fs::read_to_string(rules_path)?)?;
    for rule in &rules.required {
        if ![
            "description",
            "input",
            "output",
            "try",
            "logger.info",
            "except",
            "logger.error",
            "raise",
        ]
        .contains(&rule.as_str())
        {
            bail!("Unknown required rule: {rule}");
        }
    }
    let nb = Notebook::from_file(path)?;
    let cells = cells(&nb)?;
    let mut issues: Vec<Value> = serde_json::from_value(python(
        json!({"mode":"lint","cells":cells,"required":rules.required}),
        interpreter,
    )?)?;
    for pattern in rules.forbidden_patterns {
        let regex = regex::Regex::new(&pattern)?;
        for cell in &cells {
            for (line, text) in cell["source"].as_str().unwrap().lines().enumerate() {
                if regex.is_match(text) {
                    issues.push(json!({"cell":cell["cell"],"line":line+1,"message":format!("Forbidden pattern: {pattern}")}));
                }
            }
        }
    }
    Ok(issues)
}
