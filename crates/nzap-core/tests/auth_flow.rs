//! End-to-end Google sign-in against the mock OAuth server: the real
//! loopback listener, PKCE, code exchange, refresh, revocation and
//! persistence — everything but a human clicking "Allow".

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nzap_core::auth::{AuthManager, DisconnectReason, OAuthClient, LOGIN_TIMEOUT};
use nzap_core::config::{Endpoints, REMOTE_REDIRECT_URI};
use nzap_core::secrets::{MemoryStore, SecretStore, StorageKind};
use nzap_core::Error;
use nzap_mock_colab::MockGoogle;

struct Harness {
    mock: MockGoogle,
    store: Arc<MemoryStore>,
    identity: PathBuf,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            mock: MockGoogle::start().await,
            store: Arc::new(MemoryStore::default()),
            identity: dir.path().join("account.json"),
            _dir: dir,
        }
    }

    fn manager(&self) -> AuthManager {
        AuthManager::new(
            nzap_core::http::build_client().unwrap(),
            Endpoints::single_host(&self.mock.base_url),
            OAuthClient { client_id: "test-client".into(), client_secret: None },
            self.store.clone(),
            self.identity.clone(),
        )
    }
}

/// Play the browser: open the consent URL and follow Google's redirect to
/// the loopback listener.
async fn approve_in_browser(auth_url: &str) {
    let browser = reqwest::Client::new();
    let response = browser.get(auth_url).send().await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    assert!(response.text().await.unwrap().contains("Google connected"));
}

async fn sign_in(manager: &AuthManager) -> nzap_core::auth::GoogleUser {
    let login = manager.begin_loopback(Some("ada@example.com")).await.unwrap();
    assert!(login.auth_url.contains("code_challenge_method=S256"));
    assert!(login.auth_url.contains("login_hint=ada%40example.com"));
    let url = login.auth_url.clone();
    let (user, ()) =
        tokio::join!(manager.finish_loopback(login, LOGIN_TIMEOUT), approve_in_browser(&url));
    user.unwrap()
}

#[tokio::test]
async fn loopback_sign_in_refresh_and_persistence() {
    let harness = Harness::new().await;
    let manager = harness.manager();

    let snapshot = manager.snapshot().await;
    assert!(!snapshot.has_credentials);
    assert_eq!(snapshot.reason, Some(DisconnectReason::NotConnected));
    assert!(matches!(manager.access_token().await, Err(Error::NotConnected)));

    let user = sign_in(&manager).await;
    assert_eq!(user.email, "ada@example.com");
    assert_eq!(user.name.as_deref(), Some("Ada Lovelace"));

    let snapshot = manager.snapshot().await;
    assert!(snapshot.has_credentials);
    assert_eq!(snapshot.reason, None);
    assert_eq!(snapshot.storage, StorageKind::Memory);
    assert!(snapshot.custom_client);

    // The refresh token is in the secret store; the identity file has no secrets.
    let refresh = harness.store.get("google-refresh-token").unwrap().unwrap();
    let identity = std::fs::read_to_string(&harness.identity).unwrap();
    assert!(identity.contains("ada@example.com"));
    assert!(!identity.contains(&refresh));

    // The first access token is cached; no refresh yet.
    let first = manager.access_token().await.unwrap();
    assert_eq!(manager.access_token().await.unwrap(), first);
    assert_eq!(harness.mock.state().refresh_count, 0);

    // Forcing a refresh mints a new token.
    let second = manager.force_refresh().await.unwrap();
    assert_ne!(first, second);
    assert_eq!(harness.mock.state().refresh_count, 1);

    // A fresh manager (app restart) picks the connection up again.
    let restarted = harness.manager();
    assert!(restarted.snapshot().await.has_credentials);
    assert_eq!(restarted.user().await.map(|user| user.email).as_deref(), Some("ada@example.com"));
    let token = restarted.access_token().await.unwrap();
    assert!(harness.mock.state().access_tokens.contains(&token));
}

