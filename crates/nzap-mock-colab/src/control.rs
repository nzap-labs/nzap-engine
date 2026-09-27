//! The Colab control plane: `/tun/m/*` on the front door and the `v1`
//! APIs, with Colab's quirks (XSSI prefix, XSRF two-step, 412 when the
//! account is full, a keep-alive that wants `X-Colab-Tunnel`).

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::MockState;
use crate::Shared;

const XSSI: &str = ")]}'\n";

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/tun/m/assignments", get(assignments))
        .route("/tun/m/assign", get(assign_token).post(assign))
        .route("/tun/m/unassign/{endpoint}", get(unassign_token).post(unassign))
        .route("/tun/m/{endpoint}/keep-alive/", get(keep_alive))
        .route("/tun/m/ccu-info", get(ccu_info))
        .route("/tun/m/credentials-propagation/{endpoint}", get(propagation_token).post(propagate))
        .route("/v1/user-info", get(user_info))
        .route("/v1beta/runtimespecs", get(runtime_specs))
}

/// A JSON body behind Colab's XSSI guard.
fn xssi(value: Value) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], format!("{XSSI}{value}")).into_response()
}

fn authorized(mock: &MockState, headers: &HeaderMap) -> bool {
    mock.is_authorized(headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()))
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthenticated"}))).into_response()
}

async fn assignments(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    let list: Vec<Value> =
        mock.assignments.iter().map(|assignment| assignment.to_json(&mock.base_url)).collect();
    xssi(json!({ "assignments": list }))
}

fn valid_assign_query(query: &HashMap<String, String>) -> bool {
    query.get("nbh").is_some_and(|nbh| nbh.len() == 44)
        && matches!(query.get("variant").map(String::as_str), Some("DEFAULT" | "GPU" | "TPU"))
        && query.contains_key("accelerator")
}

async fn assign_token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    if !valid_assign_query(&query) {
        return (StatusCode::BAD_REQUEST, "bad assign query").into_response();
    }
    let token = mock.next_id("xsrf");
    mock.xsrf_tokens.insert(token.clone());
    xssi(
        json!({ "acc": "acc-1", "nbh": query["nbh"], "token": token, "variant": query["variant"] }),
    )
}

async fn assign(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    let xsrf = headers.get("x-goog-colab-token").and_then(|value| value.to_str().ok());
    if !xsrf.is_some_and(|token| mock.xsrf_tokens.remove(token)) {
        return (StatusCode::FORBIDDEN, "missing or stale XSRF token").into_response();
    }
    if !valid_assign_query(&query) {
        return (StatusCode::BAD_REQUEST, "bad assign query").into_response();
    }
    let accelerator = query["accelerator"].clone();
    if mock.denied_accelerators.contains(&accelerator) {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "not entitled"}))).into_response();
    }
    if mock.assignments.len() >= mock.max_assignments {
        return (StatusCode::PRECONDITION_FAILED, Json(json!({"error": "too many assignments"})))
            .into_response();
    }
    let variant = match query["variant"].as_str() {
        "GPU" => 1,
        "TPU" => 2,
        _ => 0,
    };
    let shape = u8::from(query.get("shape").map(String::as_str) == Some("hm"));
    let assignment = mock.add_assignment(&accelerator, variant, shape);
    let body = assignment.to_json(&mock.base_url);
    xssi(body)
}

async fn unassign_token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(endpoint): Path<String>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    if mock.assignment(&endpoint).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = mock.next_id("unassign");
    mock.xsrf_tokens.insert(token.clone());
    xssi(json!({ "token": token }))
}

async fn unassign(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(endpoint): Path<String>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    let xsrf = headers.get("x-goog-colab-token").and_then(|value| value.to_str().ok());
    if !xsrf.is_some_and(|token| mock.xsrf_tokens.remove(token)) {
        return (StatusCode::FORBIDDEN, "missing or stale XSRF token").into_response();
    }
    let before = mock.assignments.len();
    mock.assignments.retain(|assignment| assignment.endpoint != endpoint);
    if mock.assignments.len() == before {
        return StatusCode::NOT_FOUND.into_response();
    }
    mock.runtimes.remove(&endpoint);
    mock.unassigned.push(endpoint);
    xssi(json!({}))
}

async fn keep_alive(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(endpoint): Path<String>,
) -> Response {
    let delay = {
        let mut mock = state.lock().expect("mock state");
        if !authorized(&mock, &headers) {
            return unauthorized();
        }
        if headers.get("x-colab-tunnel").and_then(|value| value.to_str().ok()) != Some("Google") {
            return (StatusCode::BAD_REQUEST, "X-Colab-Tunnel header missing").into_response();
        }
        if mock.assignment(&endpoint).is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }
        *mock.keepalives.entry(endpoint).or_default() += 1;
        mock.keepalive_delay_ms
    };
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    StatusCode::OK.into_response()
}

async fn ccu_info(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    let mut info = mock.ccu_info.clone();
    info["assignmentsCount"] = json!(mock.assignments.len());
    xssi(info)
}

async fn user_info(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    match &mock.user_info {
        Some(info) => Json(info.clone()).into_response(),
        None => (StatusCode::FORBIDDEN, Json(json!({"error": {"code": 403}}))).into_response(),
    }
}

async fn runtime_specs(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    match &mock.runtime_specs {
        Some(specs) => Json(specs.clone()).into_response(),
        None => StatusCode::FORBIDDEN.into_response(),
    }
}

async fn propagation_token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(endpoint): Path<String>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    if mock.assignment(&endpoint).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = mock.next_id("propagation");
    mock.xsrf_tokens.insert(token.clone());
    xssi(json!({ "token": token }))
}

async fn propagate(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(endpoint): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut mock = state.lock().expect("mock state");
    if !authorized(&mock, &headers) {
        return unauthorized();
    }
    // The XSRF token stays valid for both the dry run and the real POST.
    let xsrf = headers.get("x-goog-colab-token").and_then(|value| value.to_str().ok());
    if !xsrf.is_some_and(|token| mock.xsrf_tokens.contains(token)) {
        return (StatusCode::FORBIDDEN, "missing or stale XSRF token").into_response();
    }
    let auth_type = query.get("authtype").cloned().unwrap_or_default();
    let dry_run = query.get("dryrun").cloned().unwrap_or_default();
    mock.propagations.push((endpoint, auth_type, dry_run));
    if !mock.drive_consent {
        return xssi(json!({
            "success": false,
            "unauthorized_redirect_uri": "https://accounts.google.com/o/oauth2/consent?x=1&y=2",
        }));
    }
    xssi(json!({ "success": true }))
}
