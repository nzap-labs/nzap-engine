//! Per-runtime history log and export — port of `colab log`.
//!
//! Mirrors colab-studio's `history.py`, which follows
//! `google-colab-cli/src/colab_cli/history.py` (append-only JSONL per
//! session, same `event_type` vocabulary) and `converter.py` (export to
//! `.ipynb` / `.md` / `.txt` / `.jsonl`). Notebooks are plain nbformat 4.5.
//!
//! Event types: `session_created` / `session_terminated`, `execution`
//! (`code` + `outputs` + `status` + `execution_count`), `input_reply`,
//! `automation` / `automation_result`, `file_operation`, `colab_request`,
//! `drive_auth_needed` / `drive_auth_success`, `console_started`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};

/// Export formats and their media types.
pub const EXPORT_FORMATS: &[(&str, &str)] = &[
    ("ipynb", "application/x-ipynb+json"),
    ("md", "text/markdown; charset=utf-8"),
    ("txt", "text/plain; charset=utf-8"),
    ("jsonl", "application/x-ndjson"),
];

/// Output events (as streamed to the UI) that belong in a notebook cell.
const OUTPUT_TYPES: &[&str] = &["stream", "result", "display", "update_display", "error"];

/// A filesystem-safe file stem for a runtime name.
fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "session".to_owned()
    } else {
        cleaned
    }
}

