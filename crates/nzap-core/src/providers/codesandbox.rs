//! CodeSandbox VM sandboxes as compute, with the user's API key.
//!
//! Endpoints and shapes follow the official SDK (`@codesandbox/sdk` 2.x,
//! `API.ts` and its generated client): every response is `{data: …}` (or
//! `{errors: […]}`), authenticated with `Authorization: Bearer <key>`.
//!
//! * `POST /sandbox/{template}/fork` — a new private sandbox from a template
//! * `POST /vm/{id}/start` — boot (or wake) its VM; returns the VM agent's
//!   address and token
//! * `POST /sandbox/{id}/tokens` — a preview token for its private ports
//! * `POST /vm/{id}/hibernate`, `POST /vm/{id}/shutdown`, `DELETE /vm/{id}`
//! * `GET /vm/running` — the account's running VMs and its limit
//!
//! After boot NZAP asks the VM agent to install and start a Jupyter Server
//! on port 8888 (with its own random token), then reaches it on the private
//! preview host `https://{id}-8888.csb.app` with the `csb-preview-token`
//! header — the SDK's `HostTokens.getHeaders`.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

use super::pitcher;
use crate::config::Endpoints;
use crate::error::{Error, Result};
use crate::secrets::SecretStore;

pub const KEY_NAME: &str = "codesandbox-api-key";
pub const JUPYTER_PORT: u16 = 8888;
/// Preview tokens live this long; the VM is deleted long before.
const PREVIEW_TOKEN_DAYS: i64 = 30;

fn error(status: Option<u16>, message: impl Into<String>) -> Error {
    Error::provider("codesandbox", status, message)
}

pub fn not_connected() -> Error {
    error(None, "Connect CodeSandbox first: add an API key in Settings → Compute providers.")
}

/// What `POST /vm/{id}/start` returns that NZAP uses.
#[derive(Clone, Debug, PartialEq)]
pub struct VmStart {
    pub pitcher_url: String,
    pub pitcher_token: String,
    pub workspace_path: String,
    pub bootup_type: String,
}

/// The shell command that makes a sandbox serve Jupyter on port 8888:
/// installed into the user's Python if needed, detached from the agent's
/// shell, protected by `token` behind the private preview host. It also
/// makes `/content` (the working directory jobs and apps use, as on Colab).
pub fn jupyter_command(token: &str) -> String {
    format!(
        "(mkdir -p /content 2>/dev/null || (sudo mkdir -p /content && sudo chown \"$(id -u)\" /content)); \
         (python3 -c 'import jupyter_server, ipykernel' 2>/dev/null || \
         python3 -m pip install --user --quiet --disable-pip-version-check jupyter_server ipykernel \
         || python3 -m pip install --user --quiet --break-system-packages jupyter_server ipykernel) \
         > /tmp/nzap-jupyter-install.log 2>&1; \
         setsid nohup python3 -m jupyter server --ip=0.0.0.0 --port={JUPYTER_PORT} --no-browser \
         --ServerApp.root_dir=/ --ServerApp.allow_origin='*' --ServerApp.allow_remote_access=True \
         --IdentityProvider.token={token} > /tmp/nzap-jupyter.log 2>&1 &"
    )
}

pub struct CodeSandbox {
    http: reqwest::Client,
    api: String,
    preview_url: String,
    secrets: Arc<dyn SecretStore>,
}

/// Fill `{id}` and `{port}` in a preview-host template.
pub fn preview_url(template: &str, id: &str, port: u16) -> String {
    template.replace("{id}", id).replace("{port}", &port.to_string())
}

