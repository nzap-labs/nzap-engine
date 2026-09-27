//! A locally tracked runtime and its UI view.

use serde::{Deserialize, Serialize};

use crate::config::{self, hardware_label, shape_display_label};

/// Colab session ceilings (free tier; documented in Colab's FAQ). The
/// keep-alive loop handles the idle timeout, but the lifetime cap still
/// applies — the UI shows how much of it remains.
pub const MAX_LIFETIME_SECONDS: f64 = 12.0 * 3600.0;
pub const IDLE_TIMEOUT_SECONDS: f64 = 90.0 * 60.0;

pub fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or_default()
}

/// Names follow the hosted NZAP rule: 1–48 of `A-Z a-z 0-9 . _ -`.
pub fn validate_name(name: &str) -> crate::Result<&str> {
    let valid = !name.is_empty()
        && name.len() <= 48
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid && name != "." && name != ".." {
        Ok(name)
    } else {
        Err(crate::Error::invalid(
            "Runtime names use 1–48 letters, digits, dots, dashes or underscores.",
        ))
    }
}

/// What `sessions.json` stores. Holds the runtime-proxy token, so it never
/// crosses IPC — the UI gets a [`SessionView`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub name: String,
    pub endpoint: String,
    pub url: String,
    pub token: String,
    #[serde(default = "default_accelerator")]
    pub accelerator: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub machine_shape: String,
    #[serde(default)]
    pub kernel_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub created_at: f64,
    pub last_activity: f64,
    #[serde(default)]
    pub last_keepalive: Option<f64>,
    #[serde(default)]
    pub keepalive_error: Option<String>,
    #[serde(default)]
    pub drive_pending_uri: Option<String>,
    #[serde(default)]
    pub drive_authorized: bool,
}

fn default_accelerator() -> String {
    "NONE".to_owned()
}

/// A JSON scalar Colab may send as a number or a string, as text.
pub fn scalar_text(value: Option<&serde_json::Value>, fallback: &str) -> String {
    match value {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Number(number)) => number.to_string(),
        _ => fallback.to_owned(),
    }
}

impl SessionState {
    /// Build from a `/tun/m/assign` or `/tun/m/assignments` entry.
    pub fn from_assignment(
        name: &str,
        assignment: &serde_json::Value,
        fallback_accelerator: &str,
    ) -> Self {
        let proxy = assignment.get("runtimeProxyInfo");
        let text = |value: Option<&serde_json::Value>| {
            value.and_then(serde_json::Value::as_str).unwrap_or_default().to_owned()
        };
        let now = unix_now();
        Self {
            name: name.to_owned(),
            endpoint: text(assignment.get("endpoint")),
            url: text(proxy.and_then(|proxy| proxy.get("url"))),
            token: text(proxy.and_then(|proxy| proxy.get("token"))),
            accelerator: scalar_text(assignment.get("accelerator"), fallback_accelerator),
            variant: scalar_text(assignment.get("variant"), "0"),
            machine_shape: scalar_text(assignment.get("machineShape"), "0"),
            kernel_id: None,
            session_id: None,
            created_at: now,
            last_activity: now,
            last_keepalive: None,
            keepalive_error: None,
            drive_pending_uri: None,
            drive_authorized: false,
        }
    }

    pub fn hardware(&self) -> String {
        hardware_label(&self.accelerator)
    }

    pub fn view(
        &self,
        colab_host: &str,
        connected: bool,
        kernel_state: Option<String>,
    ) -> SessionView {
        let now = unix_now();
        let uptime = (now - self.created_at).max(0.0);
        let idle = (now - self.last_activity).max(0.0);
        SessionView {
            name: self.name.clone(),
            endpoint: self.endpoint.clone(),
            accelerator: self.hardware(),
            variant: self.variant.clone(),
            shape: shape_display_label(&self.machine_shape).to_owned(),
            kernel_id: self.kernel_id.clone(),
            session_id: self.session_id.clone(),
            created_at: self.created_at,
            last_activity: self.last_activity,
            last_keepalive: self.last_keepalive,
            keepalive_error: self.keepalive_error.clone(),
            drive_pending_uri: self.drive_pending_uri.clone(),
            drive_authorized: self.drive_authorized,
            connected,
            kernel_state: if connected { kernel_state } else { None },
            colab_url: (!self.endpoint.is_empty())
                .then(|| config::colab_connect_url(&self.endpoint, colab_host)),
            uptime_seconds: uptime as u64,
            idle_seconds: idle as u64,
            lifetime_remaining_seconds: (MAX_LIFETIME_SECONDS - uptime).max(0.0) as u64,
            idle_remaining_seconds: (IDLE_TIMEOUT_SECONDS - idle).max(0.0) as u64,
        }
    }
}

/// The UI's view of a runtime (never includes the proxy token).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub name: String,
    pub endpoint: String,
    /// `CPU`, `T4`, `L4`, `A100`, `H100`, `V5E1`, `V6E1`…
    pub accelerator: String,
    pub variant: String,
    /// `Standard` | `High-RAM`
    pub shape: String,
    pub kernel_id: Option<String>,
    pub session_id: Option<String>,
    pub created_at: f64,
    pub last_activity: f64,
    pub last_keepalive: Option<f64>,
    pub keepalive_error: Option<String>,
    pub drive_pending_uri: Option<String>,
    pub drive_authorized: bool,
    pub connected: bool,
    /// Last kernel `execution_state` while connected.
    pub kernel_state: Option<String>,
    /// Opens Colab's web UI attached to this VM (the `colab url` format).
    pub colab_url: Option<String>,
    pub uptime_seconds: u64,
    pub idle_seconds: u64,
    pub lifetime_remaining_seconds: u64,
    pub idle_remaining_seconds: u64,
}

/// A VM the account holds, as listed on the Runtimes page.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentView {
    pub endpoint: String,
    pub accelerator: String,
    pub variant: String,
    pub shape: String,
    /// True when the engine already tracks the VM as a runtime.
    pub managed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names() {
        assert!(validate_name("gpu-box_1.a").is_ok());
        let long = "x".repeat(49);
        for bad in ["", "..", "a/b", "a b", "ü", long.as_str()] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn from_assignment_and_view() {
        let state = SessionState::from_assignment(
            "box",
            &json!({
                "endpoint": "ep-1",
                "accelerator": "T4",
                "variant": 1,
                "machineShape": 1,
                "runtimeProxyInfo": {"url": "https://p/", "token": "secret"},
            }),
            "NONE",
        );
        assert_eq!((state.variant.as_str(), state.machine_shape.as_str()), ("1", "1"));
        let view = state.view("https://colab.research.google.com", true, Some("idle".into()));
        assert_eq!(view.accelerator, "T4");
        assert_eq!(view.shape, "High-RAM");
        assert_eq!(view.kernel_state.as_deref(), Some("idle"));
        assert!(view.colab_url.unwrap().contains("dbu=%2Ftun%2Fm%2Fep-1"));
        assert!(view.lifetime_remaining_seconds > 43_000);
        let json = serde_json::to_string(&state.view("h", false, Some("busy".into()))).unwrap();
        assert!(!json.contains("secret"), "the proxy token never reaches the UI");
        assert!(json.contains("\"kernelState\":null"));
    }

    #[test]
    fn stored_state_is_forward_compatible() {
        let parsed: SessionState = serde_json::from_value(json!({
            "name": "a", "endpoint": "e", "url": "u", "token": "t",
            "createdAt": 1.0, "lastActivity": 2.0, "futureField": true
        }))
        .unwrap();
        assert_eq!(parsed.accelerator, "NONE");
        assert!(!parsed.drive_authorized);
    }
}
