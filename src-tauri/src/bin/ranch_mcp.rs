//! Standalone MCP server entry point.
//!
//! Claude Desktop / Cursor configs invoke this binary directly via stdio.
//! It shares the same SQLite database + keychain entries as the main
//! Tauri app: both point at the OS app-data dir for the `dev.ranch.app`
//! bundle identifier.
//!
//! The Tauri GUI does NOT spawn this binary; it's launched by the MCP
//! client. That keeps the GUI build pipeline simple and avoids "MCP
//! mode" branching in the main app.

use std::sync::Arc;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::bullmq_redis::BullmqRedisBackends;
use ranch_lib::adapters::hmac_sealer::{self, HmacSealer};
use ranch_lib::adapters::secrets::{InMemoryKeychain, OsKeychain};
use ranch_lib::adapters::sqlite::{self, Db, SqliteConnectionRepository, SqliteSettingsStore};
use ranch_lib::adapters::system_clock::SystemClock;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::application::ports::{Clock, ConnectionRepository, Sealer, SettingsStore};
use ranch_lib::shells::mcp::{run_stdio, McpServerState};
use sqlx::SqlitePool;

const BUNDLE_IDENTIFIER: &str = "dev.ranch.app";

/// Resolve the Tauri-equivalent app-data dir without dragging in the
/// full Tauri runtime. Mirrors the per-OS conventions tauri uses for
/// `app.path().app_data_dir()`:
///   - macOS:   `$HOME/Library/Application Support/{bundle}/`
///   - Linux:   `$XDG_DATA_HOME/{bundle}/` or `$HOME/.local/share/{bundle}/`
///   - Windows: `%APPDATA%\{bundle}\`
fn resolve_app_data_dir() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    // Another database than the app's: integration tests drive the real
    // binary against a throwaway one, with an empty keychain.
    // Debug builds only: in a release, a client config (a project's
    // `.mcp.json`) could point ranch-mcp at a crafted database (approvals
    // off, and a row reusing a real connection id to send its keychain
    // password to another host).
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("RANCH_MCP_DATA_DIR") {
        return Ok(std::path::PathBuf::from(dir));
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")?;
        Ok(std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(BUNDLE_IDENTIFIER))
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.local/share")))?;
        Ok(std::path::PathBuf::from(base).join(BUNDLE_IDENTIFIER))
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA")?;
        Ok(std::path::PathBuf::from(appdata).join(BUNDLE_IDENTIFIER))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err("unsupported OS".into())
    }
}

/// How often a running process checks that the app has not migrated the
/// database since it started.
const SCHEMA_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
/// How long the calls running may take to finish once the schema changed.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Stop when the app migrates the database under this process (an update):
/// this code no longer knows its tables. No new call starts, the calls
/// running finish (an approved write keeps its audit row), for
/// [`STOP_GRACE`] at most, then the process exits. The client reports the
/// server gone, and the message (on stderr, in the client's logs) says to
/// restart it.
fn watch_schema(pool: SqlitePool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SCHEMA_CHECK_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = sqlite::db::check_schema(&pool).await {
                if matches!(e, ranch_lib::domain::errors::RanchError::Migration(_)) {
                    tracing::error!("{e}");
                    ranch_lib::shells::mcp::calls::stop(STOP_GRACE).await;
                    std::process::exit(1);
                }
                tracing::warn!(error = %e, "could not check the database schema");
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialise minimal logging to stderr (stdout is reserved for the
    // MCP JSON-RPC frames). The MCP transport must NEVER write to
    // stdout outside of the JSON-RPC channel.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let app_data_dir = resolve_app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;
    tracing::info!(?app_data_dir, "ranch-mcp starting");

    // A test's throwaway database (debug builds only) was never sealed by an
    // app, and its connections keep no secret in the OS keychain: seals are
    // not checked there and the keychain is not read, as in the integration
    // tests that drive this binary. A crafted database cannot reach the
    // user's keychain this way either.
    #[cfg(debug_assertions)]
    let throwaway = std::env::var_os("RANCH_MCP_DATA_DIR").is_some();
    #[cfg(not(debug_assertions))]
    let throwaway = false;
    // The app migrates the database; this process only opens it at the
    // schema it was built for. Older or newer, it says what to do and stops
    // (the client shows the message), instead of failing call by call or
    // migrating the database under an older app.
    let db = if throwaway {
        Db::init(&app_data_dir).await?
    } else {
        Db::open_current(&app_data_dir).await.inspect_err(|e| {
            tracing::error!("{e}");
        })?
    };
    watch_schema(db.pool().clone());
    let (keychain, sealer): (Arc<dyn SecretStore>, Arc<dyn Sealer>) = if throwaway {
        (
            Arc::new(InMemoryKeychain::new()),
            Arc::new(HmacSealer::unsealed()),
        )
    } else {
        let keychain: Arc<dyn SecretStore> = Arc::new(OsKeychain::new());
        // Check the app's seals on approvals and safety settings: this
        // process never trusts what the database says on its own
        // (`application::ports::sealer`). No key (not created yet, or no
        // keychain): every agent write is refused.
        let key = hmac_sealer::load(&OsKeychain::for_integrity_key(), keychain.as_ref())
            .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "cannot read the integrity key: agent writes are refused");
            None
        });
        (keychain, Arc::new(HmacSealer::for_agents(key)))
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let repository: Arc<dyn ConnectionRepository> = Arc::new(SqliteConnectionRepository::new(
        db.pool().clone(),
        Arc::clone(&sealer),
        Arc::clone(&clock),
    ));
    let settings: Arc<dyn SettingsStore> = Arc::new(SqliteSettingsStore::new(
        db.pool().clone(),
        Arc::clone(&sealer),
        clock,
    ));
    // Queues are BullMQ on Redis, one actor per connection, spawned on
    // first use.
    let backends = Arc::new(BullmqRedisBackends::new(
        sqlite::write_pipeline(db.pool()),
        Arc::clone(&repository),
        keychain,
        ConnectionRegistry::new(),
    ));

    let state = McpServerState {
        pool: db.pool().clone(),
        repository,
        settings,
        sealer,
        backends,
    };

    run_stdio(state).await?;
    Ok(())
}
