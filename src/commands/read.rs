use anyhow::Result;
use crate::notebook::Notebook;
use crate::selection;

pub fn run(
    notebook: &str,
    selection: &str,
    type_filter: Option<&str>,
    show_outputs: bool,
    as_json: bool,
) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;
    let indices = selection::resolve(selection, nb.len())?;

    for idx in indices {
        let cell = &nb.cells[idx];
        let cell_num = idx + 1; // 1-based for display

        // Apply type filter
        if let Some(t) = type_filter {
            if cell.cell_type != t {
                continue;
            }
        }

        if as_json {
            println!("{}", serde_json::to_string_pretty(cell)?);
        } else {
            println!("[Cell {cell_num} | {}]", cell.cell_type);
            print!("{}", cell.source_str());
            // Ensure output ends with a newline
            if !cell.source_str().ends_with('\n') {
                println!();
            }

            if show_outputs && cell.cell_type == "code" && !cell.outputs.is_empty() {
                println!("--- outputs ---");
                for output in &cell.outputs {
                    print_output(output);
                }
            }
            println!();
        }
    }

    Ok(())
}

fn print_output(output: &serde_json::Value) {
    let output_type = output.get("output_type").and_then(|v| v.as_str()).unwrap_or("unknown");
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
