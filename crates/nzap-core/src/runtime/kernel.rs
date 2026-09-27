//! A live kernel WebSocket.
//!
//! Port of colab-studio's `KernelChannel` (itself replacing the CLI's
//! `jupyter_kernel_client`): the *default* (unversioned) Jupyter
//! sub-protocol — plain JSON frames with a `channel` key — over
//! `{proxy}/api/kernels/{id}/channels`. Executions are multiplexed by
//! message id, so several cells can stream at once; the VM's
//! `colab_request` messages (Drive / Cloud credentials) are handed to the
//! owner of the channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use super::proxy::RuntimeProxy;
use super::ws;
use crate::error::{Error, Result};

pub const PROTOCOL_VERSION: &str = "5.3";
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// After the reply arrives, wait this long for the kernel to go idle so the
/// last outputs are delivered.
const IDLE_GRACE: Duration = Duration::from_secs(5);

/// A `colab_request` from the VM: it wants us to do something (propagate
/// Drive or Cloud credentials) and blocks until we answer on stdin.
#[derive(Clone, Debug)]
pub struct ColabRequest {
    /// The request message's header; the reply's parent header.
    pub header: Value,
    pub colab_msg_id: Value,
    /// `content.request` (carries `authType`).
    pub request: Value,
}

impl ColabRequest {
    pub fn auth_type(&self) -> Option<&str> {
        self.request
            .get("authType")
            .or_else(|| self.request.get("auth_type"))
            .and_then(Value::as_str)
    }

    /// The `colab_reply` that unblocks the kernel.
    pub fn reply(&self) -> Value {
        json!({ "type": "colab_reply", "colab_msg_id": self.colab_msg_id })
    }
}

struct Pending {
    emit: Option<mpsc::UnboundedSender<Value>>,
    reply: Option<oneshot::Sender<Value>>,
    idle: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct Shared {
    pending: Mutex<HashMap<String, Pending>>,
    execution_state: Mutex<Option<String>>,
    connected: AtomicBool,
    last_error: Mutex<Option<String>>,
}

/// Removes a pending entry when the waiting future ends or is dropped
/// (cancelled executions must not leak bookkeeping).
struct PendingGuard {
    shared: Arc<Shared>,
    msg_id: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.shared.pending.lock() {
            pending.remove(&self.msg_id);
        }
    }
}

