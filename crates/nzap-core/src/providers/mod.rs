//! Compute beyond Colab: GitHub Codespaces and CodeSandbox VM sandboxes.
//!
//! Every provider ends the same way: a Jupyter Server the engine reaches
//! over HTTPS with that provider's credentials ([`ProxyAuth::Headers`]), so
//! kernels, files, terminals, notebooks, apps and jobs work unchanged. What
//! differs is how the VM is created, woken and deleted, which lives here.
//!
//! Lifecycle, as with Colab runtimes: a runtime is created on demand, stays
//! warm for reuse (its kernel and loaded models stay in memory), and is
//! deleted when the user releases it — or automatically, once it has been
//! idle for `auto_release_minutes` (the session manager's reaper while the
//! app runs; GitHub's retention period / CodeSandbox hibernation otherwise).

pub mod codesandbox;
pub mod codespaces;
pub mod pitcher;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

pub use codesandbox::CodeSandbox;
pub use codespaces::{Codespaces, DeviceLogin, GithubUser};

use crate::config::{Endpoints, Provider, RuntimeRequest};
use crate::error::{Error, Result};
use crate::runtime::proxy::{ProxyAuth, RuntimeProxy};
use crate::secrets::SecretStore;
use crate::session::state::SessionState;
use crate::settings::{Settings, CODESANDBOX_TIERS};

/// The provider settings in force (a copy of the relevant [`Settings`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderConfig {
    pub codespaces_repository: String,
    pub codespaces_devcontainer_path: String,
    pub codespaces_machine: String,
    pub codesandbox_template: String,
    pub codesandbox_tier: String,
    pub auto_release_minutes: u64,
}

impl From<&Settings> for ProviderConfig {
    fn from(settings: &Settings) -> Self {
        Self {
            codespaces_repository: settings.codespaces_repository.clone(),
            codespaces_devcontainer_path: settings.codespaces_devcontainer_path.clone(),
            codespaces_machine: settings.codespaces_machine.clone(),
            codesandbox_template: settings.codesandbox_template.clone(),
            codesandbox_tier: settings.codesandbox_tier.clone(),
            auto_release_minutes: settings.auto_release_minutes,
        }
    }
}

/// A machine the new-runtime form can offer.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineOption {
    pub name: String,
    pub label: String,
    pub cpus: Option<u64>,
    pub memory_gb: Option<f64>,
}

/// Settings → Compute providers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: Provider,
    pub connected: bool,
    /// GitHub login, or `null`.
    pub account: Option<String>,
    pub detail: Option<String>,
    pub error: Option<String>,
}

/// CodeSandbox tiers with their specs (`VMTier` in the SDK).
pub fn codesandbox_machines() -> Vec<MachineOption> {
    let specs: [(u64, f64); 7] =
        [(1, 2.0), (2, 4.0), (4, 8.0), (8, 16.0), (16, 32.0), (32, 64.0), (64, 128.0)];
    CODESANDBOX_TIERS
        .iter()
        .zip(specs)
        .map(|(name, (cpus, memory))| MachineOption {
            name: (*name).to_owned(),
            label: format!("{name} · {cpus} CPU · {memory:.0} GB"),
            cpus: Some(cpus),
            memory_gb: Some(memory),
        })
        .collect()
}

fn codespaces_machine(machine: &Value) -> Option<MachineOption> {
    let name = machine["name"].as_str()?.to_owned();
    let label = machine["display_name"].as_str().unwrap_or(&name).to_owned();
    Some(MachineOption {
        name,
        label,
        cpus: machine["cpus"].as_u64(),
        memory_gb: machine["memory_in_bytes"].as_f64().map(|bytes| (bytes / 1e9).round()),
    })
}

fn random_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// How long to wait for VMs and how often to look (shortened in tests).
#[derive(Clone, Copy, Debug)]
struct Timing {
    jupyter_timeout: Duration,
    poll_interval: Duration,
    /// Overrides `auto_release_minutes` (tests).
    auto_release: Option<Option<Duration>>,
}

pub struct Providers {
    pub codespaces: Codespaces,
    pub codesandbox: CodeSandbox,
    http: reqwest::Client,
    config: RwLock<ProviderConfig>,
    timing: Mutex<Timing>,
}