#[tokio::test]
async fn short_lived_tokens_are_refreshed_ahead_of_expiry() {
    let harness = Harness::new().await;
    // Anything under the 120 s margin counts as "about to expire".
    harness.mock.state().access_ttl = 60;
    let manager = harness.manager();
    sign_in(&manager).await;

    let a = manager.access_token().await.unwrap();
    let b = manager.access_token().await.unwrap();
    assert_ne!(a, b);
    assert_eq!(harness.mock.state().refresh_count, 2);
}

#[tokio::test]
async fn rotated_refresh_tokens_are_stored() {
    let harness = Harness::new().await;
    harness.mock.state().rotate_refresh_tokens = true;
    let manager = harness.manager();
    sign_in(&manager).await;
    let before = harness.store.get("google-refresh-token").unwrap();

    manager.force_refresh().await.unwrap();
    let after = harness.store.get("google-refresh-token").unwrap();
    assert_ne!(before, after);

    // The rotated token keeps working.
    manager.force_refresh().await.unwrap();
    assert_eq!(harness.mock.state().refresh_count, 2);
}

#[tokio::test]
async fn revoked_grant_is_reported_and_remembered() {
    let harness = Harness::new().await;
    let manager = harness.manager();
    sign_in(&manager).await;

    harness.mock.state().revoke_grant();
    let error = manager.force_refresh().await.unwrap_err();
    assert!(matches!(error, Error::AuthExpired(_)), "{error:?}");
    assert!(error.is_auth_failure());

    let snapshot = manager.snapshot().await;
    assert!(!snapshot.has_credentials);
    assert_eq!(snapshot.reason, Some(DisconnectReason::Revoked));
    assert_eq!(snapshot.user.map(|user| user.email).as_deref(), Some("ada@example.com"));
    assert_eq!(harness.store.get("google-refresh-token").unwrap(), None);

    // After a restart the app still knows who was connected, and why not now.
    let restarted = harness.manager();
    assert_eq!(restarted.snapshot().await.reason, Some(DisconnectReason::Revoked));
    assert!(matches!(restarted.access_token().await, Err(Error::AuthExpired(_))));

    // Connecting again clears the revoked state.
    sign_in(&restarted).await;
    assert_eq!(restarted.snapshot().await.reason, None);
}

/// The app and an agent server (`nzap-engine mcp`) share one stored token.
#[tokio::test]
async fn processes_sharing_the_stored_connection() {
    let harness = Harness::new().await;
    let agent = harness.manager();
    assert!(matches!(agent.access_token().await, Err(Error::NotConnected)));

    // The app connects while the agent server runs; the agent picks it up.
    let app = harness.manager();
    sign_in(&app).await;
    agent.reload().await;
    assert!(agent.snapshot().await.has_credentials);
    assert_eq!(agent.user().await.map(|user| user.email).as_deref(), Some("ada@example.com"));
    agent.access_token().await.unwrap();

    // The app reconnects (a new grant) and the old one dies. The agent,
    // still holding the old token, switches to the stored one instead of
    // deleting it.
    let old = harness.store.get("google-refresh-token").unwrap().unwrap();
    sign_in(&app).await;
    let new = harness.store.get("google-refresh-token").unwrap().unwrap();
    assert_ne!(old, new);
    harness.mock.state().refresh_tokens.remove(&old);
    agent.force_refresh().await.unwrap();
    assert_eq!(harness.store.get("google-refresh-token").unwrap(), Some(new));
    assert!(agent.snapshot().await.has_credentials);
}

