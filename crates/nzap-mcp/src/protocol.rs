//! MCP over stdio: one JSON-RPC 2.0 message per line in each direction.
//!
//! Requests run concurrently (a long job does not block `ping` or a second
//! call), `notifications/cancelled` stops one, and a call that carries a
//! `progressToken` receives `notifications/progress` as its work advances.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};

use crate::server::Server;
use crate::tools;

/// Newest first. A client asking for another version gets the newest.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

#[derive(Debug)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn to_json(&self) -> Value {
        json!({ "code": self.code, "message": self.message })
    }
}

/// The client end of one request: where its progress notifications go.
#[derive(Clone)]
pub struct Peer {
    out: mpsc::UnboundedSender<Value>,
    progress_token: Option<Value>,
    step: Arc<AtomicU64>,
}

impl Peer {
    /// Report progress, when the client asked for it.
    pub fn progress(&self, message: &str) {
        let Some(token) = &self.progress_token else { return };
        let step = self.step.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.out.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": { "progressToken": token, "progress": step, "message": message },
        }));
    }

    #[cfg(test)]
    pub fn silent() -> Self {
        let (out, _) = mpsc::unbounded_channel();
        Self { out, progress_token: None, step: Arc::default() }
    }
}

type InFlight = Arc<Mutex<HashMap<String, AbortHandle>>>;

/// Serve one client until its input ends. Work still running is stopped;
/// releasing runtimes is the caller's job ([`Server::shutdown`]).
pub async fn serve<R, W>(server: Arc<Server>, input: R, output: W) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out, mut outgoing) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(message) = outgoing.recv().await {
            let mut line = serde_json::to_vec(&message).unwrap_or_default();
            line.push(b'\n');
            if output.write_all(&line).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });

    let in_flight: InFlight = Arc::default();
    let mut tasks = JoinSet::new();
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        while tasks.try_join_next().is_some() {}
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                let error = RpcError::new(PARSE_ERROR, format!("Parse error: {error}"));
                let _ = out.send(json!({"jsonrpc": "2.0", "id": null, "error": error.to_json()}));
                continue;
            }
        };
        let Some(method) = message.get("method").and_then(Value::as_str).map(str::to_owned) else {
            // A response to a request of ours (we send none), or junk.
            if message.get("id").is_some()
                && message.get("result").is_none()
                && message.get("error").is_none()
            {
                let error = RpcError::new(INVALID_REQUEST, "Invalid request: no method.");
                let _ = out
                    .send(json!({"jsonrpc": "2.0", "id": message["id"], "error": error.to_json()}));
            }
            continue;
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let Some(id) = message.get("id").cloned() else {
            notification(&method, &params, &in_flight);
            continue;
        };

        let peer = Peer {
            out: out.clone(),
            progress_token: params.pointer("/_meta/progressToken").cloned(),
            step: Arc::default(),
        };
        let key = id.to_string();
        let server = server.clone();
        let registry = in_flight.clone();
        // Hold the registry while spawning, so a fast task cannot finish
        // (and unregister) before it is registered.
        let mut running = in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let task_key = key.clone();
        let handle = tasks.spawn(async move {
            let response = match dispatch(&server, &method, params, &peer).await {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error.to_json()}),
            };
            registry.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&task_key);
            let _ = peer.out.send(response);
        });
        running.insert(key, handle);
    }

    tasks.shutdown().await;
    drop(out);
    let _ = writer.await;
    Ok(())
}

fn notification(method: &str, params: &Value, in_flight: &InFlight) {
    if method == "notifications/cancelled" {
        let Some(id) = params.get("requestId") else { return };
        let handle = in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id.to_string());
        if let Some(handle) = handle {
            tracing::info!("Request {id} cancelled by the client");
            handle.abort();
        }
    }
    // `notifications/initialized` and anything else need no answer.
}

async fn dispatch(
    server: &Arc<Server>,
    method: &str,
    params: Value,
    peer: &Peer,
) -> Result<Value, RpcError> {
    match method {
        "initialize" => {
            let requested =
                params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|version| **version == requested)
                .copied()
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            if let Some(client) = params.pointer("/clientInfo/name").and_then(Value::as_str) {
                tracing::info!("Agent connected: {client} (MCP {version})");
            }
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": "nzap-engine",
                    "title": "NZAP Engine",
                    "version": nzap_core::VERSION,
                },
                "instructions": tools::INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call needs a tool name."))?;
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            tools::call(server, name, &arguments, peer).await
        }
        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    }
}
