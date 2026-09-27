//! The engine's error model.
//!
//! Every failure maps to a stable [`ErrorCode`] that crosses IPC as
//! `{ code, message }`, so the UI can react to *kinds* of failure (reconnect
//! Google, release a runtime, fix a field) without parsing messages.

use serde::Serialize;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No Google account is connected.
    #[error("Connect your Google account first.")]
    NotConnected,

    /// The stored Google credentials no longer work (revoked or expired
    /// refresh token); the user must connect again.
    #[error("{0}")]
    AuthExpired(String),

    /// The OAuth flow itself failed (cancelled, bad state, exchange refused).
    #[error("{0}")]
    Auth(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    InvalidInput(String),

    /// HTTP 412 from `/tun/m/assign`: the account already holds the maximum
    /// number of VMs (colab-cli's `TooManyAssignmentsError`).
    #[error(
        "This account already holds the maximum number of Colab VMs. \
         Release one of the existing runtimes first."
    )]
    TooManyAssignments,

    /// Colab refused the requested accelerator (no quota or entitlement).
    #[error("{0}")]
    Quota(String),

    /// A Colab control-plane request failed. `body` is kept for callers that
    /// need to inspect it (consent redirects) and is never shown verbatim.
    #[error("{message}")]
    Colab {
        status: Option<u16>,
        message: String,
        body: String,
    },

    /// The runtime's Jupyter server (via its proxy) failed.
    #[error("{message}")]
    Runtime {
        status: Option<u16>,
        message: String,
    },

    /// Network-level failure (DNS, TLS, timeout). URLs are stripped because
    /// runtime-proxy URLs carry tokens in their query strings.
    #[error("{0}")]
    Network(String),

    #[error("{0}")]
    Io(String),

    /// The operation was cancelled by the user.
    #[error("Cancelled.")]
    Cancelled,

    #[error("{0}")]
    Internal(String),
}

/// Stable, machine-readable error kinds shared with the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotConnected,
    AuthExpired,
    Auth,
    NotFound,
    InvalidInput,
    TooManyRuntimes,
    Quota,
    Colab,
    Runtime,
    Network,
    Io,
    Cancelled,
    Internal,
}

impl Error {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotConnected => ErrorCode::NotConnected,
            Self::AuthExpired(_) => ErrorCode::AuthExpired,
            Self::Auth(_) => ErrorCode::Auth,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::TooManyAssignments => ErrorCode::TooManyRuntimes,
            Self::Quota(_) => ErrorCode::Quota,
            Self::Colab { .. } => ErrorCode::Colab,
            Self::Runtime { .. } => ErrorCode::Runtime,
            Self::Network(_) => ErrorCode::Network,
            Self::Io(_) => ErrorCode::Io,
            Self::Cancelled => ErrorCode::Cancelled,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    /// The HTTP status behind the error, when there was one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Colab { status, .. } | Self::Runtime { status, .. } => *status,
            Self::TooManyAssignments => Some(412),
            Self::NotFound(_) => Some(404),
            _ => None,
        }
    }

    /// True when Google rejected the credentials themselves (401/403 from a
    /// Colab endpoint, or an expired/revoked refresh token).
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, Self::AuthExpired(_) | Self::NotConnected)
            || matches!(self, Self::Colab { status: Some(401), .. })
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn runtime(status: Option<u16>, message: impl Into<String>) -> Self {
        Self::Runtime {
            status,
            message: message.into(),
        }
    }

    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
            status: self.status(),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        // `without_url` drops the URL (and any token in its query string).
        let error = error.without_url();
        if error.is_timeout() {
            Self::Network("The request timed out.".to_owned())
        } else if error.is_connect() {
            Self::Network(format!("Could not connect: {error}"))
        } else {
            Self::Network(error.to_string())
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Internal(format!("Unexpected response format: {error}"))
    }
}

/// What crosses IPC for a failed command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_payloads() {
        let error = Error::TooManyAssignments;
        let payload = error.payload();
        assert_eq!(payload.code, ErrorCode::TooManyRuntimes);
        assert_eq!(payload.status, Some(412));
        let json = serde_json::to_value(&payload).unwrap_or_default();
        assert_eq!(json["code"], "too_many_runtimes");
    }

    #[test]
    fn colab_body_is_not_part_of_the_message() {
        let error = Error::Colab {
            status: Some(403),
            message: "GET /tun/m/assign failed: 403".to_owned(),
            body: "secret-looking body".to_owned(),
        };
        assert!(!error.to_string().contains("secret"));
        assert!(!error.is_auth_failure());
        let auth = Error::Colab {
            status: Some(401),
            message: String::new(),
            body: String::new(),
        };
        assert!(auth.is_auth_failure());
    }
}
