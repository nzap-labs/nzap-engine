//! A mock Colab Python kernel on `/proxy/{endpoint}/api/kernels/{id}/channels`.
//!
//! Speaks the default Jupyter sub-protocol (JSON frames with a `channel`
//! key) and "runs" code with a tiny, deterministic interpreter:
//!
//! | code contains                         | behaviour                                   |
//! |---------------------------------------|---------------------------------------------|
//! | `print("…")` / `print(params["k"])`   | a stdout stream per call                     |
//! | `params = {…}` (JSON)                 | defines `params` for later `print`s          |
//! | `sys.exit(N)`                         | `SystemExit` error with value `N`            |
//! | `raise` / `fail`                      | `ValueError: boom`                           |
//! | `input(`                              | `input_request`, resumes on `input_reply`    |
//! | `drive.mount(` / `authenticate_user(` | `colab_request` (Drive / Cloud), resumes on its `colab_reply` |
//! | `wait_for_interrupt`                  | blocks until the kernel is interrupted       |
//! | `slow_output`                         | two streams 100 ms apart                     |
//! | `display_image` / `clear_output` / `answer` | display data / clear / `execute_result` 42 |
//! | `uv', 'pip', 'install'`               | the install automation's success line        |
//! | `__NZAP_ARTIFACTS__`                  | lists files under `content/out` as the artifact marker |

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};

use crate::runtime::check_runtime_token;
use crate::state::MockFile;
use crate::Shared;

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/proxy/{endpoint}/api/kernels/{kernel}/channels", get(channels))
}

async fn channels(
    State(state): State<Shared>,
    Path((endpoint, kernel)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    {
        let mock = state.lock().expect("mock state");
        if let Err(response) = check_runtime_token(&mock, &endpoint, &headers, &query) {
            return response;
        }
        let known = mock.runtime(&endpoint).is_some_and(|runtime| runtime.kernels.contains(&kernel));
        if !known {
            return (StatusCode::NOT_FOUND, "no such kernel").into_response();
        }
    }
    upgrade.on_upgrade(move |socket| Kernel::new(state, endpoint).serve(socket))
}

enum Paused {
    Input { parent: Value, count: u64 },
    Colab { parent: Value, count: u64, auth_type: String },
}

struct Kernel {
    state: Shared,
    endpoint: String,
    count: u64,
    paused: Option<Paused>,
    params: Value,
}

fn message(parent: &Value, channel: &str, msg_type: &str, content: Value, metadata: Value) -> Message {
    let frame = json!({
        "header": {
            "msg_id": format!("mock-{msg_type}-{}", uniq()),
            "username": "kernel",
            "session": "mock-session",
            "msg_type": msg_type,
            "version": "5.3",
        },
        "parent_header": parent,
        "metadata": metadata,
        "content": content,
        "channel": channel,
        "buffers": [],
    });
    Message::Text(frame.to_string().into())
}

fn uniq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The argument of `print(...)` on one line, evaluated.
fn print_arg(line: &str, params: &Value) -> Option<String> {
    let inner = line.trim().strip_prefix("print(")?.strip_suffix(')')?.trim();
    for quote in ['"', '\''] {
        if inner.len() >= 2 && inner.starts_with(quote) && inner.ends_with(quote) {
            return Some(inner[1..inner.len() - 1].to_owned());
        }
    }
    let key = inner.strip_prefix("params[")?.strip_suffix(']')?.trim_matches(|c| c == '"' || c == '\'');
    Some(match params.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => format!("KeyError: '{key}'"),
    })
}

impl Kernel {
    fn new(state: Shared, endpoint: String) -> Self {
        Self { state, endpoint, count: 0, paused: None, params: json!({}) }
    }

    async fn serve(mut self, mut socket: WebSocket) {
        while let Some(Ok(frame)) = socket.recv().await {
            let text = match frame {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Message::Close(_) => break,
                _ => continue,
            };
            let Ok(incoming) = serde_json::from_str::<Value>(&text) else { continue };
            if self.handle(&mut socket, incoming).await.is_err() {
                break;
            }
        }
    }

