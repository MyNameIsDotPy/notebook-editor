use crate::notebook::Notebook;
use crate::output_limit;
use crate::selection;
use anyhow::Result;
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
pub fn run(
    notebook: &str,
    selection: &str,
    type_filter: Option<&str>,
    show_outputs: bool,
    as_json: bool,
    lines_expr: Option<&str>,
    max_output_lines: Option<usize>,
) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;

    for idx in indices {
        let cell = &nb.cells[idx];
        let cell_num = idx + 1;

        if let Some(t) = type_filter {
            if cell.cell_type != t {
                continue;
            }
        }

        if as_json {
            let mut value = serde_json::to_value(cell)?;
            if let Some(outputs) = value.get("outputs").and_then(Value::as_array).cloned() {
                value["outputs"] =
                    Value::Array(output_limit::limit_outputs(&outputs, max_output_lines));
            }
            println!("{}", serde_json::to_string_pretty(&value)?);
            continue;
        }

        let source = cell.source_str();
        let all_lines: Vec<&str> = source.lines().collect();

        println!("[Cell {cell_num} | {}]", cell.cell_type);

        if let Some(expr) = lines_expr {
            // Print only the requested lines (1-based selection over the cell's lines)
            if all_lines.is_empty() {
                println!("<empty cell>");
            } else {
                let line_indices = selection::resolve(expr, all_lines.len())?;
                for li in line_indices {
                    println!("{:>4}  {}", li + 1, all_lines[li]);
                }
            }
        } else {
            print!("{source}");
            if !source.ends_with('\n') {
                println!();
            }

            if show_outputs && cell.cell_type == "code" && !cell.outputs.is_empty() {
                println!("--- outputs ---");
                let outputs = output_limit::limit_outputs(&cell.outputs, max_output_lines);
                for output in &outputs {
                    print_output(output);
                }
            }
        }

        println!();
    }

    Ok(())
}

fn print_output(output: &serde_json::Value) {
    let output_type = output
        .get("output_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    match output_type {
        "stream" => {
            let text = output
                .get("text")
                .map(multiline_value_to_string)
                .unwrap_or_default();
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
        "display_data" | "execute_result" => {
            if let Some(data) = output.get("data") {
                if let Some(text) = data.get("text/plain") {
                    let s = multiline_value_to_string(text);
                    print!("{s}");
                    if !s.ends_with('\n') {
                        println!();
                    }
                }
            }
        }
        "error" => {
            let ename = output.get("ename").and_then(|v| v.as_str()).unwrap_or("");
            let evalue = output.get("evalue").and_then(|v| v.as_str()).unwrap_or("");
            println!("{ename}: {evalue}");
            if let Some(traceback) = output.get("traceback") {
                let s = multiline_value_to_string(traceback);
                if !s.is_empty() {
                    print!("{s}");
                    if !s.ends_with('\n') {
                        println!();
                    }
                }
            }
        }
        _ => {
            println!("[output type: {output_type}]");
        }
    }
}

fn multiline_value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}
