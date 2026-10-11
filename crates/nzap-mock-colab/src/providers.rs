//! Mock GitHub Codespaces and CodeSandbox, plus the Jupyter Server bits
//! those VMs expose that Colab's proxy does not (`api/status`, terminals).
//!
//! * `/github/*` — the REST API (`user`, codespaces, machines), bearer
//!   token checked against `github_token`
//! * `/github-web/login/*` — the OAuth device flow (pending once, then a
//!   token)
//! * `/csb/*` — the CodeSandbox API (`{data: …}` envelopes), bearer key
//!   checked against `csb_api_key`
//! * `/csb-pitcher/{id}` — the VM agent: MessagePack `client/join` and
//!   `shell/create`; a command that starts `jupyter server` brings the
//!   sandbox's Jupyter up with the token it names
//!
//! Provider VMs serve Jupyter on the same `/proxy/{endpoint}/…` routes as
//! Colab runtimes; [`check_provider_runtime`] stands in for GitHub's private
//! port forwarding and CodeSandbox's private preview hosts.

use std::collections::HashMap;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Form, Json, Router};
use serde_json::{json, Value};

use crate::state::{MockCodespace, MockRuntime, MockSandbox, MockState};
use crate::tty::{run, PROMPT};
use crate::Shared;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        // GitHub
        .route("/github/user", get(github_user))
        .route("/github/repos/{owner}/{repo}/codespaces/machines", get(codespace_machines))
        .route("/github/repos/{owner}/{repo}/codespaces", post(create_codespace))
        .route("/github/user/codespaces/{name}", get(get_codespace).delete(delete_codespace))
        .route("/github/user/codespaces/{name}/start", post(start_codespace))
        .route("/github/user/codespaces/{name}/stop", post(stop_codespace))
        .route("/github-web/login/device/code", post(device_code))
        .route("/github-web/login/oauth/access_token", post(device_token))
        // CodeSandbox
        .route("/csb/vm/running", get(csb_running))
        .route("/csb/sandbox/{template}/fork", post(csb_fork))
        .route("/csb/vm/{id}/start", post(csb_start))
        .route("/csb/sandbox/{id}/tokens", post(csb_preview_token))
        .route("/csb/vm/{id}/hibernate", post(csb_hibernate))
        .route("/csb/vm/{id}/shutdown", post(csb_shutdown))
        .route("/csb/vm/{id}", delete(csb_delete))
        .route("/csb-pitcher/{id}/", get(pitcher))
        // Jupyter Server extras, for every runtime
        .route("/proxy/{endpoint}/api/status", get(jupyter_status))
        .route("/proxy/{endpoint}/api/terminals", post(create_terminal))
        .route("/proxy/{endpoint}/api/terminals/{name}", delete(delete_terminal))
        .route("/proxy/{endpoint}/terminals/websocket/{name}", get(terminal_socket))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    header(headers, "authorization").and_then(|value| value.strip_prefix("Bearer "))
}

/// GitHub's private port forwarding / CodeSandbox's private preview host:
/// the right header, and a VM that is up and serving Jupyter (502 otherwise).
pub(crate) fn check_provider_runtime(
    mock: &mut MockState,
    endpoint: &str,
    headers: &HeaderMap,
) -> Result<(), Response> {
    if let Some(codespace) = mock.codespaces.get(endpoint) {
        if header(headers, "x-github-token") != Some(mock.github_token.as_str()) {
            return Err((StatusCode::UNAUTHORIZED, "sign in to GitHub").into_response());
        }
        if codespace.state != "Available" {
            return Err((StatusCode::BAD_GATEWAY, "codespace is not running").into_response());
        }
        return Ok(());
    }
    if let Some(sandbox) = mock.sandboxes.get_mut(endpoint) {
        if sandbox.preview_token.is_none()
            || header(headers, "csb-preview-token") != sandbox.preview_token.as_deref()
        {
            return Err((StatusCode::FORBIDDEN, "private preview").into_response());
        }
        if !sandbox.running {
            return Err((StatusCode::BAD_GATEWAY, "sandbox is not running").into_response());
        }
        // `automatic_wakeup_config.http`: a request wakes a hibernated VM.
        sandbox.hibernated = false;
        if !sandbox.jupyter_running {
            return Err((StatusCode::BAD_GATEWAY, "nothing listens on 8888").into_response());
        }
        let expected = sandbox.jupyter_token.as_ref().map(|token| format!("token {token}"));
        if header(headers, "authorization") != expected.as_deref() {
            return Err((StatusCode::FORBIDDEN, "jupyter token").into_response());
        }
        return Ok(());
    }
    Err((StatusCode::NOT_FOUND, "no such runtime").into_response())
}

// ---------------------------------------------------------------- GitHub

