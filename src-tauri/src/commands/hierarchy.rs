//! Hierarchy-scope commands.
//!
//! Operations on groups + environments that aren't tied to a specific
//! connection: reorder, the group / environment **selectors** that
//! `add_connection` and `update_connection` take, and
//! `change_environment_color`. Group CRUD lives in `commands::groups`.
//!
//! Reorder uses an UPSERT-style "set the full new order" contract — the
//! caller passes the complete `ordered_ids` list, the backend validates it
//! matches the current set, then updates `display_order` per index. This
//! avoids partial-update races when drag-and-drop produces a new ordering
//! and lets the frontend submit one mutation per drop instead of N.

use crate::app_state::AppState;
use crate::core::errors::{RanchError, RanchErrorDto};
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use std::collections::HashSet;

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
    reorder_groups_inner(&args.ordered_ids, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn reorder_groups_inner(
    ordered_ids: &[String],
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    let current: Vec<(String,)> = sqlx::query_as("SELECT id FROM groups")
        .fetch_all(pool)
        .await?;
    validate_full_set("group", &current, ordered_ids)?;

    let mut tx = pool.begin().await?;
    let now = chrono::Utc::now().to_rfc3339();
    for (idx, id) in ordered_ids.iter().enumerate() {
        sqlx::query("UPDATE groups SET display_order = ?, updated_at = ? WHERE id = ?")
            .bind(idx as i64)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReorderEnvironmentsArgs {
    pub group_id: String,
    pub ordered_ids: Vec<String>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn reorder_environments(
    args: ReorderEnvironmentsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    reorder_environments_inner(&args.group_id, &args.ordered_ids, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn reorder_environments_inner(
    group_id: &str,
    ordered_ids: &[String],
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    let current: Vec<(String,)> = sqlx::query_as("SELECT id FROM environments WHERE group_id = ?")
        .bind(group_id)
        .fetch_all(pool)
        .await?;
    validate_full_set("environment", &current, ordered_ids)?;

    let mut tx = pool.begin().await?;
    let now = chrono::Utc::now().to_rfc3339();
    for (idx, id) in ordered_ids.iter().enumerate() {
        sqlx::query("UPDATE environments SET display_order = ?, updated_at = ? WHERE id = ?")
            .bind(idx as i64)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Verify `ordered_ids` exactly matches the current set fetched from the DB
/// (no missing, no extra, no duplicates). Returns `InvalidInput` on any
/// mismatch so the frontend can show a clear error and re-fetch the truth.
fn validate_full_set(
    kind: &str,
    current: &[(String,)],
    ordered_ids: &[String],
) -> Result<(), RanchError> {
    let current_set: HashSet<&str> = current.iter().map(|(s,)| s.as_str()).collect();
    let provided_set: HashSet<&str> = ordered_ids.iter().map(|s| s.as_str()).collect();

    if provided_set.len() != ordered_ids.len() {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} ordered_ids contains duplicates"),
        });
    }
    if current_set != provided_set {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} ordered_ids does not match current set"),
        });
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Explicit group + environment selection
// ─────────────────────────────────────────────────────────────────────────────

/// Longest accepted group / environment name, in characters (after trimming).
pub const HIERARCHY_NAME_MAX_CHARS: usize = 64;

/// The curated palette — shared verbatim with `palette.ts`.
pub const ENV_COLOR_TOKENS: &[&str] = &[
    "env-production",
    "env-staging",
    "env-local",
    "env-custom-1",
    "env-custom-2",
    "env-custom-3",
    "env-custom-4",
    "env-custom-5",
    "env-custom-6",
    "env-custom-7",
];

/// The one color that triggers the production safety defaults.
pub const PRODUCTION_COLOR_TOKEN: &str = "env-production";

/// Trim + validate a group or environment name.
pub fn normalize_name(kind: &str, raw: &str) -> Result<String, RanchError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} name cannot be empty"),
        });
    }
    if name.chars().count() > HIERARCHY_NAME_MAX_CHARS {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} name cannot exceed {HIERARCHY_NAME_MAX_CHARS} characters"),
        });
    }
    Ok(name.to_string())
}

fn validate_color_token(token: &str) -> Result<(), RanchError> {
    if ENV_COLOR_TOKENS.contains(&token) {
        Ok(())
    } else {
        Err(RanchError::InvalidInput {
            reason: format!("Unknown environment color: {token}"),
        })
    }
}

