//! A runtime's Jupyter server behind its proxy (`/proxy/{endpoint}/…`):
//! the contents API over an in-memory file tree, kernels, sessions and
//! `api/colab/resources`. Every route checks the runtime-proxy token.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::state::{MockFile, MockRuntime, MockState};
use crate::Shared;

type Params = Query<HashMap<String, String>>;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/proxy/{endpoint}/api/contents", get(get_root))
        .route("/proxy/{endpoint}/api/contents/", get(get_root))
        .route(
            "/proxy/{endpoint}/api/contents/{*path}",
            get(get_contents).put(put_contents).patch(rename_contents).delete(delete_contents),
        )
        .route("/proxy/{endpoint}/api/kernels", get(list_kernels).post(start_kernel))
        .route("/proxy/{endpoint}/api/kernels/{kernel}", axum::routing::delete(shutdown_kernel))
        .route("/proxy/{endpoint}/api/kernels/{kernel}/restart", post(restart_kernel))
        .route("/proxy/{endpoint}/api/kernels/{kernel}/interrupt", post(interrupt_kernel))
        .route("/proxy/{endpoint}/api/sessions", get(list_sessions).post(create_session))
        .route("/proxy/{endpoint}/api/colab/resources", get(resources))
}

/// The proxy token, from the query parameter or the header.
pub(crate) fn check_runtime_token(
    mock: &MockState,
    endpoint: &str,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(), Response> {
    let Some(assignment) = mock.assignment(endpoint) else {
        return Err((StatusCode::NOT_FOUND, "no such runtime").into_response());
    };
    let header = headers.get("x-colab-runtime-proxy-token").and_then(|value| value.to_str().ok());
    let param = query.get("colab-runtime-proxy-token").map(String::as_str);
    if header == Some(assignment.proxy_token.as_str())
        || param == Some(assignment.proxy_token.as_str())
    {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "bad runtime proxy token").into_response())
    }
}

macro_rules! runtime_or_return {
    ($mock:expr, $endpoint:expr, $headers:expr, $query:expr) => {{
        if let Err(response) = check_runtime_token(&$mock, &$endpoint, &$headers, &$query) {
            return response;
        }
        match $mock.runtimes.get_mut(&$endpoint) {
            Some(runtime) => runtime,
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    }};
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or_default().to_owned()
}

fn children<'a>(runtime: &'a MockRuntime, dir: &str) -> Vec<(&'a String, &'a MockFile)> {
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    runtime
        .files
        .iter()
        .filter(|(path, _)| {
            path.starts_with(&prefix)
                && !path[prefix.len()..].is_empty()
                && !path[prefix.len()..].contains('/')
        })
        .collect()
}

fn model(runtime: &MockRuntime, path: &str, file: &MockFile, with_content: bool) -> Value {
    let base = |kind: &str, format: Value, mimetype: Value, size: Value, content: Value| {
        json!({
            "name": name_of(path),
            "path": path,
            "type": kind,
            "format": format,
            "mimetype": mimetype,
            "size": size,
            "writable": true,
            "created": "2026-01-01T00:00:00Z",
            "last_modified": "2026-01-01T00:00:00Z",
            "content": content,
        })
    };
    match file {
        MockFile::Directory => {
            let content = if with_content {
                Value::Array(
                    children(runtime, path)
                        .into_iter()
                        .map(|(child, file)| model(runtime, child, file, false))
                        .collect(),
                )
            } else {
                Value::Null
            };
            base("directory", json!("json"), Value::Null, Value::Null, content)
        }
        MockFile::Text(text) => base(
            "file",
            json!("text"),
            json!("text/plain"),
            json!(text.len()),
            if with_content { json!(text) } else { Value::Null },
        ),
        MockFile::Binary(bytes) => base(
            "file",
            json!("base64"),
            json!("application/octet-stream"),
            json!(bytes.len()),
            if with_content { json!(STANDARD.encode(bytes)) } else { Value::Null },
        ),
        MockFile::Notebook(notebook) => base(
            "notebook",
            json!("json"),
            Value::Null,
            json!(notebook.to_string().len()),
            if with_content { notebook.clone() } else { Value::Null },
        ),
    }
}

fn read(runtime: &MockRuntime, path: &str, query: &HashMap<String, String>) -> Response {
    let path = path.trim_matches('/');
    let with_content = query.get("content").map(String::as_str) != Some("0");
    if path.is_empty() {
        return Json(model(runtime, "", &MockFile::Directory, true)).into_response();
    }
    match runtime.files.get(path) {
        Some(file) => Json(model(runtime, path, file, with_content)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("No such file or directory: {path}")})),
        )
            .into_response(),
    }
}

async fn get_root(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    read(runtime, "", &query)
}

async fn get_contents(
    State(state): State<Shared>,
    Path((endpoint, path)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    read(runtime, &path, &query)
}

fn ensure_parents(runtime: &mut MockRuntime, path: &str) {
    let mut current = String::new();
    let parts: Vec<&str> = path.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        runtime.files.entry(current.clone()).or_insert(MockFile::Directory);
    }
}

