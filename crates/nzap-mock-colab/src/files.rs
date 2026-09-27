//! Google Drive v3 file reads and a static file host.
//!
//! * `/drive/v3/files/{id}` — `?fields=name` metadata and `?alt=media`
//!   content, bearer-protected, 404 for unknown ids (what Drive answers
//!   outside the `drive.file` scope).
//! * `/static/{*path}` — anything a test wants served over plain HTTP (the
//!   GitHub notebook catalog, links to import), with ETag / 304 support and
//!   `REDIRECT:<url>` entries that answer 302.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::Shared;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/drive/v3/files/{id}", get(drive_file))
        .route("/static/{*path}", get(static_file))
}

async fn drive_file(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mock = state.lock().expect("mock state");
    if !mock.is_authorized(headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some((name, content)) = mock.drive_files.get(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": {"code": 404}}))).into_response();
    };
    if query.get("alt").map(String::as_str) == Some("media") {
        return content.clone().into_response();
    }
    Json(json!({ "name": name })).into_response()
}

async fn static_file(State(state): State<Shared>, Path(path): Path<String>, headers: HeaderMap) -> Response {
    let mut mock = state.lock().expect("mock state");
    *mock.static_hits.entry(path.clone()).or_default() += 1;
    let Some(content) = mock.static_files.get(&path).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(target) = content.strip_prefix("REDIRECT:") {
        return (StatusCode::FOUND, [(header::LOCATION, target.to_owned())]).into_response();
    }
    let etag = format!("\"{:x}\"", Sha256::digest(content.as_bytes()));
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag);
    if matches {
        mock.static_not_modified += 1;
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    ([(header::ETAG, etag)], content).into_response()
}
