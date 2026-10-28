//! The menu bar icon: a click shows the main window, its menu opens or quits
//! Ranch. Closing the main window hides it, so Ranch keeps running here.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("system-tray")
        .setup(|app, _| {
            let menu = Menu::with_id_and_items(
                app,
                "system-tray",
                &[
                    &MenuItem::with_id(app, "show", "Open Ranch", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, "quit", "Quit Ranch", true, None::<&str>)?,
                ],
            )?;

            // On macOS the menu bar icon is a template image: black bars on
            // transparent, which the system draws in the menu bar's colour.
            // Elsewhere a panel may be dark or light, so the app icon stays.
            #[cfg(target_os = "macos")]
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true);
            #[cfg(not(target_os = "macos"))]
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone());
            tray.menu(&menu)
                .tooltip("Ranch")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    // Left click to show main window
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_ready(move |window| {
            // Only the main window hides: the others (New Window, New Tab,
            // Settings) close for real, so none lingers out of sight.
            if window.label() != "main" {
                return;
            }
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    let _ = window_clone.hide();
                    api.prevent_close();
                }
            });
        })
        .build()
}
