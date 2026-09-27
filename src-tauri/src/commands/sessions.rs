//! Runtimes: lifecycle, execution streams, automations, file runs, jobs,
//! imports, assignments, the terminal and history.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nzap_core::colab::Resources;
use nzap_core::config::RuntimeRequest;
use nzap_core::ops::automation::{self, AutomationRequest, Operation};
use nzap_core::ops::import::{import_from_url, ImportOptions, ImportedFile};
use nzap_core::ops::jobs::{self, JobRequest};
use nzap_core::ops::runfile::{self, RunFileRequest};
use nzap_core::runtime::TerminalEvent;
use nzap_core::session::{AssignmentView, SessionView, StopOutcome, DEFAULT_EXECUTE_TIMEOUT};
use nzap_core::Error;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use super::dialogs;
use crate::state::{emit_to, AppState, CmdResult};

#[tauri::command]
pub fn sessions_list(state: State<'_, AppState>) -> Vec<SessionView> {
    state.engine.sessions.views()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedSession {
    pub session: SessionView,
    pub connected: bool,
}

/// Assign a CPU / GPU / TPU runtime and connect a kernel to it.
#[tauri::command]
pub async fn session_create(state: State<'_, AppState>, request: RuntimeRequest) -> CmdResult<CreatedSession> {
    let (session, connected) = state.engine.sessions.create_and_connect(request).await?;
    Ok(CreatedSession { session, connected })
}

#[tauri::command]
pub fn session_get(state: State<'_, AppState>, name: String) -> CmdResult<SessionView> {
    state.engine.sessions.view(&name).map_err(Into::into)
}

#[tauri::command]
pub async fn session_connect(state: State<'_, AppState>, name: String) -> CmdResult<Value> {
    state.engine.sessions.connect(&name).await.map_err(Into::into)
}

#[tauri::command]
pub fn session_disconnect(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.engine.sessions.disconnect(&name).map_err(Into::into)
}

#[tauri::command]
pub async fn session_keepalive(state: State<'_, AppState>, name: String) -> CmdResult<Value> {
    state.engine.sessions.keepalive(&name).await.map_err(Into::into)
}

#[tauri::command]
pub async fn session_restart(state: State<'_, AppState>, name: String) -> CmdResult<Value> {
    state.engine.sessions.restart_kernel(&name).await.map_err(Into::into)
}

#[tauri::command]
pub async fn session_interrupt(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.engine.sessions.interrupt(&name).await.map_err(Into::into)
}

/// Answer an `input()` prompt.
#[tauri::command]
pub async fn session_stdin(state: State<'_, AppState>, name: String, value: String) -> CmdResult<()> {
    state.engine.sessions.send_stdin(&name, &value).await.map_err(Into::into)
}

/// Retry Drive / Cloud credential propagation after the user consented.
#[tauri::command]
pub async fn session_drive_authorize(state: State<'_, AppState>, name: String) -> CmdResult<Value> {
    state.engine.sessions.authorize_drive(&name).await.map_err(Into::into)
}

/// Stop the kernel, release the VM and forget the runtime.
#[tauri::command]
pub async fn session_stop(state: State<'_, AppState>, name: String) -> CmdResult<StopOutcome> {
    state.engine.sessions.stop(&name, true).await.map_err(Into::into)
}

#[tauri::command]
pub async fn session_resources(state: State<'_, AppState>, name: String) -> CmdResult<Resources> {
    state.engine.sessions.resources(&name).await.map_err(Into::into)
}

/// Run a cell; events stream through `on_event`, the final
/// `execute_reply` is also the result.
#[tauri::command]
pub async fn session_execute(
    state: State<'_, AppState>,
    name: String,
    code: String,
    timeout_seconds: Option<u64>,
    stream_id: Option<String>,
    on_event: Channel<Value>,
) -> CmdResult<Value> {
    let engine = state.engine.clone();
    let emit = emit_to(on_event);
    let timeout = timeout_seconds.map(Duration::from_secs).unwrap_or(DEFAULT_EXECUTE_TIMEOUT);
    state
        .run_stream(stream_id, async move { engine.sessions.execute(&name, &code, timeout, true, &emit).await })
        .await
}

/// `install` / `drivemount` / `gcp-auth`, streamed.
#[tauri::command]
pub async fn session_automation(
    state: State<'_, AppState>,
    name: String,
    op: String,
    request: AutomationRequest,
    stream_id: Option<String>,
    on_event: Channel<Value>,
) -> CmdResult<String> {
    let op = Operation::parse(&op)?;
    // Validate before streaming so bad input fails the call itself.
    automation::plan(op, &request)?;
    let engine = state.engine.clone();
    let emit = emit_to(on_event);
    state
        .run_stream(stream_id, async move { automation::run(&engine.sessions, &name, op, &request, &emit).await })
        .await
}

/// Run a local `.py` / `.ipynb`, streaming every cell.
#[tauri::command]
pub async fn session_run_file(
    state: State<'_, AppState>,
    name: String,
    request: RunFileRequest,
    stream_id: Option<String>,
    on_event: Channel<Value>,
) -> CmdResult<Value> {
    runfile::prepare(&request)?;
    let engine = state.engine.clone();
    let emit = emit_to(on_event);
    state
        .run_stream(stream_id, async move { runfile::run(&engine.sessions, &name, &request, &emit).await })
        .await
}

/// `colab run`: a fresh VM runs one script, its artifacts are saved to the
/// artifacts folder (Settings) and the VM is released.
#[tauri::command]
pub async fn job_run(
    app: AppHandle,
    state: State<'_, AppState>,
    request: JobRequest,
    stream_id: Option<String>,
    on_event: Channel<Value>,
) -> CmdResult<Value> {
    let spec = jobs::plan(&request)?;
    let root = match state.engine.settings.get().artifacts_dir {
        Some(dir) => PathBuf::from(dir),
        None => app
            .path()
            .download_dir()
            .map(|dir| dir.join("NZAP Engine"))
            .map_err(|error| Error::Io(format!("No downloads folder: {error}")))?,
    };
    let artifacts_dir = root.join(&spec.runtime.name);
    let engine = state.engine.clone();
    let emit = emit_to(on_event);
    state
        .run_stream(stream_id, async move { jobs::run(&engine.sessions, spec, Some(artifacts_dir), &emit).await })
        .await
}

/// Import a notebook from a Colab / Drive / GitHub / https link.
#[tauri::command]
pub async fn import_notebook_url(state: State<'_, AppState>, url: String) -> CmdResult<ImportedFile> {
    import_from_url(&state.engine.auth, &url, ImportOptions::default()).await.map_err(Into::into)
}

// ------------------------------------------------------------ assignments

#[tauri::command]
pub async fn assignments_list(state: State<'_, AppState>) -> CmdResult<Vec<AssignmentView>> {
    state.engine.sessions.server_assignments().await.map_err(Into::into)
}

#[tauri::command]
pub async fn assignment_release(state: State<'_, AppState>, endpoint: String) -> CmdResult<()> {
    state.engine.sessions.release_endpoint(&endpoint).await.map_err(Into::into)
}

#[tauri::command]
pub async fn assignment_adopt(
    state: State<'_, AppState>,
    endpoint: String,
    name: Option<String>,
) -> CmdResult<SessionView> {
    state.engine.sessions.adopt(&endpoint, name.as_deref()).await.map_err(Into::into)
}

// --------------------------------------------------------------- terminal

/// Open a shell on the VM. Frames arrive on `on_frame` as
/// `{"type": "frame", "data": "<raw upstream frame>"}`, then one
/// `{"type": "closed", "reason": …}`.
#[tauri::command]
pub async fn terminal_open(state: State<'_, AppState>, name: String, on_frame: Channel<Value>) -> CmdResult<u32> {
    let sink = Arc::new(move |event: TerminalEvent| {
        let message = match event {
            TerminalEvent::Frame(data) => json!({ "type": "frame", "data": data }),
            TerminalEvent::Closed(reason) => json!({ "type": "closed", "reason": reason }),
        };
        let _ = on_frame.send(message);
    });
    let terminal = state.engine.sessions.open_terminal(&name, sink).await?;
    Ok(state.add_terminal(terminal))
}

/// Forward one upstream frame (`{"data": …}` or `{"cols": …, "rows": …}`).
#[tauri::command]
pub fn terminal_send(state: State<'_, AppState>, id: u32, frame: String) -> CmdResult<()> {
    state
        .with_terminal(id, |terminal| terminal.send(&frame))
        .unwrap_or_else(|| Err(Error::not_found("That terminal is closed.")))
        .map_err(Into::into)
}

#[tauri::command]
pub fn terminal_close(state: State<'_, AppState>, id: u32) {
    if let Some(terminal) = state.remove_terminal(id) {
        terminal.close();
    }
}

// ---------------------------------------------------------------- history

#[tauri::command]
pub fn history_get(state: State<'_, AppState>, name: String, limit: Option<usize>) -> Vec<Value> {
    let events = state.engine.sessions.history().get(&name);
    match limit {
        Some(limit) if limit > 0 && events.len() > limit => events[events.len() - limit..].to_vec(),
        _ => events,
    }
}

/// Export the history as `.ipynb` / `.md` / `.txt` / `.jsonl` to a file the
/// user picks. Returns the saved path, or `None` when cancelled.
#[tauri::command]
pub async fn history_export(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    format: String,
) -> CmdResult<Option<String>> {
    let events = state.engine.sessions.history().get(&name);
    let (body, _) = nzap_core::history::export(&events, &name, &format)?;
    let extension = format.trim().trim_start_matches('.').to_ascii_lowercase();
    dialogs::save_bytes(&app, &format!("{name}.{extension}"), Some(("History", [extension.as_str()].as_slice())), body.into_bytes())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub fn history_clear(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.engine.sessions.history().clear(&name).map_err(Into::into)
}
