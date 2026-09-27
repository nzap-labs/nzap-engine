//! Google OAuth 2.0 primitives: client configuration, PKCE, the consent
//! URL, code exchange, refresh, revoke and userinfo.
//!
//! The parameters are the ones colab-vscode's `LocalServerFlow` and
//! google-colab-cli's `auth.py` use: PKCE S256, `access_type=offline` and
//! `prompt=consent` (so a refresh token is always issued), and
//! `token_usage=remote` for the copy/paste flow.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{self, DEFAULT_OAUTH_CLIENT_ID, DEFAULT_OAUTH_CLIENT_SECRET};
use crate::error::{Error, Result};

/// An installed-app OAuth client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthClient {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl Default for OAuthClient {
    fn default() -> Self {
        Self {
            client_id: DEFAULT_OAUTH_CLIENT_ID.to_owned(),
            client_secret: Some(DEFAULT_OAUTH_CLIENT_SECRET.to_owned()),
        }
    }
}

impl OAuthClient {
    /// Parse a client file as downloaded from Google Cloud Console
    /// (`{"installed": {...}}`), a `{"web": {...}}` document or a bare
    /// `{client_id, client_secret}` object.
    pub fn from_json(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| Error::invalid(format!("OAuth client JSON is invalid: {error}")))?;
        let inner = value.get("installed").or_else(|| value.get("web")).unwrap_or(&value);
        let client: Self = serde_json::from_value(inner.clone())
            .map_err(|_| Error::invalid("OAuth client JSON has no client_id."))?;
        if client.client_id.trim().is_empty() {
            return Err(Error::invalid("OAuth client JSON has an empty client_id."));
        }
        Ok(client)
    }

    pub fn is_default(&self) -> bool {
        self.client_id == DEFAULT_OAUTH_CLIENT_ID
    }
}

/// A PKCE verifier and its S256 challenge (RFC 7636).
#[derive(Clone, Debug)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        Self::from_verifier(random_token(48))
    }

    pub fn from_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

/// `bytes` random bytes, base64url-encoded without padding.
pub fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

/// Everything needed to build a consent URL.
pub struct AuthRequest<'a> {
    pub auth_uri: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub challenge: &'a str,
    pub login_hint: Option<&'a str>,
    /// colab-cli's copy/paste flow (`token_usage=remote`).
    pub remote: bool,
}

pub fn build_auth_url(request: &AuthRequest<'_>) -> Result<String> {
    let mut url = url::Url::parse(request.auth_uri)
        .map_err(|error| Error::internal(format!("Bad auth URI: {error}")))?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", request.client_id)
            .append_pair("redirect_uri", request.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &config::SCOPES.join(" "))
            .append_pair("state", request.state)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("include_granted_scopes", "true")
            .append_pair("code_challenge", request.challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(hint) = request.login_hint.filter(|hint| !hint.is_empty()) {
            query.append_pair("login_hint", hint);
        }
        if request.remote {
            query.append_pair("token_usage", "remote");
        }
    }
    Ok(url.into())
}

/// The token endpoint's answer.
#[derive(Clone, Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenErrorBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// The signed-in Google identity (OpenID Connect userinfo).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleUser {
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

