//! The tools an agent sees (`tools/list`) and how a call becomes a result.

use std::sync::Arc;

use serde_json::{json, Value};

use nzap_core::{Error, ErrorCode};

use crate::protocol::{Peer, RpcError, INVALID_PARAMS};
use crate::server::{hardware_choices, Reply, Server};

pub const INSTRUCTIONS: &str = "\
NZAP Engine runs work on the user's own Google Colab VMs (CPU, GPU or TPU), so heavy jobs \
need nothing installed locally: speech to text, text to speech, audio/video conversion with \
ffmpeg (preinstalled on Colab), model inference, data processing.

- One-off processing: run_job. It starts a fresh VM, uploads 'inputs', runs a Python script \
  (shell commands via subprocess), downloads 'artifacts' and releases the VM.
- Ready-made AI apps (text to speech, sentiment, …): list_apps, then run_app. The first run \
  sets the model up; reuse the same runtime for fast follow-ups.
- Interactive work: start_runtime, then run_code / upload_file / download_file, then \
  stop_runtime. Code runs in a Jupyter (IPython) kernel in /content, so '!cmd' shell lines \
  and '%pip install' work.

Runtimes use the user's Colab compute units: prefer cpu unless a GPU is needed, and stop \
runtimes when done (the server also releases them when the agent disconnects). Local paths \
must be inside the shared folders reported by status. If a tool reports that Google is not \
connected, ask the user to open the NZAP Engine app and click Connect Google.";

fn hardware_schema() -> Value {
    json!({
        "type": "string",
        "enum": hardware_choices(),
        "description": "cpu (default), a GPU (t4 is the free-tier one) or a TPU.",
    })
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        },
        "annotations": {
            "title": title,
            "readOnlyHint": read_only,
            "openWorldHint": true,
        },
    })
}

pub fn definitions() -> Vec<Value> {
    let runtime = json!({"type": "string", "description": "A runtime name from start_runtime or list_runtimes."});
    let timeout = json!({"type": "integer", "minimum": 1, "description": "Seconds before giving up (default 600)."});
    vec![
        tool(
            "status",
            "NZAP Engine status",
            "Whether Google is connected, the runtimes this agent holds and its limits, the \
             hardware names, and the local folders shared with NZAP Engine.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "list_runtimes",
            "List runtimes",
            "Runtimes this agent holds, plus other Colab VMs on the account (for example the \
             NZAP Engine app's) that attach_runtime can use.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "start_runtime",
            "Start a runtime",
            "Allocate a Colab VM and connect a Python kernel. Takes 10–60 s. Uses compute units \
             until stopped.",
            json!({
                "hardware": hardware_schema(),
                "high_mem": {"type": "boolean", "description": "High-RAM machine (some hardware is always High-RAM)."},
                "name": {"type": "string", "description": "Optional name (letters, digits, - and _)."},
                "keep_after_disconnect": {"type": "boolean", "description": "Keep it running after the agent disconnects (default false)."},
            }),
            &[],
            false,
        ),
        tool(
            "attach_runtime",
            "Attach a runtime",
            "Use a VM this agent did not start (from list_runtimes' otherRuntimesOnAccount). It \
             is not released when the agent disconnects.",
            json!({
                "endpoint": {"type": "string", "description": "The VM's endpoint from list_runtimes."},
                "name": {"type": "string", "description": "Optional name to use for it."},
            }),
            &["endpoint"],
            false,
        ),
        tool(
            "stop_runtime",
            "Stop a runtime",
            "Release a runtime this agent started (it stops using compute units), or detach \
             from an attached one.",
            json!({
                "runtime": runtime,
                "release": {"type": "boolean", "description": "Release the VM at Colab (default: true if this agent started it)."},
            }),
            &["runtime"],
            false,
        ),
        tool(
            "run_code",
            "Run code on a runtime",
            "Run Python in the runtime's IPython kernel (state persists between calls; the \
             working directory is /content; '!ffmpeg …' and '%pip install …' work). Returns \
             the output, results, errors and displayed images.",
            json!({
                "runtime": runtime,
                "code": {"type": "string", "description": "Python (IPython) source."},
                "timeout_seconds": timeout,
            }),
            &["runtime", "code"],
            false,
        ),
        tool(
            "list_files",
            "List files on a runtime",
            "List a folder on the runtime (default /content).",
            json!({
                "runtime": runtime,
                "path": {"type": "string", "description": "A folder on the VM (default /content)."},
            }),
            &["runtime"],
            true,
        ),
        tool(
            "upload_file",
            "Upload a file to a runtime",
            "Copy a local file (inside the shared folders) to the runtime.",
            json!({
                "runtime": runtime,
                "local_path": {"type": "string", "description": "Local file, absolute or relative to the project."},
                "remote_path": {"type": "string", "description": "Destination on the VM (default /content/<file name>; relative paths are under /content)."},
            }),
            &["runtime", "local_path"],
            false,
        ),
        tool(
            "download_file",
            "Download a file from a runtime",
            "Copy a file from the runtime to the local machine (default: the output folder).",
            json!({
                "runtime": runtime,
                "remote_path": {"type": "string", "description": "File on the VM (relative paths are under /content)."},
                "local_path": {"type": "string", "description": "Where to save it, inside the shared folders."},
            }),
            &["runtime", "remote_path"],
            false,
        ),
        tool(
            "run_job",
            "Run a one-off job",
            "Start a fresh VM, upload inputs, run a Python script with `python script.py ARGS` \
             semantics in /content, download the artifacts, and release the VM — all in one \
             call. Example: inputs=[{local_path: 'talk.mp4'}], script=\"import subprocess; \
             subprocess.run(['ffmpeg','-i','talk.mp4','-vn','talk.mp3'], check=True)\", \
             artifacts=['talk.mp3'].",
            json!({
                "script": {"type": "string", "description": "Python source to run."},
                "script_path": {"type": "string", "description": "Or a local .py file to run."},
                "filename": {"type": "string", "description": "Script file name (default script.py)."},
                "args": {"type": "array", "items": {"type": "string"}, "description": "sys.argv[1:]."},
                "env": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Environment variables."},
                "inputs": {
                    "type": "array",
                    "description": "Local files to upload first.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "local_path": {"type": "string"},
                            "remote_path": {"type": "string", "description": "Default: /content/<file name>."},
                        },
                        "required": ["local_path"],
                    },
                },
                "artifacts": {"type": "array", "items": {"type": "string"}, "description": "Paths or globs (relative to /content) to download afterwards."},
                "hardware": hardware_schema(),
                "high_mem": {"type": "boolean"},
                "timeout_seconds": {"type": "integer", "minimum": 1, "description": "Seconds the script may run (default 600)."},
                "output_dir": {"type": "string", "description": "Local folder for artifacts (default: <output folder>/jobs/<job>)."},
            }),
            &[],
            false,
        ),
        tool(
            "list_apps",
            "List AI apps",
            "Ready-made AI apps from the NZAP collection (text to speech, sentiment, …) with \
             their parameters, recommended hardware and typical timings.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "run_app",
            "Run an AI app",
            "Run an app from list_apps. Without 'runtime' it starts one with the app's \
             recommended hardware (or reuses the one it used before, where the model is still \
             loaded). Media results are saved to the output folder; text results are returned.",
            json!({
                "app": {"type": "string", "description": "App id or slug, e.g. public:kokoro-tts or kokoro-tts."},
                "params": {"type": "object", "description": "Parameter values by key (see list_apps)."},
                "files": {"type": "object", "additionalProperties": {"type": "string"}, "description": "File parameters: key → local path."},
                "runtime": runtime,
                "hardware": hardware_schema(),
                "high_mem": {"type": "boolean"},
                "output_dir": {"type": "string", "description": "Local folder for media results."},
            }),
            &["app"],
            false,
        ),
    ]
}

