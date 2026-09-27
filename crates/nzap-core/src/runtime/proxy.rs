//! HTTP client for one runtime's Jupyter server.
//!
//! Every Colab VM exposes a Jupyter Server behind the per-assignment proxy
//! in `runtimeProxyInfo`. Both upstream clients authenticate to it with the
//! proxy token as the `colab-runtime-proxy-token` query parameter *and* the
//! `X-Colab-Runtime-Proxy-Token` header (google-colab-cli `contents.py` /
//! `runtime.py`). Port of colab-studio's `RuntimeProxy`.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use reqwest::Method;
use serde_json::{json, Value};

use crate::config::{
    CLIENT_AGENT, CLIENT_AGENT_HEADER, DEFAULT_REQUEST_TIMEOUT, QUOTE_SAFE_NONE,
    RUNTIME_PROXY_TOKEN_HEADER, RUNTIME_PROXY_TOKEN_PARAM,
};
use crate::error::{Error, Result};

/// Python's `quote(path, safe='/')` for contents paths.
const QUOTE_PATH: &percent_encoding::AsciiSet = &QUOTE_SAFE_NONE.remove(b'/');

/// A contents path, stripped of surrounding slashes and percent-encoded.
pub fn quote_path(path: &str) -> String {
    percent_encoding::utf8_percent_encode(path.trim_matches('/'), QUOTE_PATH).to_string()
}

fn to_ws_scheme(url: &url::Url) -> &'static str {
    if url.scheme() == "https" {
        "wss"
    } else {
        "ws"
    }
}

