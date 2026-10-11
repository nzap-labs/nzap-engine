//! User settings (`settings.json` in the config directory).

use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::notebooks::DEFAULT_CATALOG_URL;
use crate::secrets::write_private;

pub const MIN_KEEP_ALIVE_SECONDS: u64 = 30;
pub const MAX_KEEP_ALIVE_SECONDS: u64 = 600;

/// The repository whose dev container starts Jupyter in a codespace
/// (`runtimes/codespaces` in this repository is its content).
pub const DEFAULT_CODESPACES_REPOSITORY: &str = "nzap-labs/nzap-runtime";
pub const DEFAULT_CODESPACES_DEVCONTAINER: &str = ".devcontainer/devcontainer.json";
pub const DEFAULT_CODESPACES_MACHINE: &str = "basicLinux32gb";
/// CodeSandbox's universal template (`getDefaultTemplateTag` in the SDK).
pub const DEFAULT_CODESANDBOX_TEMPLATE: &str = "pcz35m";
pub const DEFAULT_CODESANDBOX_TIER: &str = "Nano";
/// The VM tiers CodeSandbox offers (`VMTier` in the SDK).
pub const CODESANDBOX_TIERS: &[&str] =
    &["Pico", "Nano", "Micro", "Small", "Medium", "Large", "XLarge"];
pub const DEFAULT_AUTO_RELEASE_MINUTES: u64 = 30;
pub const MAX_AUTO_RELEASE_MINUTES: u64 = 24 * 60;

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
    /// `owner/repo` whose dev container serves Jupyter in a codespace.
    pub codespaces_repository: String,
    pub codespaces_devcontainer_path: String,
    pub codespaces_machine: String,
    /// The template (sandbox id or tag) new CodeSandbox VMs fork.
    pub codesandbox_template: String,
    pub codesandbox_tier: String,
    /// Delete an idle Codespaces / CodeSandbox runtime after this many
    /// minutes without a cell or file operation (0 = never).
    pub auto_release_minutes: u64,
    /// Your own GitHub OAuth app for "Sign in with GitHub" (device flow).
    pub github_client_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            catalog_url: DEFAULT_CATALOG_URL.to_owned(),
            keep_alive: true,
            keep_alive_interval_seconds: 60,
            close_to_tray: false,
            artifacts_dir: None,
            codespaces_repository: DEFAULT_CODESPACES_REPOSITORY.to_owned(),
            codespaces_devcontainer_path: DEFAULT_CODESPACES_DEVCONTAINER.to_owned(),
            codespaces_machine: DEFAULT_CODESPACES_MACHINE.to_owned(),
            codesandbox_template: DEFAULT_CODESANDBOX_TEMPLATE.to_owned(),
            codesandbox_tier: DEFAULT_CODESANDBOX_TIER.to_owned(),
            auto_release_minutes: DEFAULT_AUTO_RELEASE_MINUTES,
            github_client_id: None,
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
    pub codespaces_repository: Option<String>,
    pub codespaces_devcontainer_path: Option<String>,
    pub codespaces_machine: Option<String>,
    pub codesandbox_template: Option<String>,
    pub codesandbox_tier: Option<String>,
    pub auto_release_minutes: Option<u64>,
    /// An empty string removes it.
    pub github_client_id: Option<String>,
}

/// `owner/repo` with GitHub's allowed characters.
pub fn valid_repository(repo: &str) -> bool {
    let part = |text: &str| {
        !text.is_empty()
            && text.len() <= 100
            && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && text != "."
            && text != ".."
    };
    matches!(repo.split_once('/'), Some((owner, name)) if part(owner) && part(name))
}

/// A relative path inside the repository, without `..`.
fn valid_repo_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path.ends_with(".json")
        && path.split('/').all(|part| !part.is_empty() && part != "..")
}

