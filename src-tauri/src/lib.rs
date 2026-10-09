//! The NZAP Engine desktop shell.
//!
//! A thin adapter between the webview and `nzap-core`: IPC commands and
//! streaming channels, plugins, and platform integration. The engine itself
//! lives in `crates/nzap-core` so it can be tested without a window.
//!
//! The same binary is also an MCP server for AI agents: `nzap-engine mcp`
//! serves `crates/nzap-mcp` on stdin/stdout without opening a window.

mod commands;
mod state;
mod tray;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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

/// Startup progress on stderr in debug builds, so a hang before the logger
/// is useful still shows how far startup got.
fn startup_mark(step: &str) {
    if cfg!(debug_assertions) {
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), "[nzap startup] {step}");
    }
}

/// The engine's configuration, shared by the window and the MCP server.
fn engine_options(paths: AppPaths, sessions_file: Option<PathBuf>) -> EngineOptions {
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
    EngineOptions {
        paths,
        endpoints,
        use_keychain: dev_env("NZAP_NO_KEYCHAIN").is_none(),
        oauth_client,
        sessions_file,
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
    let engine = Engine::new(engine_options(paths, None))?;
    Ok(AppState::new(engine, dev_env("NZAP_E2E_OPEN_LOG").map(PathBuf::from)))
}

/// The app's folders without a running app: the same platform directories
/// Tauri's path API resolves (`<data|config|cache dir>/<identifier>`).
fn headless_paths() -> Option<AppPaths> {
    if let Some(root) = dev_env("NZAP_DATA_DIR") {
        return Some(AppPaths::under(Path::new(&root)));
    }
    let identifier = context().config().identifier.clone();
    Some(AppPaths {
        data_dir: dirs::data_dir()?.join(&identifier),
        config_dir: dirs::config_dir()?.join(&identifier),
        cache_dir: dirs::cache_dir()?.join(&identifier),
    })
}

/// `nzap-engine mcp [options]`: serve an AI agent over stdin/stdout (see
/// docs/MCP.md). Uses the app's Google connection and settings, keeps its
/// own runtime list, and returns the process exit code.
pub fn run_mcp(args: Vec<String>) -> i32 {
    nzap_mcp::init_stderr_logger();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let options = match nzap_mcp::Options::parse(args, &cwd) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return if message.starts_with("Usage:") { 0 } else { 2 };
        }
    };
    let Some(paths) = headless_paths() else {
        eprintln!("Cannot find this user's application data folders.");
        return 1;
    };
    // One list per server process: agents may run several at once.
    let sessions_file =
        paths.data_dir.join("agents").join(format!("mcp-{}.json", std::process::id()));
    let engine = match Engine::new(engine_options(paths, Some(sessions_file.clone()))) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("NZAP Engine could not start: {error}");
            return 1;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("NZAP Engine could not start: {error}");
            return 1;
        }
    };
    log::info!("NZAP Engine {} serving MCP on stdio", nzap_core::VERSION);
    let server = nzap_mcp::Server::new(Arc::new(engine), options);
    let result = runtime.block_on(nzap_mcp::run_stdio(server));
    let _ = std::fs::remove_file(&sessions_file);
    // A blocking stdin read must not hold the process open.
    runtime.shutdown_timeout(Duration::from_secs(1));
    match result {
        Ok(()) => 0,
        Err(error) => {
            log::error!("The MCP connection failed: {error}");
            1
        }
    }
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

/// The compiled configuration, adjusted for WebDriver runs.
fn context() -> tauri::Context<tauri::Wry> {
    #[allow(unused_mut)]
    let mut context = tauri::generate_context!();
    // msedgedriver hands WebView2 its remote-debugging flags through
    // WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, which the browser arguments wry
    // sets explicitly can shadow. Merge them in debug builds so the desktop
    // E2E suite can attach.
    #[cfg(all(windows, debug_assertions))]
    {
        if let Some(extra) = dev_env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            startup_mark(&format!("WebView2 arguments from the environment: {extra}"));
            for window in &mut context.config_mut().app.windows {
                window.additional_browser_args = Some(format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection {extra}"
                ));
            }
        }
    }
    context
}

/// Build and run the application.
pub fn run() {
    startup_mark("building the app");
    let app = tauri::Builder::default()
        // Must be first: a second launch focuses the running window instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        // nzap:// links (the website's "Open in NZAP Engine"); right after
        // single-instance so a second launch hands its link to this one.
        .plugin(tauri_plugin_deep_link::init())
        .plugin(log_plugin())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // In-app updates: signed bundles from the release feed (see docs/RELEASING.md).
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(tray::on_window_event)
        .on_page_load(|webview, payload| {
            startup_mark(&format!(
                "page {:?}: {}",
                payload.event(),
                webview.url().map(|url| url.to_string()).unwrap_or_default()
            ));
        })
        .setup(|app| {
            startup_mark("setup: building the engine");
            let state = build_state(app.handle())?;
            startup_mark("setup: engine ready");
            let engine = state.engine.clone();
            let close_to_tray = engine.settings.get().close_to_tray;
            app.manage(state);
            if close_to_tray {
                if let Err(error) = tray::ensure(app.handle()) {
                    log::warn!("No system tray available: {error}");
                }
            }
            // Installers register nzap:// themselves; this covers AppImage and dev builds.
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(error) = app.deep_link().register_all() {
                    log::warn!("Could not register nzap:// links: {error}");
                }
            }
            log::info!("NZAP Engine {} started", nzap_core::VERSION);
            startup_mark("setup: done");
            // Reconnect to runtimes that survived the last session.
            tauri::async_runtime::spawn(async move { engine.resume().await });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::open_url,
            commands::app::reveal_path,
            commands::app::open_log_dir,
            commands::app::stream_cancel,
            commands::app::config_get,
            commands::app::settings_get,
            commands::app::settings_update,
            commands::app::settings_set_oauth_client,
            commands::app::mcp_info,
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
            commands::files::files_upload_bytes,
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
        .build(context())
        .expect("NZAP Engine failed to start");

    startup_mark("running the event loop");
    app.run(|handle, event| match event {
        RunEvent::Ready => startup_mark("event loop ready"),
        RunEvent::Exit => {
            if let Some(state) = handle.try_state::<AppState>() {
                // Runtimes keep running on Google's side; only local work stops.
                state.shutdown();
            }
        }
        // macOS: clicking the Dock icon brings a hidden window back.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => tray::show_main(handle),
        _ => {}
    });
}
