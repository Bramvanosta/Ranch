//! The MCP approval queue.
//!
//! See `crates/sqlite/migrations/0006_mcp_pending_approvals.sql` for the
//! state machine. `ranch-mcp` enqueues and polls; the GUI lists and decides;
//! the connection actor consumes, through the write pipeline's
//! `ApprovalStore` (`adapters::sqlite::approvals`). Every transition is a conditional `UPDATE` whose
//! affected-row count is the verdict: that is what makes a forged, expired,
//! denied or already-used id worthless.

use std::time::Duration;

use sqlx::SqlitePool;

use crate::application::ports::sealer::{approval_parts, ApprovalShown, Check, Sealer};
use crate::application::ports::settings::{SafetySetting, Sealed};
use crate::application::ports::SettingsStore;
use crate::core::mcp::gate::ToolKind;
use crate::core::mcp::heartbeat::is_gui_alive;
use crate::domain::errors::RanchError;

pub const APPROVAL_MODE_KEY: &str = SafetySetting::ApprovalMode.key();
pub const APPROVAL_TIMEOUT_KEY: &str = "mcp_approval_timeout_seconds";
pub const DEFAULT_APPROVAL_TIMEOUT_SECONDS: u32 = 120;
/// Bounds accepted for the timeout setting.
pub const MIN_APPROVAL_TIMEOUT_SECONDS: u32 = 10;
pub const MAX_APPROVAL_TIMEOUT_SECONDS: u32 = 3600;
pub const APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Requests older than this are purged by the GUI…
pub const APPROVAL_RETENTION: Duration = Duration::from_secs(3600);
/// …except a decided one (approved, denied, consumed), kept this long so the
/// audit log can say who approved a write and after how long.
pub const DECISION_RETENTION: Duration = Duration::from_secs(90 * 24 * 3600);

/// Global approval mode: off, writes only, or all. Not per connection: the
/// per-connection dimension is MCP read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Never ask. Writes fall through to the session lock.
    Off,
    /// Ask for every write tool. **Default.**
    WritesOnly,
    /// Ask for every tool, reads included.
    All,
}

impl ApprovalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::WritesOnly => "writes_only",
            Self::All => "all",
        }
    }

    /// Unknown / missing values mean the default, never `Off`.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("off") => Self::Off,
            Some("all") => Self::All,
            _ => Self::WritesOnly,
        }
    }

    pub fn requires_approval(self, kind: ToolKind) -> bool {
        match self {
            Self::Off => false,
            Self::WritesOnly => kind == ToolKind::Write,
            Self::All => true,
        }
    }
}

pub async fn approval_mode(settings: &dyn SettingsStore) -> Result<ApprovalMode, RanchError> {
    let Sealed { value, check } = settings.safety(SafetySetting::ApprovalMode).await?;
    // A mode the app did not seal asks for every write, like an unreadable one.
    if check != Check::Valid {
        return Ok(ApprovalMode::parse(None));
    }
    Ok(ApprovalMode::parse(value.as_deref()))
}

pub async fn approval_timeout(settings: &dyn SettingsStore) -> Result<Duration, RanchError> {
    let seconds = settings
        .get(APPROVAL_TIMEOUT_KEY)
        .await?
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_APPROVAL_TIMEOUT_SECONDS)
        .clamp(MIN_APPROVAL_TIMEOUT_SECONDS, MAX_APPROVAL_TIMEOUT_SECONDS);
    Ok(Duration::from_secs(seconds as u64))
}

/// What `ranch-mcp` writes when it needs a decision.
#[derive(Debug, Clone)]
pub struct NewApproval {
    pub client_hint: Option<String>,
    pub connection_id: String,
    pub tool: String,
    pub kind: ToolKind,
    /// The tool's arguments, as the agent sent them.
    pub arguments: serde_json::Value,
    /// What the human needs to see to decide (job summary, affected count…).
    pub target_preview: Option<serde_json::Value>,
}

/// A request as the GUI shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub id: String,
    pub created_at: String,
    pub client_hint: Option<String>,
    pub connection_id: String,
    pub connection_name: String,
    pub group_name: String,
    pub environment_name: String,
    pub env_color: String,
    pub tool: String,
    /// `"read"` or `"write"`.
    pub kind: String,
    /// JSON text of the tool arguments.
    pub arguments_json: String,
    /// JSON text of the target preview, when one could be built.
    pub target_preview_json: Option<String>,
    /// Seconds after `created_at` at which the agent gives up.
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

