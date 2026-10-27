//! Integration tests for `kv_settings` commands.
//!
//! Hermetic — uses tempdir SQLite. Verifies the contract:
//!   - get on a missing key returns Ok(None) (NOT an error)
//!   - set followed by get returns the value
//!   - set twice on the same key updates rather than duplicates

use ranch_lib::commands::settings::{get_kv_setting_inner, set_kv_setting_inner};
use ranch_lib::core::db::Db;
use sqlx::Row;
use tempfile::tempdir;

#[tokio::test]
async fn get_missing_returns_none() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");

    let result = get_kv_setting_inner(db.pool(), "does_not_exist")
        .await
        .expect("get");
    assert_eq!(result, None);
}

#[tokio::test]
async fn set_then_get_round_trips() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");

    set_kv_setting_inner(db.pool(), "selected_connection_id", "abc-123")
        .await
        .expect("set");

    let result = get_kv_setting_inner(db.pool(), "selected_connection_id")
        .await
        .expect("get");
    assert_eq!(result.as_deref(), Some("abc-123"));
}

#[tokio::test]
async fn set_twice_updates_in_place_no_duplicates() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");

    set_kv_setting_inner(db.pool(), "k", "v1")
        .await
        .expect("set v1");
    set_kv_setting_inner(db.pool(), "k", "v2")
        .await
        .expect("set v2");

    let value = get_kv_setting_inner(db.pool(), "k").await.expect("get");
    assert_eq!(value.as_deref(), Some("v2"));

    // Exactly one row exists for that key.
    let row_count: i64 = sqlx::query("SELECT COUNT(*) FROM kv_settings WHERE key = ?")
        .bind("k")
        .fetch_one(db.pool())
        .await
        .expect("count")
        .get(0);
    assert_eq!(row_count, 1);
}
