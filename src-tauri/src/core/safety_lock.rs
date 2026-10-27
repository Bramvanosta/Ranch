//! per-connection read-only lock state + enforcement helper.
//!
//! The connection actor (`core::connection_actor`) owns the live
//! [`ReadOnlyState`] for each connection. This module just exposes the
//! type + the single enforcement helper [`check_lock_or_err`].
//!
//! ## Architectural invariant
//!
//! **`check_lock_or_err` is the single enforcement point for the lock.**
//!
//! Every destructive actor handler (`handle_retry_job`,
//! `handle_remove_job`, `handle_promote_job`, `handle_pause_queue`, ...)
//! MUST call this helper as its FIRST line. If the helper returns
//! `Err(ConnectionLocked)` the handler returns the error WITHOUT
//! touching Redis. The actor's mailbox is the only place destructive
//! ops route through, which means UI clicks AND MCP tools
//! both honor the same lock by construction.
//!
//! The toggle commands (`unlock_connection` / `lock_connection`) themselves
//! do NOT call `check_lock_or_err` — they only flip the in-memory state,
//! they don't touch Redis.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::errors::RanchError;

/// Keystroke hint surfaced to the user when a destructive op is rejected.
/// Single-source-of-truth so both the Rust error variant + the frontend
/// banner share the same string. The frontend conditions the prefix
/// (`cmd-shift-U` vs `ctrl-shift-U`) per platform.
pub const UNLOCK_HINT: &str =
    "Press cmd-shift-U (or ctrl-shift-U on Linux/Windows) to unlock for this session.";

/// Per-connection read-only state.
///
/// Initialized from the `connections.is_locked` column at actor-spawn
/// time. In-memory authoritative once the actor is alive — restart
/// re-spawns the actor and re-reads `is_locked`, so unlocks do NOT
/// leak across launches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReadOnlyState {
    Locked,
    Unlocked,
}

/// The single enforcement point for the connection lock. Every
/// destructive actor handler MUST call this as its first
/// line. Returns `Err(ConnectionLocked)` if the state is `Locked`;
/// returns `Ok(())` otherwise.
///
/// The error carries the connection's id, display name, and the
/// [`UNLOCK_HINT`] so the frontend can render an inline call-to-action
/// without an additional roundtrip.
pub fn check_lock_or_err(
    state: ReadOnlyState,
    connection_id: &str,
    connection_name: &str,
) -> Result<(), RanchError> {
    match state {
        ReadOnlyState::Unlocked => Ok(()),
        ReadOnlyState::Locked => Err(RanchError::ConnectionLocked {
            connection_id: connection_id.to_string(),
            connection_name: connection_name.to_string(),
            unlock_hint: UNLOCK_HINT.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlocked_state_passes_check() {
        let r = check_lock_or_err(ReadOnlyState::Unlocked, "conn-x", "Acme · prod");
        assert!(r.is_ok());
    }

    #[test]
    fn locked_state_returns_typed_error_with_identity_fields() {
        let err = check_lock_or_err(ReadOnlyState::Locked, "conn-x", "Acme · prod").unwrap_err();
        match err {
            RanchError::ConnectionLocked {
                connection_id,
                connection_name,
                unlock_hint,
            } => {
                assert_eq!(connection_id, "conn-x");
                assert_eq!(connection_name, "Acme · prod");
                assert!(unlock_hint.contains("cmd-shift-U"));
                assert!(unlock_hint.contains("ctrl-shift-U"));
            }
            other => panic!("expected ConnectionLocked, got {other:?}"),
        }
    }

    #[test]
    fn read_only_state_round_trips_through_camel_case() {
        // `Locked` / `Unlocked` serialize lowercase first letter via
        // serde's camelCase rename rule. Confirm the wire shape matches
        // what the frontend will see.
        let locked = serde_json::to_string(&ReadOnlyState::Locked).unwrap();
        assert_eq!(locked, "\"locked\"");
        let unlocked = serde_json::to_string(&ReadOnlyState::Unlocked).unwrap();
        assert_eq!(unlocked, "\"unlocked\"");

        let back: ReadOnlyState = serde_json::from_str("\"locked\"").unwrap();
        assert_eq!(back, ReadOnlyState::Locked);
        let back: ReadOnlyState = serde_json::from_str("\"unlocked\"").unwrap();
        assert_eq!(back, ReadOnlyState::Unlocked);
    }

    #[test]
    fn state_is_copy_so_callers_can_pass_by_value() {
        // Compile-time guarantee: `ReadOnlyState` is `Copy` so callers
        // can pass it into `check_lock_or_err` without consuming the
        // actor's stored state.
        fn assert_copy<T: Copy>() {}
        assert_copy::<ReadOnlyState>();
    }
}