fn github_auth(mock: &MockState, headers: &HeaderMap) -> Result<(), Response> {
    if bearer(headers) == Some(mock.github_token.as_str()) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, Json(json!({"message": "Bad credentials"}))).into_response())
    }
}

fn codespace_json(mock: &MockState, codespace: &MockCodespace) -> Value {
    json!({
        "name": codespace.name,
        "display_name": codespace.display_name,
        "state": codespace.state,
        "machine": {"name": codespace.machine},
        "repository": {"full_name": codespace.repository},
        "devcontainer_path": codespace.devcontainer_path,
        "idle_timeout_minutes": codespace.idle_timeout_minutes,
        "retention_period_minutes": codespace.retention_period_minutes,
        "web_url": format!("{}/codespaces/{}", mock.base_url, codespace.name),
    })
}

async fn github_user(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    let body = Json(json!({"login": "ada", "name": "Ada Lovelace", "avatar_url": null}));
    match &mock.github_scopes {
        Some(scopes) => ([("x-oauth-scopes", scopes.clone())], body).into_response(),
        None => body.into_response(),
    }
}

async fn codespace_machines(
    State(state): State<Shared>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    if !mock.codespace_repositories.contains(&format!("{owner}/{repo}")) {
        return (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response();
    }
    Json(json!({"total_count": 2, "machines": [
        {"name": "basicLinux32gb", "display_name": "2 cores, 8 GB RAM, 32 GB storage",
         "cpus": 2, "memory_in_bytes": 8_589_934_592u64, "storage_in_bytes": 34_359_738_368u64},
        {"name": "standardLinux32gb", "display_name": "4 cores, 16 GB RAM, 32 GB storage",
         "cpus": 4, "memory_in_bytes": 17_179_869_184u64, "storage_in_bytes": 34_359_738_368u64},
    ]}))
    .into_response()
}

async fn create_codespace(
    State(state): State<Shared>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    let repository = format!("{owner}/{repo}");
    if !mock.codespace_repositories.contains(&repository) {
        return (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response();
    }
    let name = mock.next_id(&format!("ada-{repo}"));
    let codespace = MockCodespace {
        name: name.clone(),
        repository,
        machine: body["machine"].as_str().unwrap_or("basicLinux32gb").to_owned(),
        devcontainer_path: body["devcontainer_path"].as_str().unwrap_or_default().to_owned(),
        display_name: body["display_name"].as_str().unwrap_or_default().to_owned(),
        state: "Queued".to_owned(),
        polls_left: mock.codespace_boot_polls,
        idle_timeout_minutes: body["idle_timeout_minutes"].as_u64().unwrap_or(30),
        retention_period_minutes: body["retention_period_minutes"].as_u64(),
    };
    let json = codespace_json(&mock, &codespace);
    mock.codespaces.insert(name.clone(), codespace);
    mock.runtimes.insert(name, MockRuntime::with_defaults());
    (StatusCode::CREATED, Json(json)).into_response()
}

async fn get_codespace(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    let Some(codespace) = mock.codespaces.get_mut(&name) else {
        return (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response();
    };
    if matches!(codespace.state.as_str(), "Queued" | "Provisioning" | "Starting") {
        if codespace.polls_left == 0 {
            codespace.state = "Available".to_owned();
        } else {
            codespace.polls_left -= 1;
            codespace.state = "Provisioning".to_owned();
        }
    }
    let codespace = codespace.clone();
    Json(codespace_json(&mock, &codespace)).into_response()
}

async fn start_codespace(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    let polls = mock.codespace_boot_polls;
    let Some(codespace) = mock.codespaces.get_mut(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    codespace.state = "Starting".to_owned();
    codespace.polls_left = polls;
    let codespace = codespace.clone();
    Json(codespace_json(&mock, &codespace)).into_response()
}

async fn stop_codespace(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    let Some(codespace) = mock.codespaces.get_mut(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    codespace.state = "Shutdown".to_owned();
    let codespace = codespace.clone();
    Json(codespace_json(&mock, &codespace)).into_response()
}

async fn delete_codespace(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = github_auth(&mock, &headers) {
        return response;
    }
    if mock.codespaces.remove(&name).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    mock.runtimes.remove(&name);
    (StatusCode::ACCEPTED, Json(json!({}))).into_response()
}

async fn device_code(
    State(state): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if form.get("client_id").is_none_or(String::is_empty) {
        return Json(json!({"error": "unauthorized_client", "error_description": "no client id"}))
            .into_response();
    }
    let code = mock.next_id("device");
    mock.device_codes.insert(code.clone(), 1);
    Json(json!({
        "device_code": code,
        "user_code": "WDJB-MJHT",
        "verification_uri": format!("{}/github-web/login/device", mock.base_url),
        "expires_in": 900,
        "interval": 1,
    }))
    .into_response()
}

async fn device_token(
    State(state): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let code = form.get("device_code").cloned().unwrap_or_default();
    let token = mock.github_token.clone();
    match mock.device_codes.get_mut(&code) {
        None => Json(json!({"error": "expired_token"})).into_response(),
        Some(0) => {
            mock.device_codes.remove(&code);
            Json(json!({"access_token": token, "token_type": "bearer", "scope": "codespace,read:user"}))
                .into_response()
        }
        Some(pending) => {
            *pending -= 1;
            Json(json!({"error": "authorization_pending"})).into_response()
        }
    }
}

// ------------------------------------------------------------- CodeSandbox

fn csb_auth(mock: &MockState, headers: &HeaderMap) -> Result<(), Response> {
    if bearer(headers) == Some(mock.csb_api_key.as_str()) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, Json(json!({"errors": ["Unauthorized"]}))).into_response())
    }
}

fn csb_missing() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"errors": ["Sandbox not found"]}))).into_response()
}

