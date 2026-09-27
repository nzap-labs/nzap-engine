//! The Google connection: sign-in flows, token refresh and persistence.
//!
//! Port of colab-studio's `AuthManager`, reshaped for a desktop app:
//!
//! * the refresh token lives in the [`SecretStore`] (OS keychain);
//! * the access token lives only in memory and is refreshed 120 s before
//!   expiry, one refresh at a time;
//! * the non-secret identity (email, name, picture) is kept in
//!   `account.json` so the UI can say *who* is connected — or *was*, after
//!   Google revokes the grant.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::loopback::LoopbackServer;
use super::oauth::{self, AuthRequest, GoogleUser, OAuthClient, Pkce, TokenResponse};
use crate::config::{Endpoints, REMOTE_REDIRECT_URI, SCOPES};
use crate::error::{Error, Result};
use crate::secrets::{write_private, SecretStore, StorageKind};

const REFRESH_TOKEN_KEY: &str = "google-refresh-token";
/// Refresh this long before Google's expiry (colab-studio used 120 s).
const REFRESH_MARGIN: Duration = Duration::from_secs(120);
/// Pending copy/paste logins expire after 15 minutes (colab-studio).
const REMOTE_LOGIN_TTL: Duration = Duration::from_secs(900);

/// Why there is no usable connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    NotConnected,
    Revoked,
}

/// What the UI needs to know about the stored connection. This is the
/// *local* view; liveness against Colab is checked by the engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSnapshot {
    pub has_credentials: bool,
    pub reason: Option<DisconnectReason>,
    pub user: Option<GoogleUser>,
    pub storage: StorageKind,
    pub custom_client: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    user: Option<GoogleUser>,
    #[serde(default)]
    connected_at: Option<String>,
    #[serde(default)]
    scopes: Option<String>,
    #[serde(default)]
    revoked: bool,
}

struct AccessToken {
    token: String,
    expires_at: Instant,
}

#[derive(Default)]
struct TokenState {
    refresh_token: Option<String>,
    access: Option<AccessToken>,
    identity: Identity,
}

/// A copy/paste login awaiting its code. Google's landing page shows the
/// code but not the state, so the most recent pending login is used.
struct PendingRemote {
    pkce: Pkce,
    created: Instant,
}

/// A loopback sign-in in progress: open [`LoopbackLogin::auth_url`] in the
/// browser, then await [`AuthManager::finish_loopback`].
pub struct LoopbackLogin {
    pub auth_url: String,
    server: LoopbackServer,
    state: String,
    pkce: Pkce,
    redirect_uri: String,
}

pub struct AuthManager {
    http: reqwest::Client,
    endpoints: Endpoints,
    client: RwLock<OAuthClient>,
    secrets: Arc<dyn SecretStore>,
    identity_path: PathBuf,
    state: tokio::sync::Mutex<TokenState>,
    remote: Mutex<Option<PendingRemote>>,
}

