//! Application state shared by the desktop commands and the HTTP server.
//!
//! The desktop app stores it in Tauri's managed-state slot during the
//! setup hook (see `lib.rs`); commands reach it through
//! `tauri::State<AppState>`. Ranch server holds it in an `Arc`.

use std::sync::Arc;

use crate::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use crate::adapters::bullmq_redis::BullmqRedisBackends;
use crate::adapters::sqlite::{self, Db, SqliteConnectionRepository, SqliteSettingsStore};
use crate::adapters::system_clock::SystemClock;
use crate::application::ports::SecretStore;
use crate::application::ports::{Backends, Clock, ConnectionRepository, Sealer, SettingsStore};

pub struct AppState {
    pub db: Arc<Db>,
    pub keychain: Arc<dyn SecretStore>,
    /// The saved connections, environments and groups.
    pub repository: Arc<dyn ConnectionRepository>,
    /// The app's settings, the safety ones sealed.
    pub settings: Arc<dyn SettingsStore>,
    /// What seals approvals, safety settings and connection flags.
    pub sealer: Arc<dyn Sealer>,
    /// The connections' queues, as every queue operation reaches them.
    pub backends: Arc<dyn Backends>,
    /// The connection actors behind `backends`. Editing or removing a
    /// connection shuts its actor down, so the next call respawns it with
    /// the new settings.
    pub connections: Arc<ConnectionRegistry>,
}

impl AppState {
    /// The composition both shells share: queues are BullMQ on Redis, one
    /// actor per connection, spawned on first use; settings and connections
    /// in `ranch.db`, sealed by `sealer`.
    pub fn new(db: Db, keychain: Arc<dyn SecretStore>, sealer: Arc<dyn Sealer>) -> Self {
        let connections = Arc::new(ConnectionRegistry::new());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let repository: Arc<dyn ConnectionRepository> = Arc::new(SqliteConnectionRepository::new(
            db.pool().clone(),
            Arc::clone(&sealer),
            Arc::clone(&clock),
        ));
        let settings: Arc<dyn SettingsStore> = Arc::new(SqliteSettingsStore::new(
            db.pool().clone(),
            Arc::clone(&sealer),
            clock,
        ));
        let backends = Arc::new(BullmqRedisBackends::new(
            sqlite::write_pipeline(db.pool()),
            Arc::clone(&repository),
            Arc::clone(&keychain),
            ConnectionRegistry::clone(&connections),
        ));
        Self {
            db: Arc::new(db),
            keychain,
            repository,
            settings,
            sealer,
            backends,
            connections,
        }
    }
}
