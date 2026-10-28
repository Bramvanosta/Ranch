//! The settings, in `ranch.db`'s `kv_settings`, with the safety settings'
//! seals beside them (`seal:<key>` rows).

use std::collections::BTreeMap;
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
    /// Safety settings this process wrote while it had no key, with the
    /// value written: sealed with the next one sealed once the key is back,
    /// if they still hold that value.
    unsealed: Mutex<BTreeMap<&'static str, Option<String>>>,
}

impl SqliteSettingsStore {
    pub fn new(pool: SqlitePool, sealer: Arc<dyn Sealer>, clock: Arc<dyn Clock>) -> Self {
        Self {
            pool,
            sealer,
            clock,
            unsealed: Mutex::new(BTreeMap::new()),
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
    /// together. Without a key, the value written is remembered, and sealed
    /// with the next setting sealed if the row still holds it (another
    /// process may have written it since). Returns the settings done with,
    /// to [`forget`](Self::forget) once the transaction is committed.
    async fn reseal(
        &self,
        conn: &mut sqlx::SqliteConnection,
        key: &str,
    ) -> Result<Vec<&'static str>, RanchError> {
        let Some(setting) = SafetySetting::of_key(key) else {
            return Ok(Vec::new());
        };
        let value = value_on(conn, setting.key()).await?;
        if !self.seal_setting(conn, setting, value.as_deref()).await? {
            self.unsealed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(setting.key(), value);
            return Ok(Vec::new());
        }
        let earlier: Vec<(&'static str, Option<String>)> = self
            .unsealed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .collect();
        let mut done = vec![setting.key()];
        for (other, written) in earlier {
            if other == setting.key() {
                continue;
            }
            if let Some(other_setting) = SafetySetting::of_key(other) {
                if value_on(conn, other).await? == written {
                    self.seal_setting(conn, other_setting, written.as_deref())
                        .await?;
                }
            }
            done.push(other);
        }
        Ok(done)
    }

    /// Drop settings [`reseal`](Self::reseal) is done with, once their
    /// transaction is committed: a rolled back one is still remembered.
    fn forget(&self, done: Vec<&'static str>) {
        let mut unsealed = self.unsealed.lock().unwrap_or_else(|e| e.into_inner());
        for key in done {
            unsealed.remove(key);
        }
    }

    /// Seal `setting` over `value`; `false` when there is no key to seal
    /// with.
    async fn seal_setting(
        &self,
        conn: &mut sqlx::SqliteConnection,
        setting: SafetySetting,
        value: Option<&str>,
    ) -> Result<bool, RanchError> {
        let Some(sealed) = self.sealer.seal(&setting_parts(setting.key(), value)) else {
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

/// A setting's value as `conn` sees it.
async fn value_on(
    conn: &mut sqlx::SqliteConnection,
    key: &str,
) -> Result<Option<String>, RanchError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM kv_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        .map_err(RanchError::database)?;
    Ok(row.map(|(v,)| v))
}

/// Write `value` under `key` (`None` deletes it) on `conn`.
async fn write_value(
    conn: &mut sqlx::SqliteConnection,
    key: &str,
    value: Option<&str>,
    now: &str,
) -> Result<(), RanchError> {
    match value {
        Some(value) => sqlx::query(
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
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(RanchError::database)?,
        None => sqlx::query("DELETE FROM kv_settings WHERE key = ?")
            .bind(key)
            .execute(&mut *conn)
            .await
            .map_err(RanchError::database)?,
    };
    Ok(())
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
        write_value(&mut tx, key, Some(value), &now).await?;
        let done = self.reseal(&mut tx, key).await?;
        tx.commit().await.map_err(RanchError::database)?;
        self.forget(done);
        Ok(())
    }

    async fn remove(&self, key: &str) -> Result<(), RanchError> {
        Self::refuse_seal_keys(key)?;
        let now = self.clock.now().to_rfc3339();
        let mut tx = begin_immediate(&self.pool).await?;
        write_value(&mut tx, key, None, &now).await?;
        let done = self.reseal(&mut tx, key).await?;
        tx.commit().await.map_err(RanchError::database)?;
        self.forget(done);
        Ok(())
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
        let mut done = Vec::new();
        for setting in SafetySetting::ALL {
            done.extend(self.reseal(&mut tx, setting.key()).await?);
        }
        tx.commit().await.map_err(RanchError::database)?;
        self.forget(done);
        Ok(())
    }

    async fn write_unsealed(
        &self,
        values: &[(SafetySetting, Option<&str>)],
    ) -> Result<(), RanchError> {
        let now = self.clock.now().to_rfc3339();
        let mut tx = begin_immediate(&self.pool).await?;
        for (setting, value) in values {
            write_value(&mut tx, setting.key(), *value, &now).await?;
        }
        tx.commit().await.map_err(RanchError::database)?;
        let mut unsealed = self.unsealed.lock().unwrap_or_else(|e| e.into_inner());
        for (setting, _) in values {
            unsealed.remove(setting.key());
        }
        Ok(())
    }
}
