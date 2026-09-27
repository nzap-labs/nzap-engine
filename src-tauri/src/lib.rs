//! The NZAP Engine desktop shell.
//!
//! A thin adapter between the webview and `nzap-core`: IPC commands and
//! streaming channels, plugins, and platform integration. The engine itself
//! lives in `crates/nzap-core` so it can be tested without a window.

/// The engine version, shown in the app's About panel.
#[tauri::command]
fn engine_version() -> &'static str {
    nzap_core::VERSION
}

/// Build and run the application.
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![engine_version])
        .run(tauri::generate_context!())
        .expect("NZAP Engine failed to start");
}