async fn csb_running(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    let running: Vec<Value> = mock
        .sandboxes
        .values()
        .filter(|sandbox| sandbox.running && !sandbox.hibernated)
        .map(|sandbox| json!({"id": sandbox.id}))
        .collect();
    Json(json!({"data": {
        "concurrent_vm_count": running.len(),
        "concurrent_vm_limit": mock.csb_vm_limit,
        "vms": running,
    }}))
    .into_response()
}

async fn csb_fork(
    State(state): State<Shared>,
    Path(template): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    if !mock.csb_templates.contains(&template) {
        return csb_missing();
    }
    let id = mock.next_id("sb");
    let title = body["title"].as_str().unwrap_or_default().to_owned();
    mock.sandboxes.insert(
        id.clone(),
        MockSandbox { id: id.clone(), template, title: title.clone(), ..MockSandbox::default() },
    );
    mock.runtimes.insert(id.clone(), MockRuntime::with_defaults());
    Json(json!({"data": {"id": id, "alias": id, "title": title}})).into_response()
}

async fn csb_start(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    let token = mock.next_id("pitcher");
    let base = mock.base_url.clone();
    let Some(sandbox) = mock.sandboxes.get_mut(&id) else { return csb_missing() };
    let resumed = sandbox.running && sandbox.hibernated;
    if !sandbox.running {
        // A clean boot: nothing runs until the agent starts it.
        sandbox.jupyter_running = false;
    }
    sandbox.running = true;
    sandbox.hibernated = false;
    sandbox.starts += 1;
    sandbox.tier = body["tier"].as_str().map(str::to_owned);
    sandbox.pitcher_token = token.clone();
    Json(json!({"data": {
        "bootup_type": if resumed { "RESUME" } else { "CLEAN" },
        "cluster": "mock",
        "id": id,
        "latest_pitcher_version": "1",
        "pitcher_version": "1",
        "pitcher_manager_version": "1",
        "pitcher_token": token,
        "pitcher_url": format!("{base}/csb-pitcher/{id}"),
        "reconnect_token": "r",
        "use_pint": false,
        "user_workspace_path": "/project/sandbox",
        "workspace_path": "/project/sandbox",
    }}))
    .into_response()
}

async fn csb_preview_token(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    let token = mock.next_id("preview");
    let Some(sandbox) = mock.sandboxes.get_mut(&id) else { return csb_missing() };
    sandbox.preview_token = Some(token.clone());
    Json(json!({"data": {"sandbox_id": id, "token": {
        "token_id": "tok-1", "token": token, "token_prefix": "prev", "expires_at": null, "last_used_at": null,
    }}}))
    .into_response()
}

async fn csb_hibernate(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    let Some(sandbox) = mock.sandboxes.get_mut(&id) else { return csb_missing() };
    sandbox.hibernated = true;
    Json(json!({"data": {}})).into_response()
}

async fn csb_shutdown(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    let Some(sandbox) = mock.sandboxes.get_mut(&id) else { return csb_missing() };
    sandbox.running = false;
    sandbox.jupyter_running = false;
    Json(json!({"data": {}})).into_response()
}

async fn csb_delete(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if let Err(response) = csb_auth(&mock, &headers) {
        return response;
    }
    if mock.sandboxes.remove(&id).is_none() {
        return csb_missing();
    }
    mock.runtimes.remove(&id);
    Json(json!({"data": {}})).into_response()
}

fn field<'a>(value: &'a rmpv::Value, key: &str) -> Option<&'a rmpv::Value> {
    value.as_map()?.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value)
}