pub struct KernelChannel {
    kernel_id: String,
    session_id: String,
    outbound: mpsc::UnboundedSender<Message>,
    shared: Arc<Shared>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

fn header(msg_type: &str, session: &str) -> Value {
    json!({
        "msg_id": uuid::Uuid::new_v4().simple().to_string(),
        "username": "nzap-engine",
        "session": session,
        "date": chrono::Utc::now().to_rfc3339(),
        "msg_type": msg_type,
        "version": PROTOCOL_VERSION,
    })
}

/// Translate a Jupyter message into the compact UI-facing event
/// (colab-studio `_emit_output`).
pub fn to_ui_event(msg_type: &str, content: &Value) -> Option<Value> {
    let get = |key: &str| content.get(key).cloned().unwrap_or(Value::Null);
    let event = match msg_type {
        "stream" => json!({
            "type": "stream",
            "name": content.get("name").and_then(Value::as_str).unwrap_or("stdout"),
            "text": content.get("text").and_then(Value::as_str).unwrap_or_default(),
        }),
        "execute_result" => json!({
            "type": "result",
            "execution_count": get("execution_count"),
            "data": content.get("data").cloned().unwrap_or_else(|| json!({})),
        }),
        "display_data" => {
            json!({ "type": "display", "data": content.get("data").cloned().unwrap_or_else(|| json!({})) })
        }
        "update_display_data" => json!({
            "type": "update_display",
            "data": content.get("data").cloned().unwrap_or_else(|| json!({})),
        }),
        "error" => json!({
            "type": "error",
            "ename": get("ename"),
            "evalue": get("evalue"),
            "traceback": content.get("traceback").cloned().unwrap_or_else(|| json!([])),
        }),
        "clear_output" => json!({ "type": "clear_output" }),
        "execute_input" => json!({ "type": "input", "execution_count": get("execution_count") }),
        "input_request" => json!({
            "type": "input_request",
            "prompt": content.get("prompt").and_then(Value::as_str).unwrap_or_default(),
            "password": content.get("password").and_then(Value::as_bool).unwrap_or(false),
        }),
        "status" => json!({ "type": "status", "state": get("execution_state") }),
        _ => return None,
    };
    Some(event)
}

impl KernelChannel {
    /// Open the kernel socket (three attempts, as colab-studio) and start
    /// the reader and writer tasks. `colab_requests` receives the VM's
    /// credential requests; without it they are answered immediately so the
    /// kernel is never left blocked.
    pub async fn connect(
        proxy: &RuntimeProxy,
        kernel_id: &str,
        session_id: Option<String>,
        colab_requests: Option<mpsc::UnboundedSender<ColabRequest>>,
    ) -> Result<Arc<Self>> {
        let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let url = proxy.kernel_ws_url(kernel_id, &session_id)?;
        let headers = proxy.auth_headers();

        let mut last_error = None;
        let mut socket = None;
        for attempt in 0..3u64 {
            match ws::connect(&url, &headers).await {
                Ok(stream) => {
                    socket = Some(stream);
                    break;
                }
                // A definitive refusal will not change on retry.
                Err(error @ Error::Runtime { status: Some(401 | 403 | 404), .. }) => {
                    return Err(error)
                }
                Err(error) => {
                    tracing::warn!("Kernel socket attempt {}/3 failed: {error}", attempt + 1);
                    last_error = Some(error);
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                    }
                }
            }
        }
        let Some(socket) = socket else {
            return Err(last_error.unwrap_or_else(|| Error::runtime(None, "Kernel socket failed.")));
        };

        let (mut sink, mut stream) = socket.split();
        let (outbound, mut outbound_rx) = mpsc::unbounded_channel::<Message>();
        let shared = Arc::new(Shared::default());
        shared.connected.store(true, Ordering::SeqCst);

        let writer = tokio::spawn(async move {
            let mut ping = tokio::time::interval(PING_INTERVAL);
            ping.tick().await;
            loop {
                tokio::select! {
                    message = outbound_rx.recv() => match message {
                        Some(message) => {
                            if sink.send(message).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                    _ = ping.tick() => {
                        if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = sink.close().await;
        });

        let reader_shared = shared.clone();
        let reader_outbound = outbound.clone();
        let reader = tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                let text = match frame {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Binary(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(error) => {
                        if let Ok(mut last) = reader_shared.last_error.lock() {
                            *last = Some(error.to_string());
                        }
                        break;
                    }
                };
                let Ok(message) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                dispatch(&reader_shared, &reader_outbound, colab_requests.as_ref(), message);
            }
            reader_shared.connected.store(false, Ordering::SeqCst);
            // Wake every waiter: dropping the senders fails their receivers.
            if let Ok(mut pending) = reader_shared.pending.lock() {
                pending.clear();
            }
            tracing::info!("Kernel socket closed");
        });

        Ok(Arc::new(Self {
            kernel_id: kernel_id.to_owned(),
            session_id,
            outbound,
            shared,
            tasks: Mutex::new(vec![writer, reader]),
        }))
    }

    pub fn kernel_id(&self) -> &str {
        &self.kernel_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::SeqCst)
    }

    /// Last `execution_state` the kernel broadcast (`idle` / `busy` /
    /// `starting`) — what `colab status` reports.
    pub fn execution_state(&self) -> Option<String> {
        self.shared.execution_state.lock().ok().and_then(|state| state.clone())
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.last_error.lock().ok().and_then(|error| error.clone())
    }

    fn send(
        &self,
        channel: &str,
        msg_type: &str,
        content: Value,
        header_value: Option<Value>,
        parent: Value,
    ) -> Result<String> {
        let header_value = header_value.unwrap_or_else(|| header(msg_type, &self.session_id));
        let msg_id = header_value["msg_id"].as_str().unwrap_or_default().to_owned();
        let message = json!({
            "header": header_value,
            "parent_header": parent,
            "metadata": {},
            "content": content,
            "channel": channel,
            "buffers": [],
        });
        if !self.is_connected() {
            return Err(Error::runtime(None, "The kernel socket is not connected."));
        }
        self.outbound
            .send(Message::Text(message.to_string().into()))
            .map_err(|_| Error::runtime(None, "The kernel socket is not connected."))?;
        Ok(msg_id)
    }

    /// Answer an `input_request` or unblock a `colab_request`.
    pub fn send_input_reply(&self, value: Value, parent_header: Value) -> Result<()> {
        self.send("stdin", "input_reply", json!({ "value": value }), None, parent_header)?;
        Ok(())
    }

    fn register(
        &self,
        msg_id: &str,
        emit: Option<mpsc::UnboundedSender<Value>>,
    ) -> Result<(oneshot::Receiver<Value>, oneshot::Receiver<()>, PendingGuard)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let (idle_tx, idle_rx) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .map_err(|_| Error::internal("Kernel state lock poisoned."))?
            .insert(
                msg_id.to_owned(),
                Pending { emit, reply: Some(reply_tx), idle: Some(idle_tx) },
            );
        Ok((
            reply_rx,
            idle_rx,
            PendingGuard { shared: self.shared.clone(), msg_id: msg_id.to_owned() },
        ))
    }

