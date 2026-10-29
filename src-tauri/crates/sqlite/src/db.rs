//! SQLite persistence: opens (or creates) `${app_data_dir}/ranch.db`,
//! runs the embedded migrations, and exposes the `SqlitePool` to downstream
//! code through `Db::pool()`.
//!
//! WAL journal mode is enabled at connection time so the audit log's
//! durability semantics (synchronous flush per write) work without retrofit.
//! Foreign keys are also enabled (sqlite default-off) so the `ON DELETE
//! CASCADE` clauses in the schema actually take effect.
//!
//! The app migrates; `ranch-mcp` does not. Before migrations run on an
//! existing database, it is copied beside itself (`ranch.db.bak-schema-<n>`),
//! since some migrations cannot be undone. A database written by a newer
//! Ranch is refused with a message that says so, as is, for `ranch-mcp`, a
//! database the app has not migrated yet ([`SchemaMismatch`]).

use std::fmt;
use std::path::{Path, PathBuf};

use ranch_domain::errors::RanchError;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool, Transaction};

/// The database file, in the app-data dir.
pub const DB_FILE: &str = "ranch.db";

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Why this binary cannot use `ranch.db` as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaMismatch {
    /// A newer Ranch migrated it: this binary does not know its schema.
    Newer { found: i64, known: i64 },
    /// Not migrated to this binary's schema yet. Only the app migrates.
    Older { found: i64, known: i64 },
}

impl fmt::Display for SchemaMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Newer { found, known } => write!(
                f,
                "ranch.db was last opened by a newer version of Ranch (database schema {found}, this one knows {known}). Update Ranch, then restart your agent client."
            ),
            Self::Older { found, known } => write!(
                f,
                "Ranch was updated and its database is not up to date yet (database schema {found}, this version needs {known}). Open the Ranch app once, then restart your agent client."
            ),
        }
    }
}

impl std::error::Error for SchemaMismatch {}

pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Where the database of `app_data_dir` lives.
    pub fn path(app_data_dir: &Path) -> PathBuf {
        app_data_dir.join(DB_FILE)
    }

    /// Open `${app_data_dir}/ranch.db` (creating if missing), apply migrations,
    /// and return the wrapped pool. Idempotent across launches. An existing
    /// database with migrations pending is backed up first.
    pub async fn init(app_data_dir: &Path) -> Result<Self, RanchError> {
        let db_path = Self::path(app_data_dir);
        let pool = connect(&db_path, true).await?;

        let known = known_version();
        match applied_version(&pool).await? {
            Some(found) if found > known => {
                return Err(RanchError::migration(SchemaMismatch::Newer {
                    found,
                    known,
                }))
            }
            Some(found) if found < known => backup(&pool, &db_path, found).await?,
            _ => {}
        }

        MIGRATOR.run(&pool).await.map_err(RanchError::migration)?;

        // Smoke-check: the migrations created the expected core tables.
        sqlx::query("SELECT count(*) FROM connections")
            .fetch_one(&pool)
            .await
            .map_err(RanchError::database)?;

        tracing::info!(?db_path, "sqlite pool initialized");

        Ok(Self { pool })
    }

    /// Open the database as the app left it, without migrating: what
    /// `ranch-mcp` does. Refused unless its schema is this binary's.
    pub async fn open_current(app_data_dir: &Path) -> Result<Self, RanchError> {
        let db_path = Self::path(app_data_dir);
        if !db_path.exists() {
            // The app has never run: it creates the database.
            return Err(RanchError::migration(SchemaMismatch::Older {
                found: 0,
                known: known_version(),
            }));
        }
        let pool = connect(&db_path, false).await?;
        check_schema(&pool).await?;
        tracing::info!(?db_path, "sqlite pool opened");
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// `Ok` while the database's schema is the one this binary was built
/// for. The app may migrate it under a running `ranch-mcp`.
pub async fn check_schema(pool: &SqlitePool) -> Result<(), RanchError> {
    let known = known_version();
    let found = applied_version(pool).await?.unwrap_or(0);
    if found > known {
        return Err(RanchError::migration(SchemaMismatch::Newer {
            found,
            known,
        }));
    }
    if found < known {
        return Err(RanchError::migration(SchemaMismatch::Older {
            found,
            known,
        }));
    }
    Ok(())
}

/// A transaction that takes the write lock as it starts. A deferred one
/// that reads, then writes after another connection wrote, fails at once
/// with "database is locked" instead of waiting its turn; this one waits
/// (the busy timeout) before it reads anything.
pub async fn begin_immediate(
    pool: &SqlitePool,
) -> Result<Transaction<'static, Sqlite>, RanchError> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(RanchError::database)
}

