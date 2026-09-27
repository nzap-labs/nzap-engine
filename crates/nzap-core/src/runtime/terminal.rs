//! A shell on the VM — VS Code's `colab.openTerminal` and the CLI's
//! `colab console`.
//!
//! Both talk to `wss://<proxy host>/colab/tty`
//! (colab-vscode `colab-terminal-websocket.ts`, google-colab-cli
//! `console.py`) with JSON text frames: `{"data": "..."}` both ways and
//! `{"cols": N, "rows": M}` for resizes. Port of colab-studio's `terminal.py`:
//! frames are relayed unchanged, and only the two upstream shapes are ever
//! forwarded to the VM.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use super::proxy::RuntimeProxy;
use super::ws;
use crate::config::{CLIENT_AGENT, CLIENT_AGENT_HEADER, RUNTIME_PROXY_TOKEN_HEADER};
use crate::error::{Error, Result};

/// What the terminal reports to its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// A frame from the VM, verbatim (`{"data": "..."}`).
    Frame(String),
    /// The session ended; `Some` carries the reason when it failed.
    Closed(Option<String>),
}

pub type TerminalSink = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

/// Only `{"data": str}` and `{"cols": int, "rows": int}` frames reach the VM.
pub fn is_valid_frame(text: &str) -> bool {
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let keys: Vec<&str> = frame.keys().map(String::as_str).collect();
    match keys.as_slice() {
        ["data"] => frame["data"].is_string(),
        ["cols", "rows"] | ["rows", "cols"] => ["cols", "rows"].iter().all(|key| {
            frame[*key].as_u64().is_some_and(|value| value > 0 && value < 10_000)
        }),
        _ => false,
    }
}

pub struct Terminal {
    input: mpsc::UnboundedSender<String>,
    task: JoinHandle<()>,
}

impl Terminal {
    /// Connect to the VM's TTY. The proxy token goes in the header only —
    /// the endpoint answers 404 when it also gets the query parameter.
    pub async fn open(
        proxy: &RuntimeProxy,
        sink: TerminalSink,
        on_activity: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self> {
        let url = proxy.tty_url()?;
        let headers = [
            (RUNTIME_PROXY_TOKEN_HEADER, proxy.token().to_owned()),
            (CLIENT_AGENT_HEADER, CLIENT_AGENT.to_owned()),
        ];
        let socket = ws::connect(&url, &headers).await?;
        let (mut upstream_sink, mut upstream) = socket.split();
        let (input, mut input_rx) = mpsc::unbounded_channel::<String>();

        let task = tokio::spawn(async move {
            let reason = loop {
                tokio::select! {
                    frame = input_rx.recv() => match frame {
                        Some(frame) => {
                            on_activity();
                            if let Err(error) = upstream_sink.send(Message::Text(frame.into())).await {
                                break Some(format!("connection to the runtime failed: {error}"));
                            }
                        }
                        None => break None,
                    },
                    message = upstream.next() => match message {
                        Some(Ok(Message::Text(text))) => sink(TerminalEvent::Frame(text.to_string())),
                        Some(Ok(Message::Binary(bytes))) => {
                            sink(TerminalEvent::Frame(String::from_utf8_lossy(&bytes).into_owned()))
                        }
                        Some(Ok(Message::Close(_))) | None => break None,
                        Some(Ok(_)) => {}
                        Some(Err(error)) => break Some(format!("connection to the runtime failed: {error}")),
                    },
                }
            };
            let _ = upstream_sink.close().await;
            sink(TerminalEvent::Closed(reason));
        });

        Ok(Self { input, task })
    }

    /// Forward one client frame. Anything but the two upstream shapes is
    /// refused.
    pub fn send(&self, frame: &str) -> Result<()> {
        if !is_valid_frame(frame) {
            return Err(Error::invalid("Invalid terminal frame."));
        }
        self.input
            .send(frame.to_owned())
            .map_err(|_| Error::runtime(None, "The terminal is closed."))
    }

    pub fn is_open(&self) -> bool {
        !self.task.is_finished()
    }

    /// End the session (the VM's shell exits when the socket closes).
    pub fn close(&self) {
        self.task.abort();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_upstream_frames_are_valid() {
        assert!(is_valid_frame(r#"{"data": "ls\r"}"#));
        assert!(is_valid_frame(r#"{"cols": 80, "rows": 24}"#));
        assert!(!is_valid_frame(r#"{"data": 1}"#));
        assert!(!is_valid_frame(r#"{"data": "x", "extra": 1}"#));
        assert!(!is_valid_frame(r#"{"cols": 0, "rows": 24}"#));
        assert!(!is_valid_frame(r#"{"cols": 80, "rows": 10000}"#));
        assert!(!is_valid_frame(r#"{"cols": -1, "rows": 2}"#));
        assert!(!is_valid_frame(r#"["data"]"#));
        assert!(!is_valid_frame("not json"));
    }
}
