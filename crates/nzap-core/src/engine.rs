//! The engine as the desktop shell sees it: every subsystem wired together,
//! plus the few operations that span them (connection status, account,
//! disconnect, settings).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::auth::{AuthManager, DisconnectReason, GoogleUser, OAuthClient};
use crate::colab::ColabClient;
use crate::config::{Endpoints, GPU_CHOICES, HIGH_MEM_ONLY_CHOICES, TPU_CHOICES};
use crate::error::{Error, Result};
use crate::history::HistoryLog;
use crate::notebooks::{Catalog, NotebookLibrary, NotebookStore};
use crate::paths::AppPaths;
use crate::secrets::{self, FileStore, SecretStore, StorageKind};
use crate::session::SessionManager;
use crate::settings::{Settings, SettingsPatch, SettingsStore};

pub struct EngineOptions {
    pub paths: AppPaths,
    pub endpoints: Endpoints,
    /// Use the OS keychain (off in tests and portable mode).
    pub use_keychain: bool,
    /// Overrides `oauth-client.json` (the `NZAP_OAUTH_CLIENT_JSON` variable).
    pub oauth_client: Option<OAuthClient>,
    /// Where runtimes are persisted instead of `sessions.json`. An agent
    /// server (`nzap-engine mcp`) keeps its own list, so it never overwrites
    /// the app's while both run.
    pub sessions_file: Option<PathBuf>,
}

/// The Google Auth card's state — hosted NZAP's `/api/colab/status`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    /// Google still accepts the stored token *for Colab* (verified live).
    pub connected: bool,
    /// `not_connected` | `revoked` | `token_expired`
    pub reason: Option<&'static str>,
    pub email: Option<String>,
    pub user: Option<GoogleUser>,
    pub warning: Option<String>,
    pub storage: StorageKind,
    pub custom_client: bool,
}

/// The hardware picker's options (colab-studio `/api/config`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareConfig {
    pub gpus: Vec<&'static str>,
    pub tpus: Vec<&'static str>,
    pub high_mem_only: Vec<&'static str>,
    pub keep_alive_interval: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub user: Option<GoogleUser>,
    /// Colab's own account block (`ccu-info`, or `user-info` when allowed).
    pub colab: Option<Value>,
    pub error: Option<String>,
}

pub struct Engine {
    pub paths: AppPaths,
    pub auth: Arc<AuthManager>,
    pub colab: Arc<ColabClient>,
    pub sessions: SessionManager,
    pub notebooks: NotebookLibrary,
    pub settings: SettingsStore,
}

impl Engine {
    pub fn new(options: EngineOptions) -> Result<Self> {
        let paths = options.paths;
        for dir in [&paths.data_dir, &paths.config_dir, &paths.cache_dir] {
            std::fs::create_dir_all(dir)?;
        }
        let settings = SettingsStore::load(paths.settings_file());
        let current = settings.get();
        let http = crate::http::build_client()?;

        let secret_store: Arc<dyn SecretStore> = if options.use_keychain {
            secrets::default_store(crate::KEYCHAIN_SERVICE, paths.secrets_fallback_file())
        } else {
            Arc::new(FileStore::new(paths.secrets_fallback_file()))
        };
        let oauth_client = options
            .oauth_client
            .or_else(|| {
                let text = std::fs::read_to_string(paths.oauth_client_file()).ok()?;
                OAuthClient::from_json(&text)
                    .map_err(|error| tracing::warn!("Ignoring oauth-client.json: {error}"))
                    .ok()
            })
            .unwrap_or_default();
        let auth = Arc::new(AuthManager::new(
            http.clone(),
            options.endpoints,
            oauth_client,
            secret_store,
            paths.identity_file(),
        ));
        let colab = Arc::new(ColabClient::new(auth.clone()));
        let sessions = SessionManager::new(
            colab.clone(),
            Arc::new(HistoryLog::new(paths.history_dir())),
            options.sessions_file.unwrap_or_else(|| paths.sessions_file()),
        );
        sessions.set_keepalive(
            current.keep_alive,
            Duration::from_secs(current.keep_alive_interval_seconds),
        );
        let notebooks = NotebookLibrary::new(
            Catalog::new(http, &current.catalog_url, paths.catalog_cache_dir()),
            NotebookStore::new(paths.notebooks_dir()),
        );
        Ok(Self { paths, auth, colab, sessions, notebooks, settings })
    }

    pub fn hardware_config(&self) -> HardwareConfig {
        HardwareConfig {
            gpus: GPU_CHOICES.to_vec(),
            tpus: TPU_CHOICES.to_vec(),
            high_mem_only: HIGH_MEM_ONLY_CHOICES.to_vec(),
            keep_alive_interval: self.settings.get().keep_alive_interval_seconds,
        }
    }

