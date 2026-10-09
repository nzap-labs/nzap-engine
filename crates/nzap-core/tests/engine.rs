//! The engine facade the desktop shell drives: connection status (the green
//! dot), account, settings, OAuth client changes and disconnecting.

use nzap_core::auth::{OAuthClient, LOGIN_TIMEOUT};
use nzap_core::config::{Endpoints, RuntimeRequest};
use nzap_core::paths::AppPaths;
use nzap_core::settings::SettingsPatch;
use nzap_core::{Engine, EngineOptions, Error};
use nzap_mock_colab::MockGoogle;
use serde_json::json;

fn engine(mock: &MockGoogle, root: &std::path::Path) -> Engine {
    Engine::new(EngineOptions {
        paths: AppPaths::under(root),
        endpoints: Endpoints::single_host(&mock.base_url),
        use_keychain: false,
        oauth_client: Some(OAuthClient { client_id: "test-client".into(), client_secret: None }),
        sessions_file: None,
    })
    .unwrap()
}

async fn connect(engine: &Engine) {
    let login = engine.auth.begin_loopback(None).await.unwrap();
    let url = login.auth_url.clone();
    let (user, response) =
        tokio::join!(engine.auth.finish_loopback(login, LOGIN_TIMEOUT), reqwest::get(url));
    assert!(response.unwrap().status().is_success());
    user.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn status_account_and_disconnect() {
    let mock = MockGoogle::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&mock, dir.path());

    let status = engine.status().await;
    assert!(!status.connected);
    assert_eq!(status.reason, Some("not_connected"));
    assert!(matches!(engine.account().await, Err(Error::NotConnected)));

    connect(&engine).await;
    let status = engine.status().await;
    assert!(status.connected, "{status:?}");
    assert_eq!(status.email.as_deref(), Some("ada@example.com"));
    assert!(status.custom_client);
    // The dot turned green only after Colab itself answered.
    assert!(!mock.state().requests_to("/tun/m/ccu-info").is_empty());

    // user-info refuses this client (403) → the ccu-info block is used.
    let account = engine.account().await.unwrap();
    assert_eq!(account.user.unwrap().name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(account.colab.unwrap()["eligibleGpus"], json!(["T4"]));

    // Disconnecting releases runtimes and revokes the grant.
    let view = engine
        .sessions
        .create(RuntimeRequest { name: "box".into(), ..RuntimeRequest::default() })
        .await
        .unwrap();
    engine.disconnect().await.unwrap();
    assert!(mock.state().unassigned.contains(&view.endpoint));
    assert!(engine.sessions.views().is_empty());
    assert_eq!(mock.state().revoked.len(), 1);
    assert_eq!(engine.status().await.reason, Some("not_connected"));
}

#[tokio::test(flavor = "multi_thread")]
async fn revoked_grants_turn_the_dot_off() {
    let mock = MockGoogle::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&mock, dir.path());
    connect(&engine).await;

    mock.state().revoke_grant();
    let status = engine.status().await;
    assert!(!status.connected);
    assert_eq!(status.reason, Some("revoked"));
    // Who was connected is still known.
    assert_eq!(status.email.as_deref(), Some("ada@example.com"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_colab_is_a_warning_not_a_disconnect() {
    let mock = MockGoogle::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&mock, dir.path());
    connect(&engine).await;

    // Colab's front door goes away but the token is still fine.
    let offline = Engine::new(EngineOptions {
        paths: AppPaths::under(dir.path()),
        endpoints: Endpoints {
            colab: "http://127.0.0.1:9".into(),
            ..Endpoints::single_host(&mock.base_url)
        },
        use_keychain: false,
        oauth_client: Some(OAuthClient { client_id: "test-client".into(), client_secret: None }),
        sessions_file: None,
    })
    .unwrap();
    let status = offline.status().await;
    assert!(status.connected);
    assert!(status.warning.unwrap().contains("liveness"));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_and_oauth_client() {
    let mock = MockGoogle::start().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&mock, dir.path());

    let settings = engine
        .update_settings(SettingsPatch {
            keep_alive_interval_seconds: Some(90),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(settings.keep_alive_interval_seconds, 90);
    assert_eq!(engine.hardware_config().keep_alive_interval, 90);
    assert_eq!(engine.hardware_config().gpus, vec!["t4", "l4", "g4", "a100", "h100"]);

    let catalog = format!("{}/static/catalog/", mock.base_url);
    engine
        .update_settings(SettingsPatch {
            catalog_url: Some(catalog.clone()),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(engine.notebooks.catalog().base_url(), catalog);

    // A custom client can be set while disconnected, and is persisted.
    let custom =
        r#"{"installed": {"client_id": "mine.apps.googleusercontent.com", "client_secret": "s"}}"#;
    assert!(engine.set_oauth_client(Some(custom)).await.unwrap());
    assert_eq!(engine.auth.oauth_client().client_id, "mine.apps.googleusercontent.com");
    assert!(dir.path().join("config").join("oauth-client.json").exists());
    assert!(engine.set_oauth_client(Some("{}")).await.is_err());

    // Back to the test client, connect, then changing it is refused.
    engine.set_oauth_client(Some(r#"{"client_id": "test-client"}"#)).await.unwrap();
    connect(&engine).await;
    let refused = engine.set_oauth_client(None).await.unwrap_err();
    assert!(refused.to_string().contains("Disconnect"));
}