impl AuthManager {
    /// Load any stored connection. Storage failures are logged and treated
    /// as "not connected" so a broken keychain never blocks startup.
    pub fn new(
        http: reqwest::Client,
        endpoints: Endpoints,
        client: OAuthClient,
        secrets: Arc<dyn SecretStore>,
        identity_path: PathBuf,
    ) -> Self {
        let refresh_token = secrets.get(REFRESH_TOKEN_KEY).unwrap_or_else(|error| {
            tracing::warn!("Could not read the stored Google connection: {error}");
            None
        });
        let identity = std::fs::read_to_string(&identity_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Identity>(&text).ok())
            .unwrap_or_default();
        Self {
            http,
            endpoints,
            client: RwLock::new(client),
            secrets,
            identity_path,
            state: tokio::sync::Mutex::new(TokenState {
                refresh_token,
                access: None,
                identity,
            }),
            remote: Mutex::new(None),
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn oauth_client(&self) -> OAuthClient {
        self.client
            .read()
            .map(|client| client.clone())
            .unwrap_or_default()
    }

    /// Switch OAuth clients (Settings → "Bring your own client"). Tokens
    /// minted by another client cannot be refreshed by this one, so the
    /// caller should disconnect first when the client actually changes.
    pub fn set_oauth_client(&self, client: OAuthClient) {
        if let Ok(mut current) = self.client.write() {
            *current = client;
        }
    }

    pub async fn snapshot(&self) -> AuthSnapshot {
        let state = self.state.lock().await;
        let has_credentials = state.refresh_token.is_some();
        let reason = if has_credentials {
            None
        } else if state.identity.user.is_some() || state.identity.revoked {
            Some(DisconnectReason::Revoked)
        } else {
            Some(DisconnectReason::NotConnected)
        };
        AuthSnapshot {
            has_credentials,
            reason,
            user: state.identity.user.clone(),
            storage: self.secrets.kind(),
            custom_client: !self.oauth_client().is_default(),
        }
    }

    pub async fn user(&self) -> Option<GoogleUser> {
        self.state.lock().await.identity.user.clone()
    }

    // ------------------------------------------------------------ sign-in

    /// Start a loopback sign-in: binds the listener and builds the consent URL.
    pub async fn begin_loopback(&self, login_hint: Option<&str>) -> Result<LoopbackLogin> {
        let server = LoopbackServer::bind().await?;
        let redirect_uri = server.redirect_uri();
        let state = oauth::random_token(24);
        let pkce = Pkce::generate();
        let client = self.oauth_client();
        let auth_url = oauth::build_auth_url(&AuthRequest {
            auth_uri: &self.endpoints.auth_uri,
            client_id: &client.client_id,
            redirect_uri: &redirect_uri,
            state: &state,
            challenge: &pkce.challenge,
            login_hint,
            remote: false,
        })?;
        Ok(LoopbackLogin {
            auth_url,
            server,
            state,
            pkce,
            redirect_uri,
        })
    }

    /// Wait for the browser redirect, then exchange the code and persist.
    pub async fn finish_loopback(
        &self,
        login: LoopbackLogin,
        timeout: Duration,
    ) -> Result<GoogleUser> {
        let LoopbackLogin {
            server,
            state,
            pkce,
            redirect_uri,
            ..
        } = login;
        let code = server.wait_for_code(&state, timeout).await?;
        self.complete(&code, &pkce.verifier, &redirect_uri).await
    }

    /// Start the copy/paste flow (colab-cli's remote flow): Google shows the
    /// code on its own page for the user to paste back.
    pub fn begin_remote(&self, login_hint: Option<&str>) -> Result<String> {
        let state = oauth::random_token(24);
        let pkce = Pkce::generate();
        let client = self.oauth_client();
        let url = oauth::build_auth_url(&AuthRequest {
            auth_uri: &self.endpoints.auth_uri,
            client_id: &client.client_id,
            redirect_uri: REMOTE_REDIRECT_URI,
            state: &state,
            challenge: &pkce.challenge,
            login_hint,
            remote: true,
        })?;
        let mut pending = self
            .remote
            .lock()
            .map_err(|_| Error::internal("Login state lock poisoned."))?;
        *pending = Some(PendingRemote {
            pkce,
            created: Instant::now(),
        });
        Ok(url)
    }

    /// Finish the copy/paste flow with the code the user pasted.
    pub async fn complete_remote(&self, code: &str) -> Result<GoogleUser> {
        let code = code.trim();
        if code.is_empty() {
            return Err(Error::invalid("Paste the code Google showed you."));
        }
        let pending = self
            .remote
            .lock()
            .map_err(|_| Error::internal("Login state lock poisoned."))?
            .take()
            .filter(|pending| pending.created.elapsed() < REMOTE_LOGIN_TTL)
            .ok_or_else(|| Error::Auth("No sign-in in progress. Start connecting again.".into()))?;
        self.complete(code, &pending.pkce.verifier, REMOTE_REDIRECT_URI)
            .await
    }

    async fn complete(&self, code: &str, verifier: &str, redirect_uri: &str) -> Result<GoogleUser> {
        let client = self.oauth_client();
        let tokens =
            oauth::exchange_code(&self.http, &self.endpoints.token_uri, &client, code, verifier, redirect_uri)
                .await?;
        let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
            Error::Auth(
                "Google did not issue a refresh token. Remove NZAP Engine from your Google \
                 account permissions and connect again."
                    .to_owned(),
            )
        })?;
        let user = oauth::fetch_userinfo(&self.http, &self.endpoints.userinfo_uri, &tokens.access_token)
            .await?;

        self.secrets.set(REFRESH_TOKEN_KEY, &refresh_token)?;
        let mut state = self.state.lock().await;
        state.refresh_token = Some(refresh_token);
        state.access = Some(access_from(&tokens));
        state.identity = Identity {
            user: Some(user.clone()),
            connected_at: Some(chrono::Utc::now().to_rfc3339()),
            scopes: tokens.scope.clone().or_else(|| Some(SCOPES.join(" "))),
            revoked: false,
        };
        self.save_identity(&state.identity);
        tracing::info!("Google connected");
        Ok(user)
    }

