//! Ranch backend entry point.
//!
//! The catalogue of operations (`application::catalogue`) is the spine:
//! every operation is listed once there, with the shells that expose it
//! (the Tauri commands, the HTTP server, the MCP tools). The commands are
//! wired into the Tauri builder, with the data-layer foundation (sqlite +
//! keychain) in managed `AppState` and tauri-specta generating typed
//! Rust↔TS bindings.

// `RanchError` carries variants whose payloads accumulate
// enough Strings to cross clippy's `result_large_err` default
// threshold (128 bytes). The typed-error approach is deliberate:
// each variant carries identity fields the UI needs to render
// actionable errors. Boxing the variants would erase that ergonomic.
// Allow at the crate level rather than sprinkling per-function allows.
#![allow(clippy::result_large_err)]

pub mod adapters;
pub mod app_state;
/// Ranch's operations, their rules and ports: the `ranch-application` crate.
pub use ranch_application as application;
pub mod commands;
pub mod contract;
pub mod core;
/// Ranch's types and rules: the `ranch-domain` crate.
pub use ranch_domain as domain;
#[cfg(feature = "desktop")]
mod plugins;
/// The HTTP shell.
#[cfg(feature = "server")]
pub mod server;
pub mod shells;
#[cfg(feature = "desktop")]
mod startup;

#[cfg(feature = "desktop")]
use std::sync::Arc;
#[cfg(feature = "desktop")]
use tauri::Manager;
#[cfg(feature = "desktop")]
use tauri_plugin_notification::NotificationExt;

/// The desktop app. The HTTP shell has its own entry point
/// (`src/server/main.rs`, feature `server`).
#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Structured logging, to the terminal and to `ranch.log`.
    let log_file = startup::init_logging();

    // tauri-specta: build the typed command registry. The Builder both produces
    // the Tauri invoke handler AND exports `bindings.ts` for the frontend.
    //
    // Export happens in debug builds at app launch, convenient when iterating
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

            // A database that does not open is shown to the user, who
            // decides; the app does not stop without a word.
            let db = startup::open_database(&app_data_dir, log_file.as_deref());
            let had_seals =
                tauri::async_runtime::block_on(adapters::sqlite::db::has_seals(db.pool()))?;
            let keychain = adapters::secrets::OsKeychain::new();

            // The key that seals approvals, the agent safety settings and
            // the connections (`application::ports::sealer`), in its own
            // keychain service. None anywhere: the run that creates it
            // seals the state as it is, unless seals were already there.
            // Then the key was lost and the state cannot be vouched for:
            // agent safety is reset to its strictest first, and the key is
            // created only once that is done (a failed reset is tried again
            // at the next launch). With a key, the connection seals made
            // before they covered the endpoint, those that still hold, are
            // carried over. A keychain that cannot be read now (locked) is
            // asked again in the background; until then nothing is sealed,
            // and ranch-mcp opens no connection. Nothing here stops the app.
            let integrity = adapters::secrets::OsKeychain::for_integrity_key();
            let found = adapters::hmac_sealer::load_for_app(&integrity, &keychain);
            let sealer = Arc::new(adapters::hmac_sealer::HmacSealer::for_app_reloading(
                Box::new(|| {
                    let integrity = adapters::secrets::OsKeychain::for_integrity_key();
                    let passwords = adapters::secrets::OsKeychain::new();
                    adapters::hmac_sealer::load(&integrity, &passwords)
                        .ok()
                        .flatten()
                }),
            ));
            let state = app_state::AppState::new(db, Arc::new(keychain), Arc::clone(&sealer));
            match found {
                Ok(Some(key)) => {
                    sealer.install(key);
                    // Not fatal: a connection left unsealed is refused to
                    // agents, not to the user.
                    let verifier = adapters::hmac_sealer::HmacSealer::for_agents(Some(key));
                    match tauri::async_runtime::block_on(state.repository.upgrade_seals(&verifier))
                    {
                        Ok(0) => {}
                        Ok(upgraded) => {
                            tracing::info!(upgraded, "connection seals now cover the endpoint")
                        }
                        Err(e) => tracing::warn!(error = %e, "could not carry connection seals over"),
                    }
                }
                Ok(None) if had_seals => {
                    tracing::warn!(
                        "integrity key missing while seals exist: agent safety reset to its strictest"
                    );
                    match tauri::async_runtime::block_on(startup::restrict_after_key_loss(&state)) {
                        Ok(()) => {
                            match adapters::hmac_sealer::create(&integrity) {
                                Ok(key) => sealer.install(key),
                                Err(e) => {
                                    tracing::warn!(error = %e, "could not create the integrity key: agents cannot write")
                                }
                            }
                            if let Err(e) = app
                                .notification()
                                .builder()
                                .title(startup::KEY_LOSS_TITLE)
                                .body(startup::KEY_LOSS_BODY)
                                .show()
                            {
                                tracing::warn!(error = %e, "could not show the safety reset notification");
                            }
                        }
                        Err(e) => tracing::error!(
                            error = %e,
                            "could not reset agent safety: no integrity key is created, agents cannot write; tried again at the next launch"
                        ),
                    }
                }
                Ok(None) => match adapters::hmac_sealer::create(&integrity) {
                    Ok(key) => {
                        sealer.install(key);
                        if let Err(e) = tauri::async_runtime::block_on(async {
                            state.settings.seal_all().await?;
                            state.repository.reseal_all().await
                        }) {
                            tracing::warn!(error = %e, "could not seal the agent safety state");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "could not create the integrity key: agents cannot write")
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "cannot read the integrity key: agents cannot write until it can be read");
                    startup::spawn_key_reload(Arc::clone(&sealer));
                }
            }

            // The GUI is the only place an agent's write can be
            // approved. Beat so `ranch-mcp` knows someone is here (written
            // once synchronously so an agent calling right after launch is
            // not told "Ranch is not running"), watch the approval queue,
            // and keep it from piling up.
            let pool = state.db.pool().clone();
            let settings = Arc::clone(&state.settings);
            tauri::async_runtime::block_on(core::mcp::heartbeat::write_heartbeat(
                settings.as_ref(),
            ))?;
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tokio::join!(
                        core::mcp::heartbeat::spawn_heartbeat_task(Arc::clone(&settings)),
                        core::mcp::watcher::spawn_approval_watcher(handle, pool.clone(), settings),
                        core::mcp::watcher::spawn_approval_cleanup(pool),
                    );
                });
            }

            app.manage(state);

            tracing::info!(?app_data_dir, "ranch backend initialized");
            Ok(())
        });

    let context = tauri::generate_context!();

    // Updater plugin only in release builds (template-shipped pattern; full
    // signed-update flow is wired in the updater commands), and only when the
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
                    let _ = tauri::async_runtime::block_on(core::mcp::heartbeat::clear_heartbeat(
                        state.settings.as_ref(),
                    ));
                }
            }
        });
}
