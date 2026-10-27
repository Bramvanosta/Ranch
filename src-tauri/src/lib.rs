//! Ranch backend entry point.
//!
//! The `commands::registry` module is the spine: every Tauri command and
//! every rmcp tool registers exactly once there. It is wired into the Tauri
//! builder, with the data-layer foundation (sqlite + keychain) in managed
//! `AppState` and tauri-specta generating typed Rust↔TS bindings.

// `RanchError` carries variants whose payloads accumulate
// enough Strings to cross clippy's `result_large_err` default
// threshold (128 bytes). The typed-error approach is deliberate —
// each variant carries identity fields the UI needs to render
// actionable errors. Boxing the variants would erase that ergonomic.
// Allow at the crate level rather than sprinkling per-function allows.
#![allow(clippy::result_large_err)]

pub mod app_state;
pub mod commands;
pub mod core;
#[cfg(feature = "desktop")]
mod plugins;
/// The HTTP shell.
#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "desktop")]
use std::sync::Arc;
#[cfg(feature = "desktop")]
use tauri::Manager;
#[cfg(feature = "desktop")]
use tracing_subscriber::EnvFilter;

/// The desktop app. The HTTP shell has its own entry point
/// (`src/server/main.rs`, feature `server`).
#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Structured logging — respects RUST_LOG (e.g. `RUST_LOG=ranch=debug`).
    // Default level is INFO. `try_init` so we don't panic if a host process
    // (test harness, embedded scenario) already installed a subscriber.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    // tauri-specta: build the typed command registry. The Builder both produces
    // the Tauri invoke handler AND exports `bindings.ts` for the frontend.
    //
    // Export happens in debug builds at app launch — convenient when iterating
    // (`pnpm tauri:dev` keeps bindings.ts fresh). For headless regeneration
    // (CI, pre-commit, codegen on demand) `cargo test --test codegen` calls
    // the same builder and writes the file. The file is committed to git so
    // CI/release builds don't re-run codegen.
    let specta_builder = commands::specta_builder();

    #[cfg(debug_assertions)]
    {
        if let Err(e) = specta_builder.export(
            specta_typescript::Typescript::default(),
            "../src/lib/tauri/bindings.ts",
        ) {
            tracing::warn!(error = %e, "tauri-specta bindings export failed; continuing");
        }
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Second-instance launch: focus the existing main window.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
                let _ = window.unminimize();
                let _ = window.show();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // `ranch://` deep-link plugin. Listens for OS
        // URL handoffs; emits `onOpenUrl` to the frontend so the
        // router can navigate to the target.
        .plugin(tauri_plugin_deep_link::init())
        // Native OS notifications via the Tauri plugin.
        // Used by `useThresholdWatcher` to alert on queue-counter
        // threshold crossings.
        .plugin(tauri_plugin_notification::init())
        .plugin(plugins::system_tray::init())
        .invoke_handler(specta_builder.invoke_handler())
        .setup(move |app| {
            specta_builder.mount_events(app);

            // Resolve per-OS app-data dir (e.g. ~/Library/Application Support/dev.ranch.app/),
            // create if missing, then init SQLite + keychain into managed state.
            let app_data_dir = app.path().app_data_dir().map_err(|e| {
                Box::<dyn std::error::Error>::from(format!("resolve app data dir: {e}"))
            })?;
            std::fs::create_dir_all(&app_data_dir)?;

            let db = tauri::async_runtime::block_on(core::db::Db::init(&app_data_dir))?;
            let keychain = core::keychain::OsKeychain::new();

            // The GUI is the only place an agent's write can be
            // approved. Beat so `ranch-mcp` knows someone is here (written
            // once synchronously so an agent calling right after launch is
            // not told "Ranch is not running"), watch the approval queue,
            // and keep it from piling up.
            let pool = db.pool().clone();
            tauri::async_runtime::block_on(core::mcp::heartbeat::write_heartbeat(&pool))?;
            {
                let handle = app.handle().clone();
                let pool = pool.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tokio::join!(
                        core::mcp::heartbeat::spawn_heartbeat_task(pool.clone()),
                        core::mcp::watcher::spawn_approval_watcher(handle, pool.clone()),
                        core::mcp::watcher::spawn_approval_cleanup(pool),
                    );
                });
            }

            app.manage(app_state::AppState {
                db: Arc::new(db),
                keychain: Arc::new(keychain),
                // Empty registry; actors lazy-spawn on first use.
                connections: Arc::new(core::connection_actor::ConnectionRegistry::new()),
            });

            tracing::info!(?app_data_dir, "ranch backend initialized");
            Ok(())
        });

    let context = tauri::generate_context!();

    // Updater plugin only in release builds (template-shipped pattern; full
    // signed-update flow is wired in the updater commands) — and only when the
    // build was given a real feed. `tauri.conf.json` ships placeholders that a
    // release pipeline substitutes; the plugin refuses them while it
    // initialises, and a release that was not given a feed (v0.2.0-rc.3, every
    // platform) panicked before its window opened.
    #[cfg(not(debug_assertions))]
    let builder =
        if commands::updater::feed_is_configured(context.config().plugins.0.get("updater")) {
            builder.plugin(tauri_plugin_updater::Builder::new().build())
        } else {
            tracing::info!("no update feed configured in this build: updater plugin not loaded");
            builder
        };

    builder
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| {
            // On a clean exit, forget the heartbeat right away so
            // an agent is told "Ranch is not running" immediately instead of
            // after the 15 s stale window. (A crash is covered by that window.)
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<app_state::AppState>() {
                    let pool = state.db.pool().clone();
                    let _ = tauri::async_runtime::block_on(core::mcp::heartbeat::clear_heartbeat(
                        &pool,
                    ));
                }
            }
        });
}
