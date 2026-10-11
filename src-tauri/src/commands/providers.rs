//! Compute beyond Colab: connecting GitHub (Codespaces) and CodeSandbox,
//! and the machines each offers. Runtimes on them are created with the
//! usual `session_create` (`provider` and `machine` in the request).

use nzap_core::config::Provider;
use nzap_core::providers::{GithubUser, MachineOption, ProviderStatus};
use nzap_core::Error;
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::app::open_external;
use crate::state::{emit_to, AppState, CmdResult};

#[tauri::command]
pub async fn providers_status(state: State<'_, AppState>) -> CmdResult<Vec<ProviderStatus>> {
    Ok(state.engine.providers.status().await)
}

#[tauri::command]
pub async fn provider_machines(
    state: State<'_, AppState>,
    provider: Provider,
) -> CmdResult<Vec<MachineOption>> {
    state.engine.providers.machines(provider).await.map_err(Into::into)
}

/// Use a pasted GitHub token (classic with `codespace`, or fine-grained
/// with Codespaces read/write).
#[tauri::command]
pub async fn github_set_token(state: State<'_, AppState>, token: String) -> CmdResult<GithubUser> {
    state.engine.providers.codespaces.set_token(&token).await.map_err(Into::into)
}

/// Use the GitHub CLI's login (`gh auth token`).
#[tauri::command]
pub async fn github_use_cli(state: State<'_, AppState>) -> CmdResult<GithubUser> {
    state.engine.providers.codespaces.import_gh_cli().await.map_err(Into::into)
}

/// The OAuth app for "Sign in with GitHub": Settings, else the
/// `NZAP_GITHUB_CLIENT_ID` the build or environment provides.
fn github_client_id(state: &AppState) -> Option<String> {
    state
        .engine
        .settings
        .get()
        .github_client_id
        .or_else(|| option_env!("NZAP_GITHUB_CLIENT_ID").map(str::to_owned))
        .or_else(|| std::env::var("NZAP_GITHUB_CLIENT_ID").ok())
        .filter(|id| !id.trim().is_empty())
}

/// Sign in with GitHub's device flow: sends `{userCode, verificationUri}`
/// on the channel, opens the page, and resolves when the user approved.
#[tauri::command]
pub async fn github_device_login(
    app: AppHandle,
    state: State<'_, AppState>,
    stream_id: Option<String>,
    on_event: Channel<Value>,
) -> CmdResult<GithubUser> {
    let client_id = github_client_id(&state).ok_or_else(|| {
        Error::invalid(
            "Sign in with GitHub needs a GitHub OAuth app. Add its client id under Settings → \
             Compute providers, or use the GitHub CLI's login or a token instead.",
        )
    })?;
    let engine = state.engine.clone();
    let login = engine.providers.codespaces.begin_device(&client_id).await?;
    let emit = emit_to(on_event);
    emit(json!({ "userCode": login.user_code, "verificationUri": login.verification_uri }));
    open_external(&app, &state, &login.verification_uri)?;
    state
        .run_stream(
            stream_id,
            async move { engine.providers.codespaces.finish_device(&login).await },
        )
        .await
}

#[tauri::command]
pub fn github_disconnect(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.providers.codespaces.disconnect().map_err(Into::into)
}

/// Check and keep a CodeSandbox API key; returns the running VMs and limit.
#[tauri::command]
pub async fn codesandbox_set_key(state: State<'_, AppState>, key: String) -> CmdResult<Value> {
    state.engine.providers.codesandbox.set_api_key(&key).await.map_err(Into::into)
}

#[tauri::command]
pub fn codesandbox_disconnect(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.providers.codesandbox.disconnect().map_err(Into::into)
}
