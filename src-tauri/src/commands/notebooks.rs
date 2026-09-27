//! The notebook library.

use nzap_core::notebooks::{CatalogStatus, Notebook, NotebookDraft, NotebookPatch};
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::dialogs;
use crate::state::{emit_to, AppState, CmdResult};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookList {
    pub notebooks: Vec<Notebook>,
    pub catalog: CatalogStatus,
}

#[tauri::command]
pub fn notebooks_list(state: State<'_, AppState>) -> NotebookList {
    let library = &state.engine.notebooks;
    NotebookList { notebooks: library.list(), catalog: library.catalog().status() }
}

/// Fetch (or revalidate) the public collection from GitHub.
#[tauri::command]
pub async fn notebooks_refresh(state: State<'_, AppState>) -> CmdResult<CatalogStatus> {
    Ok(state.engine.notebooks.catalog().refresh().await)
}

#[tauri::command]
pub async fn notebook_get(state: State<'_, AppState>, id: String) -> CmdResult<Notebook> {
    state.engine.notebooks.get(&id).await.map_err(Into::into)
}

#[tauri::command]
pub fn notebook_create(state: State<'_, AppState>, draft: NotebookDraft) -> CmdResult<Notebook> {
    state.engine.notebooks.create(draft).map_err(Into::into)
}

#[tauri::command]
pub fn notebook_update(
    state: State<'_, AppState>,
    id: String,
    patch: NotebookPatch,
) -> CmdResult<Notebook> {
    state.engine.notebooks.update(&id, patch).map_err(Into::into)
}

#[tauri::command]
pub fn notebook_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.engine.notebooks.delete(&id).map_err(Into::into)
}

/// Copy a notebook (usually a public one) into your collection.
#[tauri::command]
pub async fn notebook_fork(state: State<'_, AppState>, id: String) -> CmdResult<Notebook> {
    state.engine.notebooks.fork(&id).await.map_err(Into::into)
}

/// Save a notebook as `<slug>.nzap.json`.
#[tauri::command]
pub async fn notebook_export(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Option<String>> {
    let (filename, text) = state.engine.notebooks.export(&id).await?;
    dialogs::save_bytes(
        &app,
        &filename,
        Some(("NZAP notebook", ["json"].as_slice())),
        text.into_bytes(),
    )
    .await
    .map_err(Into::into)
}

/// Add a notebook from a `.nzap.json` file the user picks.
#[tauri::command]
pub async fn notebook_import(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Notebook>> {
    let Some(file) = dialogs::pick_files(&app, Some(("NZAP notebook", ["json"].as_slice())), false)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let text = dialogs::read_text(&file).await?;
    Ok(Some(state.engine.notebooks.import(&text)?))
}

/// Run a notebook on a runtime with parameter values; output streams
/// through `on_event` like a console cell.
#[tauri::command]
pub async fn notebook_run(
    state: State<'_, AppState>,
    id: String,
    session: String,
    params: Map<String, Value>,
    stream_id: Option<String>,
    on_event: Channel<Value>,
) -> CmdResult<Value> {
    let engine = state.engine.clone();
    let emit = emit_to(on_event);
    state
        .run_stream(stream_id, async move {
            engine.notebooks.run(&engine.sessions, &id, &session, &params, &emit).await
        })
        .await
}
