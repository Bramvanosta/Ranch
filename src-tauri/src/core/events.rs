//! The one seam between the core and a GUI shell.
//!
//! Long operations report progress by emitting events to the window. The core
//! used to name `tauri::AppHandle` for that — which made the whole crate need
//! Tauri, WebKit included, even for a headless build. It now names this
//! module's [`AppHandle`]:
//!
//!   - with the `desktop` feature it *is* `tauri::AppHandle`, and [`emit`]
//!     is `tauri::Emitter::emit`;
//!   - without it, it is a type nobody can construct: every
//!     `Option<AppHandle>` is `None`, and [`emit`] is never reached. The HTTP
//!     shell runs the same code and simply reports the final
//!     result instead of live progress.

#[cfg(feature = "desktop")]
pub type AppHandle = tauri::AppHandle;

/// Uninhabited: there is no window to emit to.
#[cfg(not(feature = "desktop"))]
#[derive(Clone, Debug)]
pub enum AppHandle {}

/// Emit `event` to the GUI. Failures are the caller's to ignore: progress is
/// best-effort, the operation's result is what counts.
#[cfg(feature = "desktop")]
pub fn emit<S: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    use tauri::Emitter;
    if let Err(e) = app.emit(event, payload) {
        tracing::debug!(error = %e, event, "event not delivered");
    }
}

#[cfg(not(feature = "desktop"))]
pub fn emit<S: serde::Serialize + Clone>(app: &AppHandle, _event: &str, _payload: S) {
    match *app {}
}