    /// Whether Google still accepts the connection for Colab. The dot is
    /// green only after a real Colab endpoint answered, not merely because a
    /// token is stored.
    pub async fn status(&self) -> ConnectionStatus {
        let snapshot = self.auth.snapshot().await;
        let base = |connected: bool, reason: Option<&'static str>, warning: Option<String>| {
            ConnectionStatus {
                connected,
                reason,
                email: snapshot
                    .user
                    .as_ref()
                    .map(|user| user.email.clone())
                    .filter(|email| !email.is_empty()),
                user: snapshot.user.clone(),
                warning,
                storage: snapshot.storage,
                custom_client: snapshot.custom_client,
            }
        };
        if !snapshot.has_credentials {
            let reason = match snapshot.reason {
                Some(DisconnectReason::Revoked) => "revoked",
                _ => "not_connected",
            };
            return base(false, Some(reason), None);
        }
        match self.colab.get_ccu_info().await {
            Ok(_) => base(true, None, None),
            Err(Error::AuthExpired(_) | Error::NotConnected) => base(false, Some("revoked"), None),
            Err(error) if matches!(error.status(), Some(401)) => base(false, Some("revoked"), None),
            // Colab itself is unreachable — the token is still good, say so.
            Err(error) => {
                base(true, None, Some(format!("Colab did not answer the liveness check: {error}")))
            }
        }
    }

    /// The signed-in user plus Colab's account block (best-effort).
    pub async fn account(&self) -> Result<AccountInfo> {
        let user = self.auth.user().await;
        if !self.auth.snapshot().await.has_credentials {
            return Err(Error::NotConnected);
        }
        match self.colab.get_user_info(true).await {
            Ok(info) => Ok(AccountInfo { user, colab: Some(info), error: None }),
            Err(Error::AuthExpired(message)) => Err(Error::AuthExpired(message)),
            Err(_) => match self.colab.get_ccu_info().await {
                Ok(info) => Ok(AccountInfo { user, colab: Some(info), error: None }),
                Err(error) => Ok(AccountInfo { user, colab: None, error: Some(error.to_string()) }),
            },
        }
    }

    /// Release every runtime this app tracks, then forget the Google
    /// connection. A release failure never blocks the disconnect.
    pub async fn disconnect(&self) -> Result<()> {
        if self.auth.snapshot().await.has_credentials {
            self.sessions.stop_all().await;
        }
        self.auth.disconnect().await
    }

    /// After launch (and after connecting): drop runtimes whose VM is gone
    /// and resume keep-alive for the rest.
    pub async fn resume(&self) {
        if !self.auth.snapshot().await.has_credentials {
            return;
        }
        if let Err(error) = self.sessions.resume().await {
            tracing::info!("Could not reconcile runtimes: {error}");
        }
    }

    pub fn update_settings(&self, patch: SettingsPatch) -> Result<Settings> {
        let settings = self.settings.update(patch)?;
        self.sessions.set_keepalive(
            settings.keep_alive,
            Duration::from_secs(settings.keep_alive_interval_seconds),
        );
        if settings.keep_alive {
            for name in self.sessions.names() {
                self.sessions.start_keepalive(&name);
            }
        }
        if self.notebooks.catalog().base_url() != settings.catalog_url {
            self.notebooks.catalog().set_base_url(&settings.catalog_url);
        }
        Ok(settings)
    }

    /// Bring your own OAuth client (`None` restores the default). Tokens are
    /// bound to the client that minted them, so this requires disconnecting.
    pub async fn set_oauth_client(&self, json: Option<&str>) -> Result<bool> {
        let client = match json.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => Some(OAuthClient::from_json(text)?),
            None => None,
        };
        let next = client.clone().unwrap_or_default();
        if next == self.auth.oauth_client() {
            return Ok(false);
        }
        if self.auth.snapshot().await.has_credentials {
            return Err(Error::invalid("Disconnect Google before changing the OAuth client."));
        }
        let file = self.paths.oauth_client_file();
        match (client, json) {
            (Some(_), Some(text)) => crate::secrets::write_private(&file, text.trim().as_bytes())?,
            _ => {
                if let Err(error) = std::fs::remove_file(&file) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(error.into());
                    }
                }
            }
        }
        self.auth.set_oauth_client(next);
        Ok(true)
    }

    /// Stop background work (app exit). Runtimes keep running on Google's side.
    pub fn shutdown(&self) {
        self.sessions.shutdown();
    }
}
