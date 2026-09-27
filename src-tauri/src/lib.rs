//! The NZAP Engine desktop shell.
//!
//! A thin adapter between the webview and `nzap-core`: IPC commands and
//! streaming channels, plugins, and platform integration. The engine itself
//! lives in `crates/nzap-core` so it can be tested without a window.

mod commands;
mod state;

use std::path::{Path, PathBuf};

use nzap_core::auth::OAuthClient;
use nzap_core::config::Endpoints;
use nzap_core::paths::AppPaths;
use nzap_core::{Engine, EngineOptions};
use tauri::{AppHandle, Manager, RunEvent};

use crate::state::AppState;

/// Development and test builds read `NZAP_*` overrides (mock Google,
/// isolated data directory, no keychain, logged instead of opened URLs).
/// Release builds ignore them, except the documented OAuth client override.
fn dev_env(name: &str) -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    } else {
        None
    }
}

fn build_state(app: &AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
    let paths = match dev_env("NZAP_DATA_DIR") {
        Some(root) => AppPaths::under(Path::new(&root)),
        None => AppPaths {
            data_dir: app.path().app_data_dir()?,
            config_dir: app.path().app_config_dir()?,
            cache_dir: app.path().app_cache_dir()?,
        },
    };
    let endpoints = if cfg!(debug_assertions) {
        Endpoints::default().with_overrides(dev_env)
    } else {
        Endpoints::default()
    };
    let oauth_client = std::env::var("NZAP_OAUTH_CLIENT_JSON").ok().and_then(|json| {
        OAuthClient::from_json(&json)
            .map_err(|error| log::warn!("NZAP_OAUTH_CLIENT_JSON: {error}"))
            .ok()
    });
    let engine = Engine::new(EngineOptions {
        paths,
        endpoints,
        use_keychain: dev_env("NZAP_NO_KEYCHAIN").is_none(),
        oauth_client,
    })?;
    Ok(AppState::new(engine, dev_env("NZAP_E2E_OPEN_LOG").map(PathBuf::from)))
}

fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(Target::new(TargetKind::Stdout))
        .target(Target::new(TargetKind::LogDir { file_name: Some("nzap-engine".into()) }))
        .level(log::LevelFilter::Info)
        // Chatty dependencies stay at warnings.
        .level_for("hyper", log::LevelFilter::Warn)
        .level_for("rustls", log::LevelFilter::Warn)
        .level_for("tungstenite", log::LevelFilter::Warn)
        .max_file_size(5 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepOne)
        .build()
}

/// Build and run the application.
pub fn run() {
    let app = tauri::Builder::default()
        // Must be first: a second launch focuses the running window instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(log_plugin())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = build_state(app.handle())?;
            let engine = state.engine.clone();
            app.manage(state);
            log::info!("NZAP Engine {} started", nzap_core::VERSION);
            // Reconnect to runtimes that survived the last session.
            tauri::async_runtime::spawn(async move { engine.resume().await });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::open_url,
            commands::app::open_log_dir,
            commands::app::stream_cancel,
            commands::app::config_get,
            commands::app::settings_get,
            commands::app::settings_update,
            commands::app::settings_set_oauth_client,
            commands::auth::auth_status,
            commands::auth::auth_connect,
            commands::auth::auth_cancel,
            commands::auth::auth_begin_remote,
            commands::auth::auth_complete_remote,
            commands::auth::auth_disconnect,
            commands::auth::account_get,
            commands::auth::quota_get,
            commands::sessions::sessions_list,
            commands::sessions::session_create,
            commands::sessions::session_get,
            commands::sessions::session_connect,
            commands::sessions::session_disconnect,
            commands::sessions::session_keepalive,
            commands::sessions::session_restart,
            commands::sessions::session_interrupt,
            commands::sessions::session_stdin,
            commands::sessions::session_drive_authorize,
            commands::sessions::session_stop,
            commands::sessions::session_resources,
            commands::sessions::session_execute,
            commands::sessions::session_automation,
            commands::sessions::session_run_file,
            commands::sessions::job_run,
            commands::sessions::import_notebook_url,
            commands::sessions::assignments_list,
            commands::sessions::assignment_release,
            commands::sessions::assignment_adopt,
            commands::sessions::terminal_open,
            commands::sessions::terminal_send,
            commands::sessions::terminal_close,
            commands::sessions::history_get,
            commands::sessions::history_export,
            commands::sessions::history_clear,
            commands::files::files_list,
            commands::files::files_read,
            commands::files::files_write,
            commands::files::files_mkdir,
            commands::files::files_rename,
            commands::files::files_delete,
            commands::files::files_download,
            commands::files::files_upload_pick,
            commands::files::files_upload_bytes,
            commands::files::open_text_file,
            commands::files::save_text_file,
            commands::notebooks::notebooks_list,
            commands::notebooks::notebooks_refresh,
            commands::notebooks::notebook_get,
            commands::notebooks::notebook_create,
            commands::notebooks::notebook_update,
            commands::notebooks::notebook_delete,
            commands::notebooks::notebook_fork,
            commands::notebooks::notebook_export,
            commands::notebooks::notebook_import,
            commands::notebooks::notebook_run,
        ])
        .build(tauri::generate_context!())
        .expect("NZAP Engine failed to start");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<AppState>() {
                // Runtimes keep running on Google's side; only local work stops.
                state.shutdown();
            }
        }
    });
}
