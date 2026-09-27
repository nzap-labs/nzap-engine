//! Native save / open dialogs. File paths always come from a dialog the
//! user drove, never from the webview, so a compromised page cannot read or
//! write arbitrary files.

use std::path::PathBuf;

use nzap_core::{Error, Result};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Largest file the app reads from disk for the UI (scripts, notebooks,
/// requirements, uploads go through [`read_for_upload`]).
pub const MAX_TEXT_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Largest single upload to a runtime.
pub const MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;

fn into_path(path: FilePath) -> Result<PathBuf> {
    path.into_path().map_err(|error| Error::Io(format!("Unsupported path: {error}")))
}

/// Ask where to save, then write `bytes`. `None` when the user cancels.
pub async fn save_bytes(
    app: &AppHandle,
    default_name: &str,
    filter: Option<(&str, &[&str])>,
    bytes: Vec<u8>,
) -> Result<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_file_name(default_name);
    if let Some((label, extensions)) = filter {
        dialog = dialog.add_filter(label, extensions);
    }
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = into_path(path)?;
    tokio::fs::write(&path, bytes).await?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// A file the user picked, with its contents.
pub struct PickedFile {
    pub name: String,
    pub path: PathBuf,
}

/// Ask for one or more files. Empty when the user cancels.
pub async fn pick_files(
    app: &AppHandle,
    filter: Option<(&str, &[&str])>,
    multiple: bool,
) -> Result<Vec<PickedFile>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file();
    if let Some((label, extensions)) = filter {
        dialog = dialog.add_filter(label, extensions);
    }
    if multiple {
        dialog.pick_files(move |paths| {
            let _ = tx.send(paths.unwrap_or_default());
        });
    } else {
        dialog.pick_file(move |path| {
            let _ = tx.send(path.into_iter().collect());
        });
    }
    let paths: Vec<FilePath> = rx.await.unwrap_or_default();
    paths
        .into_iter()
        .map(|path| {
            let path = into_path(path)?;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".to_owned());
            Ok(PickedFile { name, path })
        })
        .collect()
}

/// Read a picked file as UTF-8 text (scripts, notebooks, requirements).
pub async fn read_text(file: &PickedFile) -> Result<String> {
    let size = tokio::fs::metadata(&file.path).await?.len();
    if size > MAX_TEXT_FILE_BYTES {
        return Err(Error::invalid(format!("{} is larger than 20 MB.", file.name)));
    }
    let bytes = tokio::fs::read(&file.path).await?;
    String::from_utf8(bytes)
        .map_err(|_| Error::invalid(format!("{} is not a text file.", file.name)))
}

/// Read a picked file for upload to a runtime.
pub async fn read_for_upload(file: &PickedFile) -> Result<Vec<u8>> {
    let size = tokio::fs::metadata(&file.path).await?.len();
    if size > MAX_UPLOAD_BYTES {
        return Err(Error::invalid(format!("{} is larger than 512 MB.", file.name)));
    }
    Ok(tokio::fs::read(&file.path).await?)
}
