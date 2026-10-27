//! GUI liveness.
//!
//! An approval can only be answered by a human in the Ranch window. If the
//! app is closed, an agent must fail **immediately** with `hostUnavailable`
//! instead of waiting two minutes for a modal nobody will ever see.
//!
//! The GUI upserts `kv_settings.gui_heartbeat` every [`HEARTBEAT_INTERVAL`];
//! `ranch-mcp` treats a value older than [`HEARTBEAT_STALE_AFTER`] as "not
//! running". Same idea as Tabularis's `tabularis.alive` file, carried by the
//! database both processes already share.

use std::time::Duration;

use sqlx::SqlitePool;

use crate::commands::settings::{get_kv_setting_inner, set_kv_setting_inner};
use crate::core::errors::RanchError;

pub const HEARTBEAT_KEY: &str = "gui_heartbeat";
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(15);

/// Record "the GUI is alive right now".
pub async fn write_heartbeat(pool: &SqlitePool) -> Result<(), RanchError> {
    set_kv_setting_inner(pool, HEARTBEAT_KEY, &chrono::Utc::now().to_rfc3339()).await
}

/// Forget the heartbeat — called on a clean GUI exit so agents fail fast
/// right away rather than after the stale window.
pub async fn clear_heartbeat(pool: &SqlitePool) -> Result<(), RanchError> {
    sqlx::query("DELETE FROM kv_settings WHERE key = ?")
        .bind(HEARTBEAT_KEY)
        .execute(pool)
        .await?;
    Ok(())
}

/// Is a Ranch window there to answer an approval?
pub async fn is_gui_alive(pool: &SqlitePool) -> Result<bool, RanchError> {
    let Some(raw) = get_kv_setting_inner(pool, HEARTBEAT_KEY).await? else {
        return Ok(false);
    };
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(&raw) else {
        return Ok(false);
    };
    let age = chrono::Utc::now().signed_duration_since(at.with_timezone(&chrono::Utc));
    // A heartbeat from the future (clock change) is not proof of life either.
    Ok(age >= chrono::Duration::zero()
        && age
            .to_std()
            .map(|a| a < HEARTBEAT_STALE_AFTER)
            .unwrap_or(false))
}

/// GUI side: beat until the process exits. Errors are logged, never fatal —
/// a missed beat only makes agents fail fast, which is the safe direction.
pub fn spawn_heartbeat_task(pool: SqlitePool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            ticker.tick().await;
            if let Err(e) = write_heartbeat(&pool).await {
                tracing::warn!(error = %e, "could not write the GUI heartbeat");
            }
        }
    })
}
