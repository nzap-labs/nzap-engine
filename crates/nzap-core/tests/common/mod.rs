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
