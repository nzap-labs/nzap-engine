//! The Colab control-plane client.
//!
//! Port of colab-studio's `ColabClient`, itself the request layer of
//! `google-colab-cli/src/colab_cli/client.py` merged with the extra RPCs of
//! `colab-vscode/src/colab/client/v1/index.ts`. The wire contract:
//!
//! * every response may carry the XSSI guard `)]}'\n` — stripped;
//! * `Accept: application/json` + `X-Colab-Client-Agent` on every call;
//! * requests to the Colab front door add `authuser=0`;
//! * allocating a VM is a two-step dance: `GET /tun/m/assign` returns an
//!   XSRF token that the `POST` must replay in `X-Goog-Colab-Token`.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::ACCEPT;
use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthManager;
use crate::config::{
    self, Accelerator, RuntimeRequest, Shape, CLIENT_AGENT, CLIENT_AGENT_HEADER,
    DEFAULT_REQUEST_TIMEOUT, KEEP_ALIVE_TIMEOUT, RUNTIME_PROXY_TOKEN_HEADER,
    RUNTIME_PROXY_TOKEN_PARAM, TUNNEL_HEADER, TUNNEL_HEADER_VALUE, TUN_ENDPOINT, XSRF_HEADER,
};
use crate::error::{Error, Result};
use crate::http::{colab_error, extract_consent_redirect, parse_json_body, parse_lenient_body};

/// What the VM asks credentials for when it sends a `colab_request`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AuthType {
    /// `drive.mount()`
    #[serde(rename = "dfs_ephemeral")]
    DfsEphemeral,
    /// `google.colab.auth.authenticate_user()`
    #[serde(rename = "auth_user_ephemeral")]
    AuthUserEphemeral,
}

impl AuthType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DfsEphemeral => "dfs_ephemeral",
            Self::AuthUserEphemeral => "auth_user_ephemeral",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dfs_ephemeral" => Some(Self::DfsEphemeral),
            "auth_user_ephemeral" => Some(Self::AuthUserEphemeral),
            _ => None,
        }
    }

    /// What the user is granting, for UI copy.
    pub fn label(self) -> &'static str {
        match self {
            Self::DfsEphemeral => "Google Drive",
            Self::AuthUserEphemeral => "Google Cloud",
        }
    }
}

/// Outcome of a credential propagation attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Propagation {
    pub success: bool,
    /// The consent page to open when the user has not granted access yet.
    pub unauthorized_redirect_uri: Option<String>,
}

/// google-colab-cli run.py: the friendly message for an accelerator the
/// account cannot use.
pub fn quota_message(accelerator: Accelerator) -> String {
    format!(
        "Colab rejected accelerator '{accelerator}'. You may not have quota or entitlement \
         for it on your account. Try a different one (e.g. T4) or pick CPU."
    )
}

/// Colab endpoints are opaque ids like `gpu-t4-s-abc123`; anything else is
/// refused before it reaches a URL path.
pub fn validate_endpoint(endpoint: &str) -> Result<&str> {
    let valid = !endpoint.is_empty()
        && endpoint.len() <= 200
        && endpoint.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(endpoint)
    } else {
        Err(Error::invalid("Invalid runtime endpoint."))
    }
}

