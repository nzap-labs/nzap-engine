//! Shared helpers for integration tests: a mock Google plus an engine
//! already connected to it.

#![allow(dead_code)]

use std::sync::Arc;

use nzap_core::auth::{AuthManager, OAuthClient};
use nzap_core::colab::ColabClient;
use nzap_core::config::Endpoints;
use nzap_core::secrets::{MemoryStore, SecretStore};
use nzap_mock_colab::MockGoogle;

pub struct Connected {
    pub mock: MockGoogle,
    pub auth: Arc<AuthManager>,
    pub client: ColabClient,
    pub dir: tempfile::TempDir,
}

/// A mock Google and an `AuthManager` holding a valid refresh token for it.
pub async fn connected() -> Connected {
    let mock = MockGoogle::start().await;
    let store = Arc::new(MemoryStore::default());
    let refresh = mock.state().grant_refresh_token();
    store.set("google-refresh-token", &refresh).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let auth = Arc::new(AuthManager::new(
        nzap_core::http::build_client().unwrap(),
        Endpoints::single_host(&mock.base_url),
        OAuthClient { client_id: "test-client".into(), client_secret: None },
        store,
        dir.path().join("account.json"),
    ));
    let client = ColabClient::new(auth.clone());
    Connected { mock, auth, client, dir }
}

impl Connected {
    /// A session manager persisting under this harness's temp directory.
    pub fn manager(&self) -> nzap_core::session::SessionManager {
        nzap_core::session::SessionManager::new(
            Arc::new(ColabClient::new(self.auth.clone())),
            Arc::new(nzap_core::history::HistoryLog::new(self.dir.path().join("history"))),
            self.dir.path().join("sessions.json"),
        )
    }
}

/// Collects streamed events for assertions.
#[derive(Clone, Default)]
pub struct Events(pub Arc<std::sync::Mutex<Vec<serde_json::Value>>>);

impl Events {
    pub fn emit(&self) -> nzap_core::session::Emit {
        let events = self.0.clone();
        Arc::new(move |event| events.lock().unwrap().push(event))
    }

    pub fn all(&self) -> Vec<serde_json::Value> {
        self.0.lock().unwrap().clone()
    }

    pub fn of_type(&self, kind: &str) -> Vec<serde_json::Value> {
        self.all().into_iter().filter(|event| event["type"] == kind).collect()
    }

    /// Concatenated stdout/stderr text.
    pub fn text(&self) -> String {
        self.of_type("stream").iter().filter_map(|event| event["text"].as_str()).collect()
    }
}

/// Poll until `condition` holds (5 s budget).
pub async fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
