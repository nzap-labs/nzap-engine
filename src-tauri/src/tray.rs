//! Close-to-tray: when enabled in Settings, closing the window hides it and a
//! tray icon keeps the app (and runtime keep-alive) running until Quit.
//!
//! The tray is created only once the setting is on, so users who never enable
//! it do not depend on a tray host (on Linux, libayatana-appindicator).

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Window, WindowEvent};

use crate::state::AppState;

const TRAY_ID: &str = "main";

/// Bring the main window back.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Create the tray icon if it does not exist yet.
pub fn ensure(app: &AppHandle) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let show = MenuItem::with_id(app, "show", "Show NZAP Engine", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit NZAP Engine", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("NZAP Engine")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Remove the tray icon (the setting was switched off).
pub fn remove(app: &AppHandle) {
    let _ = app.remove_tray_by_id(TRAY_ID);
}

/// Window close: hide to the tray when the setting is on and the tray works;
/// otherwise let the window close and the app exit as usual.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else { return };
    let app = window.app_handle();
    let Some(state) = app.try_state::<AppState>() else { return };
    if !state.engine.settings.get().close_to_tray {
        return;
    }
    match ensure(app) {
        Ok(()) => {
            api.prevent_close();
            let _ = window.hide();
        }
        Err(error) => log::warn!("No system tray available, closing instead: {error}"),
    }
}
