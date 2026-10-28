//! read-only lock Tauri commands.
//!
//! Three commands surface the per-connection lock state to the frontend
//! and let the user toggle it for the current session:
//!
//! 1. [`get_connection_lock_state`]: lazy-spawn the actor + return its
//!    `ReadOnlyState`. Read by the lock button of a connection's toolbar.
//! 2. [`unlock_connection`]: flip to `Unlocked` + write an
//!    `unlockConnection` audit entry. Triggered by `cmd-shift-U`.
//! 3. [`lock_connection`]: symmetric to unlock, action
//!    `lockConnection`, triggered by `cmd-shift-L`.
//!
//! ## Why the audit write happens here, not in the backend
//!
//! A toggle has no queue-side step whose outcome decides the audit row:
//! the backend flips its live lock state and says so. The row is recorded
//! here, after the backend confirmed the new state, through
//! `adapters::sqlite::audit::record` like every other row. Queue writes are
//! audited by the backend's write pipeline instead, because only the
//! backend sees their outcome.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;

use crate::adapters::sqlite::audit::record as audit_record;
use crate::app_state::AppState;
use crate::application::ports::Backends;
use crate::contract::connections::ReadOnlyState;
use crate::contract::errors::RanchErrorDto;
use crate::domain::audit::{AuditAction, AuditEntry, AuditOutcome, AuditSource, AuditTarget};
use crate::domain::errors::RanchError;
use crate::domain::lock;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LockArgs {
    pub connection_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// get_connection_lock_state
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_connection_lock_state(
    args: LockArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ReadOnlyState, RanchErrorDto> {
    get_connection_lock_state_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_connection_lock_state_inner(
    args: LockArgs,
    backends: &dyn Backends,
) -> Result<ReadOnlyState, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    Ok(backend.connection().lock_state().await?.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// unlock_connection
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn unlock_connection(
    args: LockArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ReadOnlyState, RanchErrorDto> {
    unlock_connection_inner(args, state.db.pool(), state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn unlock_connection_inner(
    args: LockArgs,
    pool: &SqlitePool,
    backends: &dyn Backends,
) -> Result<ReadOnlyState, RanchError> {
    Ok(toggle_lock_inner(args, pool, backends, ToggleKind::Unlock)
        .await?
        .into())
}

// ─────────────────────────────────────────────────────────────────────────────
// lock_connection
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn lock_connection(
    args: LockArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ReadOnlyState, RanchErrorDto> {
    lock_connection_inner(args, state.db.pool(), state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn lock_connection_inner(
    args: LockArgs,
    pool: &SqlitePool,
    backends: &dyn Backends,
) -> Result<ReadOnlyState, RanchError> {
    Ok(toggle_lock_inner(args, pool, backends, ToggleKind::Lock)
        .await?
        .into())
}

// ─────────────────────────────────────────────────────────────────────────────
// shared toggle implementation
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum ToggleKind {
    Unlock,
    Lock,
}

impl ToggleKind {
    fn audit_action(self) -> AuditAction {
        match self {
            Self::Unlock => AuditAction::UnlockConnection,
            Self::Lock => AuditAction::LockConnection,
        }
    }
}

async fn toggle_lock_inner(
    args: LockArgs,
    pool: &SqlitePool,
    backends: &dyn Backends,
    kind: ToggleKind,
) -> Result<lock::ReadOnlyState, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    let new_state = match kind {
        ToggleKind::Unlock => backend.connection().unlock().await?,
        ToggleKind::Lock => backend.connection().lock().await?,
    };

    // Record the audit entry after the backend confirmed the flip. A
    // failed audit write reaches the caller: the state is flipped, and the
    // user learns that auditing is broken. A missed row never leaves a
    // silent unlock: an unlock lasts for the session only, a restart
    // locks again.
    audit_record(
        pool,
        AuditEntry {
            source: AuditSource::Ui,
            action: kind.audit_action(),
            target: AuditTarget::Connection,
            outcome: AuditOutcome::Success,
            outcome_reason: None,
            connection_id: Some(args.connection_id.clone()),
            connection_name: Some(backend.stamp().name.clone()),
            env_color: backend.stamp().env_color.clone(),
            agent_client: None,
            agent_transport: None,
            approval_id: None,
        },
    )
    .await?;

    Ok(new_state)
}
