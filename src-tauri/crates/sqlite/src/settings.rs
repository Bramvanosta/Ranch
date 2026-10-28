//! The settings, in `ranch.db`'s `kv_settings`, with the safety settings'
//! seals beside them (`seal:<key>` rows).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::db::begin_immediate;
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
    /// Safety settings this process wrote while it had no key: sealed with
    /// the next one sealed once the key is back.
    unsealed: Mutex<BTreeSet<&'static str>>,
}

impl SqliteSettingsStore {
    pub fn new(pool: SqlitePool, sealer: Arc<dyn Sealer>, clock: Arc<dyn Clock>) -> Self {
        Self {
            pool,
            sealer,
            clock,
            unsealed: Mutex::new(BTreeSet::new()),
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

    /// Seal a safety setting as it now stands (no-op for other keys), on
    /// the connection that wrote it: the value and its seal are committed
    /// together.
    async fn reseal(&self, conn: &mut sqlx::SqliteConnection, key: &str) -> Result<(), RanchError> {
        let Some(setting) = SafetySetting::of_key(key) else {
            return Ok(());
        };
        if !self.seal_setting(conn, setting).await? {
            self.unsealed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(setting.key());
            return Ok(());
        }
        let earlier = std::mem::take(&mut *self.unsealed.lock().unwrap_or_else(|e| e.into_inner()));
        for other in earlier.into_iter().filter(|other| *other != setting.key()) {
            if let Some(other) = SafetySetting::of_key(other) {
                self.seal_setting(conn, other).await?;
            }
        }
        Ok(())
    }

    /// `false` when there is no key to seal with.
    async fn seal_setting(
        &self,
        conn: &mut sqlx::SqliteConnection,
        setting: SafetySetting,
    ) -> Result<bool, RanchError> {
        let key = setting.key();
        let value: Option<(String,)> =
            sqlx::query_as("SELECT value FROM kv_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&mut *conn)
                .await
                .map_err(RanchError::database)?;
        let Some(sealed) = self
            .sealer
            .seal(&setting_parts(key, value.as_ref().map(|(v,)| v.as_str())))
        else {
            return Ok(false);
        };
        sqlx::query(
            "INSERT INTO kv_settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(seal_key(setting))
        .bind(sealed)
        .bind(self.clock.now().to_rfc3339())
        .execute(&mut *conn)
        .await
        .map_err(RanchError::database)?;
        Ok(true)
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
        let mut tx = begin_immediate(&self.pool).await?;
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
        .execute(&mut *tx)
        .await
        .map_err(RanchError::database)?;
        self.reseal(&mut tx, key).await?;
        tx.commit().await.map_err(RanchError::database)
    }

    async fn remove(&self, key: &str) -> Result<(), RanchError> {
        Self::refuse_seal_keys(key)?;
        let mut tx = begin_immediate(&self.pool).await?;
        sqlx::query("DELETE FROM kv_settings WHERE key = ?")
            .bind(key)
            .execute(&mut *tx)
            .await
            .map_err(RanchError::database)?;
        self.reseal(&mut tx, key).await?;
        tx.commit().await.map_err(RanchError::database)
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
        let mut tx = begin_immediate(&self.pool).await?;
        for setting in SafetySetting::ALL {
            self.reseal(&mut tx, setting.key()).await?;
        }
        tx.commit().await.map_err(RanchError::database)
    }
}