impl CodeSandbox {
    pub fn new(
        http: reqwest::Client,
        endpoints: &Endpoints,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            http,
            api: endpoints.codesandbox_api.trim_end_matches('/').to_owned(),
            preview_url: endpoints.codesandbox_preview_url.clone(),
            secrets,
        }
    }

    pub fn api_key(&self) -> Option<String> {
        self.secrets.get(KEY_NAME).ok().flatten().filter(|key| !key.is_empty())
    }

    /// Check a key against the API, then keep it in the keychain.
    pub async fn set_api_key(&self, key: &str) -> Result<Value> {
        let key = key.trim();
        if key.is_empty() {
            return Err(Error::invalid("Paste a CodeSandbox API key."));
        }
        let running = self.call_with(key, Method::GET, "/vm/running", None).await?;
        self.secrets.set(KEY_NAME, key)?;
        Ok(running)
    }

    pub fn disconnect(&self) -> Result<()> {
        self.secrets.delete(KEY_NAME)
    }

    /// Running VMs and the workspace's concurrency limit.
    pub async fn running(&self) -> Result<Value> {
        self.call(Method::GET, "/vm/running", None).await
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.api_key().ok_or_else(not_connected)?;
        self.call_with(&key, method, path, body).await
    }

    async fn call_with(
        &self,
        key: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.api))
            .bearer_auth(key)
            .timeout(Duration::from_secs(120));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body.get("data").cloned().unwrap_or(body));
        }
        let detail = body["errors"]
            .as_array()
            .and_then(|errors| errors.first())
            .map(|first| first.as_str().map(str::to_owned).unwrap_or_else(|| first.to_string()))
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_owned());
        let message = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => format!(
                "CodeSandbox rejected the API key ({detail}). Add a new key in Settings → Compute \
                 providers."
            ),
            StatusCode::NOT_FOUND => format!("CodeSandbox: not found ({detail})."),
            StatusCode::TOO_MANY_REQUESTS => {
                format!("CodeSandbox is rate limiting or the VM limit is reached ({detail}).")
            }
            _ => format!("CodeSandbox: {detail}"),
        };
        Err(error(Some(status.as_u16()), message))
    }

    /// A new private sandbox (private preview hosts too) from `template`.
    pub async fn fork(&self, template: &str, title: &str) -> Result<String> {
        let body = json!({
            "title": title,
            "privacy": 2,
            "private_preview": true,
            "tags": ["nzap"],
            "path": "/NZAP",
        });
        let data =
            self.call(Method::POST, &format!("/sandbox/{template}/fork"), Some(body)).await?;
        data["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| error(None, "CodeSandbox did not return the new sandbox's id."))
    }

    /// Boot or wake the VM. Requests to its preview host wake it again after
    /// it hibernates.
    pub async fn start(&self, id: &str, tier: &str, hibernation_seconds: u64) -> Result<VmStart> {
        let body = json!({
            "tier": tier,
            "hibernation_timeout_seconds": hibernation_seconds.max(60),
            "automatic_wakeup_config": { "http": true, "websocket": true },
        });
        let mut last = None;
        // The SDK retries starts three times as well.
        for attempt in 0..3 {
            match self.call(Method::POST, &format!("/vm/{id}/start"), Some(body.clone())).await {
                Ok(data) => {
                    let text = |key: &str| data[key].as_str().unwrap_or_default().to_owned();
                    if text("pitcher_url").is_empty() {
                        return Err(error(
                            None,
                            "CodeSandbox did not return the VM agent's address.",
                        ));
                    }
                    return Ok(VmStart {
                        pitcher_url: text("pitcher_url"),
                        pitcher_token: text("pitcher_token"),
                        workspace_path: text("workspace_path"),
                        bootup_type: text("bootup_type"),
                    });
                }
                Err(error @ Error::Provider { status: Some(401 | 403 | 404), .. }) => {
                    return Err(error)
                }
                Err(error) => {
                    tracing::info!("CodeSandbox start attempt {} failed: {error}", attempt + 1);
                    last = Some(error);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| error(None, "CodeSandbox did not start the VM.")))
    }

    /// A token for the sandbox's private preview hosts.
    pub async fn preview_token(&self, id: &str) -> Result<String> {
        let expires = chrono::Utc::now() + chrono::Duration::days(PREVIEW_TOKEN_DAYS);
        let data = self
            .call(
                Method::POST,
                &format!("/sandbox/{id}/tokens"),
                Some(json!({ "expires_at": expires.to_rfc3339() })),
            )
            .await?;
        data["token"]["token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| error(None, "CodeSandbox did not return a preview token."))
    }

    /// Start Jupyter in the VM through its agent.
    pub async fn start_jupyter(&self, vm: &VmStart, jupyter_token: &str) -> Result<()> {
        let cwd = if vm.workspace_path.is_empty() { "/project" } else { &vm.workspace_path };
        pitcher::run_command(
            &vm.pitcher_url,
            &vm.pitcher_token,
            &jupyter_command(jupyter_token),
            cwd,
        )
        .await
    }

    pub async fn hibernate(&self, id: &str) -> Result<()> {
        self.call(Method::POST, &format!("/vm/{id}/hibernate"), Some(json!({}))).await.map(|_| ())
    }

    pub async fn shutdown(&self, id: &str) -> Result<()> {
        self.call(Method::POST, &format!("/vm/{id}/shutdown"), Some(json!({}))).await.map(|_| ())
    }

    /// Permanently delete the sandbox and its VM.
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.call(Method::DELETE, &format!("/vm/{id}"), None).await.map(|_| ())
    }

    /// The preview host serving Jupyter, and the headers that open it.
    pub fn jupyter_target(
        &self,
        id: &str,
        preview_token: &str,
        jupyter_token: &str,
    ) -> (String, Vec<(&'static str, String)>) {
        (
            preview_url(&self.preview_url, id, JUPYTER_PORT),
            vec![
                ("csb-preview-token", preview_token.to_owned()),
                ("Authorization", format!("token {jupyter_token}")),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_hosts_and_the_jupyter_command() {
        assert_eq!(
            preview_url("https://{id}-{port}.csb.app", "abc123", 8888),
            "https://abc123-8888.csb.app"
        );
        let command = jupyter_command("t0k");
        assert!(command.contains("--IdentityProvider.token=t0k"));
        assert!(command.contains("--port=8888"));
        assert!(command.contains("setsid nohup"));
        assert!(command.starts_with("(mkdir -p /content"));
        assert!(!command.contains('\n'));
    }
}
