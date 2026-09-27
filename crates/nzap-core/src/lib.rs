//! NZAP Engine core.
//!
//! A pure-Rust port of `colab-studio` (itself a port of `google-colab-cli`
//! and `colab-vscode`): Google OAuth, the Colab control plane, the runtime's
//! Jupyter server, kernels, terminals, history, jobs and the notebook
//! library. It has no Tauri dependency so the whole engine is testable with
//! plain `cargo test` against `nzap-mock-colab`.

/// The engine version, reported in the app's About panel and client agent.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        assert_eq!(VERSION.split('.').count(), 3);
    }
}
