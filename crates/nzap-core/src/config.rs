//! Constants and runtime-shape resolution.
//!
//! A 1:1 port of colab-studio's `app/config.py`, which lifts every value from
//! the two upstream Colab clients:
//!
//! * `google-colab-cli` — `src/colab_cli/client.py`, `auth.py`,
//!   `commands/session.py`, `commands/utility.py`
//! * `colab-vscode` — `src/colab/client/v1/index.ts`, `src/auth/scopes.ts`
//!
//! No Colab-specific value here is invented; references are given inline.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Colab backends (google-colab-cli/src/colab_cli/client.py: Prod)
// ---------------------------------------------------------------------------

/// Colab's web front door, which serves the `/tun/m/*` control plane.
pub const COLAB_DOMAIN: &str = "https://colab.research.google.com";
/// Colab's API domain (`v1/user-info`, `v1beta/runtimespecs`, …).
pub const COLAB_API_DOMAIN: &str = "https://colab.pa.googleapis.com";
/// Drive v3 files endpoint, used to import notebooks by id.
pub const DRIVE_FILES_ENDPOINT: &str = "https://www.googleapis.com/drive/v3/files";

/// google-colab-cli/src/colab_cli/client.py: `TUN_ENDPOINT`
pub const TUN_ENDPOINT: &str = "/tun/m";

// ---------------------------------------------------------------------------
// Standard Colab headers (google-colab-cli/src/colab_cli/client.py)
// ---------------------------------------------------------------------------

pub const CLIENT_AGENT: &str = "nzap-engine";
pub const CLIENT_AGENT_HEADER: &str = "X-Colab-Client-Agent";
pub const TUNNEL_HEADER: &str = "X-Colab-Tunnel";
pub const TUNNEL_HEADER_VALUE: &str = "Google";
pub const XSRF_HEADER: &str = "X-Goog-Colab-Token";
pub const RUNTIME_PROXY_TOKEN_HEADER: &str = "X-Colab-Runtime-Proxy-Token";
pub const RUNTIME_PROXY_TOKEN_PARAM: &str = "colab-runtime-proxy-token";

/// XSSI guard prefix that Colab's endpoints prepend to JSON bodies.
pub const XSSI_PREFIX: &str = ")]}'\n";

/// google-colab-cli/src/colab_cli/client.py: `KEEP_ALIVE_TIMEOUT`
pub const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(10);
/// The CLI's keep-alive cadence.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(60);
/// The CLI's keep-alive ceiling: a VM is kept alive for at most 24 hours.
pub const KEEP_ALIVE_MAX: Duration = Duration::from_secs(24 * 3600);
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// OAuth
//
// Scopes are the union of colab-vscode/src/auth/scopes.ts (REQUIRED_SCOPES +
// DRIVE_SCOPES) and google-colab-cli/src/colab_cli/auth.py (PUBLIC_SCOPES).
// ---------------------------------------------------------------------------

pub const SCOPES: &[&str] = &[
    "openid",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/colaboratory",
    "https://www.googleapis.com/auth/drive.file",
];

pub const AUTH_URI: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URI: &str = "https://oauth2.googleapis.com/revoke";
pub const USERINFO_URI: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// google-colab-cli/src/colab_cli/auth.py: `REMOTE_REDIRECT_URI` — the
/// copy/paste landing page registered to the default client.
pub const REMOTE_REDIRECT_URI: &str = "https://sdk.cloud.google.com/applicationdefaultauthcode.html";

/// google-colab-cli/src/colab_cli/oauth_config.json — the installed-app
/// client Google ships with the Cloud SDK, reused by colab-cli. Installed-app
/// secrets are not confidential (they ship inside every copy of gcloud).
/// Deployments can bring their own client; see `docs/OAUTH.md`.
pub const DEFAULT_OAUTH_CLIENT_ID: &str =
    "764086051850-6qr4p6gpi6hn506pt8ejuq83di341hur.apps.googleusercontent.com";
pub const DEFAULT_OAUTH_CLIENT_SECRET: &str = "d-FL95Q19q7MQmFpd7hHD0Ty";

