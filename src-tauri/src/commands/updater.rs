//! Auto-update orchestration.
//!
//! Wraps `tauri-plugin-updater` with:
//!   - a 24h debounce stored in `kv_settings`,
//!   - a disable mechanism via env var + CLI flag.
//!
//! Signature verification, manifest fetching, and the apply step
//! are handled by the plugin itself; we only orchestrate the user
//! flow: `check_for_updates` asks the feed, `install_update` downloads,
//! verifies, replaces the app and restarts it, on the user's click, never
//! on its own. While it runs, `install_update` reports each phase on the
//! `updater:progress` event, so the window can say how far it has got: a
//! download of a dozen megabytes on a slow line is otherwise a frozen button.
//!
//! The feed (docs/releasing.md, "Auto-updates") is a static manifest, an
//! asset of the `update-feed` GitHub release: `latest.json` for stable builds,
//! `rc.json` for pre-releases (`scripts/set-version.mjs` switches the endpoint
//! at build time).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::contract::errors::RanchErrorDto;

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
/// updater plugin must not be loaded; it panics on a URL it cannot parse.
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

/// Why this build cannot update itself: `None` when it can. Pure, so the
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

/// The event `install_update` reports its progress on.
pub const PROGRESS_EVENT: &str = "updater:progress";

/// The `updater:progress` event: the phase the install the user started is
/// in, in order. A failure is not an event: it is the command's error, and the
/// last phase reported says where it happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum UpdateProgressEvent {
    /// Bytes received so far, and the size of the download when the server
    /// sent one (`None`: no `Content-Length`, the window shows activity only).
    Downloading { received: u64, total: Option<u64> },
    /// Downloaded: the signature is checked against the key built into Ranch.
    Verifying,
    /// The signature holds: the new version replaces this one.
    Installing,
    /// Installed: Ranch restarts.
    Restarting,
}

/// Turns the plugin's per-chunk callback (thousands of calls for one
/// download) into a report at most every [`DownloadMeter::EVERY`], plus the
/// one that completes the download, so the window is not flooded with events.
#[derive(Debug, Default)]
pub struct DownloadMeter {
    received: u64,
    last_report: Option<Instant>,
}

impl DownloadMeter {
    /// Often enough for the bar to move smoothly, rarely enough to cost nothing.
    pub const EVERY: Duration = Duration::from_millis(100);

    /// Count a chunk of `len` bytes; the report to send now, if one is due.
    pub fn chunk(
        &mut self,
        len: usize,
        total: Option<u64>,
        now: Instant,
    ) -> Option<UpdateProgressEvent> {
        self.received += len as u64;
        let complete = total.is_some_and(|t| self.received >= t);
        let due = self
            .last_report
            .is_none_or(|at| now.duration_since(at) >= Self::EVERY);
        if !(due || complete) {
            return None;
        }
        self.last_report = Some(now);
        Some(UpdateProgressEvent::Downloading {
            received: self.received,
            total,
        })
    }
}

/// One install at a time: a second click (or the sidebar and Settings both
/// asking) would download twice and race to replace the app.
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Held for the whole install; dropping it (the install failed) frees the slot
/// for a retry. On success the app restarts with it still held.
#[derive(Debug)]
pub struct InstallSlot(());

impl InstallSlot {
    /// `None` while another install runs.
    pub fn take() -> Option<Self> {
        INSTALLING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(()))
    }
}

