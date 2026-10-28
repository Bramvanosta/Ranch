//! The saved connections, environments and groups, in `ranch.db`.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;

use ranch_application::ports::connections::{
    AgentAccess, BeforeCommit, ConnectionEdit, ConnectionRepository, CopySecrets, NewConnection,
    StoredSettings,
};
use ranch_application::ports::sealer::{connection_parts, Check, Sealer};
use ranch_application::ports::Clock;
use ranch_domain::endpoint::{BackendKind, Endpoint};
use ranch_domain::errors::RanchError;
use ranch_domain::hierarchy::{
    next_duplicate_name, normalize_name, validate_color_token, validate_full_set, Connection,
    Environment, EnvironmentSelector, Group, GroupSelector, MoveConnectionResult,
};
use ranch_domain::production;

pub struct SqliteConnectionRepository {
    pool: SqlitePool,
    sealer: Arc<dyn Sealer>,
    clock: Arc<dyn Clock>,
}

impl SqliteConnectionRepository {
    pub fn new(pool: SqlitePool, sealer: Arc<dyn Sealer>, clock: Arc<dyn Clock>) -> Self {
        Self {
            pool,
            sealer,
            clock,
        }
    }

    /// The `created_at` / `updated_at` of a write.
    fn now(&self) -> String {
        self.clock.now().to_rfc3339()
    }
}

