//! Top-level error type for Ranch backend operations.
//!
//! The variants cover the data layer, the keychain and the Redis side. An
//! error crosses every boundary as `contract::errors::RanchErrorDto`, so
//! `RanchError` can grow without churning the wire.
//!
//! A store's own error (sqlx's, redis-rs's) is carried boxed, as a
//! [`StoreError`]: the domain names neither crate, and the adapter that
//! met the error wraps it (`RanchError::database`, `RanchError::redis`).
//! Its message and its source are the store's.

use crate::jobs::JobState;
use thiserror::Error;

/// A store's own error, boxed. See the module docs.
pub type StoreError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Error, Debug)]
pub enum RanchError {
    /// An `sqlx::Error`.
    #[error("Database error: {0}")]
    Database(#[source] StoreError),

    /// An `sqlx::migrate::MigrateError`.
    #[error("Database migration failed: {0}")]
    Migration(#[source] StoreError),

    #[error("Cannot reach the OS keychain: {reason}. See {doc_url} for setup instructions.")]
    KeychainUnavailable { reason: String, doc_url: String },

    /// The secret store refused or failed (the OS keychain on the desktop, the
    /// secrets file in the server). A message, not the `keyring` error type:
    /// the core must build without that crate (it is an optional dependency).
    #[error("Keychain operation failed: {0}")]
    Keychain(String),

    #[error("Cannot resolve or create the application data directory: {0}")]
    AppData(#[from] std::io::Error),

    /// Surfaced by `parse_redis_url` when the user pastes garbage
    /// into the URL field. The frontend renders `reason` inline.
    #[error("Invalid Redis URL: {reason}")]
    InvalidRedisUrl { reason: String },

    /// low-level `redis-rs` errors (a `redis::RedisError`) that bubble up
    /// unchanged. `test_connection` maps these into a structured
    /// `TestConnectionError` before returning to the frontend, but other
    /// commands propagate them as they are.
    #[error("Redis client error: {0}")]
    Redis(#[source] StoreError),

    /// Surfaced when a command is asked to operate on a connection
    /// id that no longer exists (e.g. deleted out-of-band between sidebar
    /// fetch and ping).
    #[error("Connection not found: {id}")]
    ConnectionNotFound { id: String },

    /// Surfaced when `rename_environment` (or any future env-
    /// targeted command) is given an unknown environment id.
    #[error("Environment not found: {id}")]
    EnvironmentNotFound { id: String },

    /// A group-targeted command was given an unknown group id.
    #[error("Group not found: {id}")]
    GroupNotFound { id: String },

    /// `delete_group` without `cascade` on a group that still
    /// holds connections. The UI answers with the confirmation dialog that
    /// lists them, then retries with `cascade = true`.
    #[error("Group \"{name}\" still contains {connection_count} connection(s)")]
    GroupNotEmpty { name: String, connection_count: u32 },

    /// Surfaced when a command's input fails a validation check
    /// before the IO step (empty label, out-of-range int, etc.). The
    /// frontend's Zod schema catches these first; this is the backend's
    /// belt-and-braces guard for direct MCP callers / programmatic use.
    #[error("Invalid input: {reason}")]
    InvalidInput { reason: String },

    /// A job search / sort scan stopped because a newer one was
    /// requested on the same connection. Not a failure: the caller
    /// has already moved on; clients must not retry it.
    #[error("Superseded by a newer job query")]
    Superseded,

    /// The operation was stopped before it finished: the caller went away
    /// or the user cancelled it.
    #[error("Operation cancelled")]
    Cancelled,

    /// The connection's actor is gone or never answered (shut down, or the
    /// runtime is exiting).
    #[error("Connection actor error: {reason}")]
    BackendUnavailable { reason: String },

    /// The connection's queue backend lacks what the operation needs.
    /// `capability` names it (`queueControl`, `schedules.pause`…).
    #[error("This connection's queue backend does not support {capability}.")]
    Unsupported { capability: String },

    /// BullMQ schema not recognized for any discovered queue
    /// on the connection. Carries the raw `version` field from the first
    /// queue's `bull:{q}:meta` hash so the UI can show the user-facing
    /// "BullMQ v7.0.0 is not yet supported: upgrade Ranch or open a
    /// compatibility issue."
    #[error("BullMQ schema not recognized (detected version: {detected_version:?})")]
    SchemaNotRecognized { detected_version: Option<String> },

    /// Surfaced by `get_job` when `HGETALL bull:{queue}:{id}`
    /// returns an empty hash. The job was either deleted between the
    /// list-jobs fetch and the drill-down click (common race), or the
    /// user deep-linked a stale URL. The UI renders an empty state with
    /// a "← Back to queue" link instead of retrying.
    #[error("Job not found: {queue}/{id}")]
    JobNotFound { queue: String, id: String },

    /// A destructive command was rejected because the
    /// connection is read-only (Locked) and the user hasn't unlocked
    /// for the session. The frontend renders the `unlock_hint` so the
    /// user knows the keystroke. MCP clients get the same DTO
    /// and can surface it to agents.
    #[error("Connection {connection_name} is locked: {unlock_hint}")]
    ConnectionLocked {
        connection_id: String,
        connection_name: String,
        unlock_hint: String,
    },

    /// An MCP write tool targeted a connection that is MCP
    /// read-only (its own flag, or the global default). Step 1 of the MCP
    /// safety gate. The message names the setting so the agent
    /// can tell the human exactly what to change.
    #[error(
        "Blocked by Ranch: connection \"{connection_name}\" is read-only for agents, so `{tool}` was not executed. A human can allow agent writes for this connection in Ranch › Agents › Setup & safety."
    )]
    BlockedReadonly {
        connection_name: String,
        tool: String,
    },

    /// A process that checks seals (`ranch-mcp`) found a connection whose
    /// endpoint, lock or agent setting the app did not seal: it opens
    /// nothing, reads included, as its password would go wherever the row
    /// points.
    #[error(
        "Ranch cannot verify connection \"{connection_name}\" (its settings were not saved by the Ranch app), so agents cannot use it. Ask the user to open Ranch, check this connection's settings and save them."
    )]
    ConnectionUnverified { connection_name: String },