    async fn send(&self, socket: &mut WebSocket, parent: &Value, channel: &str, msg_type: &str, content: Value) -> Result<(), axum::Error> {
        socket.send(message(parent, channel, msg_type, content, json!({}))).await
    }

    async fn finish(&self, socket: &mut WebSocket, parent: &Value, status: &str, count: u64) -> Result<(), axum::Error> {
        self.send(socket, parent, "shell", "execute_reply", json!({"status": status, "execution_count": count})).await?;
        self.send(socket, parent, "iopub", "status", json!({"execution_state": "idle"})).await
    }

    async fn handle(&mut self, socket: &mut WebSocket, incoming: Value) -> Result<(), axum::Error> {
        let parent = incoming.get("header").cloned().unwrap_or_else(|| json!({}));
        let msg_type = parent.get("msg_type").and_then(Value::as_str).unwrap_or_default().to_owned();
        let content = incoming.get("content").cloned().unwrap_or_else(|| json!({}));

        match msg_type.as_str() {
            "kernel_info_request" => {
                self.send(socket, &parent, "iopub", "status", json!({"execution_state": "busy"})).await?;
                self.send(
                    socket,
                    &parent,
                    "shell",
                    "kernel_info_reply",
                    json!({
                        "status": "ok",
                        "protocol_version": "5.3",
                        "language_info": {"name": "python", "version": "3.12.3"},
                        "banner": "mock colab kernel",
                    }),
                )
                .await?;
                self.send(socket, &parent, "iopub", "status", json!({"execution_state": "idle"})).await
            }
            "execute_request" => {
                let code = content.get("code").and_then(Value::as_str).unwrap_or_default().to_owned();
                self.execute(socket, parent, code).await
            }
            "input_reply" => {
                let value = content.get("value").cloned().unwrap_or(Value::Null);
                match self.paused.take() {
                    Some(Paused::Input { parent, count }) => {
                        let name = value.as_str().unwrap_or_default().to_owned();
                        self.send(socket, &parent, "iopub", "stream", json!({"name": "stdout", "text": format!("Hello, {name}!\n")}))
                            .await?;
                        self.finish(socket, &parent, "ok", count).await
                    }
                    Some(Paused::Colab { parent, count, auth_type })
                        if value.get("type").and_then(Value::as_str) == Some("colab_reply") =>
                    {
                        let text = if auth_type == "dfs_ephemeral" {
                            "Mounted at /content/drive\n"
                        } else {
                            "Authenticated with Google Cloud.\n"
                        };
                        self.send(socket, &parent, "iopub", "stream", json!({"name": "stdout", "text": text})).await?;
                        self.finish(socket, &parent, "ok", count).await
                    }
                    other => {
                        self.paused = other;
                        Ok(())
                    }
                }
            }
            _ => Ok(()),
        }
    }

    async fn stream(&self, socket: &mut WebSocket, parent: &Value, text: String) -> Result<(), axum::Error> {
        self.send(socket, parent, "iopub", "stream", json!({"name": "stdout", "text": text})).await
    }

    async fn error(&self, socket: &mut WebSocket, parent: &Value, ename: &str, evalue: &str) -> Result<(), axum::Error> {
        self.send(
            socket,
            parent,
            "iopub",
            "error",
            json!({"ename": ename, "evalue": evalue, "traceback": [format!("{ename}: {evalue}")]}),
        )
        .await
    }