    async fn wait(
        reply: oneshot::Receiver<Value>,
        idle: oneshot::Receiver<()>,
        timeout: Duration,
    ) -> Result<Value> {
        let content = match tokio::time::timeout(timeout, reply).await {
            Err(_) => {
                return Err(Error::runtime(None, "Timed out waiting for the kernel to reply."))
            }
            Ok(Err(_)) => return Err(Error::runtime(None, "The kernel connection closed.")),
            Ok(Ok(content)) => content,
        };
        let _ = tokio::time::timeout(IDLE_GRACE, idle).await;
        Ok(content)
    }

    /// Run `code`, sending UI events to `emit`, and return the
    /// `execute_reply` content. `allow_stdin` lets `input()` prompt the UI.
    pub async fn execute(
        &self,
        code: &str,
        emit: mpsc::UnboundedSender<Value>,
        timeout: Duration,
        allow_stdin: bool,
    ) -> Result<Value> {
        let header_value = header("execute_request", &self.session_id);
        let msg_id = header_value["msg_id"].as_str().unwrap_or_default().to_owned();
        let (reply, idle, _guard) = self.register(&msg_id, Some(emit))?;
        self.send(
            "shell",
            "execute_request",
            json!({
                "code": code,
                "silent": false,
                "store_history": true,
                "user_expressions": {},
                "allow_stdin": allow_stdin,
                "stop_on_error": true,
            }),
            Some(header_value),
            json!({}),
        )?;
        Self::wait(reply, idle, timeout).await
    }

    pub async fn kernel_info(&self, timeout: Duration) -> Result<Value> {
        let header_value = header("kernel_info_request", &self.session_id);
        let msg_id = header_value["msg_id"].as_str().unwrap_or_default().to_owned();
        let (reply, idle, _guard) = self.register(&msg_id, None)?;
        self.send("shell", "kernel_info_request", json!({}), Some(header_value), json!({}))?;
        Self::wait(reply, idle, timeout).await
    }

