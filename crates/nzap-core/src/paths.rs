//! Where the engine keeps its files. The desktop shell resolves the
//! platform directories (via Tauri's path API) and hands them in, so the
//! core never guesses OS conventions and tests can use a temp directory.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    /// Durable user data: runtimes, history, private notebooks.
    pub data_dir: PathBuf,
    /// Settings and the non-secret Google identity.
    pub config_dir: PathBuf,
    /// Re-downloadable data: the public notebook catalog.
    pub cache_dir: PathBuf,
}

impl AppPaths {
    /// All three directories under one root (tests, portable installs).
    pub fn under(root: &Path) -> Self {
        Self {
            data_dir: root.join("data"),
            config_dir: root.join("config"),
            cache_dir: root.join("cache"),
        }
    }

    pub fn identity_file(&self) -> PathBuf {
        self.config_dir.join("account.json")
    }

    /// Only used when no OS keychain is available.
    pub fn secrets_fallback_file(&self) -> PathBuf {
        self.config_dir.join("secrets.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// A user-supplied OAuth client (`oauth-client.json`), as colab-studio
    /// read from `~/.config/colab-studio/oauth-client.json`.
    pub fn oauth_client_file(&self) -> PathBuf {
        self.config_dir.join("oauth-client.json")
    }

    pub fn sessions_file(&self) -> PathBuf {
        self.data_dir.join("sessions.json")
    }

    pub fn history_dir(&self) -> PathBuf {
        self.data_dir.join("history")
    }

    pub fn notebooks_dir(&self) -> PathBuf {
        self.data_dir.join("notebooks")
    }

    pub fn catalog_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("catalog")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let paths = AppPaths::under(Path::new("/tmp/nzap"));
        assert_eq!(paths.identity_file(), Path::new("/tmp/nzap/config/account.json"));
        assert_eq!(paths.history_dir(), Path::new("/tmp/nzap/data/history"));
        assert_eq!(paths.catalog_cache_dir(), Path::new("/tmp/nzap/cache/catalog"));
    }
}