impl Drop for InstallSlot {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::Release);
    }
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
/// the user's click. Each phase is reported on [`PROGRESS_EVENT`]. On success
/// this does not return: the app restarts.
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
    let Some(_slot) = InstallSlot::take() else {
        return Err(updater_error(
            "An update is already installing",
            "wait for it to finish",
        ));
    };
    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;
    let report = |progress: UpdateProgressEvent| {
        if let Err(e) = app.emit(PROGRESS_EVENT, progress) {
            tracing::debug!(error = %e, "update progress not delivered");
        }
    };
    let update = app
        .updater()
        .map_err(|e| updater_error("Cannot start the update", e))?
        .check()
        .await
        .map_err(|e| updater_error("Cannot read the update feed", e))?
        .ok_or_else(|| updater_error("Nothing to install", "Ranch is up to date"))?;
    tracing::info!(version = %update.version, "installing update");
    // `download` checks the signature after its last chunk: an error after
    // that point is the signature's, not the network's.
    let downloaded = AtomicBool::new(false);
    let mut meter = DownloadMeter::default();
    let bytes = update
        .download(
            |len, total| {
                if let Some(progress) = meter.chunk(len, total, Instant::now()) {
                    report(progress);
                }
            },
            || {
                downloaded.store(true, Ordering::Relaxed);
                report(UpdateProgressEvent::Verifying);
            },
        )
        .await
        .map_err(|e| {
            if downloaded.load(Ordering::Relaxed) {
                updater_error("The update's signature could not be verified", e)
            } else {
                updater_error("The update could not be downloaded", e)
            }
        })?;
    report(UpdateProgressEvent::Installing);
    update
        .install(bytes)
        .map_err(|e| updater_error("The update could not be installed", e))?;
    report(UpdateProgressEvent::Restarting);
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_placeholders_of_an_unconfigured_build_are_not_a_feed() {
        // What tauri.conf.json ships, and what reached v0.2.0-rc.3.
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
                "https://github.com/Bramvanosta/Ranch/releases/download/update-feed/latest.json"
            )]
        );
    }

    #[test]
    fn a_build_says_why_it_cannot_update_itself() {
        // Release build, real feed, macOS / Windows / an AppImage: it can.
        assert_eq!(self_update_unavailable(false, true, "macos", false), None);
        assert_eq!(self_update_unavailable(false, true, "linux", true), None);
        // A debug build never loads the plugin, whatever else is true.
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

    #[test]
    fn the_download_is_reported_at_a_steady_pace_and_on_its_last_chunk() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut meter = DownloadMeter::default();
        // The first chunk is reported at once: the bar starts moving.
        assert_eq!(
            meter.chunk(1_000, Some(10_000), at(0)),
            Some(UpdateProgressEvent::Downloading {
                received: 1_000,
                total: Some(10_000)
            })
        );
        // Chunks within the interval are counted, not reported.
        assert_eq!(meter.chunk(2_000, Some(10_000), at(40)), None);
        assert_eq!(meter.chunk(2_000, Some(10_000), at(80)), None);
        assert_eq!(
            meter.chunk(1_000, Some(10_000), at(100)),
            Some(UpdateProgressEvent::Downloading {
                received: 6_000,
                total: Some(10_000)
            })
        );
        // The chunk that completes it is reported, due or not.
        assert_eq!(
            meter.chunk(4_000, Some(10_000), at(120)),
            Some(UpdateProgressEvent::Downloading {
                received: 10_000,
                total: Some(10_000)
            })
        );
    }

    #[test]
    fn a_download_of_unknown_size_reports_what_it_received() {
        let start = Instant::now();
        let mut meter = DownloadMeter::default();
        assert_eq!(
            meter.chunk(512, None, start),
            Some(UpdateProgressEvent::Downloading {
                received: 512,
                total: None
            })
        );
        assert_eq!(meter.chunk(512, None, start), None);
    }

    #[test]
    fn the_progress_event_reads_as_the_window_expects() {
        let downloading = UpdateProgressEvent::Downloading {
            received: 42,
            total: None,
        };
        assert_eq!(
            serde_json::to_value(downloading).unwrap(),
            serde_json::json!({ "phase": "downloading", "received": 42, "total": null })
        );
        assert_eq!(
            serde_json::to_value(UpdateProgressEvent::Verifying).unwrap(),
            serde_json::json!({ "phase": "verifying" })
        );
    }

    #[test]
    fn one_install_at_a_time_and_a_failed_one_frees_the_slot() {
        let first = InstallSlot::take().expect("nothing installs yet");
        assert!(InstallSlot::take().is_none(), "a second install must wait");
        drop(first);
        assert!(InstallSlot::take().is_some(), "a retry can start");
    }

    // Shared lock: these tests mutate process-wide env.
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