impl Providers {
    pub fn new(
        http: reqwest::Client,
        endpoints: &Endpoints,
        secrets: Arc<dyn SecretStore>,
        settings: &Settings,
    ) -> Self {
        Self {
            codespaces: Codespaces::new(http.clone(), endpoints, secrets.clone()),
            codesandbox: CodeSandbox::new(http.clone(), endpoints, secrets),
            http,
            config: RwLock::new(ProviderConfig::from(settings)),
            timing: Mutex::new(Timing {
                jupyter_timeout: Duration::from_secs(15 * 60),
                poll_interval: Duration::from_secs(3),
                auto_release: None,
            }),
        }
    }

    fn timing(&self) -> Timing {
        *self.timing.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Shorten every wait (tests against the mock).
    pub fn set_timing(&self, poll_interval: Duration, jupyter_timeout: Duration) {
        let mut timing = self.timing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        timing.poll_interval = poll_interval;
        timing.jupyter_timeout = jupyter_timeout;
        self.codespaces.set_poll_interval(poll_interval);
    }

    /// Replace the auto-release delay regardless of Settings (tests).
    pub fn override_auto_release(&self, after: Option<Duration>) {
        self.timing.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).auto_release =
            Some(after);
    }

    pub fn configure(&self, settings: &Settings) {
        if let Ok(mut config) = self.config.write() {
            *config = ProviderConfig::from(settings);
        }
    }

    pub fn config(&self) -> ProviderConfig {
        self.config
            .read()
            .map(|config| config.clone())
            .unwrap_or_else(|_| ProviderConfig::from(&Settings::default()))
    }

    /// Delete idle runtimes after this long (`None` = never).
    pub fn auto_release(&self) -> Option<Duration> {
        if let Some(after) = self.timing().auto_release {
            return after;
        }
        match self.config().auto_release_minutes {
            0 => None,
            minutes => Some(Duration::from_secs(minutes * 60)),
        }
    }

    pub async fn status(&self) -> Vec<ProviderStatus> {
        let config = self.config();
        let codespaces = match self.codespaces.user().await {
            Ok(Some(user)) => ProviderStatus {
                provider: Provider::Codespaces,
                connected: true,
                account: Some(user.login),
                detail: Some(format!("Runtimes from {}", config.codespaces_repository)),
                error: None,
            },
            Ok(None) => ProviderStatus {
                provider: Provider::Codespaces,
                connected: false,
                account: None,
                detail: None,
                error: None,
            },
            Err(error) => ProviderStatus {
                provider: Provider::Codespaces,
                connected: false,
                account: None,
                detail: None,
                error: Some(error.to_string()),
            },
        };
        let codesandbox = if self.codesandbox.api_key().is_none() {
            ProviderStatus {
                provider: Provider::Codesandbox,
                connected: false,
                account: None,
                detail: None,
                error: None,
            }
        } else {
            match self.codesandbox.running().await {
                Ok(running) => ProviderStatus {
                    provider: Provider::Codesandbox,
                    connected: true,
                    account: None,
                    detail: Some(format!(
                        "{} of {} VMs running",
                        running["concurrent_vm_count"].as_u64().unwrap_or(0),
                        running["concurrent_vm_limit"]
                            .as_u64()
                            .map_or_else(|| "?".to_owned(), |limit| limit.to_string()),
                    )),
                    error: None,
                },
                Err(error) => ProviderStatus {
                    provider: Provider::Codesandbox,
                    connected: false,
                    account: None,
                    detail: None,
                    error: Some(error.to_string()),
                },
            }
        };
        vec![codespaces, codesandbox]
    }

    pub async fn machines(&self, provider: Provider) -> Result<Vec<MachineOption>> {
        match provider {
            Provider::Codespaces => {
                let repository = self.config().codespaces_repository;
                Ok(self
                    .codespaces
                    .machines(&repository)
                    .await?
                    .iter()
                    .filter_map(codespaces_machine)
                    .collect())
            }
            Provider::Codesandbox => Ok(codesandbox_machines()),
            Provider::Colab => Err(Error::invalid("Colab hardware comes from the Colab settings.")),
        }
    }