pub struct HistoryLog {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl HistoryLog {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, lock: Mutex::new(()) }
    }

    fn path(&self, session: &str) -> PathBuf {
        self.dir.join(format!("{}.jsonl", safe_filename(session)))
    }

    /// Append one event. Logging never fails the operation it records: I/O
    /// errors are logged and swallowed.
    pub fn log(&self, session: &str, event_type: &str, data: Value) {
        if let Err(error) = self.try_log(session, event_type, data) {
            tracing::warn!("Could not write history for {session}: {error}");
        }
    }

    fn try_log(&self, session: &str, event_type: &str, data: Value) -> Result<()> {
        let mut event = Map::new();
        event.insert("timestamp".into(), json!(chrono::Utc::now().to_rfc3339()));
        event.insert("event_type".into(), json!(event_type));
        if let Value::Object(fields) = data {
            for (key, value) in fields {
                event.entry(key).or_insert(value);
            }
        }
        let mut line = serde_json::to_string(&Value::Object(event))?;
        line.push('\n');
        let _guard = self.lock.lock().map_err(|_| Error::internal("History lock poisoned."))?;
        fs::create_dir_all(&self.dir)?;
        let mut file = OpenOptions::new().create(true).append(true).open(self.path(session))?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Every event for a runtime (kept after the runtime is released).
    pub fn get(&self, session: &str) -> Vec<Value> {
        let Ok(text) = fs::read_to_string(self.path(session)) else {
            return Vec::new();
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    pub fn clear(&self, session: &str) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| Error::internal("History lock poisoned."))?;
        match fs::remove_file(self.path(session)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Output capture
// ---------------------------------------------------------------------------

fn kind(event: &Value) -> &str {
    event.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// Keep the output-bearing events of one execution, merging stream runs and
/// honouring `clear_output`.
pub fn collect_outputs<'a>(events: impl IntoIterator<Item = &'a Value>) -> Vec<Value> {
    let mut outputs: Vec<Value> = Vec::new();
    for event in events {
        let kind = kind(event);
        if kind == "clear_output" {
            outputs.clear();
            continue;
        }
        if !OUTPUT_TYPES.contains(&kind) {
            continue;
        }
        if kind == "stream" {
            if let Some(last) = outputs.last_mut() {
                if kind_is_same_stream(last, event) {
                    let merged = text_of(last.get("text")) + &text_of(event.get("text"));
                    last["text"] = json!(merged);
                    continue;
                }
            }
        }
        outputs.push(event.clone());
    }
    outputs
}

fn kind_is_same_stream(last: &Value, event: &Value) -> bool {
    kind(last) == "stream" && last.get("name") == event.get("name")
}

/// UI events → nbformat v4 output dicts (converter.py `_map_outputs`).
pub fn to_nbformat_outputs(outputs: &[Value]) -> Vec<Value> {
    outputs
        .iter()
        .filter_map(|out| match kind(out) {
            "stream" => Some(json!({
                "output_type": "stream",
                "name": out.get("name").and_then(Value::as_str).unwrap_or("stdout"),
                "text": text_of(out.get("text")),
            })),
            "result" => Some(json!({
                "output_type": "execute_result",
                "execution_count": out.get("execution_count").cloned().unwrap_or(Value::Null),
                "data": out.get("data").cloned().unwrap_or_else(|| json!({})),
                "metadata": {},
            })),
            "display" | "update_display" => Some(json!({
                "output_type": "display_data",
                "data": out.get("data").cloned().unwrap_or_else(|| json!({})),
                "metadata": {},
            })),
            "error" => Some(json!({
                "output_type": "error",
                "ename": out.get("ename").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Error"),
                "evalue": out.get("evalue").and_then(Value::as_str).unwrap_or_default(),
                "traceback": out.get("traceback").cloned().unwrap_or_else(|| json!([])),
            })),
            _ => None,
        })
        .collect()
}

fn plain_text(out: &Value) -> Option<String> {
    match kind(out) {
        "stream" => Some(text_of(out.get("text"))),
        "error" => {
            let traceback: Vec<&str> = out
                .get("traceback")
                .and_then(Value::as_array)
                .map(|lines| lines.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if traceback.is_empty() {
                Some(format!(
                    "{}: {}",
                    out.get("ename").and_then(Value::as_str).unwrap_or_default(),
                    out.get("evalue").and_then(Value::as_str).unwrap_or_default()
                ))
            } else {
                Some(traceback.join("\n"))
            }
        }
        _ => {
            let text = text_of(out.get("data").and_then(|data| data.get("text/plain")));
            (!text.is_empty()).then_some(text)
        }
    }
}

// ---------------------------------------------------------------------------
// Export (converter.py)
// ---------------------------------------------------------------------------

/// `2026-01-01 12:00:00` from an RFC 3339 timestamp.
fn ts(event: &Value) -> String {
    let raw = event.get("timestamp").and_then(Value::as_str).unwrap_or_default();
    raw.split('.')
        .next()
        .unwrap_or_default()
        .split('+')
        .next()
        .unwrap_or_default()
        .replace('T', " ")
}

fn field<'a>(event: &'a Value, key: &str) -> &'a str {
    event.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn cell_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_owned()
}

fn markdown_cell(text: String) -> Value {
    json!({ "cell_type": "markdown", "id": cell_id(), "metadata": {}, "source": text })
}

fn code_cell(code: &str, outputs: Vec<Value>, count: Value) -> Value {
    json!({
        "cell_type": "code",
        "id": cell_id(),
        "metadata": {},
        "execution_count": count,
        "source": code,
        "outputs": outputs,
    })
}

fn outputs_of(event: &Value) -> Vec<Value> {
    event.get("outputs").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `convert_history_to_ipynb`: one code cell per execution.
pub fn to_notebook(events: &[Value], session: &str) -> Value {
    let mut cells = vec![markdown_cell(format!(
        "# Colab Session: {session}\nGenerated from the NZAP Engine history log."
    ))];
    for event in events {
        let when = ts(event);
        match field(event, "event_type") {
            "session_created" => cells.push(markdown_cell(format!(
                "**Session Created**: {when}\n- Endpoint: `{}`\n- Hardware: `{}`",
                field(event, "endpoint"),
                field(event, "accelerator")
            ))),
            "execution" => cells.push(code_cell(
                field(event, "code"),
                to_nbformat_outputs(&outputs_of(event)),
                event.get("execution_count").cloned().unwrap_or(Value::Null),
            )),
            "automation" => {
                cells.push(markdown_cell(format!(
                    "### Automation: {} ({when})",
                    field(event, "op")
                )));
                let code = field(event, "code");
                if !code.is_empty() {
                    cells.push(code_cell(code, Vec::new(), Value::Null));
                }
            }
            "automation_result" => {
                let outputs = outputs_of(event);
                if outputs.is_empty() {
                    continue;
                }
                let mapped = to_nbformat_outputs(&outputs);
                // Attach to the automation cell emitted just before, when present.
                let attach = cells.last().is_some_and(|cell| {
                    cell["cell_type"] == "code"
                        && cell["outputs"].as_array().is_some_and(|outputs| outputs.is_empty())
                });
                if attach {
                    if let Some(cell) = cells.last_mut() {
                        cell["outputs"] = json!(mapped);
                    }
                } else {
                    cells.push(code_cell("# Result of previous automation", mapped, Value::Null));
                }
            }
            "file_operation" => cells.push(markdown_cell(format!(
                "*File Operation*: `{}` on `{}`",
                field(event, "op"),
                field(event, "path")
            ))),
            "input_reply" => cells.push(markdown_cell(format!(
                "> **User Input**: `{}`",
                display_value(event.get("value"))
            ))),
            "session_terminated" => cells.push(markdown_cell(format!(
                "**Session Terminated**: {when} ({})",
                field(event, "reason")
            ))),
            _ => {}
        }
    }
    json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3 (Google Colab)",
                "language": "python",
                "name": "python3",
            },
            "language_info": { "name": "python" },
        },
        "cells": cells,
    })
}