    /// The human denied the agent's request in Ranch.
    #[error("Denied by the user in Ranch: `{tool}` was not executed.{}", reason.as_deref().map(|r| format!(" Reason: {r}")).unwrap_or_default())]
    ApprovalDenied {
        tool: String,
        reason: Option<String>,
    },

    /// Nobody answered the approval request in time.
    #[error(
        "Approval timed out after {seconds}s: `{tool}` was not executed. The user has to approve agent writes in the Ranch app."
    )]
    ApprovalTimeout { tool: String, seconds: u32 },

    /// The Ranch app is not running (or closed while the request
    /// was waiting), so nobody can approve. Fails fast instead of waiting
    /// for the timeout.
    #[error(
        "The Ranch app is not running, so `{tool}` cannot be approved and was not executed. Ask the user to open Ranch, then try again."
    )]
    HostUnavailable { tool: String },

    /// AI features are off (Settings › AI features): every MCP
    /// tool, read or write, is refused.
    #[error(
        "AI features are turned off in Ranch, so `{tool}` was not executed. Only a human can turn them back on, in Ranch › Settings › AI features."
    )]
    AiDisabled { tool: String },

    /// The connection actor refused an approval grant: unknown,
    /// not approved, already used, or issued for another connection / tool.
    /// Never expected in normal operation.
    #[error("The approval for `{tool}` is not valid (unknown, expired, denied or already used); nothing was executed.")]
    ApprovalInvalid { tool: String },

    /// A single-job destructive op (retry / remove / promote)
    /// was rejected because the job is not in the required state. E.g.
    /// retry requires `failed`, promote requires `delayed`. The frontend
    /// disables the button for visual feedback, but the backend ALSO
    /// guards so MCP / programmatic callers can't bypass.
    #[error(
        "Cannot {action} job {job_id}: current state is {current_state}, required {required_state}"
    )]
    InvalidJobState {
        action: String,
        connection_id: String,
        queue_name: String,
        job_id: String,
        current_state: JobState,
        required_state: JobState,
    },

    /// A scheduler-targeted op (`pause_scheduler` /
    /// `resume_scheduler`) could not locate the scheduler key in the
    /// `:repeat` ZSET. Surfaces when the user races with a concurrent
    /// removal (BullMQ's spawner ran and re-armed the scheduler with
    /// a different key, or another client deleted it).
    #[error("Scheduler not found: {queue_name}/{scheduler_key}")]
    SchedulerNotFound {
        connection_id: String,
        queue_name: String,
        scheduler_key: String,
    },

    /// The bastion rejected the SSH credential. Either the
    /// password was wrong, the public key wasn't accepted, or no
    /// credential was supplied. Surfaced verbatim through a toast / the
    /// form's error switch (`kind: "sshAuthFailed"`).
    #[error("SSH authentication failed: {reason}")]
    SshAuthFailed { reason: String },

    /// couldn't read or parse the SSH key file. `path` is
    /// the path the user typed; `reason` carries the underlying IO or
    /// parse error from `tokio::fs` / russh-keys.
    #[error("SSH key file error at {path}: {reason}")]
    SshKeyFileError { path: String, reason: String },

    /// catch-all for the SSH tunnel session itself: bastion
    /// unreachable, host-key mismatch, channel-open refused, jump-host
    /// hop failure, etc. The `reason` carries the disambiguation.
    #[error("SSH tunnel failed: {reason}")]
    SshTunnelFailed { reason: String },
}

impl RanchError {
    /// A database error (`sqlx::Error`), as [`RanchError::Database`].
    pub fn database(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Database(Box::new(e))
    }

    /// A failed migration (`sqlx::migrate::MigrateError`), as
    /// [`RanchError::Migration`].
    pub fn migration(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Migration(Box::new(e))
    }

    /// A Redis client error (`redis::RedisError`), as [`RanchError::Redis`].
    pub fn redis(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Redis(Box::new(e))
    }
}