/// Which group a connection goes into. `New` is find-or-create: a name that
/// already exists (case-insensitively) resolves to that group, so a retried
/// save can never produce two "Lumen" groups.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GroupSelector {
    Existing { id: String },
    New { name: String },
}

impl GroupSelector {
    pub fn named(name: &str) -> Self {
        Self::New {
            name: name.to_string(),
        }
    }
}

/// Which environment a connection goes into. `New` is find-or-create by label
/// within the group; when the label already exists, the existing environment
/// — and its color — wins. The color is an attribute of the environment and
/// is **never** used to pick one.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EnvironmentSelector {
    Existing {
        id: String,
    },
    #[serde(rename_all = "camelCase")]
    New {
        name: String,
        color_token: String,
    },
}

impl EnvironmentSelector {
    pub fn named(name: &str, color_token: &str) -> Self {
        Self::New {
            name: name.to_string(),
            color_token: color_token.to_string(),
        }
    }

    /// A new environment carrying the palette's default name for `color_token`
    /// ("Production", "Staging", "Local", "Custom 3"…) — what the form
    /// pre-fills when the user picks a swatch before typing a name.
    pub fn for_color(color_token: &str) -> Self {
        Self::named(&default_env_name_for_color(color_token), color_token)
    }
}

/// Palette default names — mirrors `ENV_COLOR_PALETTE` in `palette.ts`.
pub fn default_env_name_for_color(color_token: &str) -> String {
    match color_token {
        "env-production" => "Production".to_string(),
        "env-staging" => "Staging".to_string(),
        "env-local" => "Local".to_string(),
        other => match other.strip_prefix("env-custom-") {
            Some(n) => format!("Custom {n}"),
            None => "Environment".to_string(),
        },
    }
}

/// Where a connection ended up after selector resolution.
#[derive(Debug, Clone)]
pub struct ResolvedHierarchy {
    pub group_id: String,
    pub group_name: String,
    pub environment_id: String,
    pub env_name: String,
    pub env_label: String,
    pub env_color: String,
}

impl ResolvedHierarchy {
    pub fn is_production(&self) -> bool {
        self.env_color == PRODUCTION_COLOR_TOKEN
    }
}