fn display_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

pub fn to_markdown(events: &[Value], session: &str) -> String {
    let mut lines = vec![format!("# Colab Session: {session}\n")];
    for event in events {
        let when = ts(event);
        match field(event, "event_type") {
            "execution" => {
                lines.push(format!(
                    "### Execution ({when})\n```python\n{}\n```\n",
                    field(event, "code")
                ));
                for out in outputs_of(event) {
                    if let Some(text) = plain_text(&out).filter(|text| !text.is_empty()) {
                        lines.push(format!("**Output**:\n```\n{}\n```\n", text.trim_end()));
                    }
                }
            }
            "session_created" => lines.push(format!(
                "## Session Created: {when}\n- Endpoint: `{}`\n",
                field(event, "endpoint")
            )),
            "session_terminated" => lines.push(format!("## Session Terminated: {when}\n")),
            "automation" => {
                lines.push(format!("### Automation: {} ({when})\n", field(event, "op")))
            }
            "file_operation" => lines.push(format!(
                "*File Operation*: `{}` on `{}`\n",
                field(event, "op"),
                field(event, "path")
            )),
            _ => {}
        }
    }
    lines.join("\n")
}

pub fn to_text(events: &[Value], session: &str) -> String {
    let mut lines = vec![format!("Colab Session: {session}"), "=".repeat(20), String::new()];
    for event in events {
        let event_type = event.get("event_type").and_then(Value::as_str).unwrap_or("unknown");
        let prefix = format!("[{}] {}: ", ts(event), event_type.to_uppercase());
        if event_type == "execution" {
            lines.push(format!("{prefix}{}", field(event, "code").trim()));
        } else {
            let detail: Map<String, Value> = event
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .filter(|(key, _)| {
                            !matches!(key.as_str(), "timestamp" | "event_type" | "outputs")
                        })
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default();
            lines.push(format!("{prefix}{}", Value::Object(detail)));
        }
    }
    lines.join("\n") + "\n"
}

