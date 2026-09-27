//! Connecting Google, the account and compute-unit quota.

use nzap_core::auth::{GoogleUser, LOGIN_TIMEOUT};
use nzap_core::colab::{quota, Quota};
use nzap_core::engine::{AccountInfo, ConnectionStatus};
use nzap_core::Error;
use tauri::{AppHandle, State};

use super::app::open_external;
use crate::state::{AppState, CmdResult};

#[tauri::command]
pub async fn auth_status(state: State<'_, AppState>) -> CmdResult<ConnectionStatus> {
    Ok(state.engine.status().await)
}

/// Sign in with the loopback flow: opens Google's consent page in the
/// browser and resolves once the redirect comes back (or fails, times out,
/// or is cancelled with `auth_cancel`).
#[tauri::command]
pub async fn auth_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    login_hint: Option<String>,
) -> CmdResult<GoogleUser> {
    let engine = state.engine.clone();
    let login = engine.auth.begin_loopback(login_hint.as_deref()).await?;
    open_external(&app, &state, &login.auth_url)?;
    let task = tokio::spawn(async move {
        let user = engine.auth.finish_loopback(login, LOGIN_TIMEOUT).await?;
        engine.resume().await;
        Ok::<_, Error>(user)
    });
    state.set_login(Some(task.abort_handle()));
    let outcome = task.await;
    match outcome {
        Ok(result) => result.map_err(Into::into),
        Err(error) if error.is_cancelled() => Err(Error::Cancelled.into()),
        Err(error) => Err(Error::internal(error.to_string()).into()),
    }
}

#[tauri::command]
pub fn auth_cancel(state: State<'_, AppState>) {
    state.set_login(None);
}

/// Start the copy/paste flow (for when the browser cannot reach this
/// machine's loopback address). Returns the consent URL, also opened.
#[tauri::command]
pub fn auth_begin_remote(
    app: AppHandle,
    state: State<'_, AppState>,
    login_hint: Option<String>,
) -> CmdResult<String> {
    let url = state.engine.auth.begin_remote(login_hint.as_deref())?;
    open_external(&app, &state, &url)?;
    Ok(url)
}

#[tauri::command]
pub async fn auth_complete_remote(
    state: State<'_, AppState>,
    code: String,
) -> CmdResult<GoogleUser> {
    let user = state.engine.auth.complete_remote(&code).await?;
    state.engine.resume().await;
    Ok(user)
}

/// Release every runtime and forget the Google connection.
#[tauri::command]
pub async fn auth_disconnect(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.disconnect().await.map_err(Into::into)
}

#[tauri::command]
pub async fn account_get(state: State<'_, AppState>) -> CmdResult<AccountInfo> {
    state.engine.account().await.map_err(Into::into)
}

/// CCU burn rate, balance and free-time estimate.
#[tauri::command]
pub async fn quota_get(state: State<'_, AppState>) -> CmdResult<Quota> {
    quota::fetch(&state.engine.colab).await.map_err(Into::into)
}
