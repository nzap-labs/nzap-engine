//! NZAP Engine core.
//!
//! A pure-Rust port of `colab-studio` (itself a port of `google-colab-cli`
//! and `colab-vscode`): Google OAuth, the Colab control plane, the runtime's
//! Jupyter server, kernels, terminals, history, jobs and the notebook
//! library. It has no Tauri dependency so the whole engine is testable with
//! plain `cargo test` against `nzap-mock-colab`.

pub mod auth;
pub mod colab;
pub mod config;
pub mod error;
pub mod http;
pub mod paths;
pub mod runtime;
pub mod secrets;

pub use error::{Error, ErrorCode, ErrorPayload, Result};

/// The engine version, reported in the app's About panel and user agent.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Service name under which secrets are stored in the OS keychain.
pub const KEYCHAIN_SERVICE: &str = "com.nzaplabs.engine";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        assert_eq!(VERSION.split('.').count(), 3);
    }
}
