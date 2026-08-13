//! Truncates large cell output text so an agent reading it doesn't burn
//! through its context window on a runaway stream or a huge traceback.
//!
//! This only affects what gets *displayed* (via `nbedit read` or the MCP
//! tools) — the notebook file on disk always keeps the untruncated outputs.

use serde_json::{json, Value};

/// Default number of lines kept per output field before truncation kicks in.
pub const DEFAULT_MAX_LINES: usize = 100;

/// Binary payloads (images, PDFs, ...) larger than this are replaced by a
/// size marker instead of being inlined — raw bytes are useless as LLM
/// context and dwarf any text output several times over.
const BINARY_OMIT_THRESHOLD_BYTES: usize = 2048;

struct LineStats {
    total_lines: usize,
    total_bytes: usize,
    truncated: bool,
}

fn truncate_lines(text: &str, max_lines: usize) -> (String, LineStats) {
    let total_bytes = text.len();
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let total_lines = lines.len();
    if total_lines <= max_lines {
        return (
            text.to_string(),
            LineStats {
                total_lines,
                total_bytes,
                truncated: false,
            },
        );
    }
    let kept = lines[..max_lines].concat();
    (
        kept,
        LineStats {
            total_lines,
            total_bytes,
            truncated: true,
        },
    )
}

/// Join the nbformat "string or array of strings" convention into one string.
fn multiline_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => Some(items.iter().filter_map(|v| v.as_str()).collect()),
        _ => None,
    }
}

/// Truncate one text-bearing field of an output object in place, preserving
/// whether it was originally a string or an array of strings.
fn truncate_field(obj: &mut serde_json::Map<String, Value>, key: &str, max_lines: usize) {
    let Some(value) = obj.get(key) else {
        return;
    };
    let was_array = value.is_array();
    let Some(text) = multiline_to_string(value) else {
        return;
    };
    let (kept, stats) = truncate_lines(&text, max_lines);
    if !stats.truncated {
        return;
    }
    let notice = format!(
        "\n... [truncated: showing {max_lines} of {} lines ({} bytes total) — pass --full-output/full_output or raise --output-lines/output_lines to see more]\n",
        stats.total_lines, stats.total_bytes
    );
    let combined = format!("{kept}{notice}");
    let new_value = if was_array {
        Value::Array(vec![Value::String(combined)])
    } else {
        Value::String(combined)
    };
    obj.insert(key.to_string(), new_value);
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json"
}

/// Replace a large binary payload (image/*, application/pdf, ...) with a
/// size marker. Small payloads (tiny icons) are left inline since they cost
/// little and omitting them would just be noise.
fn omit_binary_field(data: &mut serde_json::Map<String, Value>, key: &str) {
    let Some(Value::String(encoded)) = data.get(key) else {
        return;
    };
    let bytes = encoded.len();
    if bytes <= BINARY_OMIT_THRESHOLD_BYTES {
        return;
    }
    data.insert(
        key.to_string(),
        json!({
            "nbedit_omitted": true,
            "mime": key,
            "bytes": bytes,
            "note": "binary output omitted; pass --full-output/full_output to include it",
        }),
    );
}

/// Truncate the large text fields of a single output (stream text, display
/// data, error tracebacks) to `max_lines` lines each, and replace large
/// binary payloads (images, PDFs, ...) with a size marker. Returns a
/// modified clone; other output types and fields are left untouched.
fn limit_output(output: &Value, max_lines: usize) -> Value {
    let mut output = output.clone();
    let Some(obj) = output.as_object_mut() else {
        return output;
    };
    let output_type = obj.get("output_type").and_then(Value::as_str).unwrap_or("");
    match output_type {
        "stream" => truncate_field(obj, "text", max_lines),
        "display_data" | "execute_result" => {
            if let Some(Value::Object(data)) = obj.get_mut("data") {
                truncate_field(data, "text/plain", max_lines);
                truncate_field(data, "text/html", max_lines);
                let mime_keys: Vec<String> = data.keys().cloned().collect();
                for mime in mime_keys {
                    if !is_text_mime(&mime) {
                        omit_binary_field(data, &mime);
                    }
                }
            }
        }
        "error" => truncate_field(obj, "traceback", max_lines),
        _ => {}
    }
    output
}

