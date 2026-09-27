//! HTTP plumbing shared by every Google-facing client.

use std::time::Duration;

use serde_json::Value;

use crate::config::XSSI_PREFIX;
use crate::error::{Error, Result};

/// The engine's shared HTTP client: rustls, HTTP/2, a connect timeout and
/// an honest user agent. Per-request timeouts are set by callers.
pub fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("nzap-engine/{}", crate::VERSION))
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| Error::internal(format!("Could not build the HTTP client: {error}")))
}

/// Strip Colab's XSSI guard (`)]}'\n`) when present.
pub fn strip_xssi(text: &str) -> &str {
    text.strip_prefix(XSSI_PREFIX).unwrap_or(text)
}

/// Parse a Colab response body: XSSI-stripped JSON, or `None` when empty.
pub fn parse_json_body(text: &str) -> Result<Option<Value>> {
    let body = strip_xssi(text);
    if body.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(body)?))
}

/// Like [`parse_json_body`] but falls back to the raw text as a JSON string
/// (colab-studio's `expect_json=False` for the propagation endpoints).
pub fn parse_lenient_body(text: &str) -> Value {
    let body = strip_xssi(text);
    if body.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_owned()))
}

/// Pull the `accounts.google.com` consent URL out of an error body
/// (colab-studio `ColabClient._extract_redirect`). The URL ends at
/// whitespace, a quote or a backslash, except that JSON-escaped ampersands
/// (`&`) are unescaped and kept.
pub fn extract_consent_redirect(body: &str) -> Option<String> {
    const PREFIX: &str = "https://accounts.google.com/";
    const ESCAPED_AMP: &str = "\\u0026";
    let start = body.find(PREFIX)?;
    let mut rest = &body[start..];
    let mut url = String::new();
    while let Some(c) = rest.chars().next() {
        if rest.starts_with(ESCAPED_AMP) {
            url.push('&');
            rest = &rest[ESCAPED_AMP.len()..];
            continue;
        }
        if c.is_whitespace() || c == '"' || c == '\\' {
            break;
        }
        url.push(c);
        rest = &rest[c.len_utf8()..];
    }
    (url.len() > PREFIX.len()).then_some(url)
}

/// Map a non-success Colab response to an [`Error`], keeping the body for
/// callers that inspect it.
pub fn colab_error(method: &str, path: &str, status: reqwest::StatusCode, body: String) -> Error {
    if status == reqwest::StatusCode::PRECONDITION_FAILED {
        return Error::TooManyAssignments;
    }
    let reason = status.canonical_reason().unwrap_or("");
    Error::Colab {
        status: Some(status.as_u16()),
        message: format!("{method} {path} failed: {} {reason}", status.as_u16())
            .trim_end()
            .to_owned(),
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xssi_prefix_is_stripped() {
        assert_eq!(strip_xssi(")]}'\n{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_xssi("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(
            parse_json_body(")]}'\n{\"token\":\"t\"}").ok().flatten(),
            Some(serde_json::json!({"token": "t"}))
        );
        assert_eq!(parse_json_body(")]}'\n").ok().flatten(), None);
        assert!(parse_json_body("not json").is_err());
        assert_eq!(parse_lenient_body("plain"), Value::String("plain".into()));
    }

    #[test]
    fn consent_redirect_is_extracted_and_unescaped() {
        let body = r#"{"error":"x","url":"https://accounts.google.com/o/oauth2/auth?a=1&b=2"}"#;
        assert_eq!(
            extract_consent_redirect(body).as_deref(),
            Some("https://accounts.google.com/o/oauth2/auth?a=1&b=2")
        );
        assert_eq!(extract_consent_redirect("no link here"), None);
        assert_eq!(
            extract_consent_redirect("see https://accounts.google.com/x then"),
            Some("https://accounts.google.com/x".to_owned())
        );
    }

    #[test]
    fn precondition_failed_means_too_many_assignments() {
        let error = colab_error(
            "POST",
            "/tun/m/assign",
            reqwest::StatusCode::PRECONDITION_FAILED,
            String::new(),
        );
        assert!(matches!(error, Error::TooManyAssignments));
        let error = colab_error("GET", "/x", reqwest::StatusCode::FORBIDDEN, "b".into());
        assert_eq!(error.to_string(), "GET /x failed: 403 Forbidden");
        assert_eq!(error.status(), Some(403));
    }
}
