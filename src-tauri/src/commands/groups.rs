//! Group commands.
//!
//! Groups are the top level of the sidebar hierarchy
//! (`groups → environments → connections`). `add_connection` auto-creates a
//! `"Default"` group; these commands make groups first-class: list (including empty ones), create, rename, delete
//! and collapse.
//!
//! All commands here are **UI-only**: there is no MCP twin. Organising the
//! sidebar is a human affordance, and `delete_group` with `cascade` removes
//! keychain entries, which an agent must never be able to trigger.

use crate::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use crate::app_state::AppState;
use crate::application::ports::ConnectionRepository;
use crate::application::ports::SecretStore;
use crate::commands::connections::remove_connection_inner;
use crate::contract::errors::RanchErrorDto;
use crate::domain::errors::RanchError;
use crate::domain::hierarchy::{normalize_name, HIERARCHY_NAME_MAX_CHARS};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

pub use crate::contract::hierarchy::Group;

/// Longest accepted group name, in characters (after trimming).
pub const GROUP_NAME_MAX_CHARS: usize = HIERARCHY_NAME_MAX_CHARS;

// ─────────────────────────────────────────────────────────────────────────────
// list_groups
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_groups(state: tauri::State<'_, AppState>) -> Result<Vec<Group>, RanchErrorDto> {
    list_groups_inner(state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_groups_inner(
    connections: &dyn ConnectionRepository,
) -> Result<Vec<Group>, RanchError> {
    let groups = connections.list_groups().await?;
    Ok(groups.into_iter().map(Into::into).collect())
}

async fn fetch_group_inner(
    id: &str,
    connections: &dyn ConnectionRepository,
) -> Result<Group, RanchError> {
    connections
        .list_groups()
        .await?
        .into_iter()
        .find(|g| g.id == id)
        .map(Into::into)
        .ok_or_else(|| RanchError::GroupNotFound { id: id.to_string() })
}

/// Trim + validate a group name. `exclude_id` lets `rename_group` keep its own
/// name (or change only its casing) without tripping the uniqueness check.
async fn validate_group_name(
    raw: &str,
    exclude_id: Option<&str>,
    connections: &dyn ConnectionRepository,
) -> Result<String, RanchError> {
    let name = normalize_name("Group", raw)?;
    let name = name.as_str();

    // Case-insensitive uniqueness. Compared in Rust rather than with SQLite's
    // `lower()`, which only folds ASCII.
    let existing = connections.list_groups().await?;
    let wanted = name.to_lowercase();
    let clash = existing
        .iter()
        .any(|g| Some(g.id.as_str()) != exclude_id && g.name.to_lowercase() == wanted);
    if clash {
        return Err(RanchError::InvalidInput {
            reason: format!("A group named \"{name}\" already exists"),
        });
    }

    Ok(name.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// create_group
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroupArgs {
    pub name: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn create_group(
    args: CreateGroupArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Group, RanchErrorDto> {
    create_group_inner(&args.name, state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn create_group_inner(
    name: &str,
    connections: &dyn ConnectionRepository,
) -> Result<Group, RanchError> {
    let name = validate_group_name(name, None, connections).await?;
    let id = connections.create_group(&name).await?;
    fetch_group_inner(&id, connections).await
}

// ─────────────────────────────────────────────────────────────────────────────
// rename_group
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameGroupArgs {
    pub id: String,
    pub name: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn rename_group(
    args: RenameGroupArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Group, RanchErrorDto> {
    rename_group_inner(&args.id, &args.name, state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn rename_group_inner(
    id: &str,
    name: &str,
    connections: &dyn ConnectionRepository,
) -> Result<Group, RanchError> {
    // Existence first so an unknown id reports GroupNotFound, not a name error.
    fetch_group_inner(id, connections).await?;
    let name = validate_group_name(name, Some(id), connections).await?;
    connections.rename_group(id, &name).await?;
    fetch_group_inner(id, connections).await
}

// ─────────────────────────────────────────────────────────────────────────────
// set_group_collapsed
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetGroupCollapsedArgs {
    pub id: String,
    pub collapsed: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_group_collapsed(
    args: SetGroupCollapsedArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    set_group_collapsed_inner(&args.id, args.collapsed, state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn set_group_collapsed_inner(
    id: &str,
    collapsed: bool,
    connections: &dyn ConnectionRepository,
) -> Result<(), RanchError> {
    connections.set_group_collapsed(id, collapsed).await
}

// ─────────────────────────────────────────────────────────────────────────────
// delete_group
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteGroupArgs {
    pub id: String,
    /// `false` → only an empty group may be deleted; a group that still holds
    /// connections is rejected with `GroupNotEmpty` so the UI can show the
    /// confirmation listing them. `true` → the confirmed cascade.
    pub cascade: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn delete_group(
    args: DeleteGroupArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    delete_group_inner(
        &args.id,
        args.cascade,
        state.repository.as_ref(),
        &state.keychain,
        Some(state.connections.as_ref()),
    )
    .await
    .map_err(Into::into)
}

pub async fn delete_group_inner(
    id: &str,
    cascade: bool,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
    registry: Option<&ConnectionRegistry>,
) -> Result<(), RanchError> {
    let group = fetch_group_inner(id, connections).await?;
    let connection_ids = connections.group_connection_ids(id).await?;

    if !connection_ids.is_empty() && !cascade {
        return Err(RanchError::GroupNotEmpty {
            name: group.name,
            connection_count: connection_ids.len() as u32,
        });
    }

    // Remove connections one by one through the same path as the
    // single-connection "Remove" action, so keychain cleanup (password, SSH
    // password, SSH host-key pin) and actor shutdown stay identical. A hard
    // keychain failure stops the cascade: what was removed stays removed, the
    // rest (and the group) stays in place for a retry. Never swallowed.
    for connection_id in &connection_ids {
        remove_connection_inner(connection_id, connections, keychain, registry).await?;
    }

    connections.delete_group(id).await
}
