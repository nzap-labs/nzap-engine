//! Mutable state behind the mock, readable and adjustable by tests.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

/// One request as the mock saw it (for asserting headers and parameters).
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct IssuedCode {
    pub challenge: String,
    pub redirect_uri: String,
    pub client_id: String,
}

#[derive(Debug)]
pub struct MockState {
    pub base_url: String,
    /// The Google identity returned by userinfo.
    pub user: Value,
    /// Seconds until issued access tokens expire.
    pub access_ttl: u64,
    /// Issue a new refresh token on every refresh.
    pub rotate_refresh_tokens: bool,
    /// Make the consent screen answer `error=access_denied`.
    pub deny_consent: bool,
    pub access_tokens: HashSet<String>,
    pub refresh_tokens: HashSet<String>,
    pub revoked: Vec<String>,
    pub refresh_count: usize,
    pub requests: Vec<RecordedRequest>,
    pub(crate) codes: HashMap<String, IssuedCode>,
    counter: u64,
}

impl MockState {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_owned(),
            user: json!({
                "sub": "1234567890",
                "email": "ada@example.com",
                "name": "Ada Lovelace",
                "picture": format!("{base_url}/avatar.png"),
            }),
            access_ttl: 3600,
            rotate_refresh_tokens: false,
            deny_consent: false,
            access_tokens: HashSet::new(),
            refresh_tokens: HashSet::new(),
            revoked: Vec::new(),
            refresh_count: 0,
            requests: Vec::new(),
            codes: HashMap::new(),
            counter: 0,
        }
    }

    pub(crate) fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }

    pub(crate) fn issue_access_token(&mut self) -> String {
        let token = self.next_id("access");
        self.access_tokens.insert(token.clone());
        token
    }

    /// Whether an `Authorization: Bearer …` header carries a live token.
    pub fn is_authorized(&self, header: Option<&str>) -> bool {
        header
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|token| self.access_tokens.contains(token))
    }

    /// Invalidate every access token (simulates expiry / revocation).
    pub fn expire_access_tokens(&mut self) {
        self.access_tokens.clear();
    }

    /// Revoke the grant: refresh tokens stop working, like a user removing
    /// the app from their Google account permissions.
    pub fn revoke_grant(&mut self) {
        self.refresh_tokens.clear();
        self.access_tokens.clear();
    }

    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        self.requests
            .iter()
            .filter(|request| request.path == path)
            .cloned()
            .collect()
    }
}
