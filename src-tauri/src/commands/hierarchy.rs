//! Hierarchy-scope commands.
//!
//! Operations on groups + environments that aren't tied to a specific
//! connection: reorder, the group / environment **selectors** that
//! `add_connection` and `update_connection` take, and
//! `change_environment_color`. Group CRUD lives in `commands::groups`.
//!
//! Reorder uses an UPSERT-style "set the full new order" contract: the
//! caller passes the complete `ordered_ids` list, the backend validates it
//! matches the current set, then updates `display_order` per index. This
//! avoids partial-update races when drag-and-drop produces a new ordering
//! and lets the frontend submit one mutation per drop instead of N.

use crate::app_state::AppState;
use crate::application::ports::ConnectionRepository;
use crate::contract::errors::RanchErrorDto;
use crate::domain::errors::RanchError;
use crate::domain::hierarchy::{normalize_name, validate_color_token};
use serde::{Deserialize, Serialize};
use specta::Type;

pub use crate::contract::hierarchy::{EnvironmentSelector, GroupSelector, MoveConnectionResult};
pub use crate::domain::hierarchy::default_env_name_for_color;

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReorderGroupsArgs {
    pub ordered_ids: Vec<String>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn reorder_groups(
    args: ReorderGroupsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    reorder_groups_inner(&args.ordered_ids, state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn reorder_groups_inner(
    ordered_ids: &[String],
    connections: &dyn ConnectionRepository,
) -> Result<(), RanchError> {
    connections.reorder_groups(ordered_ids).await
}

// ─────────────────────────────────────────────────────────────────────────────
// change_environment_color
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeEnvironmentColorArgs {
    pub environment_id: String,
    pub color_token: String,
}

/// Recolor an environment. The color is a plain attribute of the environment,
/// shared by every connection in it.
///
/// Turning an environment Production-colored locks its connections (the safe
/// direction). Leaving Production never unlocks anything. Returns the ids of
/// every connection in the environment, so the caller refreshes their live
/// actors: each keeps the lock and the color it was spawned with, and the
/// audit log takes the color from it.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn change_environment_color(
    args: ChangeEnvironmentColorArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    let stale = change_environment_color_inner(
        &args.environment_id,
        &args.color_token,
        state.repository.as_ref(),
    )
    .await
    .map_err(RanchErrorDto::from)?;
    // The actor caches its lock state and color at spawn; respawn so they
    // apply now.
    for connection_id in stale {
        state.connections.shutdown(&connection_id).await;
    }
    Ok(())
}

pub async fn change_environment_color_inner(
    environment_id: &str,
    color_token: &str,
    connections: &dyn ConnectionRepository,
) -> Result<Vec<String>, RanchError> {
    validate_color_token(color_token)?;
    connections
        .recolor_environment(environment_id, color_token)
        .await?;
    Ok(connections
        .list()
        .await?
        .into_iter()
        .filter(|c| c.environment_id == environment_id)
        .map(|c| c.id)
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Environments as manageable entities + moving connections
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateEnvironmentArgs {
    pub group_id: String,
    pub name: String,
    pub color_token: String,
}

/// Create an empty environment in a group ("New environment" in the sidebar),
/// so a connection can then be moved or added into it. Unlike the
/// find-or-create `EnvironmentSelector::New` used by the connection form, an
/// explicit create **rejects** a label that already exists in the group.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn create_environment(
    args: CreateEnvironmentArgs,
    state: tauri::State<'_, AppState>,
) -> Result<String, RanchErrorDto> {
    create_environment_inner(
        &args.group_id,
        &args.name,
        &args.color_token,
        state.repository.as_ref(),
    )
    .await
    .map_err(Into::into)
}

/// Returns the new environment's id.
pub async fn create_environment_inner(
    group_id: &str,
    name: &str,
    color_token: &str,
    connections: &dyn ConnectionRepository,
) -> Result<String, RanchError> {
    let name = normalize_name("Environment", name)?;
    validate_color_token(color_token)?;
    connections
        .create_environment(group_id, &name, color_token)
        .await
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MoveConnectionArgs {
    pub connection_id: String,
    pub environment_id: String,
}

/// Move a connection to another environment (any group). The connection id
/// and its credentials are untouched; entering Production locks it (the safe
/// direction). Its actor is respawned, so the lock and the new environment's
/// color apply at once.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn move_connection(
    args: MoveConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<MoveConnectionResult, RanchErrorDto> {
    let result = move_connection_inner(
        &args.connection_id,
        &args.environment_id,
        state.repository.as_ref(),
    )
    .await
    .map_err(RanchErrorDto::from)?;
    // The actor keeps the color (for the audit log) and the lock it was
    // spawned with: respawn it in its new environment.
    state.connections.shutdown(&args.connection_id).await;
    Ok(result)
}

pub async fn move_connection_inner(
    connection_id: &str,
    environment_id: &str,
    connections: &dyn ConnectionRepository,
) -> Result<MoveConnectionResult, RanchError> {
    connections
        .move_connection(connection_id, environment_id)
        .await
        .map(Into::into)
}
