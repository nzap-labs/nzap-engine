//! Just enough of CodeSandbox's VM agent ("pitcher") to run one command.
//!
//! The SDK (`@codesandbox/sdk` 2.x, `AgentConnection` / `AgentClient`)
//! talks to `{pitcher_url}/?token={pitcher_token}` over a WebSocket with
//! MessagePack frames shaped like JSON-RPC: requests `{id, method, params}`,
//! responses `{id, result}` or `{id, error}`, notifications `{method,
//! params}`. A session starts with `client/join`; `shell/create` with
//! `type: "COMMAND"` runs a command the way `sandbox.commands.run` does.
//! NZAP uses it once per VM boot: to start Jupyter.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rmpv::Value;
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Error, Result};
use crate::runtime::ws;

/// The protocol version the SDK announces (`pitcher-protocol` `version`).
pub const PROTOCOL_VERSION: &str = "0.367.10";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn error(message: impl Into<String>) -> Error {
    Error::provider("codesandbox", None, message)
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(entries.into_iter().map(|(key, value)| (Value::from(key), value)).collect())
}

/// A request frame: `{id, method, params}`.
pub fn encode_request(id: u64, method: &str, params: Value) -> Result<Vec<u8>> {
    let frame =
        map(vec![("id", Value::from(id)), ("method", Value::from(method)), ("params", params)]);
    let mut bytes = Vec::new();
    rmpv::encode::write_value(&mut bytes, &frame)
        .map_err(|e| error(format!("Could not encode a VM agent message: {e}")))?;
    Ok(bytes)
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_map()?.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value)
}

/// `Some(Ok(result))` / `Some(Err(message))` for the response to `id`;
/// `None` for notifications and other responses.
pub fn decode_response(bytes: &[u8], id: u64) -> Option<std::result::Result<Value, String>> {
    let value = rmpv::decode::read_value(&mut &bytes[..]).ok()?;
    if field(&value, "id").and_then(Value::as_u64) != Some(id) {
        return None;
    }
    if let Some(error) = field(&value, "error") {
        let message = field(error, "message").and_then(Value::as_str).unwrap_or("request failed");
        return Some(Err(message.to_owned()));
    }
    Some(Ok(field(&value, "result").cloned().unwrap_or(Value::Nil)))
}

/// `client/join`: what the SDK sends first, with only the subscriptions a
/// command runner needs.
pub fn join_params() -> Value {
    map(vec![
        (
            "clientInfo",
            map(vec![
                ("protocolVersion", Value::from(PROTOCOL_VERSION)),
                ("appId", Value::from("sdk")),
            ]),
        ),
        ("asyncProgress", Value::from(false)),
        ("subscriptions", map(vec![("shell", map(vec![("status", Value::from(true))]))])),
    ])
}

/// `shell/create` for a command, as `commands.runBackground` builds it.
pub fn shell_params(command: &str, cwd: &str) -> Value {
    let escaped = command.replace('\'', r"'\''");
    map(vec![
        ("command", Value::from(format!("bash -c '{escaped}'"))),
        ("size", map(vec![("cols", Value::from(128)), ("rows", Value::from(24))])),
        ("type", Value::from("COMMAND")),
        ("isSystemShell", Value::from(true)),
        ("cwd", Value::from(cwd)),
    ])
}

/// The agent's socket URL (`{pitcher_url}/?token=…`, as a WebSocket URL).
pub fn socket_url(pitcher_url: &str, token: &str) -> Result<String> {
    let mut url = url::Url::parse(pitcher_url.trim_end_matches('/'))
        .map_err(|_| error("CodeSandbox returned an invalid VM agent address."))?;
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        _ => "ws",
    };
    url.set_scheme(scheme).map_err(|()| error("Could not build the VM agent address."))?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url.query_pairs_mut().append_pair("token", token);
    Ok(url.into())
}

/// Join the VM agent and start `command` in a shell. Returns once the agent
/// accepted the shell; the command keeps running in the VM.
pub async fn run_command(pitcher_url: &str, token: &str, command: &str, cwd: &str) -> Result<()> {
    let url = socket_url(pitcher_url, token)?;
    let socket = ws::connect(&url, &[]).await?;
    let (mut sink, mut stream) = socket.split();
    for (id, method, params) in
        [(0, "client/join", join_params()), (1, "shell/create", shell_params(command, cwd))]
    {
        let frame = encode_request(id, method, params)?;
        sink.send(Message::Binary(frame.into()))
            .await
            .map_err(|e| error(format!("The VM agent closed the connection: {e}")))?;
        let reply = tokio::time::timeout(REQUEST_TIMEOUT, async {
            while let Some(message) = stream.next().await {
                let bytes = match message {
                    Ok(Message::Binary(bytes)) => bytes.to_vec(),
                    Ok(Message::Text(text)) => text.as_bytes().to_vec(),
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(e) => return Err(error(format!("The VM agent connection failed: {e}"))),
                };
                if let Some(reply) = decode_response(&bytes, id) {
                    return reply.map_err(|message| {
                        error(format!("The VM agent refused {method}: {message}"))
                    });
                }
            }
            Err(error("The VM agent closed the connection."))
        })
        .await
        .map_err(|_| error(format!("The VM agent did not answer {method}.")))?;
        reply?;
    }
    let _ = sink.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let bytes =
            encode_request(1, "shell/create", shell_params("echo 'hi'", "/project")).unwrap();
        let value = rmpv::decode::read_value(&mut &bytes[..]).unwrap();
        assert_eq!(field(&value, "method").and_then(Value::as_str), Some("shell/create"));
        let params = field(&value, "params").unwrap();
        assert_eq!(
            field(params, "command").and_then(Value::as_str),
            Some(r"bash -c 'echo '\''hi'\'''")
        );
        assert_eq!(field(params, "type").and_then(Value::as_str), Some("COMMAND"));

        let mut reply = Vec::new();
        rmpv::encode::write_value(
            &mut reply,
            &map(vec![("id", 1.into()), ("result", "ok".into())]),
        )
        .unwrap();
        assert_eq!(decode_response(&reply, 1), Some(Ok(Value::from("ok"))));
        assert_eq!(decode_response(&reply, 2), None);
        let mut failed = Vec::new();
        rmpv::encode::write_value(
            &mut failed,
            &map(vec![("id", 0.into()), ("error", map(vec![("message", "nope".into())]))]),
        )
        .unwrap();
        assert_eq!(decode_response(&failed, 0), Some(Err("nope".into())));
        let mut notification = Vec::new();
        rmpv::encode::write_value(&mut notification, &map(vec![("method", "shell/out".into())]))
            .unwrap();
        assert_eq!(decode_response(&notification, 0), None);
    }

    #[test]
    fn socket_urls() {
        assert_eq!(
            socket_url("https://abc.pitcher.csb.app", "t k").unwrap(),
            "wss://abc.pitcher.csb.app/?token=t+k"
        );
        assert_eq!(
            socket_url("http://127.0.0.1:9/csb-pitcher/sb1", "x").unwrap(),
            "ws://127.0.0.1:9/csb-pitcher/sb1/?token=x"
        );
    }
}