/// Seal one connection as it now stands. Called by every write of its flags,
/// in the same transaction.
async fn reseal(
    conn: &mut sqlx::SqliteConnection,
    sealer: &dyn Sealer,
    id: &str,
) -> Result<(), RanchError> {
    let row: Option<(Option<i64>, i64)> =
        sqlx::query_as("SELECT mcp_readonly, is_locked FROM connections WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(RanchError::database)?;
    if let Some((mcp_readonly, is_locked)) = row {
        sqlx::query("UPDATE connections SET safety_seal = ? WHERE id = ?")
            .bind(sealer.seal(&connection_parts(id, mcp_readonly, is_locked)))
            .bind(id)
            .execute(&mut *conn)
            .await
            .map_err(RanchError::database)?;
    }
    Ok(())
}

/// Row shape shared by `list` and `get`. A struct rather than a tuple
/// because sqlx only implements `FromRow` for tuples of up to 16 columns.
#[derive(sqlx::FromRow)]
struct ConnectionRow {
    id: String,
    environment_id: String,
    name: String,
    backend: String,
    endpoint: String, // JSON
    created_at: String,
    env_color: String,
    env_label: String,
    env_name: String,
    group_id: String,
    group_name: String,
    mcp_readonly: Option<i64>,
    sidebar_collapsed: i64,
}

/// SELECT + JOINs producing a `ConnectionRow`; callers append WHERE / ORDER BY.
const CONNECTION_SELECT: &str = r#"
    SELECT c.id, c.environment_id, c.name, c.backend, c.endpoint, c.created_at,
           e.color_token AS env_color, e.label AS env_label, e.name AS env_name,
           g.id AS group_id, g.name AS group_name,
           c.mcp_readonly, c.sidebar_collapsed
    FROM connections c
    JOIN environments e ON c.environment_id = e.id
    JOIN groups g ON e.group_id = g.id
"#;

impl ConnectionRow {
    fn into_connection(self) -> Connection {
        let endpoint = decode_endpoint_column(&self.endpoint, &self.id);
        Connection {
            id: self.id,
            environment_id: self.environment_id,
            group_id: self.group_id,
            name: self.name,
            backend: BackendKind::new(self.backend),
            endpoint,
            env_color: self.env_color,
            env_name: self.env_name,
            env_label: self.env_label,
            group_name: self.group_name,
            created_at: self.created_at,
            mcp_readonly: self.mcp_readonly.map(|v| v != 0),
            sidebar_collapsed: self.sidebar_collapsed != 0,
        }
    }
}

/// Decode the JSON `endpoint` column. Defensive: on a corrupt JSON, log a
/// warning and read an empty endpoint so the sidebar keeps working; the
/// backend then refuses it when the connection is opened, and the user can
/// edit the connection to fix it.
fn decode_endpoint_column(raw: &str, connection_id: &str) -> Endpoint {
    match serde_json::from_str(raw) {
        Ok(value) => Endpoint::new(value),
        Err(e) => {
            tracing::warn!(
                connection_id = %connection_id,
                error = %e,
                "endpoint JSON decode failed; reading an empty endpoint"
            );
            Endpoint::empty()
        }
    }
}

/// The JSON `endpoint` column.
fn encode_endpoint(endpoint: &Endpoint) -> String {
    endpoint.as_json().to_string()
}

/// Row tuple for `stored_settings`, aliased so clippy's `type_complexity`
/// lint stays quiet (the columns include `is_locked`, `name`, and the
/// joined `env.color_token` so the backend can open with full identity +
/// initial lock state).
type SettingsRow = (
    String,         // backend
    String,         // endpoint (JSON)
    String,         // keychain_ref
    i64,            // is_locked
    String,         // name
    Option<String>, // env_color (joined from environments.color_token)
);

/// Where a connection ended up after selector resolution.
struct ResolvedHierarchy {
    group_id: String,
    group_name: String,
    environment_id: String,
    env_name: String,
    env_label: String,
    env_color: String,
}

/// Resolve (creating what is `New`) the group and environment for a
/// connection, inside the caller's transaction so a later failure (keychain
/// included) rolls the new group / environment back too.
async fn resolve_hierarchy(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    group: &GroupSelector,
    environment: &EnvironmentSelector,
    now: &str,
) -> Result<ResolvedHierarchy, RanchError> {
    // ── group ────────────────────────────────────────────────────────────
    let (group_id, group_name, group_is_new) = match group {
        GroupSelector::Existing { id } => {
            let row: Option<(String,)> = sqlx::query_as("SELECT name FROM groups WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(RanchError::database)?;
            let (name,) = row.ok_or_else(|| RanchError::GroupNotFound { id: id.clone() })?;
            (id.clone(), name, false)
        }
        GroupSelector::New { name } => {
            let name = normalize_name("Group", name)?;
            let wanted = name.to_lowercase();
            let existing: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM groups")
                .fetch_all(&mut **tx)
                .await
                .map_err(RanchError::database)?;
            match existing
                .into_iter()
                .find(|(_, other)| other.to_lowercase() == wanted)
            {
                Some((id, existing_name)) => (id, existing_name, false),
                None => {
                    let id = uuid::Uuid::now_v7().to_string();
                    let (next_order,): (i64,) =
                        sqlx::query_as("SELECT COALESCE(MAX(display_order) + 1, 0) FROM groups")
                            .fetch_one(&mut **tx)
                            .await
                            .map_err(RanchError::database)?;
                    sqlx::query(
                        r#"
                        INSERT INTO groups
                            (id, name, label, display_order, collapsed, created_at, updated_at)
                        VALUES (?, ?, 'Client', ?, 0, ?, ?)
                        "#,
                    )
                    .bind(&id)
                    .bind(&name)
                    .bind(next_order)
                    .bind(now)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(RanchError::database)?;
                    (id, name, true)
                }
            }
        }
    };

    // ── environment ──────────────────────────────────────────────────────
    let (environment_id, env_name, env_label, env_color) = match environment {
        EnvironmentSelector::Existing { id } => {
            let row: Option<(String, String, String, String)> = sqlx::query_as(
                "SELECT group_id, name, label, color_token FROM environments WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(RanchError::database)?;
            let (env_group_id, name, label, color) =
                row.ok_or_else(|| RanchError::EnvironmentNotFound { id: id.clone() })?;
            if group_is_new || env_group_id != group_id {
                return Err(RanchError::InvalidInput {
                    reason: "Environment does not belong to the selected group".to_string(),
                });
            }
            (id.clone(), name, label, color)
        }
        EnvironmentSelector::New { name, color_token } => {
            let name = normalize_name("Environment", name)?;
            validate_color_token(color_token)?;
            let wanted = name.to_lowercase();
            let existing: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT id, name, label, color_token FROM environments WHERE group_id = ?",
            )
            .bind(&group_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(RanchError::database)?;
            match existing
                .into_iter()
                .find(|(_, _, label, _)| label.to_lowercase() == wanted)
            {
                Some(found) => found,
                None => {
                    let id = uuid::Uuid::now_v7().to_string();
                    let (next_order,): (i64,) = sqlx::query_as(
                        "SELECT COALESCE(MAX(display_order) + 1, 0) FROM environments WHERE group_id = ?",
                    )
                    .bind(&group_id)
                    .fetch_one(&mut **tx)
                    .await.map_err(RanchError::database)?;
                    sqlx::query(
                        r#"
                        INSERT INTO environments
                            (id, group_id, name, label, color_token, display_order, created_at, updated_at)
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                        "#,
                    )
                    .bind(&id)
                    .bind(&group_id)
                    .bind(&name)
                    .bind(&name)
                    .bind(color_token)
                    .bind(next_order)
                    .bind(now)
                    .bind(now)
                    .execute(&mut **tx)
                    .await.map_err(RanchError::database)?;
                    (id, name.clone(), name, color_token.clone())
                }
            }
        }
    };

    Ok(ResolvedHierarchy {
        group_id,
        group_name,
        environment_id,
        env_name,
        env_label,
        env_color,
    })
}

#[async_trait]
impl ConnectionRepository for SqliteConnectionRepository {
    // ── connections ─────────────────────────────────────────────────────

    async fn list(&self) -> Result<Vec<Connection>, RanchError> {
        // Flat list in sidebar order; the frontend shapes the tree (`treeShape.ts`).
        let sql =
            format!("{CONNECTION_SELECT} ORDER BY g.display_order, e.display_order, c.created_at");
        let rows: Vec<ConnectionRow> = sqlx::query_as(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(RanchError::database)?;
        Ok(rows
            .into_iter()
            .map(ConnectionRow::into_connection)
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<Connection>, RanchError> {
        let sql = format!("{CONNECTION_SELECT} WHERE c.id = ?");
        let row: Option<ConnectionRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(RanchError::database)?;
        Ok(row.map(ConnectionRow::into_connection))
    }

    async fn stored_settings(&self, id: &str) -> Result<Option<StoredSettings>, RanchError> {
        // Join `environments` to pull `color_token` so the actor's
        // `env_color` snapshot is filled for audit-log row stripes. `LEFT
        // JOIN` so a connection with a missing env row (shouldn't happen:
        // FK constraint, but defensive) still resolves with `env_color =
        // None`.
        let row: Option<SettingsRow> = sqlx::query_as(
            r#"
            SELECT c.backend, c.endpoint, c.keychain_ref,
                   c.is_locked, c.name, e.color_token AS env_color
            FROM connections c
            LEFT JOIN environments e ON c.environment_id = e.id
            WHERE c.id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(RanchError::database)?;
        Ok(row.map(
            |(backend, endpoint, keychain_ref, is_locked, name, env_color)| StoredSettings {
                backend: BackendKind::new(backend),
                endpoint: decode_endpoint_column(&endpoint, id),
                keychain_ref,
                locked: is_locked != 0,
                name,
                env_color,
            },
        ))
    }

    async fn agent_access(&self, id: &str) -> Result<Option<AgentAccess>, RanchError> {
        let row: Option<(Option<i64>, i64, Option<String>, String)> = sqlx::query_as(
            "SELECT mcp_readonly, is_locked, safety_seal, name FROM connections WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(RanchError::database)?;
        Ok(row.map(|(explicit, is_locked, seal, name)| AgentAccess {
            mcp_readonly: explicit.map(|v| v != 0),
            name,
            sealed: self
                .sealer
                .check(&connection_parts(id, explicit, is_locked), seal.as_deref())
                == Check::Valid,
        }))
    }

    async fn reseal_all(&self) -> Result<(), RanchError> {
        let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM connections")
            .fetch_all(&self.pool)
            .await
            .map_err(RanchError::database)?;
        let mut conn = self.pool.acquire().await.map_err(RanchError::database)?;
        for (id,) in ids {
            reseal(&mut conn, self.sealer.as_ref(), &id).await?;
        }
        Ok(())
    }

    async fn keychain_ref(&self, id: &str) -> Result<Option<String>, RanchError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT keychain_ref FROM connections WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(RanchError::database)?;
        Ok(row.map(|(r,)| r))
    }

    async fn insert(
        &self,
        new: NewConnection,
        before_commit: BeforeCommit<'_>,
    ) -> Result<Connection, RanchError> {
        let now = self.now();
        let connection_id = new.id;

        // One transaction, so a keychain failure leaves SQLite untouched.
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;

        // The user picks group + environment; anything `New` is
        // created inside this transaction so a later failure rolls it back too.
        let placed = resolve_hierarchy(&mut tx, &new.group, &new.environment, &now).await?;
        let env_id = placed.environment_id.clone();

        // A connection in a Production environment starts locked, and
        // read-only for agents (elsewhere it follows the global default,
        // NULL). The column declares `DEFAULT 0`; the computed value is
        // bound explicitly so the intent is on-screen at the INSERT site.
        let safety = production::for_new_connection(&placed.env_color, new.locked);
        let is_locked: i64 = i64::from(safety.locked);
        let mcp_readonly: Option<i64> = safety.mcp_readonly.map(i64::from);

        sqlx::query(
            r#"
            INSERT INTO connections (
                id, environment_id, name, backend, endpoint,
                keychain_ref, is_locked, mcp_readonly, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&connection_id)
        .bind(&env_id)
        .bind(&new.name)
        .bind(new.backend.as_str())
        .bind(encode_endpoint(&new.endpoint))
        .bind(&connection_id) // keychain_ref convention: same as connection id
        .bind(is_locked)
        .bind(mcp_readonly)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(RanchError::database)?;
        reseal(&mut tx, self.sealer.as_ref(), &connection_id).await?;

        // A failure here drops the transaction without a commit: no row is
        // left pointing at a missing secret.
        before_commit()?;

        tx.commit().await.map_err(RanchError::database)?;

        Ok(Connection {
            id: connection_id,
            environment_id: env_id,
            group_id: placed.group_id,
            name: new.name,
            backend: new.backend,
            endpoint: new.endpoint,
            env_color: placed.env_color,
            env_name: placed.env_name,
            env_label: placed.env_label,
            group_name: placed.group_name,
            created_at: now,
            mcp_readonly: mcp_readonly.map(|v| v != 0),
            sidebar_collapsed: false,
        })
    }

    async fn update(
        &self,
        edit: ConnectionEdit,
        before_commit: BeforeCommit<'_>,
    ) -> Result<Connection, RanchError> {
        let now = self.now();
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;

        // Existence check first so an unknown id reports ConnectionNotFound
        // rather than a hierarchy error.
        let current: Option<(String,)> =
            sqlx::query_as("SELECT environment_id FROM connections WHERE id = ?")
                .bind(&edit.id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        let (current_env_id,) = current.ok_or_else(|| RanchError::ConnectionNotFound {
            id: edit.id.clone(),
        })?;

        // Explicit placement. A different environment = a move.
        let placed = resolve_hierarchy(&mut tx, &edit.group, &edit.environment, &now).await?;
        let target_env_id = placed.environment_id.clone();
        let moved_into_production =
            production::applies_on_move(&current_env_id, &target_env_id, &placed.env_color);

        sqlx::query(
            r#"
            UPDATE connections SET
                name = ?, backend = ?, endpoint = ?, environment_id = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&edit.name)
        .bind(edit.backend.as_str())
        .bind(encode_endpoint(&edit.endpoint))
        .bind(&target_env_id)
        .bind(&now)
        .bind(&edit.id)
        .execute(&mut *tx)
        .await
        .map_err(RanchError::database)?;

        // Moving a connection INTO a Production-colored environment
        // applies the production default (locked). Moving out never unlocks.
        if moved_into_production {
            sqlx::query("UPDATE connections SET is_locked = 1, mcp_readonly = 1 WHERE id = ?")
                .bind(&edit.id)
                .execute(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        }
        // An explicit choice wins.
        if let Some(locked) = edit.locked {
            sqlx::query("UPDATE connections SET is_locked = ? WHERE id = ?")
                .bind(i64::from(locked))
                .bind(&edit.id)
                .execute(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        }
        reseal(&mut tx, self.sealer.as_ref(), &edit.id).await?;

        // Same rollback semantics as `insert`.
        before_commit()?;

        tx.commit().await.map_err(RanchError::database)?;

        self.get(&edit.id)
            .await?
            .ok_or(RanchError::ConnectionNotFound { id: edit.id })
    }

    async fn duplicate(
        &self,
        source_id: &str,
        new_id: &str,
        copy_secrets: CopySecrets<'_>,
    ) -> Result<Connection, RanchError> {
        let now = self.now();
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;

        // Read source row.
        type SourceRow = (
            String,       // environment_id
            String,       // name
            String,       // backend
            String,       // endpoint (JSON, copied as stored)
            String,       // keychain_ref
            bool,         // is_locked
            Option<bool>, // mcp_readonly
        );
        let source: Option<SourceRow> = sqlx::query_as(
            r#"
            SELECT environment_id, name, backend, endpoint,
                   keychain_ref, is_locked, mcp_readonly
            FROM connections WHERE id = ?
            "#,
        )
        .bind(source_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(RanchError::database)?;

        let (env_id, source_name, backend, endpoint, source_keychain_ref, is_locked, mcp_readonly) =
            source.ok_or_else(|| RanchError::ConnectionNotFound {
                id: source_id.to_string(),
            })?;

        // Resolve a non-colliding name within the same env.
        let existing_names: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM connections WHERE environment_id = ?")
                .bind(&env_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        let existing_names: Vec<String> = existing_names.into_iter().map(|(n,)| n).collect();
        let new_name = next_duplicate_name(&source_name, &existing_names);

        // Insert new row pointing at the new keychain_ref (same convention:
        // keychain_ref = id), with the source's backend and endpoint. And its
        // protections: a copy of a production connection is just as locked,
        // and just as read-only for agents, as the original: the same
        // server behind the same credentials.
        sqlx::query(
            r#"
            INSERT INTO connections (
                id, environment_id, name, backend, endpoint,
                keychain_ref, created_at, updated_at, is_locked, mcp_readonly
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(new_id)
        .bind(&env_id)
        .bind(&new_name)
        .bind(&backend)
        .bind(&endpoint)
        .bind(new_id)
        .bind(&now)
        .bind(&now)
        .bind(is_locked)
        .bind(mcp_readonly)
        .execute(&mut *tx)
        .await
        .map_err(RanchError::database)?;
        reseal(&mut tx, self.sealer.as_ref(), new_id).await?;

        copy_secrets(&source_keychain_ref)?;

        tx.commit().await.map_err(RanchError::database)?;

        self.get(new_id)
            .await?
            .ok_or(RanchError::ConnectionNotFound {
                id: new_id.to_string(),
            })
    }

    async fn delete(&self, id: &str) -> Result<(), RanchError> {
        sqlx::query("DELETE FROM connections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;
        Ok(())
    }

    async fn set_collapsed(&self, id: &str, collapsed: bool) -> Result<(), RanchError> {
        // No `updated_at` bump: folding is not an edit of the connection.
        let result = sqlx::query("UPDATE connections SET sidebar_collapsed = ? WHERE id = ?")
            .bind(if collapsed { 1_i64 } else { 0_i64 })
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;
        if result.rows_affected() == 0 {
            return Err(RanchError::ConnectionNotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn set_mcp_readonly(
        &self,
        id: &str,
        mcp_readonly: Option<bool>,
    ) -> Result<(), RanchError> {
        let now = self.now();
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;
        let result =
            sqlx::query("UPDATE connections SET mcp_readonly = ?, updated_at = ? WHERE id = ?")
                .bind(mcp_readonly.map(|v| if v { 1_i64 } else { 0_i64 }))
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        if result.rows_affected() == 0 {
            return Err(RanchError::ConnectionNotFound { id: id.to_string() });
        }
        reseal(&mut tx, self.sealer.as_ref(), id).await?;
        tx.commit().await.map_err(RanchError::database)?;
        Ok(())
    }

    async fn move_connection(
        &self,
        id: &str,
        environment_id: &str,
    ) -> Result<MoveConnectionResult, RanchError> {
        let now = self.now();
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;

        let current: Option<(String, i64)> =
            sqlx::query_as("SELECT environment_id, is_locked FROM connections WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        let (current_env_id, is_locked) =
            current.ok_or_else(|| RanchError::ConnectionNotFound { id: id.to_string() })?;

        let target: Option<(String,)> =
            sqlx::query_as("SELECT color_token FROM environments WHERE id = ?")
                .bind(environment_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        let (target_color,) = target.ok_or_else(|| RanchError::EnvironmentNotFound {
            id: environment_id.to_string(),
        })?;

        if current_env_id == environment_id {
            return Ok(MoveConnectionResult {
                newly_locked: false,
            });
        }

        let entering_production = production::is_production(&target_color);
        let newly_locked = entering_production && is_locked == 0;
        // Entering Production also makes the connection read-only for
        // agents; elsewhere the existing MCP setting is kept.
        sqlx::query(
            r#"
            UPDATE connections
               SET environment_id = ?, is_locked = ?,
                   mcp_readonly = CASE WHEN ? THEN 1 ELSE mcp_readonly END,
                   updated_at = ?
             WHERE id = ?
            "#,
        )
        .bind(environment_id)
        .bind(if newly_locked { 1 } else { is_locked })
        .bind(entering_production)
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(RanchError::database)?;
        reseal(&mut tx, self.sealer.as_ref(), id).await?;
        tx.commit().await.map_err(RanchError::database)?;

        Ok(MoveConnectionResult { newly_locked })
    }

    // ── environments ────────────────────────────────────────────────────

    async fn list_environments(&self) -> Result<Vec<Environment>, RanchError> {
        // Sidebar order: group, then environment.
        let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT e.id, e.group_id, g.name, e.label, e.color_token, e.name
            FROM environments e
            JOIN groups g ON e.group_id = g.id
            ORDER BY g.display_order, g.created_at, e.display_order, e.created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RanchError::database)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, group_id, group_name, label, color_token, name)| Environment {
                    id,
                    group_id,
                    group_name,
                    label,
                    color_token,
                    name,
                },
            )
            .collect())
    }

    async fn create_environment(
        &self,
        group_id: &str,
        name: &str,
        color_token: &str,
    ) -> Result<String, RanchError> {
        let now = self.now();
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;

        let group: Option<(String,)> = sqlx::query_as("SELECT id FROM groups WHERE id = ?")
            .bind(group_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(RanchError::database)?;
        if group.is_none() {
            return Err(RanchError::GroupNotFound {
                id: group_id.to_string(),
            });
        }

        let labels: Vec<(String,)> =
            sqlx::query_as("SELECT label FROM environments WHERE group_id = ?")
                .bind(group_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        let wanted = name.to_lowercase();
        if labels.iter().any(|(l,)| l.to_lowercase() == wanted) {
            return Err(RanchError::InvalidInput {
                reason: format!("An environment named \"{name}\" already exists in this group"),
            });
        }

        let resolved = resolve_hierarchy(
            &mut tx,
            &GroupSelector::Existing {
                id: group_id.to_string(),
            },
            &EnvironmentSelector::named(name, color_token),
            &now,
        )
        .await?;
        tx.commit().await.map_err(RanchError::database)?;
        Ok(resolved.environment_id)
    }

    async fn rename_environment(&self, id: &str, label: &str) -> Result<(), RanchError> {
        let now = self.now();
        let result = sqlx::query("UPDATE environments SET label = ?, updated_at = ? WHERE id = ?")
            .bind(label)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;

        if result.rows_affected() == 0 {
            return Err(RanchError::EnvironmentNotFound { id: id.to_string() });
        }

        Ok(())
    }

    async fn recolor_environment(
        &self,
        id: &str,
        color_token: &str,
    ) -> Result<Vec<String>, RanchError> {
        let now = self.now();
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;

        let result =
            sqlx::query("UPDATE environments SET color_token = ?, updated_at = ? WHERE id = ?")
                .bind(color_token)
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        if result.rows_affected() == 0 {
            return Err(RanchError::EnvironmentNotFound { id: id.to_string() });
        }

        let mut newly_locked = Vec::new();
        if production::is_production(color_token) {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT id FROM connections WHERE environment_id = ? AND is_locked = 0",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await
            .map_err(RanchError::database)?;
            newly_locked = rows.into_iter().map(|(id,)| id).collect();
            // And read-only for agents: the production defaults go together.
            sqlx::query(
                "UPDATE connections SET is_locked = 1, mcp_readonly = 1, updated_at = ? WHERE environment_id = ?",
            )
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await.map_err(RanchError::database)?;
            let all: Vec<(String,)> =
                sqlx::query_as("SELECT id FROM connections WHERE environment_id = ?")
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(RanchError::database)?;
            for (connection_id,) in all {
                reseal(&mut tx, self.sealer.as_ref(), &connection_id).await?;
            }
        }

        tx.commit().await.map_err(RanchError::database)?;
        Ok(newly_locked)
    }

    async fn delete_environment(&self, id: &str) -> Result<(), RanchError> {
        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;
        let row: Option<(String,)> = sqlx::query_as("SELECT label FROM environments WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(RanchError::database)?;
        let (label,) = row.ok_or_else(|| RanchError::EnvironmentNotFound { id: id.to_string() })?;

        let (count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM connections WHERE environment_id = ?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        if count > 0 {
            return Err(RanchError::InvalidInput {
                reason: format!(
                    "Environment \"{label}\" still contains {count} connection(s); move or remove them first"
                ),
            });
        }

        sqlx::query("DELETE FROM environments WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(RanchError::database)?;
        tx.commit().await.map_err(RanchError::database)?;
        Ok(())
    }

    async fn reorder_environments(
        &self,
        group_id: &str,
        ordered_ids: &[String],
    ) -> Result<(), RanchError> {
        let current: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM environments WHERE group_id = ?")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await
                .map_err(RanchError::database)?;
        validate_full_set("environment", &current, ordered_ids)?;

        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;
        let now = self.now();
        for (idx, id) in ordered_ids.iter().enumerate() {
            sqlx::query("UPDATE environments SET display_order = ?, updated_at = ? WHERE id = ?")
                .bind(idx as i64)
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        }
        tx.commit().await.map_err(RanchError::database)?;
        Ok(())
    }

    // ── groups ──────────────────────────────────────────────────────────

    async fn list_groups(&self) -> Result<Vec<Group>, RanchError> {
        let rows: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
            r#"
            SELECT g.id, g.name, g.display_order, g.collapsed,
                   (SELECT count(*)
                      FROM connections c
                      JOIN environments e ON c.environment_id = e.id
                     WHERE e.group_id = g.id) AS connection_count
            FROM groups g
            ORDER BY g.display_order, g.created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RanchError::database)?;

        Ok(rows
            .into_iter()
            .map(
                |(id, name, display_order, collapsed, connection_count)| Group {
                    id,
                    name,
                    display_order: display_order as i32,
                    collapsed: collapsed != 0,
                    connection_count: connection_count as i32,
                },
            )
            .collect())
    }

    async fn create_group(&self, name: &str) -> Result<String, RanchError> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = self.now();

        // Append at the end of the current order.
        let (next_order,): (i64,) =
            sqlx::query_as("SELECT COALESCE(MAX(display_order) + 1, 0) FROM groups")
                .fetch_one(&self.pool)
                .await
                .map_err(RanchError::database)?;

        sqlx::query(
            r#"
            INSERT INTO groups (id, name, label, display_order, collapsed, created_at, updated_at)
            VALUES (?, ?, 'Client', ?, 0, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(name)
        .bind(next_order)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(RanchError::database)?;
        Ok(id)
    }

    async fn rename_group(&self, id: &str, name: &str) -> Result<(), RanchError> {
        let now = self.now();
        sqlx::query("UPDATE groups SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;
        Ok(())
    }

    async fn set_group_collapsed(&self, id: &str, collapsed: bool) -> Result<(), RanchError> {
        let result = sqlx::query("UPDATE groups SET collapsed = ? WHERE id = ?")
            .bind(if collapsed { 1_i64 } else { 0_i64 })
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;
        if result.rows_affected() == 0 {
            return Err(RanchError::GroupNotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn group_connection_ids(&self, id: &str) -> Result<Vec<String>, RanchError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT c.id
            FROM connections c
            JOIN environments e ON c.environment_id = e.id
            WHERE e.group_id = ?
            ORDER BY c.created_at
            "#,
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map_err(RanchError::database)?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn delete_group(&self, id: &str) -> Result<(), RanchError> {
        // Environments go with the group via ON DELETE CASCADE.
        sqlx::query("DELETE FROM groups WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RanchError::database)?;
        Ok(())
    }

    async fn reorder_groups(&self, ordered_ids: &[String]) -> Result<(), RanchError> {
        let current: Vec<(String,)> = sqlx::query_as("SELECT id FROM groups")
            .fetch_all(&self.pool)
            .await
            .map_err(RanchError::database)?;
        validate_full_set("group", &current, ordered_ids)?;

        let mut tx = self.pool.begin().await.map_err(RanchError::database)?;
        let now = self.now();
        for (idx, id) in ordered_ids.iter().enumerate() {
            sqlx::query("UPDATE groups SET display_order = ?, updated_at = ? WHERE id = ?")
                .bind(idx as i64)
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(RanchError::database)?;
        }
        tx.commit().await.map_err(RanchError::database)?;
        Ok(())
    }
}
