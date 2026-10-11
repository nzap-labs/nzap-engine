//! GitHub Codespaces as compute, with the user's own GitHub sign-in.
//!
//! The Codespaces REST API creates, starts, stops and deletes codespaces but
//! cannot run commands in one, so the codespace's dev container starts a
//! Jupyter Server itself (`runtimes/codespaces/` in this repository is a
//! ready-made one). NZAP reaches it on the forwarded port
//! `https://{name}-8888.app.github.dev`, which stays private: GitHub only
//! lets requests through that carry the owner's token in `X-Github-Token`.
//!
//! Sign-in: GitHub's OAuth device flow (with an OAuth app client id from
//! Settings), the GitHub CLI's existing login (`gh auth token`), or a
//! pasted token. Tokens need the `codespace` scope (classic) or the
//! Codespaces read/write permission (fine-grained).

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::Endpoints;
use crate::error::{Error, Result};
use crate::secrets::SecretStore;

pub const TOKEN_KEY: &str = "github-token";
/// The port the runtime's dev container serves Jupyter on.
pub const JUPYTER_PORT: u16 = 8888;
const API_VERSION: &str = "2022-11-28";
const SCOPES: &str = "codespace read:user";

fn error(status: Option<u16>, message: impl Into<String>) -> Error {
    Error::provider("codespaces", status, message)
}

pub fn not_connected() -> Error {
    error(None, "Connect GitHub first (Settings → Compute providers).")
}

/// Who the token belongs to.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubUser {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// A device-flow sign-in waiting for the user to enter `user_code`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLogin {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(skip)]
    pub device_code: String,
    #[serde(skip)]
    pub interval: u64,
    #[serde(skip)]
    pub client_id: String,
}

pub struct Codespaces {
    http: reqwest::Client,
    api: String,
    web: String,
    port_url: String,
    secrets: Arc<dyn SecretStore>,
    /// How often state is polled while waiting (shortened in tests).
    poll_interval: std::sync::Mutex<Duration>,
}

/// Fill `{name}` and `{port}` in a forwarded-port URL template.
pub fn port_url(template: &str, name: &str, port: u16) -> String {
    template.replace("{name}", name).replace("{port}", &port.to_string())
}

