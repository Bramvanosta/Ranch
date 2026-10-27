//! Native windows and tabs.
//!
//! `open_window` opens a route of the app in a new window, or as a new tab of
//! the calling window: on macOS, windows share the tabbing identifier `ranch`
//! and a tab is attached with `addTabbedWindow:ordered:` (the system's own
//! tab bar, ⌘W, Merge All Windows); elsewhere a tab is a window.
//!
//! The Settings window is separate: one, with its own label.
//!
//! Desktop only: the server image has one browser tab per window already.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::errors::RanchErrorDto;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenWindowArgs {
    /// The app route to show, path and query (`/connections/c1/queues/emails?view=jobs`).
    pub route: String,
    /// `true` = a tab of the calling window (macOS), `false` = a window of its own.
    pub tab: bool,
}

/// The routes a window may open: the app's own, nothing else.
pub fn is_app_route(route: &str) -> bool {
    route.starts_with('/') && !route.starts_with("//") && !route.contains('\\')
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn open_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    args: OpenWindowArgs,
) -> Result<(), RanchErrorDto> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    if !is_app_route(&args.route) {
        return Err(window_error(format!("not an app route: {}", args.route)));
    }
    let label = format!(
        "win-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );
    // The page boots at index.html and replaces its URL with the route before
    // the router starts (`main.tsx`): a query string survives, and no asset
    // server has to know the app's routes.
    let route = serde_json::to_string(&args.route).map_err(|e| window_error(e.to_string()))?;
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("Ranch")
        .inner_size(1280.0, 800.0)
        .min_inner_size(1100.0, 640.0)
        .initialization_script(format!("window.__RANCH_OPEN_ROUTE__ = {route};"));
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(18.0, 22.0))
        .tabbing_identifier("ranch");
    let created = builder.build().map_err(|e| window_error(e.to_string()))?;

    #[cfg(target_os = "macos")]
    if args.tab {
        let parent = window.clone();
        app.run_on_main_thread(move || {
            if let (Ok(parent), Ok(child)) = (parent.ns_window(), created.ns_window()) {
                // SAFETY: both pointers are live NSWindow objects owned by
                // Tauri, and AppKit is only touched on the main thread.
                unsafe {
                    let parent = &*(parent as *mut objc2::runtime::AnyObject);
                    let child = child as *mut objc2::runtime::AnyObject;
                    // NSWindowOrderingMode.above = 1
                    let _: () = objc2::msg_send![parent, addTabbedWindow: child, ordered: 1isize];
                }
            }
        })
        .map_err(|e| window_error(e.to_string()))?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (&window, &created);
    Ok(())
}

/// The pages of the Settings window (its toolbar tabs).
pub const SETTINGS_TABS: &[&str] = &[
    "general",
    "appearance",
    "refresh",
    "alerts",
    "updates",
    "advanced",
    "ai",
];

/// The Settings window's label: one window, whoever asks.
pub const SETTINGS_WINDOW: &str = "settings";

/// The route the Settings window shows for a tab (General when unknown).
pub fn settings_route(tab: Option<&str>) -> String {
    let tab = tab
        .filter(|t| SETTINGS_TABS.contains(t))
        .unwrap_or("general");
    format!("/settings?tab={tab}")
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenSettingsArgs {
    /// The page to show (`general`, `appearance`, …); `None` keeps the
    /// current one of an open window, or General for a new one.
    pub tab: Option<String>,
}

/// Ranch › Settings… (⌘,): one Settings window. Already open, it comes to the front (on the asked page);
/// otherwise it opens beside the other windows, not as one of their tabs.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn open_settings_window(
    app: tauri::AppHandle,
    args: OpenSettingsArgs,
) -> Result<(), RanchErrorDto> {
    use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

    if let Some(existing) = app.get_webview_window(SETTINGS_WINDOW) {
        if let Some(tab) = args.tab.as_deref().filter(|t| SETTINGS_TABS.contains(t)) {
            // The window's router follows (`useSettingsWindowTab`).
            existing
                .emit_to(SETTINGS_WINDOW, "settings:show-tab", tab)
                .map_err(|e| window_error(e.to_string()))?;
        }
        let _ = existing.unminimize();
        existing.show().map_err(|e| window_error(e.to_string()))?;
        existing
            .set_focus()
            .map_err(|e| window_error(e.to_string()))?;
        return Ok(());
    }

    let route = serde_json::to_string(&settings_route(args.tab.as_deref()))
        .map_err(|e| window_error(e.to_string()))?;
    let builder =
        WebviewWindowBuilder::new(&app, SETTINGS_WINDOW, WebviewUrl::App("index.html".into()))
            .title("Settings")
            .inner_size(760.0, 620.0)
            .min_inner_size(640.0, 480.0)
            .maximizable(false)
            .center()
            .initialization_script(format!("window.__RANCH_OPEN_ROUTE__ = {route};"));
    // Its own tabbing identifier: Settings never merges into a Ranch window.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(16.0, 20.0))
        .tabbing_identifier("ranch-settings");
    builder.build().map_err(|e| window_error(e.to_string()))?;
    Ok(())
}

#[cfg(feature = "desktop")]
fn window_error(reason: String) -> RanchErrorDto {
    crate::core::errors::RanchError::InvalidInput {
        reason: format!("Could not open a window: {reason}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::{is_app_route, settings_route};

    #[test]
    fn the_settings_window_opens_on_a_known_page() {
        assert_eq!(settings_route(Some("alerts")), "/settings?tab=alerts");
        assert_eq!(settings_route(None), "/settings?tab=general");
        assert_eq!(settings_route(Some("../x")), "/settings?tab=general");
        assert!(is_app_route(&settings_route(Some("ai"))));
    }

    #[test]
    fn only_the_apps_own_routes_open() {
        assert!(is_app_route("/connections/c1/queues/emails?view=jobs"));
        assert!(is_app_route("/"));
        assert!(!is_app_route("https://example.com"));
        assert!(!is_app_route("//example.com/x"));
        assert!(!is_app_route("javascript:alert(1)"));
        assert!(!is_app_route("\\\\host\\share"));
    }
}