/// Truncate every output in `outputs`. `max_lines` of `None` disables
/// truncation and returns the outputs unchanged.
pub fn limit_outputs(outputs: &[Value], max_lines: Option<usize>) -> Vec<Value> {
    match max_lines {
        None => outputs.to_vec(),
        Some(max_lines) => outputs.iter().map(|o| limit_output(o, max_lines)).collect(),
    }
}

/// Whether any output in `outputs` is an error (an unhandled exception).
pub fn has_error(outputs: &[Value]) -> bool {
    outputs
        .iter()
        .any(|o| o.get("output_type").and_then(Value::as_str) == Some("error"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn short_stream_output_is_unchanged() {
        let outputs = vec![json!({"output_type": "stream", "name": "stdout", "text": "a\nb\n"})];
        let limited = limit_outputs(&outputs, Some(10));
        assert_eq!(limited, outputs);
    }

    #[test]
    fn long_stream_output_is_truncated_with_notice() {
        let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let outputs = vec![json!({"output_type": "stream", "name": "stdout", "text": text})];
        let limited = limit_outputs(&outputs, Some(5));
        let shown = limited[0]["text"].as_str().unwrap();
        assert!(shown.starts_with("line 0\nline 1\nline 2\nline 3\nline 4\n"));
        assert!(shown.contains("showing 5 of 500 lines"));
        assert!(!shown.contains("line 5\n"));
    }

    #[test]
    fn full_output_none_disables_truncation() {
        let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let outputs =
            vec![json!({"output_type": "stream", "name": "stdout", "text": text.clone()})];
        let limited = limit_outputs(&outputs, None);
        assert_eq!(limited[0]["text"].as_str().unwrap(), text);
    }

    #[test]
    fn error_traceback_array_is_truncated_and_stays_an_array() {
        let traceback: Vec<Value> = (0..50)
            .map(|i| Value::String(format!("frame {i}\n")))
            .collect();
        let outputs = vec![json!({
            "output_type": "error", "ename": "ValueError", "evalue": "bad",
            "traceback": traceback
        })];
        let limited = limit_outputs(&outputs, Some(3));
        let tb = limited[0]["traceback"].as_array().unwrap();
        assert_eq!(tb.len(), 1);
        assert!(tb[0].as_str().unwrap().contains("showing 3 of 50 lines"));
    }

    #[test]
    fn display_data_text_plain_is_truncated() {
        let text: String = (0..200).map(|i| format!("row {i}\n")).collect();
        let outputs = vec![
            json!({"output_type": "execute_result", "execution_count": 1, "data": {"text/plain": text}}),
        ];
        let limited = limit_outputs(&outputs, Some(20));
        let shown = limited[0]["data"]["text/plain"].as_str().unwrap();
        assert!(shown.contains("showing 20 of 200 lines"));
    }

    #[test]
    fn large_binary_payload_is_replaced_with_size_marker() {
        let image: String = "A".repeat(10_000);
        let outputs = vec![
            json!({"output_type": "display_data", "data": {"image/png": image, "text/plain": "<Figure>"}}),
        ];
        let limited = limit_outputs(&outputs, Some(100));
        assert_eq!(limited[0]["data"]["image/png"]["bytes"], 10_000);
        assert_eq!(limited[0]["data"]["image/png"]["nbedit_omitted"], true);
        assert_eq!(limited[0]["data"]["text/plain"], "<Figure>");
    }

    #[test]
    fn small_binary_payload_is_left_inline() {
        let icon = "A".repeat(100);
        let outputs = vec![json!({"output_type": "display_data", "data": {"image/png": icon}})];
        let limited = limit_outputs(&outputs, Some(100));
        assert_eq!(limited[0]["data"]["image/png"].as_str().unwrap().len(), 100);
    }

    #[test]
    fn full_output_none_skips_binary_omission_too() {
        let image: String = "A".repeat(10_000);
        let outputs =
            vec![json!({"output_type": "display_data", "data": {"image/png": image.clone()}})];
        let limited = limit_outputs(&outputs, None);
        assert_eq!(limited[0]["data"]["image/png"].as_str().unwrap(), image);
    }

    #[test]
    fn has_error_detects_error_outputs() {
        let clean = vec![json!({"output_type": "stream", "text": "ok\n"})];
        let failing =
            vec![json!({"output_type": "error", "ename": "E", "evalue": "x", "traceback": []})];
        assert!(!has_error(&clean));
        assert!(has_error(&failing));
    }
}