async fn put_contents(
    State(state): State<Shared>,
    Path((endpoint, path)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    let path = path.trim_matches('/').to_owned();
    let kind = body.get("type").and_then(Value::as_str).unwrap_or("file");
    let file = match kind {
        "directory" => MockFile::Directory,
        "notebook" => MockFile::Notebook(body.get("content").cloned().unwrap_or(Value::Null)),
        _ => {
            let content = body.get("content").and_then(Value::as_str).unwrap_or_default();
            match body.get("format").and_then(Value::as_str) {
                Some("base64") => match STANDARD.decode(content) {
                    Ok(bytes) => MockFile::Binary(bytes),
                    Err(_) => return (StatusCode::BAD_REQUEST, "bad base64").into_response(),
                },
                _ => MockFile::Text(content.to_owned()),
            }
        }
    };
    if matches!(runtime.files.get(&path), Some(MockFile::Directory)) && file != MockFile::Directory
    {
        return (StatusCode::BAD_REQUEST, "a directory exists at that path").into_response();
    }
    // Jupyter's chunked upload: chunk 1 starts the file, later chunks
    // (2, 3, …, and -1 for the last) append to it.
    let chunk = body.get("chunk").and_then(Value::as_i64);
    let file = match (chunk, file) {
        (Some(number), MockFile::Binary(bytes)) if number != 1 => match runtime.files.get(&path) {
            Some(MockFile::Binary(existing)) => {
                MockFile::Binary(existing.iter().chain(&bytes).copied().collect())
            }
            _ => return (StatusCode::BAD_REQUEST, "no upload in progress").into_response(),
        },
        (_, file) => file,
    };
    if let Some(number) = chunk {
        runtime.upload_chunks.push(number);
    }
    ensure_parents(runtime, &path);
    runtime.files.insert(path.clone(), file.clone());
    (StatusCode::CREATED, Json(model(runtime, &path, &file, false))).into_response()
}

async fn rename_contents(
    State(state): State<Shared>,
    Path((endpoint, path)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    let from = path.trim_matches('/').to_owned();
    let to =
        body.get("path").and_then(Value::as_str).unwrap_or_default().trim_matches('/').to_owned();
    if to.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing new path").into_response();
    }
    if !runtime.files.contains_key(&from) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if runtime.files.contains_key(&to) {
        return (StatusCode::CONFLICT, "target exists").into_response();
    }
    let moved: Vec<(String, MockFile)> = runtime
        .files
        .iter()
        .filter(|(key, _)| **key == from || key.starts_with(&format!("{from}/")))
        .map(|(key, file)| (key.clone(), file.clone()))
        .collect();
    for (key, file) in moved {
        runtime.files.remove(&key);
        runtime.files.insert(format!("{to}{}", &key[from.len()..]), file);
    }
    ensure_parents(runtime, &to);
    let file = runtime.files.get(&to).cloned().unwrap_or(MockFile::Directory);
    Json(model(runtime, &to, &file, false)).into_response()
}

async fn delete_contents(
    State(state): State<Shared>,
    Path((endpoint, path)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    let path = path.trim_matches('/').to_owned();
    if runtime.files.remove(&path).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let prefix = format!("{path}/");
    runtime.files.retain(|key, _| !key.starts_with(&prefix));
    StatusCode::NO_CONTENT.into_response()
}

async fn list_kernels(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    let list: Vec<Value> = runtime
        .kernels
        .iter()
        .map(|id| json!({"id": id, "name": "python3", "execution_state": "idle"}))
        .collect();
    Json(list).into_response()
}

async fn start_kernel(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let id = mock.next_id("kernel");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    runtime.kernels.push(id.clone());
    (StatusCode::CREATED, Json(json!({"id": id, "name": "python3", "execution_state": "starting"})))
        .into_response()
}

async fn restart_kernel(
    State(state): State<Shared>,
    Path((endpoint, kernel)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    if !runtime.kernels.contains(&kernel) {
        return StatusCode::NOT_FOUND.into_response();
    }
    runtime.restarts += 1;
    Json(json!({"id": kernel, "name": "python3"})).into_response()
}

async fn interrupt_kernel(
    State(state): State<Shared>,
    Path((endpoint, kernel)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    if !runtime.kernels.contains(&kernel) {
        return StatusCode::NOT_FOUND.into_response();
    }
    runtime.interrupts += 1;
    StatusCode::NO_CONTENT.into_response()
}

async fn shutdown_kernel(
    State(state): State<Shared>,
    Path((endpoint, kernel)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    let before = runtime.kernels.len();
    runtime.kernels.retain(|id| *id != kernel);
    if runtime.kernels.len() == before {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn list_sessions(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    Json(runtime.sessions.clone()).into_response()
}

async fn create_session(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Params,
    Json(body): Json<Value>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    let id = mock.next_id("session");
    let runtime = runtime_or_return!(mock, endpoint, headers, query);
    let kernel = body.get("kernel").cloned().unwrap_or(Value::Null);
    let kernel_id = kernel.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
    if !runtime.kernels.contains(&kernel_id) {
        return (StatusCode::BAD_REQUEST, "unknown kernel").into_response();
    }
    let session = json!({
        "id": id,
        "name": body.get("name"),
        "path": body.get("path"),
        "type": body.get("type"),
        "kernel": kernel,
    });
    runtime.sessions.push(session.clone());
    (StatusCode::CREATED, Json(session)).into_response()
}

async fn resources(
    State(state): State<Shared>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
    Query(query): Params,
) -> Response {
    let mock = state.lock().expect("mock state");
    // colab-vscode sends the token header for telemetry; require it.
    if headers.get("x-colab-runtime-proxy-token").is_none() {
        return (StatusCode::FORBIDDEN, "token header required").into_response();
    }
    if let Err(response) = check_runtime_token(&mock, &endpoint, &headers, &query) {
        return response;
    }
    Json(mock.resources.clone()).into_response()
}
