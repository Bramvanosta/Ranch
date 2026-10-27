//! SQLite persistence — opens (or creates) `${app_data_dir}/ranch.db`,
//! runs the embedded migrations, and exposes the `SqlitePool` to downstream
//! code through `Db::pool()`.
//!
//! WAL journal mode is enabled at connection time so the audit log's
//! durability semantics (synchronous flush per write) work without retrofit.
//! Foreign keys are also enabled (sqlite default-off) so the `ON DELETE
//! CASCADE` clauses in the schema actually take effect.

use crate::core::errors::RanchError;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Open `${app_data_dir}/ranch.db` (creating if missing), apply migrations,
    /// and return the wrapped pool. Idempotent across launches.
    pub async fn init(app_data_dir: &Path) -> Result<Self, RanchError> {
        let db_path = app_data_dir.join("ranch.db");
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        let opts = SqliteConnectOptions::from_str(&db_url)?
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        // Smoke-check: the migrations created the expected core tables.
        sqlx::query("SELECT count(*) FROM connections")
            .fetch_one(&pool)
            .await?;

        tracing::info!(?db_path, "sqlite pool initialized");

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