impl Codespaces {
    pub fn new(
        http: reqwest::Client,
        endpoints: &Endpoints,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            http,
            api: endpoints.github_api.trim_end_matches('/').to_owned(),
            web: endpoints.github_web.trim_end_matches('/').to_owned(),
            port_url: endpoints.codespaces_port_url.clone(),
            secrets,
            poll_interval: std::sync::Mutex::new(Duration::from_secs(3)),
        }
    }

    pub fn set_poll_interval(&self, interval: Duration) {
        *self.poll_interval.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = interval;
    }

    fn poll_interval(&self) -> Duration {
        *self.poll_interval.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ------------------------------------------------------------- the token

    pub fn token(&self) -> Option<String> {
        self.secrets.get(TOKEN_KEY).ok().flatten().filter(|token| !token.is_empty())
    }

    fn require_token(&self) -> Result<String> {
        self.token().ok_or_else(not_connected)
    }

    /// Check a token with GitHub, then keep it in the keychain.
    pub async fn set_token(&self, token: &str) -> Result<GithubUser> {
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::invalid("Paste a GitHub token."));
        }
        let user = self.user_for(token).await?;
        self.secrets.set(TOKEN_KEY, token)?;
        Ok(user)
    }

    pub fn disconnect(&self) -> Result<()> {
        self.secrets.delete(TOKEN_KEY)
    }

    /// The connected account (`None` when no token is stored).
    pub async fn user(&self) -> Result<Option<GithubUser>> {
        match self.token() {
            Some(token) => self.user_for(&token).await.map(Some),
            None => Ok(None),
        }
    }

    async fn user_for(&self, token: &str) -> Result<GithubUser> {
        let response = self.send(token, Method::GET, "/user", None).await?;
        // Classic tokens list their scopes; fine-grained ones do not.
        let scopes = response
            .headers()
            .get("x-oauth-scopes")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body: Value = Self::json(response).await?;
        if let Some(scopes) = scopes {
            if !scopes.split(',').any(|scope| scope.trim() == "codespace") {
                return Err(error(
                    Some(403),
                    "This GitHub token cannot manage codespaces: it needs the `codespace` scope. \
                     With the GitHub CLI: gh auth refresh -h github.com -s codespace",
                ));
            }
        }
        Ok(GithubUser {
            login: body["login"].as_str().unwrap_or_default().to_owned(),
            name: body["name"].as_str().map(str::to_owned),
            avatar_url: body["avatar_url"].as_str().map(str::to_owned),
        })
    }

    /// Use the GitHub CLI's login (`gh auth token`).
    pub async fn import_gh_cli(&self) -> Result<GithubUser> {
        let output = tokio::process::Command::new("gh")
            .args(["auth", "token", "--hostname", "github.com"])
            .output()
            .await
            .map_err(|_| {
                error(
                    None,
                    "The GitHub CLI (gh) is not installed. Install it, or sign in another way.",
                )
            })?;
        let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || token.is_empty() {
            return Err(error(
                None,
                "The GitHub CLI is not signed in. Run: gh auth login -s codespace",
            ));
        }
        self.set_token(&token).await
    }

    /// Start GitHub's device flow with an OAuth app's client id.
    pub async fn begin_device(&self, client_id: &str) -> Result<DeviceLogin> {
        let response = self
            .http
            .post(format!("{}/login/device/code", self.web))
            .header("Accept", "application/json")
            .form(&[("client_id", client_id), ("scope", SCOPES)])
            .send()
            .await?;
        let body: Value = Self::json(response).await?;
        let text = |key: &str| body[key].as_str().map(str::to_owned);
        match (text("device_code"), text("user_code"), text("verification_uri")) {
            (Some(device_code), Some(user_code), Some(verification_uri)) => Ok(DeviceLogin {
                user_code,
                verification_uri,
                expires_in: body["expires_in"].as_u64().unwrap_or(900),
                device_code,
                interval: body["interval"].as_u64().unwrap_or(5).max(1),
                client_id: client_id.to_owned(),
            }),
            _ => Err(error(
                None,
                format!(
                    "GitHub refused the sign-in: {}",
                    body["error_description"].as_str().unwrap_or("check the OAuth app's client id")
                ),
            )),
        }
    }

    /// Wait until the user approves the device login, then store the token.
    pub async fn finish_device(&self, login: &DeviceLogin) -> Result<GithubUser> {
        let mut interval = Duration::from_secs(login.interval);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(login.expires_in);
        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(error(None, "The GitHub sign-in expired. Start again."));
            }
            // GitHub's interval, unless polling was shortened (tests).
            let poll = self.poll_interval();
            let wait = if poll < Duration::from_secs(1) { poll } else { interval };
            tokio::time::sleep(wait).await;
            let response = self
                .http
                .post(format!("{}/login/oauth/access_token", self.web))
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", login.client_id.as_str()),
                    ("device_code", login.device_code.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await?;
            let body: Value = Self::json(response).await?;
            if let Some(token) = body["access_token"].as_str() {
                return self.set_token(token).await;
            }
            match body["error"].as_str() {
                Some("authorization_pending") => {}
                Some("slow_down") => interval += Duration::from_secs(5),
                Some("access_denied") => {
                    return Err(error(None, "The GitHub sign-in was declined."))
                }
                Some("expired_token") => {
                    return Err(error(None, "The GitHub sign-in expired. Start again."))
                }
                other => {
                    return Err(error(
                        None,
                        format!("GitHub sign-in failed: {}", other.unwrap_or("unknown error")),
                    ))
                }
            }
        }
    }

    // ------------------------------------------------------------ REST calls

    async fn send(
        &self,
        token: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.api))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .timeout(Duration::from_secs(60));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let message = response
            .json::<Value>()
            .await
            .ok()
            .and_then(|body| body["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_owned());
        let text = match status {
            StatusCode::UNAUTHORIZED => {
                "GitHub rejected the stored token. Connect GitHub again (Settings → Compute providers)."
                    .to_owned()
            }
            StatusCode::NOT_FOUND if path.starts_with("/repos/") => format!(
                "GitHub cannot find the runtime repository ({message}). Check Settings → Compute \
                 providers → Codespaces repository."
            ),
            _ => format!("GitHub Codespaces: {message}"),
        };
        Err(error(Some(status.as_u16()), text))
    }

    async fn json(response: reqwest::Response) -> Result<Value> {
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let token = self.require_token()?;
        Self::json(self.send(&token, method, path, body).await?).await
    }

    /// Machine types the repository's codespaces can use.
    pub async fn machines(&self, repository: &str) -> Result<Vec<Value>> {
        let body = self
            .call(Method::GET, &format!("/repos/{repository}/codespaces/machines"), None)
            .await?;
        Ok(body["machines"].as_array().cloned().unwrap_or_default())
    }

    /// `POST /repos/{owner}/{repo}/codespaces`. `retention_minutes` makes
    /// GitHub delete it on its own once it has been idle that long.
    pub async fn create(
        &self,
        repository: &str,
        display_name: &str,
        machine: &str,
        devcontainer_path: &str,
        idle_timeout_minutes: u64,
        retention_minutes: Option<u64>,
    ) -> Result<Value> {
        let mut body = json!({
            "machine": machine,
            "devcontainer_path": devcontainer_path,
            "display_name": display_name.chars().take(48).collect::<String>(),
            "idle_timeout_minutes": idle_timeout_minutes.clamp(5, 240),
        });
        if let Some(minutes) = retention_minutes {
            body["retention_period_minutes"] = json!(minutes);
        }
        self.call(Method::POST, &format!("/repos/{repository}/codespaces"), Some(body)).await
    }

    pub async fn get(&self, name: &str) -> Result<Value> {
        self.call(Method::GET, &format!("/user/codespaces/{name}"), None).await
    }

    pub async fn start(&self, name: &str) -> Result<Value> {
        self.call(Method::POST, &format!("/user/codespaces/{name}/start"), None).await
    }

    pub async fn stop(&self, name: &str) -> Result<Value> {
        self.call(Method::POST, &format!("/user/codespaces/{name}/stop"), None).await
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        self.call(Method::DELETE, &format!("/user/codespaces/{name}"), None).await.map(|_| ())
    }

    /// Wait until the codespace is `Available`, starting it if it stopped.
    pub async fn wait_available(&self, name: &str, timeout: Duration) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut started = false;
        loop {
            let codespace = self.get(name).await?;
            match codespace["state"].as_str().unwrap_or("Unknown") {
                "Available" => return Ok(codespace),
                "Shutdown" if !started => {
                    self.start(name).await?;
                    started = true;
                }
                state @ ("Failed" | "Deleted" | "Archived" | "Unavailable" | "Moved") => {
                    return Err(error(None, format!("The codespace is {state}.")))
                }
                _ => {}
            }
            if tokio::time::Instant::now() > deadline {
                return Err(error(None, "The codespace did not become available in time."));
            }
            tokio::time::sleep(self.poll_interval()).await;
        }
    }

    /// Where the codespace's Jupyter Server answers, and the header that
    /// lets requests through its private port.
    pub fn jupyter_target(&self, name: &str) -> Result<(String, Vec<(&'static str, String)>)> {
        let token = self.require_token()?;
        Ok((port_url(&self.port_url, name, JUPYTER_PORT), vec![("X-Github-Token", token)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_port_urls() {
        assert_eq!(
            port_url("https://{name}-{port}.app.github.dev", "fluffy-space-x9", 8888),
            "https://fluffy-space-x9-8888.app.github.dev"
        );
        assert_eq!(
            port_url("http://127.0.0.1:9/proxy/{name}", "cs", 8888),
            "http://127.0.0.1:9/proxy/cs"
        );
    }
}
