//! `kv_settings` commands.
//!
//! Generic key/value getter/setter backed by the `kv_settings` SQLite table.
//! Used to persist `selected_connection_id` for cross-restart
//! restoration, and the same pair serves the onboarding-completed
//! flag, sidebar-collapse state, theme preference, etc.
//!
//! Design rules:
//! - Keys are namespaced flat strings (e.g. `selected_connection_id`,
//!   `onboarding.completed`). No nested structure on the key side.
//! - Values are opaque strings. Frontend serializes if needed.
//! - `get` returns `Ok(None)` when missing — first-launch is the common case
//!   and not an error.
//! - `set` is an UPSERT — single row per key, no history.

use crate::app_state::AppState;
use crate::core::errors::{RanchError, RanchErrorDto};
use sqlx::SqlitePool;

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_kv_setting(
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, RanchErrorDto> {
    get_kv_setting_inner(state.db.pool(), &key)
        .await
        .map_err(Into::into)
}

pub async fn get_kv_setting_inner(
    pool: &SqlitePool,
    key: &str,
) -> Result<Option<String>, RanchError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM kv_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_kv_setting(
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    set_kv_setting_inner(state.db.pool(), &key, &value)
        .await
        .map_err(Into::into)
}

pub async fn set_kv_setting_inner(
    pool: &SqlitePool,
    key: &str,
    value: &str,
) -> Result<(), RanchError> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT INTO kv_settings (key, value, updated_at)
        VALUES (?, ?, ?)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(key)
    .bind(value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}
