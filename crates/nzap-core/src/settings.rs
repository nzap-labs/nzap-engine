//! User settings (`settings.json` in the config directory).

use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::notebooks::DEFAULT_CATALOG_URL;
use crate::secrets::write_private;

pub const MIN_KEEP_ALIVE_SECONDS: u64 = 30;
pub const MAX_KEEP_ALIVE_SECONDS: u64 = 600;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Base URL of the public notebook collection.
    pub catalog_url: String,
    /// Ping every tracked runtime so Colab does not idle it out.
    pub keep_alive: bool,
    pub keep_alive_interval_seconds: u64,
    /// Closing the window keeps the app (and its keep-alive) in the tray.
    pub close_to_tray: bool,
    /// Where job artifacts are saved; `None` = the OS downloads folder.
    pub artifacts_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            catalog_url: DEFAULT_CATALOG_URL.to_owned(),
            keep_alive: true,
            keep_alive_interval_seconds: 60,
            close_to_tray: false,
            artifacts_dir: None,
        }
    }
}

/// A partial update from the Settings page.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub catalog_url: Option<String>,
    pub keep_alive: Option<bool>,
    pub keep_alive_interval_seconds: Option<u64>,
    pub close_to_tray: Option<bool>,
    /// An empty string resets to the default folder.
    pub artifacts_dir: Option<String>,
}

/// `https://…`, or plain `http://` to a loopback host (local catalogs and
/// tests).
pub fn valid_catalog_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        "http" => matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        _ => false,
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: RwLock<Settings>,
}

impl SettingsStore {
    /// Load settings; a missing or unreadable file means defaults.
    pub fn load(path: PathBuf) -> Self {
        let current = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .map(|mut settings| {
                settings.keep_alive_interval_seconds = settings
                    .keep_alive_interval_seconds
                    .clamp(MIN_KEEP_ALIVE_SECONDS, MAX_KEEP_ALIVE_SECONDS);
                if !valid_catalog_url(&settings.catalog_url) {
                    settings.catalog_url = DEFAULT_CATALOG_URL.to_owned();
                }
                settings
            })
            .unwrap_or_default();
        Self { path, current: RwLock::new(current) }
    }

    pub fn get(&self) -> Settings {
        self.current.read().map(|settings| settings.clone()).unwrap_or_default()
    }

    pub fn update(&self, patch: SettingsPatch) -> Result<Settings> {
        let mut next = self.get();
        if let Some(url) = patch.catalog_url {
            let url = url.trim().to_owned();
            if !valid_catalog_url(&url) {
                return Err(Error::invalid("The catalog URL must be an https:// address."));
            }
            next.catalog_url = if url.ends_with('/') { url } else { format!("{url}/") };
        }
        if let Some(enabled) = patch.keep_alive {
            next.keep_alive = enabled;
        }
        if let Some(seconds) = patch.keep_alive_interval_seconds {
            if !(MIN_KEEP_ALIVE_SECONDS..=MAX_KEEP_ALIVE_SECONDS).contains(&seconds) {
                return Err(Error::invalid(format!(
                    "The keep-alive interval must be between {MIN_KEEP_ALIVE_SECONDS} and \
                     {MAX_KEEP_ALIVE_SECONDS} seconds."
                )));
            }
            next.keep_alive_interval_seconds = seconds;
        }
        if let Some(close_to_tray) = patch.close_to_tray {
            next.close_to_tray = close_to_tray;
        }
        if let Some(dir) = patch.artifacts_dir {
            let dir = dir.trim().to_owned();
            next.artifacts_dir = (!dir.is_empty()).then_some(dir);
        }
        write_private(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        if let Ok(mut current) = self.current.write() {
            *current = next.clone();
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_updates_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let store = SettingsStore::load(path.clone());
        assert_eq!(store.get(), Settings::default());

        let updated = store
            .update(SettingsPatch {
                catalog_url: Some("https://example.com/catalog".into()),
                keep_alive_interval_seconds: Some(120),
                artifacts_dir: Some("/tmp/out".into()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(updated.catalog_url, "https://example.com/catalog/");
        assert_eq!(updated.keep_alive_interval_seconds, 120);
        assert_eq!(SettingsStore::load(path.clone()).get(), updated);

        let reset = store.update(SettingsPatch { artifacts_dir: Some(" ".into()), ..SettingsPatch::default() });
        assert_eq!(reset.unwrap().artifacts_dir, None);

        for url in ["http://example.com/", "ftp://x/", "not a url", "file:///etc/"] {
            let result = store.update(SettingsPatch { catalog_url: Some(url.into()), ..SettingsPatch::default() });
            assert!(result.is_err(), "{url}");
        }
        assert!(valid_catalog_url("http://127.0.0.1:8080/static/catalog/"));
        assert!(store
            .update(SettingsPatch { keep_alive_interval_seconds: Some(5), ..SettingsPatch::default() })
            .is_err());
    }

    #[test]
    fn a_tampered_file_is_sanitised() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"catalogUrl": "javascript:alert(1)", "keepAliveIntervalSeconds": 1}"#).unwrap();
        let settings = SettingsStore::load(path).get();
        assert_eq!(settings.catalog_url, DEFAULT_CATALOG_URL);
        assert_eq!(settings.keep_alive_interval_seconds, MIN_KEEP_ALIVE_SECONDS);
        assert!(settings.keep_alive);
    }
}
