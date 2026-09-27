//! Mutable state behind the mock, readable and adjustable by tests.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Value};

/// One request as the mock saw it (for asserting headers and parameters).
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct IssuedCode {
    pub challenge: String,
    pub redirect_uri: String,
    pub client_id: String,
}

/// A VM held by the account.
#[derive(Clone, Debug)]
pub struct MockAssignment {
    pub endpoint: String,
    pub accelerator: String,
    /// Colab reports the variant as a number: 0 default, 1 GPU, 2 TPU.
    pub variant: u8,
    /// 0 standard, 1 High-RAM.
    pub machine_shape: u8,
    pub proxy_token: String,
}

impl MockAssignment {
    pub fn proxy_url(&self, base_url: &str) -> String {
        format!("{base_url}/proxy/{}", self.endpoint)
    }

    pub fn to_json(&self, base_url: &str) -> Value {
        json!({
            "endpoint": self.endpoint,
            "accelerator": self.accelerator,
            "variant": self.variant,
            "machineShape": self.machine_shape,
            "runtimeProxyInfo": {
                "url": self.proxy_url(base_url),
                "token": self.proxy_token,
                "tokenExpiresInSeconds": 3600,
            },
        })
    }
}

/// A file on a mock runtime's Jupyter server.
#[derive(Clone, Debug, PartialEq)]
pub enum MockFile {
    Directory,
    Text(String),
    Binary(Vec<u8>),
    Notebook(Value),
}

/// The Jupyter side of one runtime.
#[derive(Clone, Debug, Default)]
pub struct MockRuntime {
    /// Paths without a leading slash (`content/a.py`).
    pub files: BTreeMap<String, MockFile>,
    pub kernels: Vec<String>,
    pub sessions: Vec<Value>,
    /// Every cell the kernel received, in order.
    pub executed: Vec<String>,
    pub interrupts: usize,
    pub restarts: usize,
}

impl MockRuntime {
    pub fn with_defaults() -> Self {
        let mut runtime = Self::default();
        runtime.files.insert("content".into(), MockFile::Directory);
        runtime.files.insert("content/sample_data".into(), MockFile::Directory);
        runtime.files.insert(
            "content/sample_data/README.md".into(),
            MockFile::Text("This directory includes a few sample datasets.\n".into()),
        );
        runtime
    }
}

#[derive(Debug)]
pub struct MockState {
    pub base_url: String,

    // -- OAuth -----------------------------------------------------------
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
    pub(crate) codes: HashMap<String, IssuedCode>,

    // -- Colab control plane --------------------------------------------
    pub assignments: Vec<MockAssignment>,
    /// How many VMs the account may hold before `assign` answers 412.
    pub max_assignments: usize,
    /// Accelerators the account is not entitled to (`assign` answers 400).
    pub denied_accelerators: HashSet<String>,
    pub unassigned: Vec<String>,
    pub keepalives: HashMap<String, usize>,
    /// Delay keep-alive answers (simulates a VM that does not reply in time).
    pub keepalive_delay_ms: u64,
    /// Whether the user already granted Drive / Cloud consent.
    pub drive_consent: bool,
    /// `(endpoint, authtype, dryrun)` for every propagation POST.
    pub propagations: Vec<(String, String, String)>,
    /// `v1/user-info` payload; `None` answers 403 like the real API does
    /// for the CLI's OAuth client.
    pub user_info: Option<Value>,
    pub ccu_info: Value,
    pub runtime_specs: Option<Value>,
    pub resources: Value,
    pub(crate) xsrf_tokens: HashSet<String>,

    // -- runtimes ------------------------------------------------------------
    pub runtimes: HashMap<String, MockRuntime>,
    /// Files every newly assigned runtime starts with (besides the defaults).
    pub new_runtime_files: BTreeMap<String, MockFile>,
    /// `(endpoint, cols, rows)` for every terminal resize.
    pub tty_resizes: Vec<(String, u64, u64)>,
    /// Every command line entered in a terminal.
    pub tty_commands: Vec<String>,

