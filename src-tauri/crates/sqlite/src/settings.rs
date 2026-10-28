//! The settings, in `ranch.db`'s `kv_settings`, with the safety settings'
//! seals beside them (`seal:<key>` rows).

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;

use ranch_application::ports::sealer::{setting_parts, Sealer};
use ranch_application::ports::settings::{SafetySetting, Sealed, SettingsStore};
use ranch_application::ports::Clock;
use ranch_domain::errors::RanchError;

/// The `kv_settings` key holding the seal of a safety setting.
fn seal_key(setting: SafetySetting) -> String {
    format!("seal:{}", setting.key())
}

pub struct SqliteSettingsStore {
    pool: SqlitePool,
    sealer: Arc<dyn Sealer>,
    clock: Arc<dyn Clock>,
}

impl SqliteSettingsStore {
    pub fn new(pool: SqlitePool, sealer: Arc<dyn Sealer>, clock: Arc<dyn Clock>) -> Self {
        Self {
            pool,
            sealer,
            clock,
        }
    }

    /// Seals are written by this store alone.
    fn refuse_seal_keys(key: &str) -> Result<(), RanchError> {
        if key.starts_with("seal:") {
            return Err(RanchError::InvalidInput {
                reason: format!("{key} is not a setting"),
            });
        }
        Ok(())
    }

    /// Seal a safety setting as it now stands (no-op for other keys).
    async fn reseal(&self, key: &str) -> Result<(), RanchError> {
        let Some(setting) = SafetySetting::of_key(key) else {
            return Ok(());
        };
        let value: Option<(String,)> =
            sqlx::query_as("SELECT value FROM kv_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(RanchError::database)?;
        let Some(sealed) = self
            .sealer
            .seal(&setting_parts(key, value.as_ref().map(|(v,)| v.as_str())))
        else {
            return Ok(());
        };
        sqlx::query(
            "INSERT INTO kv_settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(seal_key(setting))
        .bind(sealed)
        .bind(self.clock.now().to_rfc3339())
        .execute(&self.pool)
        .await.map_err(RanchError::database)?;
        Ok(())
    }
}

#[async_trait]
impl SettingsStore for SqliteSettingsStore {
    async fn get(&self, key: &str) -> Result<Option<String>, RanchError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM kv_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(RanchError::database)?;
        Ok(row.map(|(v,)| v))
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), RanchError> {
        Self::refuse_seal_keys(key)?;
        let now = self.clock.now().to_rfc3339();
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
        .execute(&self.pool)
        .await
        .map_err(RanchError::database)?;
        self.reseal(key).await
    }

    async fn remove(&self, key: &str) -> Result<(), RanchError> {
        Self::refuse_seal_keys(key)?;
        sqlx::query("DELETE FROM kv_settings WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;
        self.reseal(key).await
    }

    async fn safety(&self, setting: SafetySetting) -> Result<Sealed, RanchError> {
        let key = setting.key();
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM kv_settings WHERE key IN (?, ?)")
                .bind(key)
                .bind(seal_key(setting))
                .fetch_all(&self.pool)
                .await
                .map_err(RanchError::database)?;
        let value = rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        let sealed = rows
            .iter()
            .find(|(k, _)| *k == seal_key(setting))
            .map(|(_, v)| v.as_str());
        let check = self
            .sealer
            .check(&setting_parts(key, value.as_deref()), sealed);
        Ok(Sealed { value, check })
    }

    async fn seal_all(&self) -> Result<(), RanchError> {
        for setting in SafetySetting::ALL {
            self.reseal(setting.key()).await?;
        }
        Ok(())
    }
}
