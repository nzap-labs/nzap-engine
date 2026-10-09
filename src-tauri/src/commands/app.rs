//! App-level commands: info, settings, external links, stream cancellation,
//! and how to connect an AI agent (MCP).

use nzap_core::auth::OAuthClient;
use nzap_core::engine::HardwareConfig;
use nzap_core::settings::{Settings, SettingsPatch};
use nzap_core::Error;
use serde::Serialize;
use serde_json::json;
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
    check_external_url(url)?;
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

/// Only `https://` leaves the app (plain `http://` to loopback is allowed in
/// development builds, for the mock Google server).
pub fn check_external_url(url: &str) -> nzap_core::Result<()> {
    let parsed = url::Url::parse(url).map_err(|_| Error::invalid("That is not a valid link."))?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"));
    let allowed = parsed.scheme() == "https"
        || (cfg!(debug_assertions) && loopback && parsed.scheme() == "http");
    if allowed {
        Ok(())
    } else {
        Err(Error::invalid("Only https:// links can be opened."))
    }
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
    app: AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> CmdResult<SettingsView> {
    let tray_change = patch.close_to_tray;
    state.engine.update_settings(patch)?;
    // Sync command, so this runs on the main thread as tray APIs require.
    match tray_change {
        Some(true) => crate::tray::ensure(&app).map_err(|error| {
            Error::internal(format!("The system tray is not available: {error}"))
        })?,
        Some(false) => crate::tray::remove(&app),
        None => {}
    }
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

/// How to connect an AI agent to this installation (Settings → AI agents).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    /// The executable an agent launches, with `args`.
    pub command: String,
    pub args: Vec<&'static str>,
    /// One line that adds NZAP Engine to Claude Code for every project.
    pub claude_code: String,
    /// `mcpServers` JSON for Claude Desktop, Cursor and other clients.
    pub config_json: String,
}

/// The installed executable. An AppImage runs from a temporary mount, so
/// agents must launch the AppImage file itself.
fn executable() -> nzap_core::Result<std::path::PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|path| !path.is_empty()) {
        return Ok(appimage.into());
    }
    std::env::current_exe()
        .map_err(|error| Error::Io(format!("Cannot find the app's executable: {error}")))
}

/// Quote a path for the user's shell: double quotes on Windows (cmd and
/// PowerShell), single quotes elsewhere.
pub fn shell_quote(text: &str, windows: bool) -> String {
    if text.chars().all(|c| c.is_ascii_alphanumeric() || "/\\._-:".contains(c)) {
        text.to_owned()
    } else if windows {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

pub fn mcp_info_for(executable: &str, windows: bool) -> McpInfo {
    let config = json!({ "mcpServers": { "nzap": { "command": executable, "args": ["mcp"] } } });
    McpInfo {
        command: executable.to_owned(),
        args: vec!["mcp"],
        claude_code: format!(
            "claude mcp add --scope user nzap -- {} mcp",
            shell_quote(executable, windows)
        ),
        config_json: serde_json::to_string_pretty(&config).unwrap_or_default(),
    }
}

#[tauri::command]
pub fn mcp_info() -> CmdResult<McpInfo> {
    let executable = executable()?;
    Ok(mcp_info_for(&executable.to_string_lossy(), cfg!(windows)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_get_a_command_for_their_shell() {
        let mac = mcp_info_for("/Applications/NZAP Engine.app/Contents/MacOS/nzap-engine", false);
        assert_eq!(
            mac.claude_code,
            "claude mcp add --scope user nzap -- \
             '/Applications/NZAP Engine.app/Contents/MacOS/nzap-engine' mcp"
        );
        let config: serde_json::Value = serde_json::from_str(&mac.config_json).unwrap();
        assert_eq!(config["mcpServers"]["nzap"]["args"], json!(["mcp"]));

        let linux = mcp_info_for("/usr/bin/nzap-engine", false);
        assert_eq!(
            linux.claude_code,
            "claude mcp add --scope user nzap -- /usr/bin/nzap-engine mcp"
        );

        let windows = mcp_info_for(r"C:\Program Files\NZAP Engine\nzap-engine.exe", true);
        assert!(windows
            .claude_code
            .ends_with(r#""C:\Program Files\NZAP Engine\nzap-engine.exe" mcp"#));
        let config: serde_json::Value = serde_json::from_str(&windows.config_json).unwrap();
        assert_eq!(
            config["mcpServers"]["nzap"]["command"],
            r"C:\Program Files\NZAP Engine\nzap-engine.exe"
        );

        assert_eq!(
            shell_quote("/home/o'neil/nzap.AppImage", false),
            r"'/home/o'\''neil/nzap.AppImage'"
        );
    }

    #[test]
    fn only_https_leaves_the_app() {
        assert!(check_external_url("https://colab.research.google.com/notebooks").is_ok());
        assert!(check_external_url("https://accounts.google.com/o/oauth2/v2/auth?x=1").is_ok());
        for refused in [
            "http://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ms-settings:privacy",
            "not a url",
        ] {
            assert!(check_external_url(refused).is_err(), "{refused}");
        }
        // The loopback exception exists only in development builds.
        assert_eq!(check_external_url("http://127.0.0.1:9/auth").is_ok(), cfg!(debug_assertions));
    }
}