    // ------------------------------------------------------------- tokens

    /// A valid access token, refreshing it when close to expiry. Concurrent
    /// callers share one refresh.
    pub async fn access_token(&self) -> Result<String> {
        let mut state = self.state.lock().await;
        if let Some(access) = &state.access {
            if access.expires_at.saturating_duration_since(Instant::now()) > REFRESH_MARGIN {
                return Ok(access.token.clone());
            }
        }
        self.refresh_locked(&mut state).await
    }

    /// Drop the cached access token and mint a new one — used when Colab
    /// rejects a token that looked fresh.
    pub async fn force_refresh(&self) -> Result<String> {
        let mut state = self.state.lock().await;
        state.access = None;
        self.refresh_locked(&mut state).await
    }

    async fn refresh_locked(&self, state: &mut TokenState) -> Result<String> {
        let Some(refresh_token) = state.refresh_token.clone() else {
            return Err(if state.identity.user.is_some() {
                Error::AuthExpired("Your Google connection was revoked. Connect again.".to_owned())
            } else {
                Error::NotConnected
            });
        };
        match oauth::refresh(
            &self.http,
            &self.endpoints.token_uri,
            &self.oauth_client(),
            &refresh_token,
        )
        .await
        {
            Ok(tokens) => {
                // Google may rotate the refresh token; keep whichever is newest.
                if let Some(rotated) = tokens.refresh_token.as_deref() {
                    if rotated != refresh_token {
                        self.secrets.set(REFRESH_TOKEN_KEY, rotated)?;
                        state.refresh_token = Some(rotated.to_owned());
                    }
                }
                let access = access_from(&tokens);
                let token = access.token.clone();
                state.access = Some(access);
                tracing::debug!("Refreshed the Google access token");
                Ok(token)
            }
            Err(error @ Error::AuthExpired(_)) => {
                // The grant is dead: forget it, remember who it was for.
                state.refresh_token = None;
                state.access = None;
                state.identity.revoked = true;
                if let Err(delete) = self.secrets.delete(REFRESH_TOKEN_KEY) {
                    tracing::warn!("Could not remove the revoked token: {delete}");
                }
                self.save_identity(&state.identity);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// Re-read the Google profile (name / picture changes).
    pub async fn refresh_user(&self) -> Result<GoogleUser> {
        let token = self.access_token().await?;
        let user = oauth::fetch_userinfo(&self.http, &self.endpoints.userinfo_uri, &token).await?;
        let mut state = self.state.lock().await;
        state.identity.user = Some(user.clone());
        self.save_identity(&state.identity);
        Ok(user)
    }

    /// Forget the connection locally and revoke it at Google (best-effort:
    /// an offline machine still disconnects).
    pub async fn disconnect(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(token) = state.refresh_token.take() {
            if let Err(error) = oauth::revoke(&self.http, &self.endpoints.revoke_uri, &token).await {
                tracing::info!("Token revocation skipped: {error}");
            }
        }
        state.access = None;
        state.identity = Identity::default();
        self.secrets.delete(REFRESH_TOKEN_KEY)?;
        match std::fs::remove_file(&self.identity_path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    fn save_identity(&self, identity: &Identity) {
        let result = serde_json::to_vec_pretty(identity)
            .map_err(Error::from)
            .and_then(|bytes| write_private(&self.identity_path, &bytes));
        if let Err(error) = result {
            tracing::warn!("Could not save the Google identity: {error}");
        }
    }
}

fn access_from(tokens: &TokenResponse) -> AccessToken {
    AccessToken {
        token: tokens.access_token.clone(),
        expires_at: Instant::now() + Duration::from_secs(tokens.expires_in.unwrap_or(3600)),
    }
}