    async fn execute(&mut self, socket: &mut WebSocket, parent: Value, code: String) -> Result<(), axum::Error> {
        self.count += 1;
        let count = self.count;
        let interrupts_before = {
            let mut mock = self.state.lock().expect("mock state");
            let runtime = mock.runtime_mut(&self.endpoint);
            runtime.map(|runtime| {
                runtime.executed.push(code.clone());
                runtime.interrupts
            })
        }
        .unwrap_or_default();

        self.send(socket, &parent, "iopub", "status", json!({"execution_state": "busy"})).await?;
        self.send(socket, &parent, "iopub", "execute_input", json!({"code": code, "execution_count": count}))
            .await?;

        if code.contains("drive.mount(") || code.contains("authenticate_user(") {
            let auth_type =
                if code.contains("drive.mount(") { "dfs_ephemeral" } else { "auth_user_ephemeral" };
            socket
                .send(message(
                    &parent,
                    "stdin",
                    "colab_request",
                    json!({"request": {"authType": auth_type}}),
                    json!({"colab_msg_id": 7}),
                ))
                .await?;
            self.paused = Some(Paused::Colab { parent, count, auth_type: auth_type.to_owned() });
            return Ok(());
        }
        if code.contains("input(") {
            self.send(socket, &parent, "stdin", "input_request", json!({"prompt": "Name? ", "password": false}))
                .await?;
            self.paused = Some(Paused::Input { parent, count });
            return Ok(());
        }
        if code.contains("wait_for_interrupt") {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let interrupted = {
                    let mock = self.state.lock().expect("mock state");
                    mock.runtime(&self.endpoint).map(|runtime| runtime.interrupts).unwrap_or_default()
                        > interrupts_before
                };
                if interrupted || Instant::now() > deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            self.error(socket, &parent, "KeyboardInterrupt", "").await?;
            return self.finish(socket, &parent, "error", count).await;
        }

        let mut status = "ok";
        for line in code.lines() {
            let trimmed = line.trim();
            if let Some(json_text) = trimmed.strip_prefix("params = ") {
                self.params = serde_json::from_str(json_text).unwrap_or_else(|_| json!({}));
            } else if let Some(text) = print_arg(trimmed, &self.params) {
                self.stream(socket, &parent, format!("{text}\n")).await?;
            }
        }
        if code.contains("slow_output") {
            self.stream(socket, &parent, "first\n".into()).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            self.stream(socket, &parent, "second\n".into()).await?;
        }
        if code.contains("uv', 'pip', 'install'") {
            self.stream(socket, &parent, "Installation Complete (via uv)!\n".into()).await?;
        }
        if code.contains("display_image") {
            self.send(
                socket,
                &parent,
                "iopub",
                "display_data",
                json!({"data": {"image/png": "aGVsbG8=", "text/plain": "<Figure>"}, "metadata": {}}),
            )
            .await?;
        }
        if code.contains("clear_output") {
            self.send(socket, &parent, "iopub", "clear_output", json!({"wait": false})).await?;
        }
        if code.contains("__NZAP_ARTIFACTS__") {
            let found: Vec<Value> = {
                let mock = self.state.lock().expect("mock state");
                mock.runtime(&self.endpoint)
                    .map(|runtime| {
                        runtime
                            .files
                            .iter()
                            .filter(|(path, _)| path.starts_with("content/out/"))
                            .filter_map(|(path, file)| {
                                let size = match file {
                                    MockFile::Text(text) => text.len(),
                                    MockFile::Binary(bytes) => bytes.len(),
                                    _ => return None,
                                };
                                Some(json!({"path": format!("/{path}"), "size": size}))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            self.stream(socket, &parent, format!("__NZAP_ARTIFACTS__{}\n", Value::from(found))).await?;
        }
        if code.contains("answer") {
            self.send(
                socket,
                &parent,
                "iopub",
                "execute_result",
                json!({"execution_count": count, "data": {"text/plain": "42"}, "metadata": {}}),
            )
            .await?;
        }
        if let Some(start) = code.find("sys.exit(") {
            let rest = &code[start + "sys.exit(".len()..];
            let value = rest.split(')').next().unwrap_or_default().trim();
            self.error(socket, &parent, "SystemExit", value).await?;
            status = "error";
        } else if code.contains("raise") || code.contains("fail") {
            self.error(socket, &parent, "ValueError", "boom").await?;
            status = "error";
        }
        self.finish(socket, &parent, status, count).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_are_evaluated() {
        let params = json!({"who": "Ada", "n": 3});
        assert_eq!(print_arg("print(\"hi\")", &params).as_deref(), Some("hi"));
        assert_eq!(print_arg("  print('hi')", &params).as_deref(), Some("hi"));
        assert_eq!(print_arg("print(params[\"who\"])", &params).as_deref(), Some("Ada"));
        assert_eq!(print_arg("print(params['n'])", &params).as_deref(), Some("3"));
        assert_eq!(print_arg("x = 1", &params), None);
    }
}
