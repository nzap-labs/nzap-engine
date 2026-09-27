//! A runtime's files, plus opening and saving local files for the UI.

use nzap_core::session::FileListing;
use nzap_core::Error;
use serde::Serialize;
use serde_json::Value;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};

use super::dialogs::{self, MAX_UPLOAD_BYTES};
use crate::state::{AppState, CmdResult};

#[tauri::command]
pub async fn files_list(
    state: State<'_, AppState>,
    name: String,
    path: Option<String>,
) -> CmdResult<FileListing> {
    state
        .engine
        .sessions
        .list_files(&name, path.as_deref().unwrap_or_default())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn files_read(
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> CmdResult<Value> {
    state.engine.sessions.read_file(&name, &path).await.map_err(Into::into)
}

#[tauri::command]
pub async fn files_write(
    state: State<'_, AppState>,
    name: String,
    path: String,
    content: String,
) -> CmdResult<Value> {
    state.engine.sessions.write_file(&name, &path, &content).await.map_err(Into::into)
}

#[tauri::command]
pub async fn files_mkdir(
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> CmdResult<Value> {
    state.engine.sessions.make_directory(&name, &path).await.map_err(Into::into)
}

#[tauri::command]
pub async fn files_rename(
    state: State<'_, AppState>,
    name: String,
    path: String,
    new_path: String,
) -> CmdResult<Value> {
    state.engine.sessions.rename_file(&name, &path, &new_path).await.map_err(Into::into)
}

#[tauri::command]
pub async fn files_delete(state: State<'_, AppState>, name: String, path: String) -> CmdResult<()> {
    state.engine.sessions.delete_file(&name, &path).await.map_err(Into::into)
}

/// Download a runtime file to a location the user picks.
#[tauri::command]
pub async fn files_download(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> CmdResult<Option<String>> {
    let bytes = state.engine.sessions.download_file(&name, &path).await?;
    let filename = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("download");
    dialogs::save_bytes(&app, filename, None, bytes).await.map_err(Into::into)
}

/// Pick local files and upload them into `dir` on the runtime. Returns the
/// remote paths written.
#[tauri::command]
pub async fn files_upload_pick(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    dir: String,
) -> CmdResult<Vec<String>> {
    let picked = dialogs::pick_files(&app, None, true).await?;
    let mut written = Vec::new();
    for file in picked {
        let bytes = dialogs::read_for_upload(&file).await?;
        let remote = join_remote(&dir, &file.name);
        state.engine.sessions.upload_file(&name, &remote, &bytes).await?;
        written.push(remote);
    }
    Ok(written)
}

fn join_remote(dir: &str, filename: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() {
        filename.to_owned()
    } else {
        format!("{dir}/{filename}")
    }
}

fn header(request: &Request<'_>, name: &str) -> Option<String> {
    let raw = request.headers().get(name)?.to_str().ok()?;
    Some(percent_encoding::percent_decode_str(raw).decode_utf8_lossy().into_owned())
}

/// Upload bytes dropped onto the file manager. The body is raw bytes; the
/// runtime and remote path travel in `x-nzap-session` / `x-nzap-path`
/// (percent-encoded).
#[tauri::command]
pub async fn files_upload_bytes(
    state: State<'_, AppState>,
    request: Request<'_>,
) -> CmdResult<Value> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::invalid("Upload a file's raw bytes.").into());
    };
    if bytes.len() as u64 > MAX_UPLOAD_BYTES {
        return Err(Error::invalid("That file is larger than 512 MB.").into());
    }
    let (Some(name), Some(path)) =
        (header(&request, "x-nzap-session"), header(&request, "x-nzap-path"))
    else {
        return Err(Error::invalid("Missing runtime or path.").into());
    };
    let bytes = bytes.clone();
    state.engine.sessions.upload_file(&name, &path, &bytes).await.map_err(Into::into)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile {
    pub name: String,
    pub content: String,
}

/// Let the user pick a local text file (a script, notebook or requirements
/// file) and return its contents.
#[tauri::command]
pub async fn open_text_file(
    app: AppHandle,
    extensions: Option<Vec<String>>,
) -> CmdResult<Option<OpenedFile>> {
    let extensions: Vec<String> = extensions.unwrap_or_default();
    let refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let filter = (!refs.is_empty()).then_some(("Files", refs.as_slice()));
    let Some(file) = dialogs::pick_files(&app, filter, false).await?.into_iter().next() else {
        return Ok(None);
    };
    let content = dialogs::read_text(&file).await?;
    Ok(Some(OpenedFile { name: file.name, content }))
}

/// Save text the UI produced (an executed notebook, a job log) to a file the
/// user picks.
#[tauri::command]
pub async fn save_text_file(
    app: AppHandle,
    filename: String,
    content: String,
) -> CmdResult<Option<String>> {
    let filename = filename
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("download.txt");
    dialogs::save_bytes(&app, filename, None, content.into_bytes()).await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths() {
        assert_eq!(join_remote("", "a.txt"), "a.txt");
        assert_eq!(join_remote("/content/", "a.txt"), "content/a.txt");
    }
}