#[derive(Default)]
struct Call {
    query: Vec<(&'static str, String)>,
    headers: Vec<(&'static str, String)>,
    json: Option<Value>,
    /// Send `file_id=empty.ipynb` as multipart form data (propagation POST).
    multipart_file_id: bool,
    timeout: Option<Duration>,
    /// Accept a non-JSON body (returned as a JSON string).
    lenient: bool,
    /// Treat a timeout as success (keep-alive: the ping landed, the VM just
    /// did not answer in time).
    timeout_ok: bool,
}

pub struct ColabClient {
    auth: Arc<AuthManager>,
}

impl ColabClient {
    pub fn new(auth: Arc<AuthManager>) -> Self {
        Self { auth }
    }

    pub fn auth(&self) -> &Arc<AuthManager> {
        &self.auth
    }

    fn colab_url(&self, path: &str) -> String {
        format!("{}{path}", self.auth.endpoints().colab.trim_end_matches('/'))
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}{path}", self.auth.endpoints().colab_api.trim_end_matches('/'))
    }

    fn is_colab_host(&self, url: &str) -> bool {
        let host = url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_owned));
        host.is_some() && host == self.auth.endpoints().colab_host()
    }

    async fn call(&self, method: Method, url: &str, call: Call) -> Result<Value> {
        let path = url::Url::parse(url).map(|parsed| parsed.path().to_owned()).unwrap_or_default();
        let mut query = call.query.clone();
        if self.is_colab_host(url) && !query.iter().any(|(key, _)| *key == "authuser") {
            query.push(("authuser", "0".to_owned()));
        }

        let mut retried = false;
        loop {
            // A 401 on a token that looked fresh gets one forced refresh.
            let token = if retried {
                self.auth.force_refresh().await?
            } else {
                self.auth.access_token().await?
            };
            let mut request = self
                .auth
                .http()
                .request(method.clone(), url)
                .bearer_auth(&token)
                .header(ACCEPT, "application/json")
                .header(CLIENT_AGENT_HEADER, CLIENT_AGENT)
                .timeout(call.timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT))
                .query(&query);
            for (name, value) in &call.headers {
                request = request.header(*name, value);
            }
            if let Some(body) = &call.json {
                request = request.json(body);
            }
            if call.multipart_file_id {
                request = request
                    .multipart(reqwest::multipart::Form::new().text("file_id", "empty.ipynb"));
            }

            let response = match request.send().await {
                Ok(response) => response,
                Err(error) if call.timeout_ok && error.is_timeout() => return Ok(Value::Null),
                Err(error) => return Err(error.into()),
            };
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && !retried {
                retried = true;
                continue;
            }
            let text = match response.text().await {
                Ok(text) => text,
                Err(error) if call.timeout_ok && error.is_timeout() => return Ok(Value::Null),
                Err(error) => return Err(error.into()),
            };
            if !status.is_success() {
                return Err(colab_error(method.as_str(), &path, status, text));
            }
            return if call.lenient {
                Ok(parse_lenient_body(&text))
            } else {
                Ok(parse_json_body(&text)?.unwrap_or(Value::Null))
            };
        }
    }

    // ---------------------------------------------------------- assignments

    /// `GET /tun/m/assignments` — every VM the account currently holds.
    pub async fn list_assignments(&self) -> Result<Vec<Value>> {
        let url = self.colab_url(&format!("{TUN_ENDPOINT}/assignments"));
        let payload = self.call(Method::GET, &url, Call::default()).await?;
        Ok(payload.get("assignments").and_then(Value::as_array).cloned().unwrap_or_default())
    }

    /// Allocate a VM (CPU/GPU/TPU, optionally High-RAM). The GET either
    /// returns an existing assignment for the notebook hash, or a token that
    /// authorises the POST which actually allocates the machine.
    pub async fn assign(
        &self,
        request: &RuntimeRequest,
        notebook_hash: Option<Uuid>,
    ) -> Result<Value> {
        let (variant, accelerator, shape) = request.resolved();
        let hash = notebook_hash.unwrap_or_else(Uuid::new_v4);
        let mut query = vec![
            ("nbh", config::uuid_to_web_safe_base64(&hash)),
            ("variant", variant.as_str().to_owned()),
            ("accelerator", accelerator.as_str().to_owned()),
        ];
        if shape == Some(Shape::HighRam) {
            query.push(("shape", "hm".to_owned()));
        }
        let url = self.colab_url(&format!("{TUN_ENDPOINT}/assign"));

        let first =
            self.call(Method::GET, &url, Call { query: query.clone(), ..Call::default() }).await?;
        if first.get("endpoint").is_some() {
            return Ok(first);
        }
        let token = first
            .get("token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| Error::Colab {
                status: None,
                message: "Colab's assign endpoint returned no token.".to_owned(),
                body: String::new(),
            })?
            .to_owned();

        let allocated = self
            .call(
                Method::POST,
                &url,
                Call { query, headers: vec![(XSRF_HEADER, token)], ..Call::default() },
            )
            .await;
        match allocated {
            Ok(assignment) => Ok(assignment),
            Err(error) if error.status() == Some(412) => Err(Error::TooManyAssignments),
            Err(error) if error.status() == Some(400) && accelerator != Accelerator::None => {
                Err(Error::Quota(quota_message(accelerator)))
            }
            Err(error) => Err(error),
        }
    }

    /// Release a VM (GET the token, then POST it back).
    pub async fn unassign(&self, endpoint: &str) -> Result<()> {
        let endpoint = validate_endpoint(endpoint)?;
        let url = self.colab_url(&format!("{TUN_ENDPOINT}/unassign/{endpoint}"));
        let payload = self.call(Method::GET, &url, Call::default()).await?;
        let token = payload.get("token").and_then(Value::as_str).unwrap_or_default().to_owned();
        self.call(
            Method::POST,
            &url,
            Call { headers: vec![(XSRF_HEADER, token)], ..Call::default() },
        )
        .await?;
        Ok(())
    }

    /// Refresh the VM's idle timer. A timeout is normal: the front door
    /// recorded the ping, the VM just did not reply in time.
    pub async fn keep_alive(&self, endpoint: &str) -> Result<()> {
        let endpoint = validate_endpoint(endpoint)?;
        let url = self.colab_url(&format!("{TUN_ENDPOINT}/{endpoint}/keep-alive/"));
        self.call(
            Method::GET,
            &url,
            Call {
                headers: vec![(TUNNEL_HEADER, TUNNEL_HEADER_VALUE.to_owned())],
                timeout: Some(KEEP_ALIVE_TIMEOUT),
                lenient: true,
                timeout_ok: true,
                ..Call::default()
            },
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------ user info

    /// `GET v1/user-info` — tier, CCU balance, machine eligibility.
    pub async fn get_user_info(&self, with_ccu: bool) -> Result<Value> {
        let url = self.api_url("/v1/user-info");
        let mut call = Call::default();
        if with_ccu {
            call.query.push(("get_ccu_consumption_info", "true".to_owned()));
        }
        self.call(Method::GET, &url, call).await
    }

    /// `GET /tun/m/ccu-info` — the Colab web app's compute-unit block. Same
    /// numbers as `v1/user-info`, served from the front door, which accepts
    /// the CLI's OAuth client (user-info answers 403 for it).
    pub async fn get_ccu_info(&self) -> Result<Value> {
        let url = self.colab_url(&format!("{TUN_ENDPOINT}/ccu-info"));
        self.call(Method::GET, &url, Call::default()).await
    }

    /// `GET v1beta/runtimespecs` — the accelerator catalog for the account.
    pub async fn list_runtime_specs(&self) -> Result<Vec<Value>> {
        let url = self.api_url("/v1beta/runtimespecs");
        let payload = self.call(Method::GET, &url, Call::default()).await?;
        Ok(payload
            .get("runtimeSpecs")
            .or_else(|| payload.get("runtimes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    // ------------------------------------------------------------ resources

    /// `GET {proxy}/api/colab/resources` — RAM / disk / GPU telemetry, raw.
    pub async fn get_resources(&self, proxy_url: &str, proxy_token: &str) -> Result<Value> {
        let url = format!("{}/api/colab/resources", proxy_url.trim_end_matches('/'));
        self.call(
            Method::GET,
            &url,
            Call {
                query: vec![
                    ("authuser", "0".to_owned()),
                    (RUNTIME_PROXY_TOKEN_PARAM, proxy_token.to_owned()),
                ],
                headers: vec![(RUNTIME_PROXY_TOKEN_HEADER, proxy_token.to_owned())],
                ..Call::default()
            },
        )
        .await
    }

    // ------------------------------------------------ credential propagation

    /// `/tun/m/credentials-propagation/{endpoint}` — answers the VM's
    /// `colab_request` for Drive (`dfs_ephemeral`) or Google Cloud
    /// (`auth_user_ephemeral`) credentials. Port of `drivefs_hook` in
    /// colab-cli's `commands/automation.py`:
    ///
    /// 1. `GET` returns an XSRF token;
    /// 2. `POST ?dryrun=true` checks consent — without it Colab answers
    ///    `{"success": false, "unauthorized_redirect_uri": …}`;
    /// 3. `POST ?dryrun=false` actually authorises the VM.
    pub async fn propagate_credentials(
        &self,
        endpoint: &str,
        auth_type: AuthType,
    ) -> Result<Propagation> {
        let endpoint = validate_endpoint(endpoint)?;
        let url = self.colab_url(&format!("{TUN_ENDPOINT}/credentials-propagation/{endpoint}"));
        let base_query = |dry_run: bool| {
            vec![
                ("authuser", "0".to_owned()),
                ("authtype", auth_type.as_str().to_owned()),
                ("version", "2".to_owned()),
                ("dryrun", dry_run.to_string()),
                ("propagate", "true".to_owned()),
                ("record", "false".to_owned()),
            ]
        };

        let challenge = self
            .call(
                Method::GET,
                &url,
                Call { query: base_query(true), lenient: true, ..Call::default() },
            )
            .await?;
        let token = challenge.get("token").and_then(Value::as_str).unwrap_or_default().to_owned();

        let check = self.propagation_post(&url, base_query(true), &token).await?;
        if check.get("success").and_then(Value::as_bool) != Some(true) {
            return Ok(Propagation {
                success: false,
                unauthorized_redirect_uri: check
                    .get("unauthorized_redirect_uri")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        let result = self.propagation_post(&url, base_query(false), &token).await?;
        // A 200 without a body still means the VM was authorised (the CLI
        // checks only the status code on this call).
        Ok(Propagation {
            success: result.get("success").and_then(Value::as_bool) != Some(false),
            unauthorized_redirect_uri: None,
        })
    }

    async fn propagation_post(
        &self,
        url: &str,
        query: Vec<(&'static str, String)>,
        token: &str,
    ) -> Result<Value> {
        let outcome = self
            .call(
                Method::POST,
                url,
                Call {
                    query,
                    headers: vec![(XSRF_HEADER, token.to_owned())],
                    multipart_file_id: true,
                    lenient: true,
                    ..Call::default()
                },
            )
            .await;
        match outcome {
            Ok(value) if value.is_object() => Ok(value),
            Ok(_) => Ok(json!({})),
            Err(Error::Colab { body, status, message }) => match extract_consent_redirect(&body) {
                Some(uri) => Ok(json!({ "success": false, "unauthorized_redirect_uri": uri })),
                None => Err(Error::Colab { status, message, body }),
            },
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_validated() {
        assert!(validate_endpoint("gpu-t4-s-abc_1.2").is_ok());
        assert!(validate_endpoint("").is_err());
        assert!(validate_endpoint("../../etc").is_err());
        assert!(validate_endpoint("a/b").is_err());
        assert!(validate_endpoint("a?b").is_err());
    }

    #[test]
    fn auth_types_round_trip() {
        for kind in [AuthType::DfsEphemeral, AuthType::AuthUserEphemeral] {
            assert_eq!(AuthType::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(AuthType::parse("other"), None);
        assert_eq!(AuthType::DfsEphemeral.label(), "Google Drive");
    }

    #[test]
    fn quota_message_names_the_accelerator() {
        assert!(quota_message(Accelerator::H100).contains("'H100'"));
    }
}