/// Where the engine sends its requests. Production uses Google; tests and
/// the desktop E2E suite point every field at `nzap-mock-colab`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoints {
    pub colab: String,
    pub colab_api: String,
    pub auth_uri: String,
    pub token_uri: String,
    pub revoke_uri: String,
    pub userinfo_uri: String,
    pub drive_files: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            colab: COLAB_DOMAIN.to_owned(),
            colab_api: COLAB_API_DOMAIN.to_owned(),
            auth_uri: AUTH_URI.to_owned(),
            token_uri: TOKEN_URI.to_owned(),
            revoke_uri: REVOKE_URI.to_owned(),
            userinfo_uri: USERINFO_URI.to_owned(),
            drive_files: DRIVE_FILES_ENDPOINT.to_owned(),
        }
    }
}

impl Endpoints {
    /// Every endpoint served from one base URL (the mock server layout).
    pub fn single_host(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            colab: base.to_owned(),
            colab_api: base.to_owned(),
            auth_uri: format!("{base}/o/oauth2/v2/auth"),
            token_uri: format!("{base}/token"),
            revoke_uri: format!("{base}/revoke"),
            userinfo_uri: format!("{base}/v1/userinfo"),
            drive_files: format!("{base}/drive/v3/files"),
        }
    }

    /// Apply `NZAP_*` overrides from a variable lookup. The desktop shell
    /// only calls this in development and E2E builds.
    pub fn with_overrides(mut self, lookup: impl Fn(&str) -> Option<String>) -> Self {
        if let Some(base) = lookup("NZAP_MOCK_GOOGLE") {
            self = Self::single_host(&base);
        }
        let fields: [(&str, &mut String); 7] = [
            ("NZAP_COLAB_DOMAIN", &mut self.colab),
            ("NZAP_COLAB_API_DOMAIN", &mut self.colab_api),
            ("NZAP_OAUTH_AUTH_URI", &mut self.auth_uri),
            ("NZAP_OAUTH_TOKEN_URI", &mut self.token_uri),
            ("NZAP_OAUTH_REVOKE_URI", &mut self.revoke_uri),
            ("NZAP_OAUTH_USERINFO_URI", &mut self.userinfo_uri),
            ("NZAP_DRIVE_FILES_ENDPOINT", &mut self.drive_files),
        ];
        for (name, field) in fields {
            if let Some(value) = lookup(name) {
                *field = value;
            }
        }
        self
    }

    /// Host of the Colab front door; requests to it carry `authuser=0`.
    pub fn colab_host(&self) -> Option<String> {
        url::Url::parse(&self.colab)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
    }
}

// ---------------------------------------------------------------------------
// Colab enums (google-colab-cli/src/colab_cli/client.py)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Accelerator {
    None,
    G4,
    T4,
    L4,
    A100,
    H100,
    V5e1,
    V6e1,
}

impl Accelerator {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::G4 => "G4",
            Self::T4 => "T4",
            Self::L4 => "L4",
            Self::A100 => "A100",
            Self::H100 => "H100",
            Self::V5e1 => "V5E1",
            Self::V6e1 => "V6E1",
        }
    }

    /// Accelerators that only exist in a single (high-memory) shape; the
    /// assign endpoint ignores `shape=hm` for these (colab-cli client.py).
    pub fn is_high_mem_only(self) -> bool {
        matches!(self, Self::L4 | Self::V5e1 | Self::V6e1)
    }

    /// Map a lower-case GPU choice (`t4`, `a100`, …) to an accelerator.
    pub fn from_gpu_choice(choice: &str) -> Option<Self> {
        match choice.to_ascii_lowercase().as_str() {
            "t4" => Some(Self::T4),
            "l4" => Some(Self::L4),
            "g4" => Some(Self::G4),
            "a100" => Some(Self::A100),
            "h100" => Some(Self::H100),
            _ => None,
        }
    }

    /// Map a lower-case TPU choice (`v5e1`, `v6e1`) to an accelerator.
    pub fn from_tpu_choice(choice: &str) -> Option<Self> {
        match choice.to_ascii_lowercase().as_str() {
            "v5e1" => Some(Self::V5e1),
            "v6e1" => Some(Self::V6e1),
            _ => None,
        }
    }
}