/// Result of a GUI decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecideOutcome {
    Decided,
    /// The request was no longer pending (already decided, expired, purged).
    /// A no-op by design: the UI just moves on.
    AlreadyDecided,
}

/// What `ranch-mcp` learns from polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Approved,
    Denied {
        reason: Option<String>,
    },
    TimedOut,
    /// The GUI went away while we were waiting.
    HostUnavailable,
}

/// Requests waiting for the human at once. Each one raises a sheet, a
/// notification and a sound: an agent firing calls in parallel must not bury
/// the user under them until they approve without reading.
pub const MAX_PENDING: i64 = 5;

/// How a request is shown on the approval sheet: the connection as it was
/// displayed when the request was written, and the preview. `ranch-mcp`
/// keeps what it wrote, and the app's seal covers it, so a row rewritten
/// before the human decides runs nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownTarget {
    pub connection_name: String,
    pub group_name: String,
    pub environment_name: String,
    pub env_color: String,
    pub target_preview_json: Option<String>,
}

impl ShownTarget {
    fn parts(&self) -> ApprovalShown<'_> {
        ApprovalShown {
            connection_name: &self.connection_name,
            group_name: &self.group_name,
            environment_name: &self.environment_name,
            env_color: &self.env_color,
            target_preview_json: self.target_preview_json.as_deref(),
        }
    }
}

/// Insert a `pending` request, snapshotting how the connection is displayed.
pub async fn enqueue(
    pool: &SqlitePool,
    settings: &dyn SettingsStore,
    new: &NewApproval,
) -> Result<String, RanchError> {
    enqueue_shown(pool, settings, new).await.map(|(id, _)| id)
}

/// [`enqueue`], and how the request is shown: what the app's seal on its
/// approval must cover ([`sealed_by_the_app`]).
pub async fn enqueue_shown(
    pool: &SqlitePool,
    settings: &dyn SettingsStore,
    new: &NewApproval,
) -> Result<(String, ShownTarget), RanchError> {
    // AI features may have been turned off since the call passed its first
    // check: nothing new waits for a decision then.
    if !crate::core::mcp::gate::ai_enabled(settings).await {
        return Err(RanchError::AiDisabled {
            tool: new.tool.clone(),
        });
    }
    let (pending,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM mcp_pending_approvals WHERE status = 'pending'")
            .fetch_one(pool)
            .await
            .map_err(RanchError::database)?;
    if pending >= MAX_PENDING {
        return Err(RanchError::InvalidInput {
            reason: format!(
                "{pending} requests already wait for the user's approval in Ranch: wait for their decision before asking again"
            ),
        });
    }
    let snapshot: Option<(String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT c.name, g.name, e.label, e.color_token
        FROM connections c
        JOIN environments e ON c.environment_id = e.id
        JOIN groups g ON e.group_id = g.id
        WHERE c.id = ?
        "#,
    )
    .bind(&new.connection_id)
    .fetch_optional(pool)
    .await
    .map_err(RanchError::database)?;
    let (connection_name, group_name, environment_name, env_color) =
        snapshot.ok_or_else(|| RanchError::ConnectionNotFound {
            id: new.connection_id.clone(),
        })?;
    let shown = ShownTarget {
        connection_name,
        group_name,
        environment_name,
        env_color,
        target_preview_json: new.target_preview.as_ref().map(|v| v.to_string()),
    };

    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        r#"
        INSERT INTO mcp_pending_approvals (
            id, created_at, client_hint, connection_id, connection_name, group_name,
            environment_name, env_color, tool, kind, arguments_json, target_preview_json, status
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending')
        "#,
    )
    .bind(&id)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&new.client_hint)
    .bind(&new.connection_id)
    .bind(&shown.connection_name)
    .bind(&shown.group_name)
    .bind(&shown.environment_name)
    .bind(&shown.env_color)
    .bind(&new.tool)
    .bind(match new.kind {
        ToolKind::Read => "read",
        ToolKind::Write => "write",
    })
    .bind(new.arguments.to_string())
    .bind(&shown.target_preview_json)
    .execute(pool)
    .await
    .map_err(RanchError::database)?;
    Ok((id, shown))
}