    /// Close the socket without shutting the kernel down (the kernel belongs
    /// to the runtime — the CLI forces `_own_kernel = False` for the same
    /// reason).
    pub fn stop(&self) {
        self.shared.connected.store(false, Ordering::SeqCst);
        if let Ok(mut tasks) = self.tasks.lock() {
            for task in tasks.drain(..) {
                task.abort();
            }
        }
        if let Ok(mut pending) = self.shared.pending.lock() {
            pending.clear();
        }
    }
}

impl Drop for KernelChannel {
    fn drop(&mut self) {
        self.stop();
    }
}

fn dispatch(
    shared: &Shared,
    outbound: &mpsc::UnboundedSender<Message>,
    colab_requests: Option<&mpsc::UnboundedSender<ColabRequest>>,
    message: Value,
) {
    let header_value = message.get("header").cloned().unwrap_or_else(|| json!({}));
    let msg_type = header_value
        .get("msg_type")
        .or_else(|| message.get("msg_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let empty = json!({});
    let content = message.get("content").unwrap_or(&empty);

    if msg_type == "colab_request" {
        let request = ColabRequest {
            header: header_value,
            colab_msg_id: message
                .get("metadata")
                .and_then(|metadata| metadata.get("colab_msg_id"))
                .cloned()
                .unwrap_or(Value::Null),
            request: content.get("request").cloned().unwrap_or_else(|| json!({})),
        };
        let unhandled = match colab_requests {
            Some(handler) => handler.send(request).err().map(|error| error.0),
            None => Some(request),
        };
        // Nothing handles it: answer anyway so the kernel is not blocked.
        if let Some(request) = unhandled {
            let reply = json!({
                "header": header("input_reply", ""),
                "parent_header": request.header,
                "metadata": {},
                "content": { "value": request.reply() },
                "channel": "stdin",
                "buffers": [],
            });
            let _ = outbound.send(Message::Text(reply.to_string().into()));
        }
        return;
    }

    if msg_type == "status" {
        if let Some(state) = content.get("execution_state").and_then(Value::as_str) {
            if let Ok(mut current) = shared.execution_state.lock() {
                *current = Some(state.to_owned());
            }
        }
    }

    let Some(parent_id) = message
        .get("parent_header")
        .and_then(|parent| parent.get("msg_id"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Ok(mut pending) = shared.pending.lock() else {
        return;
    };
    let Some(entry) = pending.get_mut(parent_id) else {
        return;
    };
    if let Some(emit) = &entry.emit {
        if let Some(event) = to_ui_event(&msg_type, content) {
            let _ = emit.send(event);
        }
    }
    match msg_type.as_str() {
        "status" if content.get("execution_state").and_then(Value::as_str) == Some("idle") => {
            if let Some(idle) = entry.idle.take() {
                let _ = idle.send(());
            }
        }
        "execute_reply" | "kernel_info_reply" | "complete_reply" => {
            if let Some(reply) = entry.reply.take() {
                let _ = reply.send(content.clone());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_events_match_colab_studio() {
        assert_eq!(
            to_ui_event("stream", &json!({"name": "stderr", "text": "x"})),
            Some(json!({"type": "stream", "name": "stderr", "text": "x"}))
        );
        assert_eq!(
            to_ui_event(
                "execute_result",
                &json!({"execution_count": 2, "data": {"text/plain": "4"}})
            ),
            Some(json!({"type": "result", "execution_count": 2, "data": {"text/plain": "4"}}))
        );
        assert_eq!(
            to_ui_event("error", &json!({"ename": "E", "evalue": "v"}))
                .and_then(|event| event.get("traceback").cloned()),
            Some(json!([]))
        );
        assert_eq!(
            to_ui_event("input_request", &json!({"prompt": "Name? "})),
            Some(json!({"type": "input_request", "prompt": "Name? ", "password": false}))
        );
        assert_eq!(
            to_ui_event("status", &json!({"execution_state": "busy"})),
            Some(json!({"type": "status", "state": "busy"}))
        );
        assert_eq!(to_ui_event("comm_open", &json!({})), None);
    }

    #[test]
    fn colab_request_reply_shape() {
        let request = ColabRequest {
            header: json!({"msg_id": "m"}),
            colab_msg_id: json!(7),
            request: json!({"authType": "dfs_ephemeral"}),
        };
        assert_eq!(request.auth_type(), Some("dfs_ephemeral"));
        assert_eq!(request.reply(), json!({"type": "colab_reply", "colab_msg_id": 7}));
    }
}