impl fmt::Display for Accelerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// GPU choices offered by the UI (`/api/config` in colab-studio).
pub const GPU_CHOICES: &[&str] = &["t4", "l4", "g4", "a100", "h100"];
/// TPU choices offered by the UI.
pub const TPU_CHOICES: &[&str] = &["v5e1", "v6e1"];
/// Choices that are always High-RAM.
pub const HIGH_MEM_ONLY_CHOICES: &[&str] = &["l4", "v5e1", "v6e1"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Variant {
    Default,
    Gpu,
    Tpu,
}

impl Variant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "DEFAULT",
            Self::Gpu => "GPU",
            Self::Tpu => "TPU",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    Standard,
    HighRam,
}

/// Map UI intent to the `shape` query param for `/tun/m/assign`.
pub fn resolve_assign_shape(accelerator: Accelerator, high_mem: bool) -> Option<Shape> {
    if !high_mem || accelerator.is_high_mem_only() {
        return None;
    }
    Some(Shape::HighRam)
}

/// Map UI selections to (variant, accelerator, shape). Direct port of
/// `resolve_runtime_options` (google-colab-cli/src/colab_cli/commands/session.py):
/// a TPU choice wins over a GPU choice, unknown GPUs fall back to A100 and
/// unknown TPUs to V6E1.
pub fn resolve_runtime_options(
    gpu: Option<&str>,
    tpu: Option<&str>,
    high_mem: bool,
) -> (Variant, Accelerator, Option<Shape>) {
    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.filter(|text| !text.trim().is_empty())
    }
    let (variant, accelerator) = if let Some(tpu) = non_empty(tpu) {
        (
            Variant::Tpu,
            Accelerator::from_tpu_choice(tpu).unwrap_or(Accelerator::V6e1),
        )
    } else if let Some(gpu) = non_empty(gpu) {
        (
            Variant::Gpu,
            Accelerator::from_gpu_choice(gpu).unwrap_or(Accelerator::A100),
        )
    } else {
        (Variant::Default, Accelerator::None)
    };
    (
        variant,
        accelerator,
        resolve_assign_shape(accelerator, high_mem),
    )
}

/// `NONE` → `CPU`; everything else passes through.
pub fn hardware_label(accelerator: &str) -> String {
    if accelerator.is_empty() || accelerator == "NONE" {
        "CPU".to_owned()
    } else {
        accelerator.to_owned()
    }
}

/// Human-friendly label for a `machineShape` value as Colab reports it
/// (`1`, `"1"` or `"HIGH_RAM"` mean High-RAM).
pub fn shape_display_label(shape: &str) -> &'static str {
    match shape {
        "1" | "HIGH_RAM" | "HighRam" => "High-RAM",
        _ => "Standard",
    }
}

/// Python's `urllib.parse.quote(value, safe='')`: everything but RFC 3986
/// unreserved characters is escaped (so `/` becomes `%2F`).
pub const QUOTE_SAFE_NONE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Browser URL that opens Colab's web UI attached to an existing VM.
///
/// Port of `colab url` (google-colab-cli/src/colab_cli/commands/utility.py):
/// both backend signals are emitted — `?dbu=` (URL-encoded path) and the raw
/// `#datalabBackendUrl=` fragment — so the frontend skips `/tun/m/assign`.
pub fn colab_connect_url(endpoint: &str, host: &str) -> String {
    let host = host.trim_end_matches('/');
    let backend_path = format!("{TUN_ENDPOINT}/{endpoint}");
    let encoded = percent_encoding::utf8_percent_encode(&backend_path, QUOTE_SAFE_NONE).to_string();
    format!("{host}/notebooks/empty.ipynb?dbu={encoded}#datalabBackendUrl={host}{backend_path}")
}

/// Encode a notebook UUID the way Colab's `nbh` param expects it (colab-cli
/// client.py `uuid_to_web_safe_base64`): dashes become underscores and the
/// value is right-padded with dots to 44 characters.
pub fn uuid_to_web_safe_base64(value: &uuid::Uuid) -> String {
    let text = value.hyphenated().to_string();
    let padding = ".".repeat(44usize.saturating_sub(text.len()));
    format!("{}{padding}", text.replace('-', "_"))
}