/// Whether the app ever sealed anything in this database: a connection or
/// a safety setting carries a seal.
pub async fn has_seals(pool: &SqlitePool) -> Result<bool, RanchError> {
    let (found,): (i64,) = sqlx::query_as(
        r#"
        SELECT EXISTS (SELECT 1 FROM connections WHERE safety_seal IS NOT NULL)
            OR EXISTS (SELECT 1 FROM kv_settings WHERE key LIKE 'seal:%')
        "#,
    )
    .fetch_one(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(found != 0)
}

async fn connect(db_path: &Path, create: bool) -> Result<SqlitePool, RanchError> {
    if create {
        create_private(db_path);
    }
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(create)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(RanchError::database)?;
    restrict_permissions(db_path);
    Ok(pool)
}

/// The newest migration this binary carries.
fn known_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0)
}

/// The newest migration applied to the database; `None` for a new one.
async fn applied_version(pool: &SqlitePool) -> Result<Option<i64>, RanchError> {
    let (tracked,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .map_err(RanchError::database)?;
    if tracked == 0 {
        return Ok(None);
    }
    let (version,): (Option<i64>,) =
        sqlx::query_as("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(pool)
            .await
            .map_err(RanchError::database)?;
    Ok(version)
}

/// Copy the database beside itself before migrating it. The first copy
/// for a schema is kept: a later attempt may start from a half-done one.
async fn backup(pool: &SqlitePool, db_path: &Path, found: i64) -> Result<(), RanchError> {
    let mut name = db_path.as_os_str().to_owned();
    name.push(format!(".bak-schema-{found}"));
    let target = PathBuf::from(name);
    if target.exists() {
        tracing::info!(?target, "a backup of this schema already exists; kept");
        return Ok(());
    }
    // Written under another name, owner-only from the start (VACUUM INTO
    // takes an empty file), then renamed: a copy cut short by a crash is
    // never taken for the backup.
    let mut partial = target.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let Some(partial_text) = partial.to_str() else {
        return Err(RanchError::InvalidInput {
            reason: format!("cannot back up the database to {}", target.display()),
        });
    };
    match std::fs::remove_file(&partial) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(RanchError::InvalidInput {
                reason: format!("cannot clear {}: {e}", partial.display()),
            })
        }
    }
    create_private(&partial);
    sqlx::query("VACUUM INTO ?")
        .bind(partial_text)
        .execute(pool)
        .await
        .map_err(RanchError::database)?;
    restrict_permissions(&partial);
    std::fs::rename(&partial, &target).map_err(|e| RanchError::InvalidInput {
        reason: format!("cannot keep the backup at {}: {e}", target.display()),
    })?;
    tracing::info!(?target, "database backed up before migrating");
    Ok(())
}

/// Create the database file readable by its owner alone, before SQLite
/// creates it with the default mode. The `-wal` and `-shm` files take the
/// database's mode.
#[cfg(unix)]
fn create_private(db_path: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    if db_path.exists() {
        return;
    }
    if let Err(e) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(db_path)
    {
        tracing::warn!(error = %e, ?db_path, "could not create the database file");
    }
}

#[cfg(not(unix))]
fn create_private(_db_path: &Path) {}

/// Owner-only permissions on the database and its journal files: other
/// accounts on the machine have no business reading connection settings or
/// the audit log.
#[cfg(unix)]
fn restrict_permissions(db_path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for suffix in ["", "-wal", "-shm"] {
        let mut name = db_path.as_os_str().to_owned();
        name.push(suffix);
        let path = PathBuf::from(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.permissions().mode() & 0o077 == 0 {
            continue;
        }
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(error = %e, ?path, "could not restrict the database's permissions");
        }
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_db_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatches_say_what_to_do() {
        let newer = SchemaMismatch::Newer {
            found: 14,
            known: 12,
        }
        .to_string();
        assert!(newer.contains("Update Ranch"), "{newer}");
        let older = SchemaMismatch::Older {
            found: 11,
            known: 12,
        }
        .to_string();
        assert!(older.contains("Open the Ranch app once"), "{older}");
        assert!(known_version() >= 12);
    }
}
