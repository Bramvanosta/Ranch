//! Resolution notes: the UI side of `core::cause_notes`.
//! Agents write notes through the MCP tool `annotate_cause`; Ranch lists
//! them next to their cause and in Agents › Notes, and dismisses them.

#[cfg(feature = "desktop")]
use crate::app_state::AppState;
#[cfg(feature = "desktop")]
use crate::core::cause_notes;

pub use crate::contract::cause_notes::{AnnotateCauseArgs, CauseNote, ListCauseNotesArgs};
#[cfg(feature = "desktop")]
use crate::contract::errors::RanchErrorDto;

/// A note written in Ranch itself (no agent client). The MCP tool of the
/// same name is how agents write theirs (parity: UI ⊇ MCP).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn annotate_cause(
    args: AnnotateCauseArgs,
    state: tauri::State<'_, AppState>,
) -> Result<CauseNote, RanchErrorDto> {
    cause_notes::annotate(state.db.pool(), args.into(), None)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_cause_notes(
    args: ListCauseNotesArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CauseNote>, RanchErrorDto> {
    cause_notes::list(state.db.pool(), args.into())
        .await
        .map(|notes| notes.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

/// UI-only and MCP-forbidden: an agent must not hide notes.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn dismiss_cause_note(
    note_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    cause_notes::dismiss(state.db.pool(), &note_id)
        .await
        .map_err(Into::into)
}
