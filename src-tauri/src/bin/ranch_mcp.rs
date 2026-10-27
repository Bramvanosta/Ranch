//! Standalone MCP server entry point.
//!
//! Claude Desktop / Cursor configs invoke this binary directly via stdio.
//! It shares the same SQLite database + keychain entries as the main
//! Tauri app — both point at the OS app-data dir for the `dev.ranch.app`
//! bundle identifier.
//!
//! The Tauri GUI does NOT spawn this binary; it's launched by the MCP
//! client. That keeps the GUI build pipeline simple and avoids "MCP
//! mode" branching in the main app.

use std::sync::Arc;

use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::{KeychainAdapter, OsKeychain};
use ranch_lib::core::mcp::{run_stdio, McpServerState};

const BUNDLE_IDENTIFIER: &str = "dev.ranch.app";

/// Resolve the Tauri-equivalent app-data dir without dragging in the
/// full Tauri runtime. Mirrors the per-OS conventions tauri uses for
/// `app.path().app_data_dir()`:
///   - macOS:   `$HOME/Library/Application Support/{bundle}/`
///   - Linux:   `$XDG_DATA_HOME/{bundle}/` or `$HOME/.local/share/{bundle}/`
///   - Windows: `%APPDATA%\{bundle}\`
fn resolve_app_data_dir() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    // Another database than the app's: integration tests drive the real
    // binary against a throwaway one (the keychain stays the user's own).
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

    let db = Db::init(&app_data_dir).await?;
    let keychain: Arc<dyn KeychainAdapter> = Arc::new(OsKeychain::new());
    let registry = Arc::new(ConnectionRegistry::new());

    let state = McpServerState {
        pool: db.pool().clone(),
        keychain,
        registry,
    };

    run_stdio(state).await?;
    Ok(())
}
