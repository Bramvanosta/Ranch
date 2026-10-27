//! read-only lock Tauri commands.
//!
//! Three commands surface the per-connection lock state to the frontend
//! and let the user toggle it for the current session:
//!
//! 1. [`get_connection_lock_state`] — lazy-spawn the actor + return its
//!    `ReadOnlyState`. Used by `ProductionLockBanner` on mount.
//! 2. [`unlock_connection`] — flip to `Unlocked` + write an
//!    `unlockConnection` audit entry. Triggered by `cmd-shift-U`.
//! 3. [`lock_connection`] — symmetric to unlock, action
//!    `lockConnection`, triggered by `cmd-shift-L`.
//!
//! ## Why the audit write happens HERE (not in the actor)
//!
//! The connection actor has no `SqlitePool` access. For the toggle
//! commands there's no Redis-side step whose success/failure determines
//! the audit outcome — the actor just flips an in-memory field. We
//! therefore record the audit entry from the Tauri command, AFTER the
//! actor has confirmed the new state. The audit funnel invariant
//! (everything routes through `core::audit::record`) is preserved.
//!
//! The destructive commands (`retry_job`, `remove_job`, etc.) write
//! their audit row from the actor handler instead, because the actor is the
//! only place that sees the Redis response. The toggle commands are
//! intentionally simpler.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::commands::connections::resolve_connection_config;
use crate::core::audit::{
    record as audit_record, AuditAction, AuditEntry, AuditOutcome, AuditSource, AuditTarget,
};
use crate::core::connection_actor::{ConnectionRegistry, ReadOnlyState};
use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::keychain::KeychainAdapter;

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
    get_connection_lock_state_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_connection_lock_state_inner(
    args: LockArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<ReadOnlyState, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    registry
        .get_lock_state(&args.connection_id, &config)
        .await
        .map_err(|e| RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        })
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
    unlock_connection_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn unlock_connection_inner(
    args: LockArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<ReadOnlyState, RanchError> {
    toggle_lock_inner(args, pool, keychain, registry, ToggleKind::Unlock).await
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
    lock_connection_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn lock_connection_inner(
    args: LockArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<ReadOnlyState, RanchError> {
    toggle_lock_inner(args, pool, keychain, registry, ToggleKind::Lock).await
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
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    kind: ToggleKind,
) -> Result<ReadOnlyState, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;

    // Snapshot identity for the audit entry BEFORE dispatching — if the
    // actor were to fail mid-flight we'd still write a `Failure` audit
    // row (future hardening; the toggle is infallible today).
    let connection_name_snapshot = config.connection_name.clone();
    let env_color_snapshot = config.env_color.clone();

    let actor_result = match kind {
        ToggleKind::Unlock => registry.unlock(&args.connection_id, &config).await,
        ToggleKind::Lock => registry.lock(&args.connection_id, &config).await,
    };

    let new_state = actor_result
        .map_err(|e| RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        })?
        // Inner Result — the toggle itself never fails, but the actor
        // side can return an audit-write error. We surface it directly.
        ?;

    // Record the audit entry AFTER the actor has confirmed the flip.
    // If the audit write itself fails (disk full, fsync rejected, ...),
    // we surface it to the caller — the lock state IS flipped but the
    // user sees an error so they know something's wrong with auditing.
    // Order matters: the in-memory flip is already done, but a
    // subsequent restart of the actor would re-read `is_locked` and
    // re-lock, so a missed audit doesn't leave a silent unlock.
    audit_record(
        pool,
        AuditEntry {
            source: AuditSource::Ui,
            action: kind.audit_action(),
            target: AuditTarget::Connection,
            outcome: AuditOutcome::Success,
            outcome_reason: None,
            connection_id: Some(args.connection_id.clone()),
            connection_name: Some(connection_name_snapshot),
            env_color: env_color_snapshot,
            agent_client: None,
            agent_transport: None,
            approval_id: None,
        },
    )
    .await?;

    Ok(new_state)
}