    /// How requests reach the runtime's Jupyter Server.
    pub fn proxy(&self, state: &SessionState) -> Result<RuntimeProxy> {
        let auth = match state.provider {
            Provider::Colab => ProxyAuth::Colab(state.token.clone()),
            Provider::Codespaces => {
                ProxyAuth::Headers(self.codespaces.jupyter_target(&state.endpoint)?.1)
            }
            Provider::Codesandbox => ProxyAuth::Headers(
                self.codesandbox
                    .jupyter_target(
                        &state.endpoint,
                        &state.token,
                        state.jupyter_token.as_deref().unwrap_or_default(),
                    )
                    .1,
            ),
        };
        Ok(RuntimeProxy::with_auth(self.http.clone(), &state.url, auth))
    }

    /// Wait until the runtime's Jupyter Server answers.
    async fn wait_for_jupyter(&self, state: &SessionState) -> Result<()> {
        let proxy = self.proxy(state)?;
        let deadline = tokio::time::Instant::now() + self.timing().jupyter_timeout;
        loop {
            match proxy.status().await {
                Ok(_) => return Ok(()),
                Err(error) => {
                    if tokio::time::Instant::now() > deadline {
                        return Err(Error::provider(
                            if state.provider == Provider::Codespaces {
                                "codespaces"
                            } else {
                                "codesandbox"
                            },
                            None,
                            format!(
                                "The {} VM did not start Jupyter in time ({error}). Check the \
                                 runtime template in Settings → Compute providers.",
                                state.provider.label()
                            ),
                        ));
                    }
                }
            }
            tokio::time::sleep(self.timing().poll_interval).await;
        }
    }

    /// Create a VM for `request`, wait until it serves Jupyter, and describe
    /// it as a runtime. A VM that never becomes ready is deleted again.
    pub async fn create(&self, name: &str, request: &RuntimeRequest) -> Result<SessionState> {
        let config = self.config();
        match request.provider {
            Provider::Colab => Err(Error::internal("Colab runtimes are assigned by Colab.")),
            Provider::Codespaces => {
                let machine =
                    request.machine.clone().unwrap_or_else(|| config.codespaces_machine.clone());
                let minutes = config.auto_release_minutes;
                let codespace = self
                    .codespaces
                    .create(
                        &config.codespaces_repository,
                        name,
                        &machine,
                        &config.codespaces_devcontainer_path,
                        if minutes == 0 { 30 } else { minutes },
                        (minutes > 0).then_some(minutes),
                    )
                    .await?;
                let codespace_name = codespace["name"]
                    .as_str()
                    .ok_or_else(|| {
                        Error::provider("codespaces", None, "GitHub did not name the codespace.")
                    })?
                    .to_owned();
                let (url, _) = self.codespaces.jupyter_target(&codespace_name)?;
                let mut state = SessionState::for_provider(
                    name,
                    Provider::Codespaces,
                    &codespace_name,
                    &url,
                    &machine,
                );
                state.web_url = codespace["web_url"].as_str().map(str::to_owned);
                let ready = async {
                    let codespace = self
                        .codespaces
                        .wait_available(&codespace_name, self.timing().jupyter_timeout)
                        .await?;
                    if let Some(url) = codespace["web_url"].as_str() {
                        state.web_url = Some(url.to_owned());
                    }
                    self.wait_for_jupyter(&state).await
                }
                .await;
                if let Err(error) = ready {
                    let _ = self.codespaces.delete(&codespace_name).await;
                    return Err(error);
                }
                Ok(state)
            }
            Provider::Codesandbox => {
                let tier =
                    request.machine.clone().unwrap_or_else(|| config.codesandbox_tier.clone());
                if !CODESANDBOX_TIERS.contains(&tier.as_str()) {
                    return Err(Error::invalid(format!(
                        "CodeSandbox tiers are {}.",
                        CODESANDBOX_TIERS.join(", ")
                    )));
                }
                let id = self.codesandbox.fork(&config.codesandbox_template, name).await?;
                let ready = async {
                    let vm =
                        self.codesandbox.start(&id, &tier, hibernation_seconds(&config)).await?;
                    let preview = self.codesandbox.preview_token(&id).await?;
                    let jupyter_token = random_token();
                    self.codesandbox.start_jupyter(&vm, &jupyter_token).await?;
                    let (url, _) = self.codesandbox.jupyter_target(&id, &preview, &jupyter_token);
                    let mut state =
                        SessionState::for_provider(name, Provider::Codesandbox, &id, &url, &tier);
                    state.token = preview;
                    state.jupyter_token = Some(jupyter_token);
                    state.web_url = Some(format!("https://codesandbox.io/p/devbox/{id}"));
                    self.wait_for_jupyter(&state).await?;
                    Ok::<_, Error>(state)
                }
                .await;
                if ready.is_err() {
                    let _ = self.codesandbox.delete(&id).await;
                }
                ready
            }
        }
    }