    // -- Drive and static hosting ---------------------------------------------
    /// Drive file id -> (name, content).
    pub drive_files: HashMap<String, (String, String)>,
    /// `/static/<path>` -> body (`REDIRECT:<url>` answers 302).
    pub static_files: HashMap<String, String>,
    pub static_hits: HashMap<String, usize>,
    pub static_not_modified: usize,

    pub requests: Vec<RecordedRequest>,
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
            codes: HashMap::new(),

            assignments: Vec::new(),
            max_assignments: 3,
            denied_accelerators: HashSet::new(),
            unassigned: Vec::new(),
            keepalives: HashMap::new(),
            keepalive_delay_ms: 0,
            drive_consent: true,
            propagations: Vec::new(),
            user_info: None,
            ccu_info: json!({
                "currentBalance": 0,
                "consumptionRateHourly": 0,
                "assignmentsCount": 0,
                "eligibleGpus": ["T4"],
                "ineligibleGpus": ["A100", "L4", "H100", "G4"],
                "eligibleTpus": ["V5E1"],
                "freeCcuQuotaInfo": {
                    "remainingTokens": "36000",
                    "nextRefillTimestampSec": "1900000000",
                },
            }),
            runtime_specs: None,
            resources: json!({
                "memory": {"totalBytes": "13609431040", "freeBytes": "11609431040"},
                "disks": [{"label": "/", "totalBytes": "115658190848", "freeBytes": "80000000000"}],
                "gpus": [],
            }),
            xsrf_tokens: HashSet::new(),

            runtimes: HashMap::new(),
            new_runtime_files: BTreeMap::new(),
            tty_resizes: Vec::new(),
            tty_commands: Vec::new(),

            drive_files: HashMap::new(),
            static_files: HashMap::new(),
            static_hits: HashMap::new(),
            static_not_modified: 0,

            requests: Vec::new(),
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

    /// Mint a refresh token as if the user had just signed in — lets tests
    /// start from "connected" without driving the consent flow.
    pub fn grant_refresh_token(&mut self) -> String {
        let token = self.next_id("refresh");
        self.refresh_tokens.insert(token.clone());
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

    /// Create a VM directly (e.g. one "allocated in the Colab web UI").
    pub fn add_assignment(
        &mut self,
        accelerator: &str,
        variant: u8,
        machine_shape: u8,
    ) -> MockAssignment {
        let id = self.next_id("vm");
        let assignment = MockAssignment {
            endpoint: format!("m-s-{}-{id}", accelerator.to_ascii_lowercase()),
            accelerator: accelerator.to_owned(),
            variant,
            machine_shape,
            proxy_token: self.next_id("proxy-token"),
        };
        self.assignments.push(assignment.clone());
        let mut runtime = MockRuntime::with_defaults();
        runtime.files.extend(self.new_runtime_files.clone());
        self.runtimes.insert(assignment.endpoint.clone(), runtime);
        assignment
    }

    pub fn assignment(&self, endpoint: &str) -> Option<&MockAssignment> {
        self.assignments.iter().find(|assignment| assignment.endpoint == endpoint)
    }

    pub fn runtime(&self, endpoint: &str) -> Option<&MockRuntime> {
        self.runtimes.get(endpoint)
    }

    pub fn runtime_mut(&mut self, endpoint: &str) -> Option<&mut MockRuntime> {
        self.runtimes.get_mut(endpoint)
    }

    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        self.requests.iter().filter(|request| request.path == path).cloned().collect()
    }

    pub fn requests_matching(&self, method: &str, path_prefix: &str) -> Vec<RecordedRequest> {
        self.requests
            .iter()
            .filter(|request| request.method == method && request.path.starts_with(path_prefix))
            .cloned()
            .collect()
    }
}