fn hint(error: &Error) -> &'static str {
    match error.code() {
        ErrorCode::NotConnected | ErrorCode::AuthExpired => {
            " Ask the user to open the NZAP Engine app and click Connect Google, then try again."
        }
        ErrorCode::TooManyRuntimes => {
            " Stop a runtime (stop_runtime, or in the NZAP Engine app) and retry."
        }
        ErrorCode::Quota => " Try hardware 'cpu' or 't4'.",
        _ => "",
    }
}

fn result(reply: Reply) -> Value {
    let mut content = vec![json!({"type": "text", "text": reply.text})];
    for (mime, data) in reply.images {
        content.push(json!({"type": "image", "mimeType": mime, "data": data}));
    }
    json!({ "content": content, "isError": reply.is_error })
}

pub async fn call(
    server: &Arc<Server>,
    name: &str,
    args: &Value,
    peer: &Peer,
) -> Result<Value, RpcError> {
    let args = if args.is_object() { args.clone() } else { json!({}) };
    // A Google connection made in the app after this server started.
    server.engine().auth.reload().await;
    let outcome = match name {
        "status" => server.status().await,
        "list_runtimes" => server.list_runtimes().await,
        "start_runtime" => server.start_runtime(&args).await,
        "attach_runtime" => server.attach_runtime(&args).await,
        "stop_runtime" => server.stop_runtime(&args).await,
        "run_code" => server.run_code(&args).await,
        "list_files" => server.list_files(&args).await,
        "upload_file" => server.upload_file(&args).await,
        "download_file" => server.download_file(&args).await,
        "run_job" => server.run_job(&args, peer).await,
        "list_apps" => server.list_apps().await,
        "run_app" => server.run_app(&args, peer).await,
        other => return Err(RpcError::new(INVALID_PARAMS, format!("Unknown tool: {other}"))),
    };
    Ok(match outcome {
        Ok(reply) => result(reply),
        Err(error) => {
            tracing::info!("Tool {name} failed: {error}");
            result(Reply {
                text: format!("{error}{}", hint(&error)),
                is_error: true,
                ..Reply::default()
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_and_a_unique_name() {
        let tools = definitions();
        let mut names: Vec<&str> =
            tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names.len(), 12);
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 12);
        for tool in &tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["description"].as_str().unwrap().len() > 20);
            for required in tool["inputSchema"]["required"].as_array().unwrap() {
                let key = required.as_str().unwrap();
                assert!(tool["inputSchema"]["properties"].get(key).is_some(), "{key}");
            }
        }
    }

    #[test]
    fn errors_point_at_the_fix() {
        let reply = Reply {
            text: format!("{}{}", Error::NotConnected, hint(&Error::NotConnected)),
            is_error: true,
            ..Reply::default()
        };
        let value = result(reply);
        assert_eq!(value["isError"], true);
        assert!(value["content"][0]["text"].as_str().unwrap().contains("Connect Google"));
    }
}
