//! GUI liveness.
//!
//! An approval can only be answered by a human in the Ranch window. If the
//! app is closed, an agent must fail **immediately** with `hostUnavailable`
//! instead of waiting two minutes for a modal nobody will ever see.
//!
//! The GUI upserts `kv_settings.gui_heartbeat` every [`HEARTBEAT_INTERVAL`];
//! `ranch-mcp` treats a value older than [`HEARTBEAT_STALE_AFTER`] as "not
//! running". The heartbeat is carried by the database both processes already
//! share.

use std::sync::Arc;
use std::time::Duration;

use crate::application::ports::SettingsStore;
use crate::domain::errors::RanchError;

pub const HEARTBEAT_KEY: &str = "gui_heartbeat";
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(15);

/// Record "the GUI is alive right now".
pub async fn write_heartbeat(settings: &dyn SettingsStore) -> Result<(), RanchError> {
    settings
        .set(HEARTBEAT_KEY, &chrono::Utc::now().to_rfc3339())
        .await
}

/// Forget the heartbeat. Called on a clean GUI exit so agents fail fast
/// right away rather than after the stale window.
pub async fn clear_heartbeat(settings: &dyn SettingsStore) -> Result<(), RanchError> {
    settings.remove(HEARTBEAT_KEY).await
}

/// Is a Ranch window there to answer an approval?
pub async fn is_gui_alive(settings: &dyn SettingsStore) -> Result<bool, RanchError> {
    let Some(raw) = settings.get(HEARTBEAT_KEY).await? else {
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

/// GUI side: beat until the process exits. Errors are logged, never fatal:
/// a missed beat only makes agents fail fast, which is the safe direction.
pub fn spawn_heartbeat_task(settings: Arc<dyn SettingsStore>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            ticker.tick().await;
            if let Err(e) = write_heartbeat(settings.as_ref()).await {
                tracing::warn!(error = %e, "could not write the GUI heartbeat");
            }
        }
    })
}