/// Pending requests, oldest first, the order the GUI shows them in.
pub async fn list_pending(
    pool: &SqlitePool,
    settings: &dyn SettingsStore,
) -> Result<Vec<PendingApproval>, RanchError> {
    let timeout_seconds = approval_timeout(settings).await?.as_secs() as u32;
    type Row = (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        r#"
        SELECT id, created_at, client_hint, connection_id, connection_name, group_name,
               environment_name, env_color, tool, kind, arguments_json, target_preview_json
        FROM mcp_pending_approvals
        WHERE status = 'pending'
        ORDER BY created_at, id
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(rows
        .into_iter()
        .map(|r| PendingApproval {
            id: r.0,
            created_at: r.1,
            client_hint: r.2,
            connection_id: r.3,
            connection_name: r.4,
            group_name: r.5,
            environment_name: r.6,
            env_color: r.7,
            tool: r.8,
            kind: r.9,
            arguments_json: r.10,
            target_preview_json: r.11,
            timeout_seconds,
        })
        .collect())
}

/// GUI: `pending → approved | denied`. Anything else is a no-op.
pub async fn decide(
    pool: &SqlitePool,
    sealer: &dyn Sealer,
    id: &str,
    decision: ApprovalDecision,
    reason: Option<&str>,
) -> Result<DecideOutcome, RanchError> {
    let status = match decision {
        ApprovalDecision::Approve => "approved",
        ApprovalDecision::Deny => "denied",
    };
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    // Read then write: take the write lock up front, so a concurrent
    // writer makes this wait out the busy timeout instead of failing.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(RanchError::database)?;
    // An approval carries the app's seal over what the human was shown:
    // ranch-mcp runs nothing on a status alone, which any process of the
    // user could write (`application::ports::sealer`).
    let decision_seal = match decision {
        ApprovalDecision::Deny => None,
        ApprovalDecision::Approve => {
            type Row = (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                Option<String>,
            );
            let row: Option<Row> = sqlx::query_as(
                "SELECT connection_id, tool, arguments_json, connection_name, group_name,
                        environment_name, env_color, target_preview_json
                   FROM mcp_pending_approvals
                  WHERE id = ? AND status = 'pending'",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(RanchError::database)?;
            row.and_then(|r| {
                let shown = ShownTarget {
                    connection_name: r.3,
                    group_name: r.4,
                    environment_name: r.5,
                    env_color: r.6,
                    target_preview_json: r.7,
                };
                sealer.seal(&approval_parts(id, &r.0, &r.1, &r.2, shown.parts()))
            })
        }
    };
    let result = sqlx::query(
        r#"
        UPDATE mcp_pending_approvals
           SET status = ?, decided_at = ?, reason = ?, decision_seal = ?
         WHERE id = ? AND status = 'pending'
        "#,
    )
    .bind(status)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(reason)
    .bind(decision_seal)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(RanchError::database)?;
    tx.commit().await.map_err(RanchError::database)?;
    Ok(if result.rows_affected() == 1 {
        DecideOutcome::Decided
    } else {
        DecideOutcome::AlreadyDecided
    })
}

/// `ranch-mcp`: `pending → expired`. Returns whether this call did it; if
/// not, the human decided in the same instant and that decision stands.
pub async fn expire(pool: &SqlitePool, id: &str) -> Result<bool, RanchError> {
    let result = sqlx::query(
        "UPDATE mcp_pending_approvals SET status = 'expired', decided_at = ? WHERE id = ? AND status = 'pending'",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(id)
    .execute(pool)
    .await.map_err(RanchError::database)?;
    Ok(result.rows_affected() == 1)
}

async fn read_status(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<(String, Option<String>)>, RanchError> {
    sqlx::query_as("SELECT status, reason FROM mcp_pending_approvals WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(RanchError::database)
}

/// `ranch-mcp`: whether the app sealed the approval of `id` over exactly
/// the call `new` describes, shown as `shown` (always true where seals are
/// not checked). A request written by an earlier release, whose seal did
/// not cover how it was shown, no longer verifies: it is refused, and the
/// agent asks again.
pub async fn sealed_by_the_app(
    pool: &SqlitePool,
    sealer: &dyn Sealer,
    id: &str,
    new: &NewApproval,
    shown: &ShownTarget,
) -> Result<bool, RanchError> {
    let seal: Option<(Option<String>,)> =
        sqlx::query_as("SELECT decision_seal FROM mcp_pending_approvals WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(RanchError::database)?;
    let seal = seal.and_then(|(s,)| s);
    let arguments = new.arguments.to_string();
    let parts = approval_parts(id, &new.connection_id, &new.tool, &arguments, shown.parts());
    Ok(sealer.check(&parts, seal.as_deref()) == Check::Valid)
}

fn outcome_of(status: &str, reason: Option<String>) -> Option<PollOutcome> {
    match status {
        "approved" => Some(PollOutcome::Approved),
        "denied" => Some(PollOutcome::Denied { reason }),
        "expired" | "consumed" => Some(PollOutcome::TimedOut),
        _ => None, // pending
    }
}

/// `ranch-mcp`: wait for the human, watching the GUI's heartbeat the whole
/// time. On timeout or a vanished GUI the request is expired, unless the
/// human's decision landed first, in which case it wins.
pub async fn poll_decision(
    pool: &SqlitePool,
    settings: &dyn SettingsStore,
    id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<PollOutcome, RanchError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match read_status(pool, id).await? {
            // Purged under our feet: nobody can approve it anymore.
            None => return Ok(PollOutcome::TimedOut),
            Some((status, reason)) => {
                if let Some(outcome) = outcome_of(&status, reason) {
                    return Ok(outcome);
                }
            }
        }

        let gui_gone = !is_gui_alive(settings).await?;
        let timed_out = tokio::time::Instant::now() >= deadline;
        if gui_gone || timed_out {
            if expire(pool, id).await? {
                return Ok(if gui_gone {
                    PollOutcome::HostUnavailable
                } else {
                    PollOutcome::TimedOut
                });
            }
            // Lost the race to a decision: loop once more to read it.
            continue;
        }
        tokio::time::sleep(poll_interval).await;
    }
}

/// GUI: drop requests nobody decided (pending, expired) older than
/// [`APPROVAL_RETENTION`], and decided ones older than
/// [`DECISION_RETENTION`]. Returns how many went.
pub async fn purge_old(pool: &SqlitePool) -> Result<u64, RanchError> {
    let now = chrono::Utc::now();
    let undecided =
        now - chrono::Duration::from_std(APPROVAL_RETENTION).unwrap_or(chrono::Duration::hours(1));
    let decided =
        now - chrono::Duration::from_std(DECISION_RETENTION).unwrap_or(chrono::Duration::days(90));
    let result = sqlx::query(
        r#"
        DELETE FROM mcp_pending_approvals
         WHERE (status IN ('pending', 'expired') AND created_at < ?)
            OR created_at < ?
        "#,
    )
    .bind(undecided.to_rfc3339())
    .bind(decided.to_rfc3339())
    .execute(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(result.rows_affected())
}

/// What the audit log says about the approval behind an agent
/// write: who asked, what was decided, when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub id: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    /// `pending`, `approved`, `denied`, `expired` or `consumed` (approved, then used).
    pub status: String,
    pub client_hint: Option<String>,
    pub tool: String,
    pub reason: Option<String>,
    /// The exact arguments the agent sent (JSON text).
    pub arguments_json: String,
}

/// `None` once the request was purged.
pub async fn record(pool: &SqlitePool, id: &str) -> Result<Option<ApprovalRecord>, RanchError> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, created_at, decided_at, status, client_hint, tool, reason, arguments_json
           FROM mcp_pending_approvals WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(row.map(
        |(id, created_at, decided_at, status, client_hint, tool, reason, arguments_json)| {
            ApprovalRecord {
                id,
                created_at,
                decided_at,
                status,
                client_hint,
                tool,
                reason,
                arguments_json,
            }
        },
    ))
}

/// Agents › Approvals: the requests already decided (approved,
/// denied, expired, consumed), newest first, with their connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalHistoryItem {
    pub id: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    /// `approved`, `denied`, `expired` or `consumed` (approved, then used).
    pub status: String,
    pub client_hint: Option<String>,
    pub tool: String,
    pub reason: Option<String>,
    pub connection_id: String,
    /// `None` when the connection was removed since.
    pub connection_name: Option<String>,
}

pub async fn history(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<ApprovalHistoryItem>, RanchError> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT a.id, a.created_at, a.decided_at, a.status, a.client_hint, a.tool, a.reason,
                a.connection_id, c.name
           FROM mcp_pending_approvals a
           LEFT JOIN connections c ON c.id = a.connection_id
          WHERE a.status <> 'pending'
          ORDER BY a.created_at DESC
          LIMIT ?",
    )
    .bind(i64::from(limit.clamp(1, 500)))
    .fetch_all(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                created_at,
                decided_at,
                status,
                client_hint,
                tool,
                reason,
                connection_id,
                connection_name,
            )| ApprovalHistoryItem {
                id,
                created_at,
                decided_at,
                status,
                client_hint,
                tool,
                reason,
                connection_id,
                connection_name,
            },
        )
        .collect())
}
