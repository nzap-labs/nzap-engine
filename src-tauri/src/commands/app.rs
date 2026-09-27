//! App-level commands: info, settings, external links, stream cancellation.

use nzap_core::auth::OAuthClient;
use nzap_core::engine::HardwareConfig;
use nzap_core::settings::{Settings, SettingsPatch};
use nzap_core::Error;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::state::{AppState, CmdResult};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo { version: nzap_core::VERSION, os: std::env::consts::OS, arch: std::env::consts::ARCH }
}

/// Open an `https://` link in the user's browser. Nothing else is allowed
/// out of the webview.
pub fn open_external(app: &AppHandle, state: &AppState, url: &str) -> nzap_core::Result<()> {
    let parsed = url::Url::parse(url).map_err(|_| Error::invalid("That is not a valid link."))?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"));
    // Plain http is only ever the local mock in development builds.
    if parsed.scheme() != "https"
        && !(cfg!(debug_assertions) && loopback && parsed.scheme() == "http")
    {
        return Err(Error::invalid("Only https:// links can be opened."));
    }
    if let Some(log) = &state.open_log {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
        writeln!(file, "{url}")?;
        return Ok(());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| Error::internal(format!("Could not open the browser: {error}")))
}

#[tauri::command]
pub fn open_url(app: AppHandle, state: State<'_, AppState>, url: String) -> CmdResult<()> {
    open_external(&app, &state, &url).map_err(Into::into)
}

/// Show a saved file (a job artifact) in the system file manager.
#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> CmdResult<()> {
    let path = std::path::PathBuf::from(path);
    if !path.exists() {
        return Err(Error::not_found("That file no longer exists.").into());
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|error| Error::internal(error.to_string()).into())
}

#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> CmdResult<()> {
    let dir = app.path().app_log_dir().map_err(|error| Error::Io(error.to_string()))?;
    std::fs::create_dir_all(&dir).map_err(Error::from)?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|error| Error::internal(error.to_string()).into())
}

#[tauri::command]
pub fn stream_cancel(state: State<'_, AppState>, stream_id: String) -> bool {
    state.cancel_stream(&stream_id)
}

#[tauri::command]
pub fn config_get(state: State<'_, AppState>) -> HardwareConfig {
    state.engine.hardware_config()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub settings: Settings,
    pub oauth_client_id: String,
    pub custom_oauth_client: bool,
    pub default_catalog_url: &'static str,
}

fn settings_view(state: &AppState) -> SettingsView {
    let client = state.engine.auth.oauth_client();
    SettingsView {
        settings: state.engine.settings.get(),
        custom_oauth_client: !client.is_default(),
        oauth_client_id: client.client_id,
        default_catalog_url: nzap_core::notebooks::DEFAULT_CATALOG_URL,
    }
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> SettingsView {
    settings_view(&state)
}

#[tauri::command]
pub fn settings_update(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> CmdResult<SettingsView> {
    state.engine.update_settings(patch)?;
    Ok(settings_view(&state))
}

/// Bring your own OAuth client (`null` restores the built-in one).
#[tauri::command]
pub async fn settings_set_oauth_client(
    state: State<'_, AppState>,
    json: Option<String>,
) -> CmdResult<SettingsView> {
    if let Some(text) = json.as_deref() {
        OAuthClient::from_json(text)?;
    }
    state.engine.set_oauth_client(json.as_deref()).await?;
    Ok(settings_view(&state))
}
