//! Auto-update orchestration.
//!
//! Wraps `tauri-plugin-updater` with:
//!   - a 24h debounce stored in `kv_settings`,
//!   - a disable mechanism via env var + CLI flag.
//!
//! Signature verification, manifest fetching, and the apply step
//! are handled by the plugin itself; we only orchestrate the user
//! flow: `check_for_updates` asks the feed, `install_update` downloads,
//! verifies, replaces the app and restarts it — on the user's click, never
//! on its own.
//!
//! The feed (docs/releasing.md, "Auto-updates") is a static manifest, an
//! asset of the `update-feed` GitHub release: `latest.json` for stable builds,
//! `rc.json` for pre-releases (`scripts/set-version.mjs` switches the endpoint
//! at build time).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::errors::RanchErrorDto;

/// Env var that disables the update check.
pub const DISABLE_ENV_VAR: &str = "RANCH_DISABLE_UPDATE_CHECK";
/// CLI flag that disables the update check.
pub const DISABLE_CLI_FLAG: &str = "--no-update-check";

/// True when either the env var or the CLI flag asks
/// Ranch to skip update checks.
pub fn is_update_check_disabled() -> bool {
    is_disabled_via_env() || is_disabled_via_cli()
}

fn is_disabled_via_env() -> bool {
    std::env::var(DISABLE_ENV_VAR)
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

fn is_disabled_via_cli() -> bool {
    std::env::args().any(|a| a == DISABLE_CLI_FLAG)
}

/// Whether `plugins.updater` of the Tauri configuration points at a real
/// feed: at least one endpoint, every endpoint an http(s) URL, and a public
/// key that is not the `__TAURI_UPDATER_PUBKEY__` placeholder. Anything else
/// (the placeholders of an unconfigured build, no section at all) means the
/// updater plugin must not be loaded — it panics on a URL it cannot parse.
pub fn feed_is_configured(updater: Option<&serde_json::Value>) -> bool {
    let Some(updater) = updater else {
        return false;
    };
    let endpoints: Vec<&str> = updater
        .get("endpoints")
        .and_then(|e| e.as_array())
        .map(|list| list.iter().filter_map(|e| e.as_str()).collect())
        .unwrap_or_default();
    let real_url = |e: &&str| e.starts_with("https://") || e.starts_with("http://");
    let pubkey = updater.get("pubkey").and_then(|k| k.as_str()).unwrap_or("");
    !endpoints.is_empty()
        && endpoints.iter().all(real_url)
        && !pubkey.trim().is_empty()
        && !pubkey.starts_with("__")
}

/// Why this build cannot update itself — `None` when it can. Pure, so the
/// rules are testable without an app:
///
///   - a debug build never loads the updater plugin (`lib.rs`);
///   - a build without a real feed does not either;
///   - on Linux only an AppImage can replace itself (`$APPIMAGE` is set by
///     its runtime); a `.deb` / `.rpm` install belongs to the package manager.
pub fn self_update_unavailable(
    debug_build: bool,
    feed_configured: bool,
    os: &str,
    running_as_appimage: bool,
) -> Option<&'static str> {
    if debug_build {
        return Some("Development build: updates are only checked in release builds");
    }
    if !feed_configured {
        return Some("This build has no update feed");
    }
    if os == "linux" && !running_as_appimage {
        return Some("Installed from a package: update Ranch with your package manager");
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub available: bool,
    /// Newly-available version (when `available == true`).
    pub version: Option<String>,
    /// Release notes of that version, as published in the feed.
    #[serde(default)]
    pub notes: Option<String>,
    /// The running app's version.
    pub current_version: String,
    /// `true` when the check was suppressed by the disable mechanism
    /// (env var / CLI flag).
    pub disabled: bool,
    /// Reason for the disable (for the UI's status display).
    pub disabled_reason: Option<String>,
}

#[cfg(feature = "desktop")]
fn updater_error(context: &str, e: impl std::fmt::Display) -> RanchErrorDto {
    RanchErrorDto {
        kind: "updater".to_string(),
        message: format!("{context}: {e}"),
        doc_url: None,
    }
}

/// `None` when this running app can update itself.
#[cfg(feature = "desktop")]
fn unavailable_here(app: &tauri::AppHandle) -> Option<&'static str> {
    self_update_unavailable(
        cfg!(debug_assertions),
        feed_is_configured(app.config().plugins.0.get("updater")),
        std::env::consts::OS,
        std::env::var_os("APPIMAGE").is_some(),
    )
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateCheckResult, RanchErrorDto> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    if is_update_check_disabled() {
        let disabled_reason = if is_disabled_via_env() {
            Some(format!(
                "Disabled via {DISABLE_ENV_VAR} environment variable"
            ))
        } else {
            Some(format!("Disabled via {DISABLE_CLI_FLAG} CLI flag"))
        };
        return Ok(UpdateCheckResult {
            available: false,
            version: None,
            notes: None,
            current_version,
            disabled: true,
            disabled_reason,
        });
    }
    // A development build says nothing at all (as it always has): "Updates off"
    // in the footer of every `tauri dev` window would only be noise.
    if cfg!(debug_assertions) {
        return Ok(UpdateCheckResult {
            available: false,
            version: None,
            notes: None,
            current_version,
            disabled: false,
            disabled_reason: None,
        });
    }
    // The plugin is not even loaded in these builds: asking it would panic.
    if let Some(reason) = unavailable_here(&app) {
        return Ok(UpdateCheckResult {
            available: false,
            version: None,
            notes: None,
            current_version,
            disabled: true,
            disabled_reason: Some(reason.to_string()),
        });
    }

    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| updater_error("Cannot start the update check", e))?
        .check()
        .await
        .map_err(|e| updater_error("Cannot read the update feed", e))?;
    Ok(UpdateCheckResult {
        available: update.is_some(),
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update
            .and_then(|u| u.body.clone())
            .filter(|n| !n.trim().is_empty()),
        current_version,
        disabled: false,
        disabled_reason: None,
    })
}