async fn pitcher(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let valid = {
        let mock = state.lock().expect("mock state");
        mock.sandboxes.get(&id).is_some_and(|sandbox| {
            sandbox.running && query.get("token") == Some(&sandbox.pitcher_token)
        })
    };
    if !valid {
        return (StatusCode::FORBIDDEN, "bad pitcher token").into_response();
    }
    upgrade.on_upgrade(move |socket| agent(socket, state, id))
}

async fn agent(mut socket: WebSocket, state: Shared, id: String) {
    while let Some(Ok(message)) = socket.recv().await {
        let Message::Binary(bytes) = message else { continue };
        let Ok(request) = rmpv::decode::read_value(&mut &bytes[..]) else { continue };
        let Some(request_id) = field(&request, "id").cloned() else { continue };
        let method =
            field(&request, "method").and_then(rmpv::Value::as_str).unwrap_or_default().to_owned();
        let result = match method.as_str() {
            "client/join" => rmpv::Value::Map(vec![("client".into(), "mock".into())]),
            "shell/create" => {
                let command = field(&request, "params")
                    .and_then(|params| field(params, "command"))
                    .and_then(rmpv::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let mut mock = state.lock().expect("mock state");
                if let Some(sandbox) = mock.sandboxes.get_mut(&id) {
                    if command.contains("jupyter server") {
                        sandbox.jupyter_token = command
                            .split("--IdentityProvider.token=")
                            .nth(1)
                            .and_then(|rest| rest.split_whitespace().next())
                            .map(str::to_owned);
                        sandbox.jupyter_running = true;
                    }
                    sandbox.commands.push(command);
                }
                rmpv::Value::Map(vec![
                    ("shellId".into(), "shell-1".into()),
                    ("status".into(), "RUNNING".into()),
                ])
            }
            _ => rmpv::Value::Nil,
        };
        let reply = rmpv::Value::Map(vec![("id".into(), request_id), ("result".into(), result)]);
        let mut frame = Vec::new();
        if rmpv::encode::write_value(&mut frame, &reply).is_err() {
            break;
        }
        if socket.send(Message::Binary(frame.into())).await.is_err() {
            break;
        }
    }
}

// ------------------------------------------------------- Jupyter extras

fn runtime_auth(
    state: &Shared,
    endpoint: &str,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(), Response> {
    let mut mock = state.lock().expect("mock state");
    crate::runtime::check_runtime_token(&mut mock, endpoint, headers, query)
}

async fn jupyter_status(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(response) = runtime_auth(&state, &endpoint, &headers, &query) {
        return response;
    }
    Json(json!({"started": "2026-01-01T00:00:00Z", "kernels": 0, "connections": 0})).into_response()
}

async fn create_terminal(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(response) = runtime_auth(&state, &endpoint, &headers, &query) {
        return response;
    }
    Json(json!({"name": "1", "last_activity": "2026-01-01T00:00:00Z"})).into_response()
}

async fn delete_terminal(
    State(state): State<Shared>,
    Path((endpoint, _name)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(response) = runtime_auth(&state, &endpoint, &headers, &query) {
        return response;
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn terminal_socket(
    State(state): State<Shared>,
    Path((endpoint, _name)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(response) = runtime_auth(&state, &endpoint, &headers, &query) {
        return response;
    }
    upgrade.on_upgrade(move |socket| terminado(socket, state, endpoint))
}

fn stdout(data: &str) -> Message {
    Message::Text(json!(["stdout", data]).to_string().into())
}

/// Jupyter's terminado protocol over the mock shell.
async fn terminado(mut socket: WebSocket, state: Shared, endpoint: String) {
    if socket.send(Message::Text(json!(["setup", {}]).to_string().into())).await.is_err()
        || socket
            .send(stdout(&format!("Welcome to the provider runtime\r\n{PROMPT}")))
            .await
            .is_err()
    {
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
        let Ok(Value::Array(frame)) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        match frame.first().and_then(Value::as_str) {
            Some("set_size") => {
                let rows = frame.get(1).and_then(Value::as_u64).unwrap_or_default();
                let cols = frame.get(2).and_then(Value::as_u64).unwrap_or_default();
                state.lock().expect("mock state").tty_resizes.push((endpoint.clone(), cols, rows));
            }
            Some("stdin") => {
                let data = frame.get(1).and_then(Value::as_str).unwrap_or_default();
                let mut output = String::new();
                for c in data.chars() {
                    if c == '\r' || c == '\n' {
                        output.push_str("\r\n");
                        state.lock().expect("mock state").tty_commands.push(line.clone());
                        output.push_str(&run(&line));
                        output.push_str(PROMPT);
                        line.clear();
                    } else {
                        line.push(c);
                        output.push(c);
                    }
                }
                if !output.is_empty() && socket.send(stdout(&output)).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}