#[derive(Clone)]
pub struct RuntimeProxy {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl RuntimeProxy {
    pub fn new(http: reqwest::Client, base_url: &str, token: &str) -> Self {
        Self { http, base_url: base_url.trim_end_matches('/').to_owned(), token: token.to_owned() }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The two headers the runtime proxy expects on every request.
    pub fn auth_headers(&self) -> [(&'static str, String); 2] {
        [
            (CLIENT_AGENT_HEADER, CLIENT_AGENT.to_owned()),
            (RUNTIME_PROXY_TOKEN_HEADER, self.token.clone()),
        ]
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        extra_query: &[(&str, &str)],
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<(Value, Vec<u8>)> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let mut request = self
            .http
            .request(method.clone(), &url)
            .timeout(timeout)
            .query(&[("authuser", "0"), (RUNTIME_PROXY_TOKEN_PARAM, &self.token)])
            .query(extra_query);
        for (name, value) in self.auth_headers() {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        // Error messages name the path only: the URL carries the token.
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::runtime(Some(404), format!("Not found: {path}")));
        }
        if !status.is_success() {
            return Err(Error::runtime(
                Some(status.as_u16()),
                format!(
                    "{method} {path} failed: {} {}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )
                .trim_end()
                .to_owned(),
            ));
        }
        let bytes = response.bytes().await?.to_vec();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        Ok((value, bytes))
    }

    async fn json(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value> {
        Ok(self.request(method, path, query, body, DEFAULT_REQUEST_TIMEOUT).await?.0)
    }

    /// `api/contents/<quoted>`; the root is `api/contents` (Jupyter's route
    /// accepts both forms, catch-all routers only the bare one).
    fn contents_path(path: &str) -> String {
        match quote_path(path) {
            quoted if quoted.is_empty() => "api/contents".to_owned(),
            quoted => format!("api/contents/{quoted}"),
        }
    }

    // ------------------------------------------------------------- contents

    /// A directory model (`type: directory`, `content: [...]`).
    pub async fn list_contents(&self, path: &str) -> Result<Value> {
        self.json(Method::GET, &Self::contents_path(path), &[], None).await
    }

    /// A file model including its content.
    pub async fn read_file(&self, path: &str) -> Result<Value> {
        self.json(Method::GET, &Self::contents_path(path), &[("content", "1")], None).await
    }

    /// The file's bytes (colab-cli `ContentsClient.download`): the model's
    /// `content`, base64-decoded for binary files, pretty JSON for notebooks.
    pub async fn download(&self, path: &str) -> Result<Vec<u8>> {
        let model = self.read_file(path).await?;
        if !model.is_object() {
            return Err(Error::runtime(None, format!("Unexpected contents response for {path}")));
        }
        if model.get("type").and_then(Value::as_str) == Some("directory") {
            return Err(Error::invalid(format!("Cannot download a directory: {path}")));
        }
        let content = model.get("content").cloned().unwrap_or(Value::Null);
        if model.get("format").and_then(Value::as_str) == Some("base64") {
            let encoded: String = content
                .as_str()
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            return STANDARD
                .decode(encoded)
                .map_err(|error| Error::runtime(None, format!("Corrupt base64 content: {error}")));
        }
        Ok(match content {
            Value::String(text) => text.into_bytes(),
            Value::Null => Vec::new(),
            notebook => pretty_json(&notebook),
        })
    }

    pub async fn write_file(&self, path: &str, content: &str, format: &str) -> Result<Value> {
        self.json(
            Method::PUT,
            &Self::contents_path(path),
            &[],
            Some(json!({ "type": "file", "format": format, "content": content })),
        )
        .await
    }

    pub async fn upload_file(&self, path: &str, data: &[u8]) -> Result<Value> {
        self.write_file(path, &STANDARD.encode(data), "base64").await
    }

    pub async fn make_directory(&self, path: &str) -> Result<Value> {
        self.json(
            Method::PUT,
            &Self::contents_path(path),
            &[],
            Some(json!({ "type": "directory" })),
        )
        .await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.json(Method::DELETE, &Self::contents_path(path), &[], None).await?;
        Ok(())
    }

    /// `PATCH api/contents` — VS Code's `colab.renameFile`.
    pub async fn rename(&self, path: &str, new_path: &str) -> Result<Value> {
        self.json(
            Method::PATCH,
            &Self::contents_path(path),
            &[],
            Some(json!({ "path": new_path.trim_matches('/') })),
        )
        .await
    }

    // -------------------------------------------------------------- kernels

    pub async fn list_kernels(&self) -> Result<Vec<Value>> {
        let value = self.json(Method::GET, "api/kernels", &[], None).await?;
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    pub async fn start_kernel(&self, name: &str) -> Result<Value> {
        self.json(Method::POST, "api/kernels", &[], Some(json!({ "name": name }))).await
    }

    /// Bind a named notebook session to a kernel. This is what makes the
    /// runtime show up with a human name on Colab's "Manage sessions" page
    /// (a bare kernel renders as "Unknown notebook"); colab-vscode does the
    /// same lookup in `assignments.ts`.
    pub async fn create_session(
        &self,
        session_name: &str,
        kernel_id: &str,
        kernel_name: &str,
    ) -> Result<Value> {
        self.json(
            Method::POST,
            "api/sessions",
            &[],
            Some(json!({
                "name": session_name,
                "path": format!("{session_name}.ipynb"),
                "type": "notebook",
                "kernel": { "id": kernel_id, "name": kernel_name },
            })),
        )
        .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<Value>> {
        let value = self.json(Method::GET, "api/sessions", &[], None).await?;
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    pub async fn restart_kernel(&self, kernel_id: &str) -> Result<Value> {
        self.json(
            Method::POST,
            &format!("api/kernels/{}/restart", quote_path(kernel_id)),
            &[],
            None,
        )
        .await
    }

    pub async fn interrupt_kernel(&self, kernel_id: &str) -> Result<()> {
        self.json(
            Method::POST,
            &format!("api/kernels/{}/interrupt", quote_path(kernel_id)),
            &[],
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn shutdown_kernel(&self, kernel_id: &str) -> Result<()> {
        self.json(Method::DELETE, &format!("api/kernels/{}", quote_path(kernel_id)), &[], None)
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------ websockets

    /// `{ws proxy}/api/kernels/{id}/channels` with the default (unversioned)
    /// Jupyter sub-protocol, as google-colab-cli `runtime.py` connects.
    pub fn kernel_ws_url(&self, kernel_id: &str, session_id: &str) -> Result<String> {
        let mut url = url::Url::parse(&format!(
            "{}/api/kernels/{}/channels",
            self.base_url,
            quote_path(kernel_id)
        ))
        .map_err(|error| Error::internal(format!("Bad runtime URL: {error}")))?;
        let scheme = to_ws_scheme(&url);
        url.set_scheme(scheme)
            .map_err(|()| Error::internal("Could not build the kernel socket URL."))?;
        url.query_pairs_mut()
            .append_pair("session_id", session_id)
            .append_pair(RUNTIME_PROXY_TOKEN_PARAM, &self.token)
            .append_pair("authuser", "0");
        Ok(url.into())
    }

    /// `{ws scheme}://{proxy host}/colab/tty` (VS Code's terminal URL). The
    /// token goes in the header only: verified against a live runtime, the
    /// endpoint answers 404 when it gets both, or `authuser`.
    pub fn tty_url(&self) -> Result<String> {
        let mut url = url::Url::parse(&self.base_url)
            .map_err(|error| Error::internal(format!("Bad runtime URL: {error}")))?;
        let scheme = to_ws_scheme(&url);
        url.set_scheme(scheme)
            .map_err(|()| Error::internal("Could not build the terminal URL."))?;
        url.set_path("/colab/tty");
        url.set_query(None);
        url.set_fragment(None);
        Ok(url.into())
    }
}

/// `json.dumps(value, indent=1)`, as colab-studio writes notebooks.
pub(crate) fn pretty_json(value: &Value) -> Vec<u8> {
    use serde::Serialize as _;
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    if value.serialize(&mut serializer).is_err() {
        return value.to_string().into_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_quoted_like_python() {
        assert_eq!(quote_path("/content/my file.txt/"), "content/my%20file.txt");
        assert_eq!(quote_path("a/b_c-d.e~f"), "a/b_c-d.e~f");
        assert_eq!(quote_path("x?y#z"), "x%3Fy%23z");
        assert_eq!(quote_path(""), "");
    }

    #[test]
    fn socket_urls() {
        let proxy = RuntimeProxy::new(reqwest::Client::new(), "https://proxy.example/base/", "tok");
        let kernel = proxy.kernel_ws_url("k-1", "s-1").unwrap();
        assert!(kernel.starts_with("wss://proxy.example/base/api/kernels/k-1/channels?"));
        assert!(kernel.contains("session_id=s-1"));
        assert!(kernel.contains("colab-runtime-proxy-token=tok"));
        assert!(kernel.contains("authuser=0"));
        assert_eq!(proxy.tty_url().unwrap(), "wss://proxy.example/colab/tty");

        let local = RuntimeProxy::new(reqwest::Client::new(), "http://127.0.0.1:9/proxy/ep", "t");
        assert_eq!(local.tty_url().unwrap(), "ws://127.0.0.1:9/colab/tty");
    }

    #[test]
    fn notebooks_are_written_with_one_space_indent() {
        let bytes = pretty_json(&json!({"cells": [1]}));
        assert_eq!(String::from_utf8(bytes).unwrap(), "{\n \"cells\": [\n  1\n ]\n}");
    }
}