    /// Make sure a stopped or hibernated VM serves Jupyter again: GitHub
    /// stops idle codespaces; CodeSandbox hibernates VMs (memory is kept,
    /// but a clean boot needs Jupyter started again).
    pub async fn ensure_running(&self, state: &SessionState) -> Result<()> {
        if state.provider == Provider::Colab || self.proxy(state)?.status().await.is_ok() {
            return Ok(());
        }
        match state.provider {
            Provider::Colab => Ok(()),
            Provider::Codespaces => {
                self.codespaces
                    .wait_available(&state.endpoint, self.timing().jupyter_timeout)
                    .await?;
                self.wait_for_jupyter(state).await
            }
            Provider::Codesandbox => {
                let config = self.config();
                let vm = self
                    .codesandbox
                    .start(
                        &state.endpoint,
                        state.machine.as_deref().unwrap_or(&config.codesandbox_tier),
                        hibernation_seconds(&config),
                    )
                    .await?;
                // A VM woken from a snapshot still runs Jupyter.
                for _ in 0..3 {
                    if self.proxy(state)?.status().await.is_ok() {
                        return Ok(());
                    }
                    tokio::time::sleep(self.timing().poll_interval).await;
                }
                let token = state.jupyter_token.clone().unwrap_or_default();
                self.codesandbox.start_jupyter(&vm, &token).await?;
                self.wait_for_jupyter(state).await
            }
        }
    }

    /// Permanently delete the VM.
    pub async fn release(&self, state: &SessionState) -> Result<()> {
        match state.provider {
            Provider::Colab => Err(Error::internal("Colab runtimes are released through Colab.")),
            Provider::Codespaces => self.codespaces.delete(&state.endpoint).await,
            Provider::Codesandbox => self.codesandbox.delete(&state.endpoint).await,
        }
    }

    /// Whether the VM still exists (`false` once deleted elsewhere).
    pub async fn exists(&self, state: &SessionState) -> Result<bool> {
        let missing = |error: &Error| matches!(error, Error::Provider { status: Some(404), .. });
        match state.provider {
            Provider::Colab => Ok(true),
            Provider::Codespaces => match self.codespaces.get(&state.endpoint).await {
                Ok(codespace) => {
                    Ok(!matches!(codespace["state"].as_str(), Some("Deleted" | "Archived")))
                }
                Err(error) if missing(&error) => Ok(false),
                Err(error) => Err(error),
            },
            // There is no cheap "get" for a VM; a dead preview host is
            // handled when the runtime is next used.
            Provider::Codesandbox => Ok(true),
        }
    }
}

/// CodeSandbox hibernates an idle VM after this long; requests to its
/// preview host wake it again.
fn hibernation_seconds(config: &ProviderConfig) -> u64 {
    match config.auto_release_minutes {
        0 => 30 * 60,
        minutes => (minutes * 60).min(30 * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_and_timeouts() {
        let tiers = codesandbox_machines();
        assert_eq!(tiers.len(), 7);
        assert_eq!(tiers[1].label, "Nano · 2 CPU · 4 GB");
        let machine = codespaces_machine(&serde_json::json!({
            "name": "standardLinux32gb", "display_name": "4 cores, 16 GB RAM, 32 GB storage",
            "cpus": 4, "memory_in_bytes": 17179869184u64
        }))
        .unwrap();
        assert_eq!(machine.cpus, Some(4));
        assert_eq!(machine.memory_gb, Some(17.0));
        let mut config = ProviderConfig::from(&Settings::default());
        assert_eq!(hibernation_seconds(&config), 1800);
        config.auto_release_minutes = 5;
        assert_eq!(hibernation_seconds(&config), 300);
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 64);
    }
}
