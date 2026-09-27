//! Run a local file on a runtime — port of `colab exec -f FILE [--env K=V]`
//! (google-colab-cli `commands/execution.py`, via colab-studio `runfile.py`).
//!
//! A `.py` file runs as one cell; a `.ipynb` runs code cell by code cell
//! (markdown skipped) with outputs written back into a copy of the notebook
//! (`<name>_output.ipynb`). The env prelude is prepended to every cell and the
//! kernel first `chdir`s to `/content`, like the CLI.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::automation::{last_status, tee};
use crate::error::{Error, Result};
use crate::history::{collect_outputs, to_nbformat_outputs};
use crate::python;
use crate::session::{Emit, SessionManager};

pub const CHDIR_CONTENT: &str =
    "import os; os.makedirs('/content', exist_ok=True); os.chdir('/content')";

/// `--env` entries: a list of `KEY=VALUE` strings or a `{KEY: VALUE}` map.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum EnvSpec {
    #[default]
    None,
    List(Vec<String>),
    Map(serde_json::Map<String, Value>),
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Ordered, validated `(key, value)` pairs.
pub fn parse_env(env: &EnvSpec) -> Result<Vec<(String, String)>> {
    let pairs: Vec<(String, String)> = match env {
        EnvSpec::None => Vec::new(),
        EnvSpec::List(items) => items
            .iter()
            .map(|item| {
                item.split_once('=')
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .ok_or_else(|| {
                        Error::invalid(format!("Invalid env value '{item}'. Expected KEY=VALUE."))
                    })
            })
            .collect::<Result<_>>()?,
        EnvSpec::Map(map) => map
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                (key.clone(), value)
            })
            .collect(),
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for (key, value) in pairs {
        if !valid_env_key(&key) {
            return Err(Error::invalid(format!("Invalid env key '{key}'.")));
        }
        match out.iter_mut().find(|(existing, _)| *existing == key) {
            Some(entry) => entry.1 = value,
            None => out.push((key, value)),
        }
    }
    Ok(out)
}

/// execution.py `_build_env_prelude`.
pub fn env_prelude(env: &[(String, String)]) -> String {
    if env.is_empty() {
        return String::new();
    }
    let mut lines = vec!["import os".to_owned()];
    lines.extend(env.iter().map(|(key, value)| {
        format!("os.environ[{}] = {}", python::literal(key), python::literal(value))
    }));
    lines.join("\n") + "\n"
}

