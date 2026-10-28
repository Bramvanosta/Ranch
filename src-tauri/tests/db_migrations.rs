//! Integration test for the SQLite migration pipeline.
//!
//! Verifies that:
//!   - `Db::init` creates all expected tables in a fresh database
//!   - WAL journal mode and foreign keys are enabled
//!   - Re-initialising on the same path is idempotent (no errors)
//!   - A database in an earlier shape upgrades with its rows converted

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use ranch_lib::adapters::sqlite::Db;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tempfile::tempdir;

/// A database at `dir/ranch.db` with the shipped migrations numbered
/// before `upto` applied, as a release before it left it.
async fn database_before(dir: &Path, upto: &str) -> SqlitePool {
    let before = dir.join(format!("migrations-before-{upto}"));
    std::fs::create_dir(&before).unwrap();
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/sqlite/migrations");
    for entry in std::fs::read_dir(&shipped).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.as_str() < upto {
            std::fs::copy(&path, before.join(&name)).unwrap();
        }
    }
    let url = format!("sqlite://{}?mode=rwc", dir.join("ranch.db").display());
    let pool = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::from_str(&url)
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::migrate::Migrator::new(before.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    pool
}

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

/// A database written before cause notes named their queue by its id: the
/// notes read back with the BullMQ id `{prefix}:{name}`, the name kept.
#[tokio::test]
async fn cause_notes_stored_with_a_prefix_move_to_the_queue_id() {
    let dir = tempdir().expect("create tempdir");
    // The migrations as they stood before the queue id, applied alone.
    let pool = database_before(dir.path(), "0011").await;
    for (id, prefix, name) in [("n1", "bull", "emails"), ("n2", "myapp:queues", "billing")] {
        sqlx::query(
            "INSERT INTO cause_notes (id, connection_id, queue_prefix, queue_name, signature, text, created_at)
             VALUES (?, 'c1', ?, ?, 'boom', 'why', '2026-10-01T00:00:00Z')",
        )
        .bind(id)
        .bind(prefix)
        .bind(name)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let db = Db::init(dir.path()).await.expect("upgrade");
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, queue_id, queue_name FROM cause_notes ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(
        rows,
        [
            ("n1".into(), "bull:emails".into(), "emails".into()),
            ("n2".into(), "myapp:queues:billing".into(), "billing".into()),
        ]
    );
    let columns: Vec<String> = sqlx::query("SELECT name FROM pragma_table_info('cause_notes')")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(!columns.iter().any(|c| c == "queue_prefix"), "{columns:?}");
}

/// The columns a connection keeps through the move to a backend and an
/// endpoint, as text, for a byte-for-byte comparison.
const KEPT: &str = "id, environment_id, name, keychain_ref, is_locked, last_reachable_at, \
     created_at, updated_at, mcp_readonly, sidebar_collapsed, safety_seal";

async fn kept_columns(pool: &SqlitePool) -> Vec<Vec<Option<String>>> {
    let cols = KEPT.split(", ").count();
    sqlx::query(&format!(
        "SELECT {} FROM connections ORDER BY id",
        KEPT.split(", ")
            .map(|c| format!("quote({c})"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| (0..cols).map(|i| row.get::<Option<String>, _>(i)).collect())
    .collect()
}

/// A database written when a connection had one column per Redis setting:
/// each connection reads back as BullMQ on Redis with its settings in the
/// endpoint, every other column as it was, and its seal, carried over by
/// the app to cover the endpoint, holds for `ranch-mcp` (a tampered one
/// still does not).
#[tokio::test]
async fn redis_columns_move_to_a_backend_and_its_endpoint() {
    use ranch_lib::adapters::bullmq_redis::endpoint::{BullmqRedisSpec, RedisEndpoint};
    use ranch_lib::adapters::hmac_sealer::HmacSealer;
    use ranch_lib::adapters::sqlite::SqliteConnectionRepository;
    use ranch_lib::adapters::system_clock::SystemClock;
    use ranch_lib::application::ports::sealer::{legacy_connection_parts, Check, Sealer};
    use ranch_lib::application::ports::{BackendSpec, ConnectionRepository};
    use serde_json::json;

    const KEY: [u8; 32] = [7u8; 32];
    // Stored as the app stores a CA: checked and trimmed.
    let ca = include_str!("fixtures/self_signed_ca.pem").trim();
    let tunnel = r#"{"host":"bastion.example.com","port":2222,"user":"ops","auth":{"kind":"password"},"jumpHost":{"host":"jump.example.com","port":22,"user":"j"}}"#;

    let dir = tempdir().expect("create tempdir");
    let pool = database_before(dir.path(), "0012").await;
    sqlx::query(
        "INSERT INTO groups (id, name, created_at, updated_at) VALUES ('g', 'Lumen Studios', 't0', 't0')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO environments (id, group_id, name, label, color_token, created_at, updated_at)
         VALUES ('e', 'g', 'Production', 'Production', 'env-production', 't0', 't0')",
    )
    .execute(&pool)
    .await
    .unwrap();

    // id, host, port, db, username, tls, CA, SSH tunnel, locked, agent
    // read-only, folded, last reachable.
    type OldRow<'a> = (
        &'a str,
        &'a str,
        i64,
        i64,
        Option<&'a str>,
        i64,
        Option<&'a str>,
        Option<&'a str>,
        i64,
        Option<i64>,
        i64,
        Option<&'a str>,
    );
    let rows: [OldRow; 5] = [
        (
            "c1",
            "127.0.0.1",
            6379,
            0,
            None,
            0,
            None,
            None,
            0,
            None,
            0,
            None,
        ),
        (
            "c2",
            "Redis.Lumen.example",
            6380,
            5,
            Some("admin"),
            1,
            Some(ca),
            None,
            1,
            Some(1),
            0,
            Some("2026-10-01T00:00:00Z"),
        ),
        (
            "c3",
            "10.0.0.5",
            6379,
            2,
            None,
            0,
            None,
            Some(tunnel),
            0,
            Some(0),
            1,
            None,
        ),
        (
            "c4",
            "h",
            1,
            0,
            None,
            0,
            None,
            Some("not json {{"),
            1,
            None,
            0,
            None,
        ),
        (
            "c5",
            "tampered.example",
            6379,
            0,
            None,
            0,
            None,
            None,
            0,
            Some(1),
            0,
            None,
        ),
    ];
    let app = HmacSealer::for_app(Some(KEY));
    for (id, host, port, db, user, tls, ca, ssh, locked, ro, folded, reachable) in rows {
        // c5's flags were changed behind the app's back: its seal is for
        // another state.
        let seal = if id == "c5" {
            app.seal(&legacy_connection_parts(id, Some(0), locked))
        } else {
            app.seal(&legacy_connection_parts(id, ro, locked))
        };
        sqlx::query(
            "INSERT INTO connections (id, environment_id, name, redis_host, redis_port, redis_db,
                redis_username, tls_enabled, tls_custom_ca, ssh_tunnel_config, keychain_ref,
                is_locked, last_reachable_at, created_at, updated_at, mcp_readonly,
                sidebar_collapsed, safety_seal)
             VALUES (?, 'e', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '2026-09-01T10:00:00Z',
                '2026-09-02T10:00:00Z', ?, ?, ?)",
        )
        .bind(id)
        .bind(format!("conn {id}"))
        .bind(host)
        .bind(port)
        .bind(db)
        .bind(user)
        .bind(tls)
        .bind(ca)
        .bind(ssh)
        .bind(id)
        .bind(locked)
        .bind(reachable)
        .bind(ro)
        .bind(folded)
        .bind(seal)
        .execute(&pool)
        .await
        .unwrap();
    }
    let kept_before = kept_columns(&pool).await;
    pool.close().await;

    let db = Db::init(dir.path()).await.expect("upgrade");

    assert_eq!(kept_columns(db.pool()).await, kept_before);
    let columns: Vec<String> = sqlx::query("SELECT name FROM pragma_table_info('connections')")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    for gone in [
        "redis_host",
        "redis_port",
        "redis_db",
        "redis_username",
        "tls_enabled",
        "tls_custom_ca",
        "ssh_tunnel_config",
    ] {
        assert!(!columns.iter().any(|c| c == gone), "{gone}: {columns:?}");
    }

    let stored: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, backend, endpoint FROM connections ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap();
    let endpoint_of = |id: &str| -> serde_json::Value {
        let (_, backend, endpoint) = stored.iter().find(|(i, _, _)| i == id).unwrap();
        assert_eq!(backend, "bullmq-redis");
        serde_json::from_str(endpoint).unwrap()
    };
    let plain = |host: &str, port: u16| {
        json!({
            "host": host, "port": port, "db": 0, "username": null, "tls": false,
            "tlsCustomCa": null, "sshTunnel": null
        })
    };
    assert_eq!(endpoint_of("c1"), plain("127.0.0.1", 6379));
    assert_eq!(
        endpoint_of("c2"),
        json!({
            "host": "Redis.Lumen.example", "port": 6380, "db": 5, "username": "admin",
            "tls": true, "tlsCustomCa": ca, "sshTunnel": null
        })
    );
    assert_eq!(
        endpoint_of("c3"),
        json!({
            "host": "10.0.0.5", "port": 6379, "db": 2, "username": null, "tls": false,
            "tlsCustomCa": null,
            "sshTunnel": serde_json::from_str::<serde_json::Value>(tunnel).unwrap()
        })
    );
    // A tunnel that was never readable is no tunnel, as the app read it.
    assert_eq!(endpoint_of("c4"), plain("h", 1));

    // The BullMQ backend reads every one of them back.
    let repo = SqliteConnectionRepository::new(
        db.pool().clone(),
        Arc::new(HmacSealer::for_agents(Some(KEY))),
        Arc::new(SystemClock),
    );
    for connection in repo.list().await.unwrap() {
        assert_eq!(connection.backend.as_str(), "bullmq-redis");
        let checked = BullmqRedisSpec.validate(&connection.endpoint).unwrap();
        assert_eq!(
            RedisEndpoint::parse(&checked).unwrap(),
            RedisEndpoint::parse(&connection.endpoint).unwrap()
        );
    }
    let c3 = repo.stored_settings("c3").await.unwrap().unwrap();
    let tunnel = RedisEndpoint::parse(&c3.endpoint)
        .unwrap()
        .ssh_tunnel
        .unwrap();
    assert_eq!(
        (tunnel.host.as_str(), tunnel.port, tunnel.user.as_str()),
        ("bastion.example.com", 2222, "ops")
    );

    // The seals were made before the upgrade and not touched by it. They
    // cover the flags only: no connection opens until the app carries them
    // over, the tampered one excepted.
    let agents = HmacSealer::for_agents(Some(KEY));
    let flags: Vec<(String, Option<i64>, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, mcp_readonly, is_locked, safety_seal FROM connections ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    for (id, ro, locked, seal) in flags {
        let expected = if id == "c5" {
            Check::Invalid
        } else {
            Check::Valid
        };
        assert_eq!(
            agents.check(&legacy_connection_parts(&id, ro, locked), seal.as_deref()),
            expected,
            "{id}"
        );
        assert!(
            !repo.agent_access(&id).await.unwrap().unwrap().sealed,
            "{id}"
        );
        assert!(
            !repo.stored_settings(&id).await.unwrap().unwrap().sealed,
            "{id}"
        );
    }
    assert_eq!(repo.upgrade_seals(&agents).await.unwrap(), 4);
    assert_eq!(
        repo.upgrade_seals(&agents).await.unwrap(),
        0,
        "once is enough"
    );
    for id in ["c1", "c2", "c3", "c4", "c5"] {
        assert_eq!(
            repo.agent_access(id).await.unwrap().unwrap().sealed,
            id != "c5",
            "{id}"
        );
        assert_eq!(
            repo.stored_settings(id).await.unwrap().unwrap().sealed,
            id != "c5",
            "{id}"
        );
    }
}