/// Resolve (creating what is `New`) the group and environment for a
/// connection, inside the caller's transaction so a later failure — keychain
/// included — rolls the new group / environment back too.
pub async fn resolve_hierarchy(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    group: &GroupSelector,
    environment: &EnvironmentSelector,
    now: &str,
) -> Result<ResolvedHierarchy, RanchError> {
    // ── group ────────────────────────────────────────────────────────────
    let (group_id, group_name, group_is_new) = match group {
        GroupSelector::Existing { id } => {
            let row: Option<(String,)> = sqlx::query_as("SELECT name FROM groups WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut **tx)
                .await?;
            let (name,) = row.ok_or_else(|| RanchError::GroupNotFound { id: id.clone() })?;
            (id.clone(), name, false)
        }
        GroupSelector::New { name } => {
            let name = normalize_name("Group", name)?;
            let wanted = name.to_lowercase();
            let existing: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM groups")
                .fetch_all(&mut **tx)
                .await?;
            match existing
                .into_iter()
                .find(|(_, other)| other.to_lowercase() == wanted)
            {
                Some((id, existing_name)) => (id, existing_name, false),
                None => {
                    let id = uuid::Uuid::now_v7().to_string();
                    let (next_order,): (i64,) =
                        sqlx::query_as("SELECT COALESCE(MAX(display_order) + 1, 0) FROM groups")
                            .fetch_one(&mut **tx)
                            .await?;
                    sqlx::query(
                        r#"
                        INSERT INTO groups
                            (id, name, label, display_order, collapsed, created_at, updated_at)
                        VALUES (?, ?, 'Client', ?, 0, ?, ?)
                        "#,
                    )
                    .bind(&id)
                    .bind(&name)
                    .bind(next_order)
                    .bind(now)
                    .bind(now)
                    .execute(&mut **tx)
                    .await?;
                    (id, name, true)
                }
            }
        }
    };

    // ── environment ──────────────────────────────────────────────────────
    let (environment_id, env_name, env_label, env_color) = match environment {
        EnvironmentSelector::Existing { id } => {
            let row: Option<(String, String, String, String)> = sqlx::query_as(
                "SELECT group_id, name, label, color_token FROM environments WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
            let (env_group_id, name, label, color) =
                row.ok_or_else(|| RanchError::EnvironmentNotFound { id: id.clone() })?;
            if group_is_new || env_group_id != group_id {
                return Err(RanchError::InvalidInput {
                    reason: "Environment does not belong to the selected group".to_string(),
                });
            }
            (id.clone(), name, label, color)
        }
        EnvironmentSelector::New { name, color_token } => {
            let name = normalize_name("Environment", name)?;
            validate_color_token(color_token)?;
            let wanted = name.to_lowercase();
            let existing: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT id, name, label, color_token FROM environments WHERE group_id = ?",
            )
            .bind(&group_id)
            .fetch_all(&mut **tx)
            .await?;
            match existing
                .into_iter()
                .find(|(_, _, label, _)| label.to_lowercase() == wanted)
            {
                Some(found) => found,
                None => {
                    let id = uuid::Uuid::now_v7().to_string();
                    let (next_order,): (i64,) = sqlx::query_as(
                        "SELECT COALESCE(MAX(display_order) + 1, 0) FROM environments WHERE group_id = ?",
                    )
                    .bind(&group_id)
                    .fetch_one(&mut **tx)
                    .await?;
                    sqlx::query(
                        r#"
                        INSERT INTO environments
                            (id, group_id, name, label, color_token, display_order, created_at, updated_at)
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                        "#,
                    )
                    .bind(&id)
                    .bind(&group_id)
                    .bind(&name)
                    .bind(&name)
                    .bind(color_token)
                    .bind(next_order)
                    .bind(now)
                    .bind(now)
                    .execute(&mut **tx)
                    .await?;
                    (id, name.clone(), name, color_token.clone())
                }
            }
        }
    };

    Ok(ResolvedHierarchy {
        group_id,
        group_name,
        environment_id,
        env_name,
        env_label,
        env_color,
    })
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
/// the connections whose lock flag changed so the caller can refresh their
/// live actors.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn change_environment_color(
    args: ChangeEnvironmentColorArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    let newly_locked =
        change_environment_color_inner(&args.environment_id, &args.color_token, state.db.pool())
            .await
            .map_err(RanchErrorDto::from)?;
    // The actor caches its lock state at spawn; respawn so the lock applies now.
    for connection_id in newly_locked {
        state.connections.shutdown(&connection_id).await;
    }
    Ok(())
}

pub async fn change_environment_color_inner(
    environment_id: &str,
    color_token: &str,
    pool: &SqlitePool,
) -> Result<Vec<String>, RanchError> {
    validate_color_token(color_token)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;

    let result =
        sqlx::query("UPDATE environments SET color_token = ?, updated_at = ? WHERE id = ?")
            .bind(color_token)
            .bind(&now)
            .bind(environment_id)
            .execute(&mut *tx)
            .await?;
    if result.rows_affected() == 0 {
        return Err(RanchError::EnvironmentNotFound {
            id: environment_id.to_string(),
        });
    }

    let mut newly_locked = Vec::new();
    if color_token == PRODUCTION_COLOR_TOKEN {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM connections WHERE environment_id = ? AND is_locked = 0")
                .bind(environment_id)
                .fetch_all(&mut *tx)
                .await?;
        newly_locked = rows.into_iter().map(|(id,)| id).collect();
        // And read-only for agents: the production defaults go together.
        sqlx::query(
            "UPDATE connections SET is_locked = 1, mcp_readonly = 1, updated_at = ? WHERE environment_id = ?",
        )
        .bind(&now)
        .bind(environment_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(newly_locked)
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
        state.db.pool(),
    )
    .await
    .map_err(Into::into)
}

/// Returns the new environment's id.
pub async fn create_environment_inner(
    group_id: &str,
    name: &str,
    color_token: &str,
    pool: &SqlitePool,
) -> Result<String, RanchError> {
    let name = normalize_name("Environment", name)?;
    validate_color_token(color_token)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;

    let group: Option<(String,)> = sqlx::query_as("SELECT id FROM groups WHERE id = ?")
        .bind(group_id)
        .fetch_optional(&mut *tx)
        .await?;
    if group.is_none() {
        return Err(RanchError::GroupNotFound {
            id: group_id.to_string(),
        });
    }

    let labels: Vec<(String,)> =
        sqlx::query_as("SELECT label FROM environments WHERE group_id = ?")
            .bind(group_id)
            .fetch_all(&mut *tx)
            .await?;
    let wanted = name.to_lowercase();
    if labels.iter().any(|(l,)| l.to_lowercase() == wanted) {
        return Err(RanchError::InvalidInput {
            reason: format!("An environment named \"{name}\" already exists in this group"),
        });
    }

    let resolved = resolve_hierarchy(
        &mut tx,
        &GroupSelector::Existing {
            id: group_id.to_string(),
        },
        &EnvironmentSelector::named(&name, color_token),
        &now,
    )
    .await?;
    tx.commit().await?;
    Ok(resolved.environment_id)
}

/// Delete an environment — only when it holds no connection (emptied
/// environments are kept around as drop targets; this is how they go away).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn delete_environment(
    environment_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    delete_environment_inner(&environment_id, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn delete_environment_inner(
    environment_id: &str,
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    let mut tx = pool.begin().await?;
    let row: Option<(String,)> = sqlx::query_as("SELECT label FROM environments WHERE id = ?")
        .bind(environment_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (label,) = row.ok_or_else(|| RanchError::EnvironmentNotFound {
        id: environment_id.to_string(),
    })?;

    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM connections WHERE environment_id = ?")
            .bind(environment_id)
            .fetch_one(&mut *tx)
            .await?;
    if count > 0 {
        return Err(RanchError::InvalidInput {
            reason: format!(
                "Environment \"{label}\" still contains {count} connection(s) — move or remove them first"
            ),
        });
    }

    sqlx::query("DELETE FROM environments WHERE id = ?")
        .bind(environment_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MoveConnectionArgs {
    pub connection_id: String,
    pub environment_id: String,
}

#[derive(Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MoveConnectionResult {
    /// `true` when the move took the connection into a Production-colored
    /// environment and it was not locked yet — the UI tells the user.
    pub newly_locked: bool,
}

/// Move a connection to another environment (any group). The connection id,
/// its credentials and its live actor are untouched — except when the move
/// enters Production: the connection is locked (the safe direction)
/// and its actor respawned so the lock applies immediately.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn move_connection(
    args: MoveConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<MoveConnectionResult, RanchErrorDto> {
    let result = move_connection_inner(&args.connection_id, &args.environment_id, state.db.pool())
        .await
        .map_err(RanchErrorDto::from)?;
    if result.newly_locked {
        state.connections.shutdown(&args.connection_id).await;
    }
    Ok(result)
}

pub async fn move_connection_inner(
    connection_id: &str,
    environment_id: &str,
    pool: &SqlitePool,
) -> Result<MoveConnectionResult, RanchError> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;

    let current: Option<(String, i64)> =
        sqlx::query_as("SELECT environment_id, is_locked FROM connections WHERE id = ?")
            .bind(connection_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (current_env_id, is_locked) = current.ok_or_else(|| RanchError::ConnectionNotFound {
        id: connection_id.to_string(),
    })?;

    let target: Option<(String,)> =
        sqlx::query_as("SELECT color_token FROM environments WHERE id = ?")
            .bind(environment_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (target_color,) = target.ok_or_else(|| RanchError::EnvironmentNotFound {
        id: environment_id.to_string(),
    })?;

    if current_env_id == environment_id {
        return Ok(MoveConnectionResult {
            newly_locked: false,
        });
    }

    let entering_production = target_color == PRODUCTION_COLOR_TOKEN;
    let newly_locked = entering_production && is_locked == 0;
    // Entering Production also makes the connection read-only for
    // agents; elsewhere the existing MCP setting is kept.
    sqlx::query(
        r#"
        UPDATE connections
           SET environment_id = ?, is_locked = ?,
               mcp_readonly = CASE WHEN ? THEN 1 ELSE mcp_readonly END,
               updated_at = ?
         WHERE id = ?
        "#,
    )
    .bind(environment_id)
    .bind(if newly_locked { 1 } else { is_locked })
    .bind(entering_production)
    .bind(&now)
    .bind(connection_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(MoveConnectionResult { newly_locked })
}