fn client_form(client: &OAuthClient, form: &mut Vec<(&'static str, String)>) {
    form.push(("client_id", client.client_id.clone()));
    if let Some(secret) = client.client_secret.as_deref().filter(|s| !s.is_empty()) {
        form.push(("client_secret", secret.to_owned()));
    }
}

async fn post_token(
    http: &reqwest::Client,
    token_uri: &str,
    form: &[(&str, String)],
    what: &str,
    grant_is_refresh: bool,
) -> Result<TokenResponse> {
    let response =
        http.post(token_uri).form(form).timeout(std::time::Duration::from_secs(30)).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if status.is_success() {
        return serde_json::from_str(&text)
            .map_err(|_| Error::Auth(format!("{what}: Google sent an unexpected response.")));
    }
    let detail: Option<TokenErrorBody> = serde_json::from_str(&text).ok();
    let (code, description) =
        detail.map(|body| (body.error, body.error_description)).unwrap_or_default();
    // On refresh, invalid_grant means the grant was revoked or expired: the
    // user has to connect again. On code exchange it means a stale code.
    if grant_is_refresh && code == "invalid_grant" {
        return Err(Error::AuthExpired(
            "Google no longer accepts this connection. Connect your account again.".to_owned(),
        ));
    }
    let description = description.unwrap_or_else(|| code.clone());
    Err(Error::Auth(format!(
        "{what} failed ({}){}",
        status.as_u16(),
        if description.is_empty() { String::new() } else { format!(": {description}") }
    )))
}

/// Exchange an authorization code (with its PKCE verifier) for tokens.
pub async fn exchange_code(
    http: &reqwest::Client,
    token_uri: &str,
    client: &OAuthClient,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    let mut form = vec![
        ("code", code.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("grant_type", "authorization_code".to_owned()),
        ("code_verifier", verifier.to_owned()),
    ];
    client_form(client, &mut form);
    post_token(http, token_uri, &form, "Code exchange", false).await
}

/// Mint a fresh access token from a refresh token.
pub async fn refresh(
    http: &reqwest::Client,
    token_uri: &str,
    client: &OAuthClient,
    refresh_token: &str,
) -> Result<TokenResponse> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token.to_owned()),
    ];
    client_form(client, &mut form);
    post_token(http, token_uri, &form, "Token refresh", true).await
}

/// Revoke a token at Google (best-effort on disconnect).
pub async fn revoke(http: &reqwest::Client, revoke_uri: &str, token: &str) -> Result<()> {
    let response = http
        .post(revoke_uri)
        .form(&[("token", token)])
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(Error::Auth(format!(
            "Google did not revoke the token ({}).",
            response.status().as_u16()
        )))
    }
}

pub async fn fetch_userinfo(
    http: &reqwest::Client,
    userinfo_uri: &str,
    access_token: &str,
) -> Result<GoogleUser> {
    let response = http
        .get(userinfo_uri)
        .bearer_auth(access_token)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Auth(format!(
            "Could not read your Google profile ({}).",
            status.as_u16()
        )));
    }
    Ok(response.json().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_appendix_b() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned());
        assert_eq!(pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_pkce_is_well_formed() {
        let pkce = Pkce::generate();
        // 48 bytes -> 64 base64url chars; RFC 7636 wants 43..=128.
        assert_eq!(pkce.verifier.len(), 64);
        assert_ne!(Pkce::generate().verifier, pkce.verifier);
        assert!(pkce.verifier.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn auth_url_carries_every_parameter() {
        let url = build_auth_url(&AuthRequest {
            auth_uri: "https://accounts.google.com/o/oauth2/v2/auth",
            client_id: "cid",
            redirect_uri: "http://localhost:5555/callback",
            state: "st",
            challenge: "ch",
            login_hint: Some("a@b.c"),
            remote: true,
        })
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs["client_id"], "cid");
        assert_eq!(pairs["redirect_uri"], "http://localhost:5555/callback");
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pairs["access_type"], "offline");
        assert_eq!(pairs["prompt"], "consent");
        assert_eq!(pairs["login_hint"], "a@b.c");
        assert_eq!(pairs["token_usage"], "remote");
        assert!(pairs["scope"].contains("https://www.googleapis.com/auth/colaboratory"));
    }

    #[test]
    fn client_json_variants() {
        let installed = r#"{"installed":{"client_id":"a","client_secret":"s","redirect_uris":["http://localhost"]}}"#;
        assert_eq!(OAuthClient::from_json(installed).unwrap().client_id, "a");
        let bare = r#"{"client_id":"b"}"#;
        let client = OAuthClient::from_json(bare).unwrap();
        assert_eq!(client.client_secret, None);
        assert!(OAuthClient::from_json("{}").is_err());
        assert!(OAuthClient::from_json(r#"{"client_id":" "}"#).is_err());
        assert!(OAuthClient::default().is_default());
    }
}