/// Render events in one of [`EXPORT_FORMATS`]. Returns `(body, media type)`.
pub fn export(events: &[Value], session: &str, format: &str) -> Result<(String, &'static str)> {
    let format = format.trim().trim_start_matches('.').to_ascii_lowercase();
    let media = EXPORT_FORMATS
        .iter()
        .find(|(name, _)| *name == format)
        .map(|(_, media)| *media)
        .ok_or_else(|| Error::invalid("Export format must be ipynb, md, txt or jsonl."))?;
    let body = match format.as_str() {
        "ipynb" => {
            let notebook = to_notebook(events, session);
            String::from_utf8(crate::runtime::proxy::pretty_json(&notebook))
                .map_err(|error| Error::internal(error.to_string()))?
        }
        "md" => to_markdown(events, session),
        "txt" => to_text(events, session),
        _ => events.iter().map(|event| format!("{event}\n")).collect(),
    };
    Ok((body, media))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Value> {
        vec![
            json!({"timestamp": "2026-01-02T03:04:05.123+00:00", "event_type": "session_created",
                   "endpoint": "ep-1", "accelerator": "T4", "shape": "Standard", "how": "assigned"}),
            json!({"timestamp": "2026-01-02T03:05:00+00:00", "event_type": "execution",
                   "code": "print('hi')", "status": "ok", "execution_count": 1,
                   "outputs": [{"type": "stream", "name": "stdout", "text": "hi\n"}]}),
            json!({"timestamp": "2026-01-02T03:06:00+00:00", "event_type": "automation",
                   "op": "install", "code": "pip install x"}),
            json!({"timestamp": "2026-01-02T03:07:00+00:00", "event_type": "automation_result",
                   "op": "install", "status": "ok",
                   "outputs": [{"type": "stream", "name": "stdout", "text": "done\n"}]}),
            json!({"timestamp": "2026-01-02T03:08:00+00:00", "event_type": "file_operation",
                   "op": "write", "path": "a.py"}),
            json!({"timestamp": "2026-01-02T03:09:00+00:00", "event_type": "input_reply", "value": "Ada"}),
            json!({"timestamp": "2026-01-02T03:10:00+00:00", "event_type": "session_terminated",
                   "reason": "user_requested"}),
        ]
    }

    #[test]
    fn log_round_trip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let log = HistoryLog::new(dir.path().join("history"));
        assert!(log.get("box").is_empty());
        log.log("box", "execution", json!({"code": "1", "event_type": "spoofed"}));
        log.log("box", "input_reply", json!({"value": "x"}));
        let events = log.get("box");
        assert_eq!(events.len(), 2);
        // The event type cannot be overridden by data.
        assert_eq!(events[0]["event_type"], "execution");
        assert!(events[0]["timestamp"].as_str().unwrap().contains('T'));
        log.clear("box").unwrap();
        log.clear("box").unwrap();
        assert!(log.get("box").is_empty());
    }

    #[test]
    fn filenames_are_safe() {
        assert_eq!(safe_filename("my box/../x"), "my_box_.._x");
        assert_eq!(safe_filename(".."), "session");
        assert_eq!(safe_filename(""), "session");
        assert_eq!(safe_filename("ok-1.2_x"), "ok-1.2_x");
    }

    #[test]
    fn outputs_merge_streams_and_honour_clear() {
        let events = [
            json!({"type": "status", "state": "busy"}),
            json!({"type": "stream", "name": "stdout", "text": "a"}),
            json!({"type": "stream", "name": "stdout", "text": "b"}),
            json!({"type": "stream", "name": "stderr", "text": "c"}),
            json!({"type": "result", "data": {"text/plain": "1"}}),
        ];
        let outputs = collect_outputs(&events);
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0]["text"], "ab");
        let cleared = collect_outputs(&[
            events[1].clone(),
            json!({"type": "clear_output"}),
            events[3].clone(),
        ]);
        assert_eq!(cleared, vec![events[3].clone()]);
    }

    #[test]
    fn nbformat_mapping() {
        let mapped = to_nbformat_outputs(&[
            json!({"type": "stream", "text": ["x", "y"]}),
            json!({"type": "result", "execution_count": 3, "data": {"text/plain": "3"}}),
            json!({"type": "display", "data": {"image/png": "abc"}}),
            json!({"type": "error", "ename": "", "evalue": "boom"}),
            json!({"type": "status"}),
        ]);
        assert_eq!(mapped.len(), 4);
        assert_eq!(mapped[0], json!({"output_type": "stream", "name": "stdout", "text": "xy"}));
        assert_eq!(mapped[1]["output_type"], "execute_result");
        assert_eq!(mapped[2]["output_type"], "display_data");
        assert_eq!(mapped[3]["ename"], "Error");
        assert_eq!(mapped[3]["traceback"], json!([]));
    }

    #[test]
    fn notebook_export() {
        let notebook = to_notebook(&sample(), "box");
        assert_eq!(notebook["nbformat"], 4);
        let cells = notebook["cells"].as_array().unwrap();
        let kinds: Vec<&str> =
            cells.iter().map(|cell| cell["cell_type"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            vec![
                "markdown", "markdown", "code", "markdown", "code", "markdown", "markdown",
                "markdown"
            ]
        );
        assert_eq!(cells[2]["source"], "print('hi')");
        assert_eq!(cells[2]["outputs"][0]["text"], "hi\n");
        // The automation result attaches to the automation's code cell.
        assert_eq!(cells[4]["outputs"][0]["text"], "done\n");
        assert!(cells[1]["source"].as_str().unwrap().contains("2026-01-02 03:04:05"));
        assert!(cells[6]["source"].as_str().unwrap().contains("`Ada`"));
    }

    #[test]
    fn markdown_text_and_jsonl_exports() {
        let (markdown, media) = export(&sample(), "box", "md").unwrap();
        assert!(media.starts_with("text/markdown"));
        assert!(markdown.starts_with("# Colab Session: box\n"));
        assert!(markdown.contains("```python\nprint('hi')\n```"));
        assert!(markdown.contains("**Output**:\n```\nhi\n```"));

        let (text, _) = export(&sample(), "box", ".TXT").unwrap();
        assert!(text.contains("] EXECUTION: print('hi')"));
        assert!(text.contains("FILE_OPERATION: {\"op\":\"write\",\"path\":\"a.py\"}"));
        assert!(!text.contains("\"outputs\""));

        let (jsonl, _) = export(&sample(), "box", "jsonl").unwrap();
        assert_eq!(jsonl.lines().count(), 7);

        let (ipynb, media) = export(&sample(), "box", "ipynb").unwrap();
        assert_eq!(media, "application/x-ipynb+json");
        assert!(serde_json::from_str::<Value>(&ipynb).is_ok());

        assert!(export(&sample(), "box", "pdf").is_err());
    }
}
