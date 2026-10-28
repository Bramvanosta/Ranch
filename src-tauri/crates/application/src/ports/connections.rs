use async_trait::async_trait;
use ranch_domain::endpoint::{BackendKind, Endpoint};
use ranch_domain::errors::RanchError;
use ranch_domain::hierarchy::{
    Connection, Environment, EnvironmentSelector, Group, GroupSelector, MoveConnectionResult,
};

use super::sealer::Sealer;

/// Work that must succeed for a write to be kept. It runs inside the
/// write's transaction, after the rows and before the commit, so a failure
/// leaves nothing behind: the secrets go to the keychain there.
pub type BeforeCommit<'a> = &'a (dyn Fn() -> Result<(), RanchError> + Send + Sync);

/// The same, for a copy: it is given the id of the connection being
/// copied.
pub type CopySecrets<'a> = &'a (dyn Fn(&str) -> Result<(), RanchError> + Send + Sync);

/// A connection to save. The lock and the agent setting it starts with
/// are decided by `domain::production` from the environment it lands in.
#[derive(Debug, Clone)]
pub struct NewConnection {
    pub id: String,
    pub name: String,
    pub backend: BackendKind,
    /// Checked by the backend of its kind before it gets here.
    pub endpoint: Endpoint,
    pub group: GroupSelector,
    pub environment: EnvironmentSelector,
    /// An explicit lock choice; `None` takes the production default.
    pub locked: Option<bool>,
}

/// An edit of a saved connection: its backend and endpoint are replaced
/// whole.
#[derive(Debug, Clone)]
pub struct ConnectionEdit {
    pub id: String,
    pub name: String,
    pub backend: BackendKind,
    /// Checked by the backend of its kind before it gets here.
    pub endpoint: Endpoint,
    pub group: GroupSelector,
    pub environment: EnvironmentSelector,
    /// An explicit lock choice; `None` leaves the lock (moving into
    /// Production still locks).
    pub locked: Option<bool>,
}

/// What a connection's backend is opened with, as stored (secrets apart).
/// Its password is the secret named after its id
/// (`ports::secrets::password_entry`).
#[derive(Debug, Clone)]
pub struct StoredSettings {
    pub backend: BackendKind,
    pub endpoint: Endpoint,
    pub locked: bool,
    pub name: String,
    pub env_color: Option<String>,
    /// The backend, the endpoint and the lock as the app sealed them. A
    /// process that checks seals opens nothing else.
    pub sealed: bool,
}

/// What decides whether agents may write to a connection, and whether the
/// app sealed it.
#[derive(Debug, Clone)]
pub struct AgentAccess {
    /// `None`: the connection follows the global default.
    pub mcp_readonly: Option<bool>,
    pub locked: bool,
    pub name: String,
    /// The agent read-only flag, the lock and the endpoint as the app
    /// sealed them.
    pub sealed: bool,
}

/// The saved connections and the hierarchy they sit in: groups, their
/// environments, and the connections in those. Every write of a
/// connection's lock or agent flag seals it in the same transaction.
// `async_trait` marks the boxed future `must_use` on top of the `Result`.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ConnectionRepository: Send + Sync {
    // ── connections ─────────────────────────────────────────────────────
    /// Every connection, in sidebar order.
    async fn list(&self) -> Result<Vec<Connection>, RanchError>;
    async fn get(&self, id: &str) -> Result<Option<Connection>, RanchError>;
    async fn stored_settings(&self, id: &str) -> Result<Option<StoredSettings>, RanchError>;
    async fn agent_access(&self, id: &str) -> Result<Option<AgentAccess>, RanchError>;
    /// Seal every connection as it stands: the first run with a key.
    async fn reseal_all(&self) -> Result<(), RanchError>;
    /// Seal again, with the endpoint, every connection whose seal from
    /// before the endpoint was sealed `verifier` finds valid; a connection
    /// whose seal does not hold stays unsealed. Returns how many were
    /// carried over.
    async fn upgrade_seals(&self, verifier: &dyn Sealer) -> Result<usize, RanchError>;
    /// Save a new connection, placing it (creating a `New` group or
    /// environment), then run `before_commit`.
    async fn insert(
        &self,
        new: NewConnection,
        before_commit: BeforeCommit<'_>,
    ) -> Result<Connection, RanchError>;
    async fn update(
        &self,
        edit: ConnectionEdit,
        before_commit: BeforeCommit<'_>,
    ) -> Result<Connection, RanchError>;
    /// Copy a connection, its protections included, under `new_id` and the
    /// next free "(copy)" name in its environment.
    async fn duplicate(
        &self,
        source_id: &str,
        new_id: &str,
        copy_secrets: CopySecrets<'_>,
    ) -> Result<Connection, RanchError>;
    async fn delete(&self, id: &str) -> Result<(), RanchError>;
    async fn set_collapsed(&self, id: &str, collapsed: bool) -> Result<(), RanchError>;
    async fn set_mcp_readonly(
        &self,
        id: &str,
        mcp_readonly: Option<bool>,
    ) -> Result<(), RanchError>;
    async fn move_connection(
        &self,
        id: &str,
        environment_id: &str,
    ) -> Result<MoveConnectionResult, RanchError>;

    // ── environments ────────────────────────────────────────────────────
    async fn list_environments(&self) -> Result<Vec<Environment>, RanchError>;
    async fn create_environment(
        &self,
        group_id: &str,
        name: &str,
        color_token: &str,
    ) -> Result<String, RanchError>;
    async fn rename_environment(&self, id: &str, label: &str) -> Result<(), RanchError>;
    /// Recolor an environment. Turning it Production locks its connections
    /// and makes them read-only for agents; returns the ones newly locked.
    async fn recolor_environment(
        &self,
        id: &str,
        color_token: &str,
    ) -> Result<Vec<String>, RanchError>;
    /// Only an empty environment can be deleted.
    async fn delete_environment(&self, id: &str) -> Result<(), RanchError>;
    async fn reorder_environments(
        &self,
        group_id: &str,
        ordered_ids: &[String],
    ) -> Result<(), RanchError>;

    // ── groups ──────────────────────────────────────────────────────────
    async fn list_groups(&self) -> Result<Vec<Group>, RanchError>;
    /// Returns the new group's id.
    async fn create_group(&self, name: &str) -> Result<String, RanchError>;
    async fn rename_group(&self, id: &str, name: &str) -> Result<(), RanchError>;
    async fn set_group_collapsed(&self, id: &str, collapsed: bool) -> Result<(), RanchError>;
    /// The connections of a group, oldest first.
    async fn group_connection_ids(&self, id: &str) -> Result<Vec<String>, RanchError>;
    /// Delete a group; its environments go with it.
    async fn delete_group(&self, id: &str) -> Result<(), RanchError>;
    async fn reorder_groups(&self, ordered_ids: &[String]) -> Result<(), RanchError>;
}
