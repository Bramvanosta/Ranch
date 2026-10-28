//! `kv_settings` commands.
//!
//! Generic key/value getter/setter over the `SettingsStore` (the
//! `kv_settings` table).
//! Used to persist `selected_connection_id` for cross-restart
//! restoration, and the same pair serves the onboarding-completed
//! flag, sidebar-collapse state, theme preference, etc.
//!
//! Design rules:
//! - Keys are namespaced flat strings (e.g. `selected_connection_id`,
//!   `onboarding.completed`). No nested structure on the key side.
//! - Values are opaque strings. Frontend serializes if needed.
//! - `get` returns `Ok(None)` when missing; first-launch is the common case
//!   and not an error.
//! - `set` is an UPSERT: single row per key, no history.

use crate::app_state::AppState;
use crate::application::ports::SettingsStore;
use crate::contract::errors::RanchErrorDto;
use crate::domain::errors::RanchError;

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_kv_setting(
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, RanchErrorDto> {
    get_kv_setting_inner(state.settings.as_ref(), &key)
        .await
        .map_err(Into::into)
}

pub async fn get_kv_setting_inner(
    settings: &dyn SettingsStore,
    key: &str,
) -> Result<Option<String>, RanchError> {
    settings.get(key).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_kv_setting(
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    set_kv_setting_inner(state.settings.as_ref(), &key, &value)
        .await
        .map_err(Into::into)
}

/// A safety setting is sealed by the store as it is written; a seal key
/// is refused.
pub async fn set_kv_setting_inner(
    settings: &dyn SettingsStore,
    key: &str,
    value: &str,
) -> Result<(), RanchError> {
    settings.set(key, value).await
}