fn source_of(cell: &Value) -> String {
    match cell.get("source") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// Parse and validate a notebook (from text or an already-parsed value).
pub fn load_notebook(content: &Value) -> Result<Value> {
    let notebook = match content {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .map_err(|error| Error::invalid(format!("Invalid notebook: {error}")))?,
        other => other.clone(),
    };
    if !notebook.get("cells").is_some_and(Value::is_array) {
        return Err(Error::invalid("That file is not a Jupyter notebook (no cells)."));
    }
    Ok(notebook)
}

pub fn output_filename(filename: &str) -> String {
    let stem = if filename.to_ascii_lowercase().ends_with(".ipynb") {
        &filename[..filename.len() - ".ipynb".len()]
    } else {
        filename
    };
    let stem = stem.rsplit(['/', '\\']).next().unwrap_or_default();
    format!("{}_output.ipynb", if stem.is_empty() { "notebook" } else { stem })
}

pub fn strip_shebang(text: &str) -> &str {
    if text.starts_with("#!") {
        text.find('\n').map(|index| &text[index + 1..]).unwrap_or("")
    } else {
        text
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFileRequest {
    pub filename: String,
    /// `.py` text, or a notebook (JSON text or object).
    pub content: Value,
    #[serde(default)]
    pub env: EnvSpec,
    #[serde(default, alias = "stop_on_error")]
    pub stop_on_error: bool,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// What will run: the notebook copy (for `.ipynb`) and the cells.
pub struct Prepared {
    pub notebook: Option<Value>,
    /// `(cell index in the notebook, source)`
    pub blocks: Vec<(usize, String)>,
    pub prelude: String,
}

/// Validate a request up front (so errors surface before streaming).
pub fn prepare(request: &RunFileRequest) -> Result<Prepared> {
    let prelude = env_prelude(&parse_env(&request.env)?);
    let (notebook, blocks) = if request.filename.to_ascii_lowercase().ends_with(".ipynb") {
        let notebook = load_notebook(&request.content)?;
        let blocks = notebook["cells"]
            .as_array()
            .map(|cells| {
                cells
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| {
                        cell.get("cell_type").and_then(Value::as_str) == Some("code")
                    })
                    .map(|(index, cell)| (index, source_of(cell)))
                    .filter(|(_, source)| !source.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        (Some(notebook), blocks)
    } else {
        let text = match &request.content {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        (None, vec![(0, strip_shebang(&text).to_owned())])
    };
    if blocks.iter().all(|(_, source)| source.trim().is_empty()) {
        return Err(Error::invalid("There is no code to run in that file."));
    }
    Ok(Prepared { notebook, blocks, prelude })
}

/// Stream a file run. Adds `{"type": "cell", index, total, state}` around
/// each cell and a final `{"type": "run_complete", ...}` carrying the
/// executed notebook for `.ipynb` inputs.
pub async fn run(
    manager: &SessionManager,
    name: &str,
    request: &RunFileRequest,
    emit: &Emit,
) -> Result<Value> {
    let prepared = prepare(request)?;
    manager.get(name)?;
    let timeout = Duration::from_secs(request.timeout_seconds.unwrap_or(3600));
    manager.history().log(
        name,
        "automation",
        json!({"op": "run-file", "filename": request.filename}),
    );

    let mut result_log = RunLog {
        manager: manager.clone(),
        name: name.to_owned(),
        filename: request.filename.clone(),
        status: "interrupted".to_owned(),
        failed: None,
    };

    // The CLI always starts from Colab's working directory.
    let silent: Emit = Arc::new(|_| {});
    manager.execute(name, CHDIR_CONTENT, timeout, false, &silent).await?;

    let mut notebook = prepared.notebook;
    let total = prepared.blocks.len();
    let mut failed = 0usize;
    for (position, (index, source)) in prepared.blocks.iter().enumerate() {
        emit(json!({"type": "cell", "index": position, "total": total, "state": "started"}));
        let (cell_emit, seen) = tee(emit);
        manager
            .execute(name, &format!("{}{source}", prepared.prelude), timeout, true, &cell_emit)
            .await?;
        let seen = seen.lock().map(|seen| seen.clone()).unwrap_or_default();
        let status = last_status(&seen);
        if let Some(cell) = notebook.as_mut().and_then(|notebook| notebook["cells"].get_mut(*index))
        {
            if cell.get("id").is_none() {
                cell["id"] = json!(uuid::Uuid::new_v4().simple().to_string()[..8].to_owned());
            }
            cell["outputs"] = json!(to_nbformat_outputs(&collect_outputs(&seen)));
            cell["execution_count"] = seen
                .iter()
                .rev()
                .find(|event| event["type"] == "execute_reply")
                .and_then(|event| event.get("execution_count").cloned())
                .unwrap_or(Value::Null);
        }
        emit(
            json!({"type": "cell", "index": position, "total": total, "state": "finished", "status": status}),
        );
        if status != "ok" {
            failed += 1;
            if request.stop_on_error {
                break;
            }
        }
    }

    let status = if failed == 0 { "ok" } else { "error" };
    let mut done = json!({"type": "run_complete", "status": status, "failed_cells": failed, "total_cells": total});
    if let Some(mut notebook) = notebook {
        if notebook.get("nbformat").is_none() {
            notebook["nbformat"] = json!(4);
        }
        if notebook.get("nbformat_minor").is_none() {
            notebook["nbformat_minor"] = json!(5);
        }
        done["filename"] = json!(output_filename(&request.filename));
        done["notebook"] = notebook;
    }
    result_log.status = status.to_owned();
    result_log.failed = Some(failed);
    emit(done.clone());
    Ok(done)
}

/// Logs the run's `automation_result`, even when cancelled.
struct RunLog {
    manager: SessionManager,
    name: String,
    filename: String,
    status: String,
    failed: Option<usize>,
}

impl Drop for RunLog {
    fn drop(&mut self) {
        self.manager.history().log(
            &self.name,
            "automation_result",
            json!({"op": "run-file", "filename": self.filename, "status": self.status, "failed_cells": self.failed}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_parsing() {
        let list = EnvSpec::List(vec!["A=1".into(), "B=x=y".into(), "A=2".into()]);
        let expected: Vec<(String, String)> =
            vec![("A".into(), "2".into()), ("B".into(), "x=y".into())];
        assert_eq!(parse_env(&list).unwrap(), expected);
        assert!(parse_env(&EnvSpec::List(vec!["NOEQUALS".into()])).is_err());
        assert!(parse_env(&EnvSpec::List(vec!["1BAD=x".into()])).is_err());
        assert!(parse_env(&EnvSpec::List(vec!["BAD-KEY=x".into()])).is_err());
        let map: EnvSpec = serde_json::from_value(json!({"K": "v", "N": 3})).unwrap();
        let expected: Vec<(String, String)> =
            vec![("K".into(), "v".into()), ("N".into(), "3".into())];
        assert_eq!(parse_env(&map).unwrap(), expected);
        assert!(parse_env(&EnvSpec::None).unwrap().is_empty());
    }

    #[test]
    fn prelude_quotes_values() {
        assert_eq!(env_prelude(&[]), "");
        assert_eq!(
            env_prelude(&[("TOKEN".into(), "a'b\"c".into())]),
            "import os\nos.environ[\"TOKEN\"] = \"a'b\\\"c\"\n"
        );
    }

    #[test]
    fn names_and_shebangs() {
        assert_eq!(output_filename("train.ipynb"), "train_output.ipynb");
        assert_eq!(output_filename("dir/train.IPYNB"), "train_output.ipynb");
        assert_eq!(output_filename(".ipynb"), "notebook_output.ipynb");
        assert_eq!(strip_shebang("#!/usr/bin/env python\nprint(1)"), "print(1)");
        assert_eq!(strip_shebang("#!only"), "");
        assert_eq!(strip_shebang("print(1)"), "print(1)");
    }

    fn request(filename: &str, content: Value) -> RunFileRequest {
        RunFileRequest {
            filename: filename.into(),
            content,
            env: EnvSpec::None,
            stop_on_error: false,
            timeout_seconds: None,
        }
    }

    #[test]
    fn preparation() {
        let notebook = json!({"cells": [
            {"cell_type": "markdown", "source": "# hi"},
            {"cell_type": "code", "source": ["print(1)\n", "print(2)"]},
            {"cell_type": "code", "source": "   "},
            {"cell_type": "code", "source": "print(3)"},
        ]});
        let prepared = prepare(&request("n.ipynb", json!(notebook.to_string()))).unwrap();
        let expected: Vec<(usize, String)> =
            vec![(1, "print(1)\nprint(2)".into()), (3, "print(3)".into())];
        assert_eq!(prepared.blocks, expected);
        assert!(prepared.notebook.is_some());

        let script = prepare(&request("s.py", json!("#!/bin/python\nprint(1)"))).unwrap();
        assert_eq!(script.blocks, vec![(0usize, String::from("print(1)"))]);
        assert!(script.notebook.is_none());

        assert!(prepare(&request("s.py", json!("  \n"))).is_err());
        assert!(prepare(&request("n.ipynb", json!("{\"no\": 1}"))).is_err());
        assert!(prepare(&request("n.ipynb", json!("not json"))).is_err());
        assert!(prepare(&request(
            "n.ipynb",
            json!({"cells": [{"cell_type": "markdown", "source": "x"}]})
        ))
        .is_err());
    }
}
