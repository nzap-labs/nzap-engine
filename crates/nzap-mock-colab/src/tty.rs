//! A mock `/colab/tty`: a tiny line-editing shell over the upstream frame
//! protocol (`{"data": "..."}` both ways, `{"cols", "rows"}` resizes).
//!
//! Like the real endpoint it authenticates with the
//! `X-Colab-Runtime-Proxy-Token` header **only**, answering 404 when the
//! token (or `authuser`) also arrives as a query parameter.

use std::collections::HashMap;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};

use crate::Shared;

const PROMPT: &str = "root@mock:/content# ";

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/colab/tty", get(tty))
}

async fn tty(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if query.contains_key("colab-runtime-proxy-token") || query.contains_key("authuser") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = headers.get("x-colab-runtime-proxy-token").and_then(|value| value.to_str().ok());
    let endpoint = {
        let mock = state.lock().expect("mock state");
        mock.assignments
            .iter()
            .find(|assignment| Some(assignment.proxy_token.as_str()) == token)
            .map(|assignment| assignment.endpoint.clone())
    };
    let Some(endpoint) = endpoint else {
        return (StatusCode::FORBIDDEN, "bad runtime proxy token").into_response();
    };
    upgrade.on_upgrade(move |socket| shell(socket, state, endpoint))
}

fn frame(data: &str) -> Message {
    Message::Text(json!({ "data": data }).to_string().into())
}

fn run(command: &str) -> String {
    match command.trim() {
        "" => String::new(),
        "whoami" => "root\r\n".to_owned(),
        "pwd" => "/content\r\n".to_owned(),
        "nvidia-smi" => "NVIDIA-SMI has failed: no GPU on this mock\r\n".to_owned(),
        other if other.starts_with("echo ") => format!("{}\r\n", &other[5..]),
        other => format!("sh: 1: {other}: not found\r\n"),
    }
}

async fn shell(mut socket: WebSocket, state: Shared, endpoint: String) {
    if socket.send(frame(&format!("Welcome to the mock runtime\r\n{PROMPT}"))).await.is_err() {
        return;
    }
    let mut line = String::new();
    while let Some(Ok(message)) = socket.recv().await {
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                break;
            }
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else { continue };
        if let (Some(cols), Some(rows)) = (value.get("cols").and_then(Value::as_u64), value.get("rows").and_then(Value::as_u64)) {
            state.lock().expect("mock state").tty_resizes.push((endpoint.clone(), cols, rows));
            continue;
        }
        let Some(data) = value.get("data").and_then(Value::as_str) else { continue };
        let mut output = String::new();
        for c in data.chars() {
            match c {
                '\r' | '\n' => {
                    output.push_str("\r\n");
                    state.lock().expect("mock state").tty_commands.push(line.clone());
                    output.push_str(&run(&line));
                    output.push_str(PROMPT);
                    line.clear();
                }
                '\u{7f}' => {
                    if line.pop().is_some() {
                        output.push_str("\u{8} \u{8}");
                    }
                }
                c => {
                    line.push(c);
                    output.push(c);
                }
            }
        }
        if !output.is_empty() && socket.send(frame(&output)).await.is_err() {
            break;
        }
    }
}