/// A normalised runtime request coming from the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRequest {
    pub name: String,
    #[serde(default)]
    pub gpu: Option<String>,
    #[serde(default)]
    pub tpu: Option<String>,
    #[serde(default)]
    pub high_mem: bool,
}

impl RuntimeRequest {
    pub fn resolved(&self) -> (Variant, Accelerator, Option<Shape>) {
        resolve_runtime_options(self.gpu.as_deref(), self.tpu.as_deref(), self.high_mem)
    }

    /// `CPU`, `T4 High-RAM`, `V6E1`, …
    pub fn label(&self) -> String {
        let (_, accelerator, shape) = self.resolved();
        let high_ram = if shape == Some(Shape::HighRam) {
            " High-RAM"
        } else {
            ""
        };
        format!("{}{high_ram}", hardware_label(accelerator.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tpu_wins_over_gpu_and_defaults_apply() {
        assert_eq!(
            resolve_runtime_options(Some("t4"), Some("v5e1"), false),
            (Variant::Tpu, Accelerator::V5e1, None)
        );
        assert_eq!(
            resolve_runtime_options(Some("mystery"), None, false),
            (Variant::Gpu, Accelerator::A100, None)
        );
        assert_eq!(
            resolve_runtime_options(None, Some("mystery"), false),
            (Variant::Tpu, Accelerator::V6e1, None)
        );
        assert_eq!(
            resolve_runtime_options(Some(""), None, true),
            (Variant::Default, Accelerator::None, Some(Shape::HighRam))
        );
    }

    #[test]
    fn high_mem_is_ignored_for_single_shape_accelerators() {
        assert_eq!(resolve_assign_shape(Accelerator::L4, true), None);
        assert_eq!(resolve_assign_shape(Accelerator::V6e1, true), None);
        assert_eq!(
            resolve_assign_shape(Accelerator::T4, true),
            Some(Shape::HighRam)
        );
        assert_eq!(resolve_assign_shape(Accelerator::T4, false), None);
    }

    #[test]
    fn labels() {
        assert_eq!(hardware_label("NONE"), "CPU");
        assert_eq!(hardware_label("T4"), "T4");
        assert_eq!(shape_display_label("1"), "High-RAM");
        assert_eq!(shape_display_label("HIGH_RAM"), "High-RAM");
        assert_eq!(shape_display_label("0"), "Standard");
        let request = RuntimeRequest {
            name: "a".into(),
            gpu: Some("t4".into()),
            tpu: None,
            high_mem: true,
        };
        assert_eq!(request.label(), "T4 High-RAM");
    }

    #[test]
    fn accelerator_serializes_like_colab() {
        assert_eq!(
            serde_json::to_string(&Accelerator::V5e1).ok().as_deref(),
            Some("\"V5E1\"")
        );
        assert_eq!(Accelerator::None.as_str(), "NONE");
    }

    #[test]
    fn connect_url_matches_colab_cli() {
        assert_eq!(
            colab_connect_url("gpu-t4-s-abc", "https://colab.research.google.com/"),
            "https://colab.research.google.com/notebooks/empty.ipynb\
             ?dbu=%2Ftun%2Fm%2Fgpu-t4-s-abc\
             #datalabBackendUrl=https://colab.research.google.com/tun/m/gpu-t4-s-abc"
        );
    }

    #[test]
    fn notebook_hash_is_44_chars() {
        let id = uuid::Uuid::nil();
        let encoded = uuid_to_web_safe_base64(&id);
        assert_eq!(encoded.len(), 44);
        assert!(encoded.starts_with("00000000_0000_"));
        assert!(encoded.ends_with("........"));
    }

    #[test]
    fn endpoint_overrides() {
        let endpoints = Endpoints::default().with_overrides(|name| match name {
            "NZAP_MOCK_GOOGLE" => Some("http://127.0.0.1:9/".to_owned()),
            "NZAP_OAUTH_TOKEN_URI" => Some("http://x/token".to_owned()),
            _ => None,
        });
        assert_eq!(endpoints.colab, "http://127.0.0.1:9");
        assert_eq!(endpoints.token_uri, "http://x/token");
        assert_eq!(endpoints.colab_host().as_deref(), Some("127.0.0.1"));
    }
}