#[tokio::test]
async fn disconnect_revokes_and_forgets() {
    let harness = Harness::new().await;
    let manager = harness.manager();
    sign_in(&manager).await;
    let refresh = harness.store.get("google-refresh-token").unwrap().unwrap();

    manager.disconnect().await.unwrap();
    assert!(harness.mock.state().revoked.contains(&refresh));
    assert_eq!(harness.store.get("google-refresh-token").unwrap(), None);
    assert!(!harness.identity.exists());
    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot.reason, Some(DisconnectReason::NotConnected));
    assert_eq!(snapshot.user, None);

    // Disconnecting twice (or offline) is harmless.
    manager.disconnect().await.unwrap();
}

#[tokio::test]
async fn cancelled_consent_fails_the_login() {
    let harness = Harness::new().await;
    harness.mock.state().deny_consent = true;
    let manager = harness.manager();
    let login = manager.begin_loopback(None).await.unwrap();
    let url = login.auth_url.clone();
    let (result, _) = tokio::join!(
        manager.finish_loopback(login, LOGIN_TIMEOUT),
        reqwest::Client::new().get(&url).send()
    );
    let error = result.unwrap_err();
    assert!(error.to_string().contains("access_denied"), "{error}");
    assert!(!manager.snapshot().await.has_credentials);
}

#[tokio::test]
async fn abandoned_login_times_out() {
    let harness = Harness::new().await;
    let manager = harness.manager();
    let login = manager.begin_loopback(None).await.unwrap();
    let error = manager.finish_loopback(login, Duration::from_millis(100)).await.unwrap_err();
    assert!(error.to_string().contains("Timed out"));
}

#[tokio::test]
async fn remote_copy_paste_flow() {
    let harness = Harness::new().await;
    let manager = harness.manager();

    assert!(manager.complete_remote("anything").await.is_err());

    let url = manager.begin_remote(None).unwrap();
    assert!(url.contains("token_usage=remote"));
    let parsed = url::Url::parse(&url).unwrap();
    let redirect = parsed
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .map(|(_, value)| value.into_owned());
    assert_eq!(redirect.as_deref(), Some(REMOTE_REDIRECT_URI));

    // Google's landing page shows the code; read it off the redirect.
    let no_redirects =
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let response = no_redirects.get(&url).send().await.unwrap();
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    let code = url::Url::parse(&location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();

    let user = manager.complete_remote(&format!("  {code}\n")).await.unwrap();
    assert_eq!(user.email, "ada@example.com");
    assert!(manager.snapshot().await.has_credentials);

    // The pending login is single-use.
    assert!(manager.complete_remote(&code).await.is_err());
}

#[tokio::test]
async fn code_exchange_enforces_pkce() {
    let harness = Harness::new().await;
    let http = nzap_core::http::build_client().unwrap();
    let endpoints = Endpoints::single_host(&harness.mock.base_url);
    let client = OAuthClient { client_id: "test-client".into(), client_secret: None };
    let pkce = nzap_core::auth::oauth::Pkce::generate();
    let url = nzap_core::auth::oauth::build_auth_url(&nzap_core::auth::oauth::AuthRequest {
        auth_uri: &endpoints.auth_uri,
        client_id: "test-client",
        redirect_uri: REMOTE_REDIRECT_URI,
        state: "s",
        challenge: &pkce.challenge,
        login_hint: None,
        remote: true,
    })
    .unwrap();
    let no_redirects =
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let location = no_redirects.get(&url).send().await.unwrap().headers()["location"]
        .to_str()
        .unwrap()
        .to_owned();
    let code = location.split("code=").nth(1).unwrap().split('&').next().unwrap();

    let wrong = nzap_core::auth::oauth::exchange_code(
        &http,
        &endpoints.token_uri,
        &client,
        code,
        "not-the-verifier",
        REMOTE_REDIRECT_URI,
    )
    .await
    .unwrap_err();
    // A bad exchange is a sign-in failure, not a revoked connection.
    assert!(matches!(wrong, Error::Auth(_)), "{wrong:?}");
}

#[test]
fn storage_kind_is_reported() {
    assert_eq!(MemoryStore::default().kind(), StorageKind::Memory);
}