/// Download the update the feed offers, verify its signature against the key
/// compiled into this app, replace the app and restart. Only ever called from
/// the user's click. On success this does not return: the app restarts.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), RanchErrorDto> {
    if is_update_check_disabled() {
        return Err(updater_error("Updates are disabled", "see Settings"));
    }
    if let Some(reason) = unavailable_here(&app) {
        return Err(updater_error("This build cannot update itself", reason));
    }
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| updater_error("Cannot start the update", e))?
        .check()
        .await
        .map_err(|e| updater_error("Cannot read the update feed", e))?
        .ok_or_else(|| updater_error("Nothing to install", "Ranch is up to date"))?;
    tracing::info!(version = %update.version, "installing update");
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| updater_error("The update could not be installed", e))?;
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_placeholders_of_an_unconfigured_build_are_not_a_feed() {
        // What tauri.conf.json ships — and what reached v0.2.0-rc.3.
        let shipped = serde_json::json!({
            "pubkey": "__TAURI_UPDATER_PUBKEY__",
            "endpoints": ["__TAURI_UPDATER_ENDPOINT__"],
        });
        assert!(!feed_is_configured(Some(&shipped)));
        assert!(!feed_is_configured(None));
        assert!(!feed_is_configured(Some(
            &serde_json::json!({ "endpoints": [] })
        )));
        // A real endpoint but still the placeholder key: updates could not be verified.
        let half = serde_json::json!({
            "pubkey": "__TAURI_UPDATER_PUBKEY__",
            "endpoints": ["https://example.org/latest.json"],
        });
        assert!(!feed_is_configured(Some(&half)));
    }

    #[test]
    fn a_real_endpoint_and_key_are_a_feed() {
        let real = serde_json::json!({
            "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWdu",
            "endpoints": ["https://git.example.org/ranch/latest.json"],
        });
        assert!(feed_is_configured(Some(&real)));
    }

    /// What this repository ships: the stable channel of the `update-feed`
    /// GitHub release, over https, with a real public key.
    /// (`scripts/set-version.mjs` switches a pre-release build to `rc.json`; it
    /// looks for this exact path.)
    #[test]
    fn the_repository_ships_the_stable_feed() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).expect("tauri.conf.json");
        let updater = conf.pointer("/plugins/updater");
        assert!(feed_is_configured(updater));
        let endpoints = updater.unwrap()["endpoints"].as_array().unwrap();
        assert_eq!(
            endpoints,
            &[serde_json::json!(
                "https://github.com/Bramvanosta/ranch/releases/download/update-feed/latest.json"
            )]
        );
    }

    #[test]
    fn a_build_says_why_it_cannot_update_itself() {
        // Release build, real feed, macOS / Windows / an AppImage: it can.
        assert_eq!(self_update_unavailable(false, true, "macos", false), None);
        assert_eq!(self_update_unavailable(false, true, "linux", true), None);
        // A debug build never loads the plugin — whatever else is true.
        assert!(self_update_unavailable(true, true, "macos", false)
            .unwrap()
            .contains("Development build"));
        assert!(self_update_unavailable(false, false, "macos", false)
            .unwrap()
            .contains("no update feed"));
        // A .deb / .rpm install cannot replace itself.
        assert!(self_update_unavailable(false, true, "linux", false)
            .unwrap()
            .contains("package manager"));
    }

    // Shared lock — these tests mutate process-wide env.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn env_var_disables() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::set_var(DISABLE_ENV_VAR, "1");
        }
        assert!(is_disabled_via_env());
        assert!(is_update_check_disabled());
        unsafe {
            std::env::remove_var(DISABLE_ENV_VAR);
        }
    }

    #[test]
    fn env_var_truthy_synonyms() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for v in ["1", "true", "TRUE", "yes"] {
            unsafe {
                std::env::set_var(DISABLE_ENV_VAR, v);
            }
            assert!(is_disabled_via_env(), "value {v} should disable");
        }
        unsafe {
            std::env::remove_var(DISABLE_ENV_VAR);
        }
    }

    #[test]
    fn env_var_falsy_does_not_disable() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var(DISABLE_ENV_VAR, "0");
        }
        assert!(!is_disabled_via_env());
        unsafe {
            std::env::remove_var(DISABLE_ENV_VAR);
        }
    }
}
