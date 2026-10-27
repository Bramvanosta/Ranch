//! Integration test for the SQLite migration pipeline.
//!
//! Verifies that:
//!   - `Db::init` creates all expected tables in a fresh database
//!   - WAL journal mode and foreign keys are enabled
//!   - Re-initialising on the same path is idempotent (no errors)

use ranch_lib::core::db::Db;
use sqlx::Row;
use tempfile::tempdir;

#[tokio::test]
async fn init_creates_expected_tables_and_pragmas() {
    let dir = tempdir().expect("create tempdir");
    let db = Db::init(dir.path()).await.expect("first init");

    // All four tables should exist.
    let tables: Vec<String> =
        sqlx::query("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(db.pool())
            .await
            .expect("list tables")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();

    for expected in [
        "_sqlx_migrations",
        "connections",
        "environments",
        "groups",
        "kv_settings",
    ] {
        assert!(
            tables.iter().any(|name| name == expected),
            "expected table `{expected}` to exist; got tables = {tables:?}",
        );
    }

    // WAL journal mode.
    let journal_mode: String = sqlx::query("PRAGMA journal_mode")
        .fetch_one(db.pool())
        .await
        .expect("read journal_mode")
        .get(0);
    assert_eq!(journal_mode.to_lowercase(), "wal");

    // Foreign keys enabled (returns 1).
    let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
        .fetch_one(db.pool())
        .await
        .expect("read foreign_keys")
        .get(0);
    assert_eq!(foreign_keys, 1);
}

#[tokio::test]
async fn init_is_idempotent_across_relaunches() {
    let dir = tempdir().expect("create tempdir");

    // First init creates everything.
    let _db1 = Db::init(dir.path()).await.expect("first init");

    // Second init on the same path must succeed (no migration error,
    // no duplicate-table error).
    let _db2 = Db::init(dir.path()).await.expect("second init");
}