/// Letters, digits, `-` and `_` (machine names, template ids, client ids).
fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
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
                settings.auto_release_minutes =
                    settings.auto_release_minutes.min(MAX_AUTO_RELEASE_MINUTES);
                let defaults = Settings::default();
                if !valid_repository(&settings.codespaces_repository) {
                    settings.codespaces_repository = defaults.codespaces_repository;
                }
                if !valid_repo_path(&settings.codespaces_devcontainer_path) {
                    settings.codespaces_devcontainer_path = defaults.codespaces_devcontainer_path;
                }
                if !valid_token(&settings.codespaces_machine) {
                    settings.codespaces_machine = defaults.codespaces_machine;
                }
                if !valid_token(&settings.codesandbox_template) {
                    settings.codesandbox_template = defaults.codesandbox_template;
                }
                if !CODESANDBOX_TIERS.contains(&settings.codesandbox_tier.as_str()) {
                    settings.codesandbox_tier = defaults.codesandbox_tier;
                }
                settings.github_client_id = settings.github_client_id.filter(|id| valid_token(id));
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
        if let Some(repo) = patch.codespaces_repository {
            let repo = repo.trim().trim_start_matches("https://github.com/").trim_end_matches('/');
            if !valid_repository(repo) {
                return Err(Error::invalid("The repository must look like owner/name."));
            }
            next.codespaces_repository = repo.to_owned();
        }
        if let Some(path) = patch.codespaces_devcontainer_path {
            let path = path.trim();
            if !valid_repo_path(path) {
                return Err(Error::invalid(
                    "The dev container path must be a .json file inside the repository.",
                ));
            }
            next.codespaces_devcontainer_path = path.to_owned();
        }
        if let Some(machine) = patch.codespaces_machine {
            let machine = machine.trim();
            if !valid_token(machine) {
                return Err(Error::invalid("That is not a Codespaces machine name."));
            }
            next.codespaces_machine = machine.to_owned();
        }
        if let Some(template) = patch.codesandbox_template {
            let template = template.trim();
            if !valid_token(template) {
                return Err(Error::invalid("That is not a CodeSandbox template id."));
            }
            next.codesandbox_template = template.to_owned();
        }
        if let Some(tier) = patch.codesandbox_tier {
            let tier = CODESANDBOX_TIERS
                .iter()
                .find(|known| known.eq_ignore_ascii_case(tier.trim()))
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "The VM tier must be one of {}.",
                        CODESANDBOX_TIERS.join(", ")
                    ))
                })?;
            next.codesandbox_tier = (*tier).to_owned();
        }
        if let Some(minutes) = patch.auto_release_minutes {
            if minutes > MAX_AUTO_RELEASE_MINUTES {
                return Err(Error::invalid(format!(
                    "Auto-delete takes 0 (never) to {MAX_AUTO_RELEASE_MINUTES} minutes."
                )));
            }
            next.auto_release_minutes = minutes;
        }
        if let Some(id) = patch.github_client_id {
            let id = id.trim();
            if !id.is_empty() && !valid_token(id) {
                return Err(Error::invalid("That is not a GitHub OAuth client id."));
            }
            next.github_client_id = (!id.is_empty()).then(|| id.to_owned());
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

        let reset = store
            .update(SettingsPatch { artifacts_dir: Some(" ".into()), ..SettingsPatch::default() });
        assert_eq!(reset.unwrap().artifacts_dir, None);

        for url in ["http://example.com/", "ftp://x/", "not a url", "file:///etc/"] {
            let result = store.update(SettingsPatch {
                catalog_url: Some(url.into()),
                ..SettingsPatch::default()
            });
            assert!(result.is_err(), "{url}");
        }
        assert!(valid_catalog_url("http://127.0.0.1:8080/static/catalog/"));
        assert!(store
            .update(SettingsPatch {
                keep_alive_interval_seconds: Some(5),
                ..SettingsPatch::default()
            })
            .is_err());
    }

    #[test]
    fn provider_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().join("settings.json"));
        let updated = store
            .update(SettingsPatch {
                codespaces_repository: Some("https://github.com/me/my-runtime/".into()),
                codespaces_machine: Some("standardLinux32gb".into()),
                codesandbox_tier: Some("micro".into()),
                auto_release_minutes: Some(0),
                github_client_id: Some("Iv1.abc123".into()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(updated.codespaces_repository, "me/my-runtime");
        assert_eq!(updated.codesandbox_tier, "Micro");
        assert_eq!(updated.auto_release_minutes, 0);
        assert_eq!(updated.github_client_id.as_deref(), Some("Iv1.abc123"));
        let cleared = store
            .update(SettingsPatch {
                github_client_id: Some(" ".into()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(cleared.github_client_id, None);
        for bad in [
            SettingsPatch {
                codespaces_repository: Some("no-slash".into()),
                ..SettingsPatch::default()
            },
            SettingsPatch {
                codespaces_repository: Some("a/../b".into()),
                ..SettingsPatch::default()
            },
            SettingsPatch {
                codespaces_devcontainer_path: Some("../x.json".into()),
                ..SettingsPatch::default()
            },
            SettingsPatch { codesandbox_tier: Some("Huge".into()), ..SettingsPatch::default() },
            SettingsPatch { codesandbox_template: Some("a b".into()), ..SettingsPatch::default() },
            SettingsPatch { auto_release_minutes: Some(100_000), ..SettingsPatch::default() },
        ] {
            assert!(store.update(bad).is_err());
        }
    }

    #[test]
    fn a_tampered_file_is_sanitised() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"catalogUrl": "javascript:alert(1)", "keepAliveIntervalSeconds": 1}"#,
        )
        .unwrap();
        let settings = SettingsStore::load(path).get();
        assert_eq!(settings.catalog_url, DEFAULT_CATALOG_URL);
        assert_eq!(settings.keep_alive_interval_seconds, MIN_KEEP_ALIVE_SECONDS);
        assert!(settings.keep_alive);
    }
}
