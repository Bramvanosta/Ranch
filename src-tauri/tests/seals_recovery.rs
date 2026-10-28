//! What the app does with the seals when their key is missing: lost while
//! seals existed (start from the strictest values), or not readable yet
//! (seal once it is); and how `ranch-mcp` opens a database it did not
//! migrate.

use std::sync::Arc;

use ranch_lib::adapters::hmac_sealer::HmacSealer;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::db as sqlite_db;
use ranch_lib::adapters::sqlite::{Db, SqliteConnectionRepository, SqliteSettingsStore};
use ranch_lib::adapters::system_clock::SystemClock;
use ranch_lib::application::ports::sealer::Check;
use ranch_lib::application::ports::settings::SafetySetting;
use ranch_lib::application::ports::{ConnectionRepository, SecretStore, SettingsStore};
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

const KEY: [u8; 32] = [5u8; 32];

fn args(name: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: Some(false),
        name: name.to_string(),
        backend: "bullmq-redis".into(),
        endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
            host: "redis.lumen.test".to_string(),
            port: 6379,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        }
        .json(),
        password: None,
        ssh_password: None,
        group: GroupSelector::named("Lumen Studios"),
        environment: EnvironmentSelector::for_color("env-staging"),
    }
}

fn repo_with(db: &Db, sealer: Arc<HmacSealer>) -> SqliteConnectionRepository {
    SqliteConnectionRepository::new(db.pool().clone(), sealer, Arc::new(SystemClock))
}

fn kv_with(db: &Db, sealer: Arc<HmacSealer>) -> SqliteSettingsStore {
    SqliteSettingsStore::new(db.pool().clone(), sealer, Arc::new(SystemClock))
}

#[tokio::test]
async fn restricting_locks_every_connection_and_seals_it_so() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    assert!(!sqlite_db::has_seals(db.pool()).await.unwrap());

    // Written by an app whose key is now gone.
    let lost = repo_with(&db, Arc::new(HmacSealer::for_app(Some([1u8; 32]))));
    let conn = add_connection_inner(
        args("lumen-api staging"),
        &lost,
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert!(sqlite_db::has_seals(db.pool()).await.unwrap());

    // The new key restricts and seals.
    let app = repo_with(&db, Arc::new(HmacSealer::for_app(Some(KEY))));
    app.restrict_all().await.unwrap();

    let agents = repo_with(&db, Arc::new(HmacSealer::for_agents(Some(KEY))));
    let access = agents.agent_access(&conn.id).await.unwrap().unwrap();
    assert!(access.sealed);
    assert!(access.locked);
    assert_eq!(access.mcp_readonly, Some(true));
}

#[tokio::test]
async fn what_the_app_saved_without_its_key_is_sealed_once_the_key_is_back() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());

    // The keychain is locked at launch.
    let store = Arc::new(InMemoryKeychain::new());
    let reader = Arc::clone(&store);
    let sealer = Arc::new(HmacSealer::for_app_reloading(Box::new(move || {
        ranch_lib::adapters::hmac_sealer::load(reader.as_ref(), reader.as_ref())
            .ok()
            .flatten()
    })));
    let app = repo_with(&db, Arc::clone(&sealer));
    let settings = kv_with(&db, Arc::clone(&sealer));
    let first = add_connection_inner(
        args("lumen-api staging"),
        &app,
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    settings
        .set(SafetySetting::ReadonlyDefault.key(), "false")
        .await
        .unwrap();

    let agents = repo_with(&db, Arc::new(HmacSealer::for_agents(Some(KEY))));
    let agent_settings = kv_with(&db, Arc::new(HmacSealer::for_agents(Some(KEY))));
    assert!(
        !agents
            .agent_access(&first.id)
            .await
            .unwrap()
            .unwrap()
            .sealed
    );

    // It opens; the next save seals what was left unsealed too.
    ranch_lib::adapters::hmac_sealer::load_or_create(store.as_ref(), store.as_ref()).unwrap();
    let key = ranch_lib::adapters::hmac_sealer::load(store.as_ref(), store.as_ref())
        .unwrap()
        .unwrap();
    let agents = repo_with(&db, Arc::new(HmacSealer::for_agents(Some(key))));
    let agent_settings_now = kv_with(&db, Arc::new(HmacSealer::for_agents(Some(key))));
    let second = add_connection_inner(
        args("lumen-api review"),
        &app,
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    settings
        .set(SafetySetting::AiEnabled.key(), "true")
        .await
        .unwrap();

    for id in [&first.id, &second.id] {
        assert!(
            agents.agent_access(id).await.unwrap().unwrap().sealed,
            "{id}"
        );
    }
    let readonly_default = agent_settings_now
        .safety(SafetySetting::ReadonlyDefault)
        .await
        .unwrap();
    assert_eq!(readonly_default.check, Check::Valid);
    // Under another key, nothing holds.
    assert_eq!(
        agent_settings
            .safety(SafetySetting::ReadonlyDefault)
            .await
            .unwrap()
            .check,
        Check::Invalid
    );
}

#[tokio::test]
async fn ranch_mcp_opens_only_the_schema_it_knows() {
    let dir = tempdir().unwrap();
    // The app has never run: no database to open.
    let missing = Db::open_current(dir.path()).await.err();
    assert!(
        matches!(missing, Some(RanchError::Migration(_))),
        "{missing:?}"
    );

    let app = Db::init(dir.path()).await.unwrap();
    assert!(Db::open_current(dir.path()).await.is_ok());

    // A newer Ranch migrated it.
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (9999, 'from a newer release', 1, x'00', 0)",
    )
    .execute(app.pool())
    .await
    .unwrap();
    let newer = Db::open_current(dir.path()).await.err();
    assert!(
        newer
            .as_ref()
            .is_some_and(|e| e.to_string().contains("Update Ranch")),
        "{newer:?}"
    );
    // The app does not migrate it back either.
    let refused = Db::init(dir.path()).await.err();
    assert!(
        matches!(refused, Some(RanchError::Migration(_))),
        "{refused:?}"
    );
    assert!(sqlite_db::check_schema(app.pool()).await.is_err());
}
