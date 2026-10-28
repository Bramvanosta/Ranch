//! The actor's side of the write pipeline (`application::writes`): it
//! supplies the connection and its live lock state, so the check and the
//! write happen in the same turn of the write loop.

use crate::connection_actor::actor::ActorState;
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;

/// Step 1 of a write. One refused by the lock leaves a failure row for
/// its target, unit or bulk, so the log shows the attempt. Best effort:
/// failing to record it never hides the refusal.
pub(super) async fn authorize_write(
    state: &ActorState,
    caller: &Caller,
    tool: &str,
    action: AuditAction,
    target: AuditTarget,
) -> Result<(), RanchError> {
    let decision = state
        .cfg
        .writes
        .authorize(&state.cfg.stamp(), state.lock_state, caller, tool)
        .await;
    if let Err(RanchError::ConnectionLocked { .. }) = &decision {
        let reason = Some("Connection locked".to_string());
        let recorded =
            write_audit_for_action(state, action, target, AuditOutcome::Failure, reason, caller)
                .await;
        if let Err(e) = recorded {
            tracing::warn!(error = %e, "lock-rejection audit write failed");
        }
    }
    decision
}

pub(super) async fn write_audit_for_action(
    state: &ActorState,
    action: AuditAction,
    target: AuditTarget,
    outcome: AuditOutcome,
    reason: Option<String>,
    caller: &Caller,
) -> Result<(), RanchError> {
    state
        .cfg
        .writes
        .record(&state.cfg.stamp(), caller, action, target, outcome, reason)
        .await
}
