//! Google OAuth 2.0 + OpenID userinfo.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::state::IssuedCode;
use crate::Shared;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/o/oauth2/v2/auth", get(consent))
        .route("/token", post(token))
        .route("/revoke", post(revoke))
        .route("/v1/userinfo", get(userinfo))
}

fn grant_error(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

/// The consent screen, auto-approved: redirects straight back to
/// `redirect_uri` with a single-use code bound to the PKCE challenge.
async fn consent(
    State(state): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let get = |key: &str| query.get(key).cloned().unwrap_or_default();
    let redirect_uri = get("redirect_uri");
    if redirect_uri.is_empty() || get("client_id").is_empty() || get("response_type") != "code" {
        return (StatusCode::BAD_REQUEST, "invalid_request").into_response();
    }
    if get("code_challenge_method") != "S256" || get("code_challenge").is_empty() {
        return (StatusCode::BAD_REQUEST, "PKCE S256 is required").into_response();
    }
    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let state_param = get("state");

    let mut mock = state.lock().expect("mock state");
    if mock.deny_consent {
        return Redirect::to(&format!(
            "{redirect_uri}{separator}error=access_denied&state={state_param}"
        ))
        .into_response();
    }
    let code = mock.next_id("code");
    mock.codes.insert(
        code.clone(),
        IssuedCode {
            challenge: get("code_challenge"),
            redirect_uri: redirect_uri.clone(),
            client_id: get("client_id"),
        },
    );
    Redirect::to(&format!("{redirect_uri}{separator}code={code}&state={state_param}")).into_response()
}

async fn token(
    State(state): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let get = |key: &str| form.get(key).cloned().unwrap_or_default();
    let mut mock = state.lock().expect("mock state");
    match get("grant_type").as_str() {
        "authorization_code" => {
            // Codes are single-use, like Google's.
            let Some(issued) = mock.codes.remove(&get("code")) else {
                return grant_error("invalid_grant");
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
            if challenge != issued.challenge
                || get("redirect_uri") != issued.redirect_uri
                || get("client_id") != issued.client_id
            {
                return grant_error("invalid_grant");
            }
            let access_token = mock.issue_access_token();
            let refresh_token = mock.next_id("refresh");
            mock.refresh_tokens.insert(refresh_token.clone());
            Json(json!({
                "access_token": access_token,
                "refresh_token": refresh_token,
                "expires_in": mock.access_ttl,
                "scope": "openid https://www.googleapis.com/auth/colaboratory",
                "token_type": "Bearer",
            }))
            .into_response()
        }
        "refresh_token" => {
            let presented = get("refresh_token");
            if !mock.refresh_tokens.contains(&presented) {
                return grant_error("invalid_grant");
            }
            mock.refresh_count += 1;
            let access_token = mock.issue_access_token();
            let mut body = json!({
                "access_token": access_token,
                "expires_in": mock.access_ttl,
                "token_type": "Bearer",
            });
            if mock.rotate_refresh_tokens {
                mock.refresh_tokens.remove(&presented);
                let rotated = mock.next_id("refresh");
                mock.refresh_tokens.insert(rotated.clone());
                body["refresh_token"] = json!(rotated);
            }
            Json(body).into_response()
        }
        _ => grant_error("unsupported_grant_type"),
    }
}

async fn revoke(
    State(state): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let token = form.get("token").cloned().unwrap_or_default();
    let mut mock = state.lock().expect("mock state");
    let known = mock.refresh_tokens.remove(&token) | mock.access_tokens.remove(&token);
    if known {
        mock.revoked.push(token);
        StatusCode::OK.into_response()
    } else {
        grant_error("invalid_token")
    }
}

async fn userinfo(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mock = state.lock().expect("mock state");
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if !mock.is_authorized(authorization) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(mock.user.clone()).into_response()
}
