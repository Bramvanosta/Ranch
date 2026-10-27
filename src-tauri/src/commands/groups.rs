//! Group commands.
//!
//! Groups are the top level of the sidebar hierarchy
//! (`groups → environments → connections`). `add_connection` auto-creates a
//! `"Default"` group; these commands make groups first-class: list (including empty ones), create, rename, delete
//! and collapse.
//!
//! All commands here are **UI-only** — there is no MCP twin. Organising the
//! sidebar is a human affordance, and `delete_group` with `cascade` removes
//! keychain entries, which an agent must never be able to trigger.

use crate::app_state::AppState;
use crate::commands::connections::remove_connection_inner;
use crate::commands::hierarchy::{normalize_name, HIERARCHY_NAME_MAX_CHARS};
use crate::core::connection_actor::ConnectionRegistry;
use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::keychain::KeychainAdapter;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use std::sync::Arc;

/// Longest accepted group name, in characters (after trimming).
pub const GROUP_NAME_MAX_CHARS: usize = HIERARCHY_NAME_MAX_CHARS;

#[derive(Debug, Serialize, Deserialize, Type, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub display_order: i32,
    pub collapsed: bool,
    /// Number of connections under this group (across all its environments).
    /// Lets the sidebar decide between "delete immediately" and "confirm the
    /// cascade" without a second round-trip.
    pub connection_count: i32,
}

// ─────────────────────────────────────────────────────────────────────────────
// list_groups
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_groups(state: tauri::State<'_, AppState>) -> Result<Vec<Group>, RanchErrorDto> {
    list_groups_inner(state.db.pool()).await.map_err(Into::into)
}

pub async fn list_groups_inner(pool: &SqlitePool) -> Result<Vec<Group>, RanchError> {
    let rows: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT g.id, g.name, g.display_order, g.collapsed,
               (SELECT count(*)
                  FROM connections c
                  JOIN environments e ON c.environment_id = e.id
                 WHERE e.group_id = g.id) AS connection_count
        FROM groups g
        ORDER BY g.display_order, g.created_at
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, name, display_order, collapsed, connection_count)| Group {
                id,
                name,
                display_order: display_order as i32,
                collapsed: collapsed != 0,
                connection_count: connection_count as i32,
            },
        )
        .collect())
}

async fn fetch_group_inner(id: &str, pool: &SqlitePool) -> Result<Group, RanchError> {
    list_groups_inner(pool)
        .await?
        .into_iter()
        .find(|g| g.id == id)
        .ok_or_else(|| RanchError::GroupNotFound { id: id.to_string() })
}

/// Trim + validate a group name. `exclude_id` lets `rename_group` keep its own
/// name (or change only its casing) without tripping the uniqueness check.
async fn validate_group_name(
    raw: &str,
    exclude_id: Option<&str>,
    pool: &SqlitePool,
) -> Result<String, RanchError> {
    let name = normalize_name("Group", raw)?;
    let name = name.as_str();

    // Case-insensitive uniqueness. Compared in Rust rather than with SQLite's
    // `lower()`, which only folds ASCII.
    let existing: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM groups")
        .fetch_all(pool)
        .await?;
    let wanted = name.to_lowercase();
    let clash = existing
        .iter()
        .any(|(id, other)| Some(id.as_str()) != exclude_id && other.to_lowercase() == wanted);
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
    create_group_inner(&args.name, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn create_group_inner(name: &str, pool: &SqlitePool) -> Result<Group, RanchError> {
    let name = validate_group_name(name, None, pool).await?;
    let id = uuid::Uuid::now_v7().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Append at the end of the current order.
    let (next_order,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(display_order) + 1, 0) FROM groups")
            .fetch_one(pool)
            .await?;

    sqlx::query(
        r#"
        INSERT INTO groups (id, name, label, display_order, collapsed, created_at, updated_at)
        VALUES (?, ?, 'Client', ?, 0, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&name)
    .bind(next_order)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    fetch_group_inner(&id, pool).await
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
    rename_group_inner(&args.id, &args.name, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn rename_group_inner(
    id: &str,
    name: &str,
    pool: &SqlitePool,
) -> Result<Group, RanchError> {
    // Existence first so an unknown id reports GroupNotFound, not a name error.
    fetch_group_inner(id, pool).await?;
    let name = validate_group_name(name, Some(id), pool).await?;
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query("UPDATE groups SET name = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;

    fetch_group_inner(id, pool).await
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
    set_group_collapsed_inner(&args.id, args.collapsed, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn set_group_collapsed_inner(
    id: &str,
    collapsed: bool,
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    let result = sqlx::query("UPDATE groups SET collapsed = ? WHERE id = ?")
        .bind(if collapsed { 1_i64 } else { 0_i64 })
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(RanchError::GroupNotFound { id: id.to_string() });
    }
    Ok(())
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
        state.db.pool(),
        &state.keychain,
        Some(state.connections.as_ref()),
    )
    .await
    .map_err(Into::into)
}

pub async fn delete_group_inner(
    id: &str,
    cascade: bool,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: Option<&ConnectionRegistry>,
) -> Result<(), RanchError> {
    let group = fetch_group_inner(id, pool).await?;

    let connection_ids: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT c.id
        FROM connections c
        JOIN environments e ON c.environment_id = e.id
        WHERE e.group_id = ?
        ORDER BY c.created_at
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

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
    // rest — and the group — stays in place for a retry. Never swallowed
    //.
    for (connection_id,) in &connection_ids {
        remove_connection_inner(connection_id, pool, keychain, registry).await?;
    }

    // Environments go with the group via ON DELETE CASCADE.
    sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}
