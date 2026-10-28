//! The MCP activity log, see migration `0007`.
//!
//! `ranch-mcp` records every tool call here, reads included; the GUI lists,
//! filters, exports and clears. Recording is best-effort by design: a failure
//! to log must never fail (or delay) the agent's call.

use std::time::Duration;

use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::application::ports::SettingsStore;
use crate::core::mcp::gate::ToolKind;
use crate::domain::errors::RanchError;

pub const ACTIVITY_ENABLED_KEY: &str = "mcp_activity_enabled";
pub const ACTIVITY_MAX_ENTRIES_KEY: &str = "mcp_activity_max_entries";
pub const SESSION_GAP_MINUTES_KEY: &str = "mcp_session_gap_minutes";
pub const DEFAULT_ACTIVITY_MAX_ENTRIES: u32 = 5000;
pub const DEFAULT_SESSION_GAP_MINUTES: u32 = 10;
/// Arguments larger than this are compacted before being stored.
const TARGET_JSON_MAX_CHARS: usize = 4096;
/// How many ids of an id list survive compaction.
const COMPACTED_IDS_KEPT: usize = 20;

/// Outcome of a tool call as the log records it. The four middle values are
/// the gate's verdicts (`data.kind` of the MCP error). Serialized as the
/// JSON export writes it.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Success,
    Error,
    BlockedReadonly,
    Denied,
    Timeout,
    HostUnavailable,
}

impl ActivityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::BlockedReadonly => "blocked_readonly",
            Self::Denied => "denied",
            Self::Timeout => "timeout",
            Self::HostUnavailable => "host_unavailable",
        }
    }

    /// From the machine-readable `data.kind` of an MCP error.
    pub fn from_error_kind(kind: Option<&str>) -> Self {
        match kind {
            Some("blockedReadonly") => Self::BlockedReadonly,
            Some("approvalDenied") => Self::Denied,
            Some("approvalTimeout") => Self::Timeout,
            Some("hostUnavailable") => Self::HostUnavailable,
            _ => Self::Error,
        }
    }

    fn parse(raw: &str) -> Self {
        match raw {
            "success" => Self::Success,
            "blocked_readonly" => Self::BlockedReadonly,
            "denied" => Self::Denied,
            "timeout" => Self::Timeout,
            "host_unavailable" => Self::HostUnavailable,
            _ => Self::Error,
        }
    }
}

/// What the server hands over after a tool call.
#[derive(Debug, Clone)]
pub struct NewActivity {
    pub session_id: String,
    pub client_hint: Option<String>,
    pub tool: String,
    pub kind: ToolKind,
    pub connection_id: Option<String>,
    /// The tool arguments as the agent sent them.
    pub arguments: Option<serde_json::Value>,
    pub status: ActivityStatus,
    pub duration: Duration,
    pub error: Option<String>,
    pub approval_id: Option<String>,
}

/// One tool call. Serialized as the JSON export ([`export`]) writes it,
/// the shape `contract::mcp::ActivityEvent` sends.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub session_id: String,
    pub timestamp: String,
    pub client_hint: Option<String>,
    pub tool: String,
    /// `"read"` or `"write"`.
    pub kind: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    /// JSON text of the (possibly compacted) arguments.
    pub target_json: Option<String>,
    pub status: ActivityStatus,
    pub duration_ms: u32,
    pub error: Option<String>,
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityFilter {
    /// RFC 3339, inclusive.
    pub from: Option<String>,
    pub to: Option<String>,
    pub client_hint: Option<String>,
    pub connection_id: Option<String>,
    /// `"read"` or `"write"`.
    pub kind: Option<String>,
    pub status: Option<ActivityStatus>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySession {
    pub session_id: String,
    pub client_hint: Option<String>,
    pub started_at: String,
    pub last_at: String,
    pub total: u32,
    pub writes: u32,
    /// Calls that did not succeed, whatever the reason.
    pub not_successful: u32,
}

async fn kv_u32(settings: &dyn SettingsStore, key: &str, default: u32) -> Result<u32, RanchError> {
    Ok(settings
        .get(key)
        .await?
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default))
}

pub async fn activity_enabled(settings: &dyn SettingsStore) -> Result<bool, RanchError> {
    Ok(settings
        .get(ACTIVITY_ENABLED_KEY)
        .await?
        .map(|v| v != "false")
        .unwrap_or(true))
}

pub async fn session_gap(settings: &dyn SettingsStore) -> Result<Duration, RanchError> {
    let minutes = kv_u32(
        settings,
        SESSION_GAP_MINUTES_KEY,
        DEFAULT_SESSION_GAP_MINUTES,
    )
    .await?;
    Ok(Duration::from_secs(minutes as u64 * 60))
}

/// Keep stored arguments small: a bulk call can carry 1,000 ids. Long id
/// lists become `{ count, first: [...] }`; if it is still too large, only a
/// marker is kept.
pub fn compact_arguments(arguments: &serde_json::Value) -> String {
    let raw = arguments.to_string();
    if raw.chars().count() <= TARGET_JSON_MAX_CHARS {
        return raw;
    }
    let mut compacted = arguments.clone();
    if let Some(map) = compacted.as_object_mut() {
        for (_, value) in map.iter_mut() {
            if let Some(list) = value.as_array() {
                if list.len() > COMPACTED_IDS_KEPT {
                    *value = serde_json::json!({
                        "count": list.len(),
                        "first": list.iter().take(COMPACTED_IDS_KEPT).collect::<Vec<_>>(),
                    });
                }
            }
        }
    }
    let raw = compacted.to_string();
    if raw.chars().count() <= TARGET_JSON_MAX_CHARS {
        raw
    } else {
        serde_json::json!({ "truncated": true, "originalChars": arguments.to_string().len() })
            .to_string()
    }
}

/// Append one event, then enforce the cap. No-op when the log is disabled.
pub async fn record(
    pool: &SqlitePool,
    settings: &dyn SettingsStore,
    new: NewActivity,
) -> Result<(), RanchError> {
    if !activity_enabled(settings).await? {
        return Ok(());
    }
    let connection_name: Option<String> = match &new.connection_id {
        Some(id) => sqlx::query_as::<_, (String,)>("SELECT name FROM connections WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(RanchError::database)?
            .map(|(n,)| n),
        None => None,
    };

    sqlx::query(
        r#"
        INSERT INTO mcp_activity_events (
            id, session_id, timestamp, client_hint, tool, kind, connection_id,
            connection_name, target_json, status, duration_ms, error, approval_id
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&new.session_id)
    .bind(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    .bind(&new.client_hint)
    .bind(&new.tool)
    .bind(match new.kind {
        ToolKind::Read => "read",
        ToolKind::Write => "write",
    })
    .bind(&new.connection_id)
    .bind(connection_name)
    .bind(new.arguments.as_ref().map(compact_arguments))
    .bind(new.status.as_str())
    .bind(new.duration.as_millis().min(u32::MAX as u128) as i64)
    .bind(&new.error)
    .bind(&new.approval_id)
    .execute(pool)
    .await
    .map_err(RanchError::database)?;

    let max = kv_u32(
        settings,
        ACTIVITY_MAX_ENTRIES_KEY,
        DEFAULT_ACTIVITY_MAX_ENTRIES,
    )
    .await?;
    sqlx::query(
        r#"
        DELETE FROM mcp_activity_events
         WHERE id IN (
            SELECT id FROM mcp_activity_events
             ORDER BY timestamp DESC, id DESC
             LIMIT -1 OFFSET ?
         )
        "#,
    )
    .bind(max as i64)
    .execute(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(())
}

fn push_filter<'a>(qb: &mut QueryBuilder<'a, Sqlite>, filter: &'a ActivityFilter) {
    qb.push(" WHERE 1=1");
    if let Some(v) = &filter.from {
        qb.push(" AND timestamp >= ").push_bind(v);
    }
    if let Some(v) = &filter.to {
        qb.push(" AND timestamp <= ").push_bind(v);
    }
    if let Some(v) = &filter.client_hint {
        qb.push(" AND client_hint = ").push_bind(v);
    }
    if let Some(v) = &filter.connection_id {
        qb.push(" AND connection_id = ").push_bind(v);
    }
    if let Some(v) = &filter.kind {
        qb.push(" AND kind = ").push_bind(v);
    }
    if let Some(v) = &filter.status {
        qb.push(" AND status = ").push_bind(v.as_str());
    }
    if let Some(v) = &filter.session_id {
        qb.push(" AND session_id = ").push_bind(v);
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: String,
    session_id: String,
    timestamp: String,
    client_hint: Option<String>,
    tool: String,
    kind: String,
    connection_id: Option<String>,
    connection_name: Option<String>,
    target_json: Option<String>,
    status: String,
    duration_ms: i64,
    error: Option<String>,
    approval_id: Option<String>,
}

impl From<EventRow> for ActivityEvent {
    fn from(r: EventRow) -> Self {
        Self {
            id: r.id,
            session_id: r.session_id,
            timestamp: r.timestamp,
            client_hint: r.client_hint,
            tool: r.tool,
            kind: r.kind,
            connection_id: r.connection_id,
            connection_name: r.connection_name,
            target_json: r.target_json,
            status: ActivityStatus::parse(&r.status),
            duration_ms: r.duration_ms.clamp(0, u32::MAX as i64) as u32,
            error: r.error,
            approval_id: r.approval_id,
        }
    }
}

/// Newest first.
pub async fn list(
    pool: &SqlitePool,
    filter: &ActivityFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<ActivityEvent>, RanchError> {
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM mcp_activity_events");
    push_filter(&mut qb, filter);
    qb.push(" ORDER BY timestamp DESC, id DESC LIMIT ")
        .push_bind(limit.clamp(1, 1000) as i64)
        .push(" OFFSET ")
        .push_bind(offset as i64);
    let rows: Vec<EventRow> = qb
        .build_query_as()
        .fetch_all(pool)
        .await
        .map_err(RanchError::database)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count(pool: &SqlitePool, filter: &ActivityFilter) -> Result<u32, RanchError> {
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT count(*) FROM mcp_activity_events");
    push_filter(&mut qb, filter);
    let (n,): (i64,) = qb
        .build_query_as()
        .fetch_one(pool)
        .await
        .map_err(RanchError::database)?;
    Ok(n.max(0) as u32)
}

/// Sessions, most recent first.
pub async fn list_sessions(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<ActivitySession>, RanchError> {
    type SessionRow = (String, Option<String>, String, String, i64, i64, i64);
    let rows: Vec<SessionRow> = sqlx::query_as(
        r#"
        SELECT session_id,
               max(client_hint),
               min(timestamp),
               max(timestamp),
               count(*),
               sum(CASE WHEN kind = 'write' THEN 1 ELSE 0 END),
               sum(CASE WHEN status <> 'success' THEN 1 ELSE 0 END)
          FROM mcp_activity_events
         GROUP BY session_id
         ORDER BY max(timestamp) DESC
         LIMIT ?
        "#,
    )
    .bind(limit.clamp(1, 500) as i64)
    .fetch_all(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(rows
        .into_iter()
        .map(|r| ActivitySession {
            session_id: r.0,
            client_hint: r.1,
            started_at: r.2,
            last_at: r.3,
            total: r.4.max(0) as u32,
            writes: r.5.max(0) as u32,
            not_successful: r.6.max(0) as u32,
        })
        .collect())
}

/// Empty the log. UI-only, and on the MCP deny-list: an agent must never be
/// able to erase its own trail. (The audit log cannot be cleared at all.)
pub async fn clear(pool: &SqlitePool) -> Result<u64, RanchError> {
    Ok(sqlx::query("DELETE FROM mcp_activity_events")
        .execute(pool)
        .await
        .map_err(RanchError::database)?
        .rows_affected())
}

fn csv_cell(value: &str) -> String {
    // Neutralise spreadsheet formulas: agent-controlled text ends up here.
    let needs_guard = matches!(
        value.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    );
    let guarded = if needs_guard {
        format!("'{value}")
    } else {
        value.to_string()
    };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

/// Export the filtered log (up to 10,000 events, newest first) as JSON or CSV.
pub async fn export(
    pool: &SqlitePool,
    filter: &ActivityFilter,
    as_csv: bool,
) -> Result<String, RanchError> {
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM mcp_activity_events");
    push_filter(&mut qb, filter);
    qb.push(" ORDER BY timestamp DESC, id DESC LIMIT 10000");
    let rows: Vec<EventRow> = qb
        .build_query_as()
        .fetch_all(pool)
        .await
        .map_err(RanchError::database)?;
    let events: Vec<ActivityEvent> = rows.into_iter().map(Into::into).collect();

    if !as_csv {
        return serde_json::to_string_pretty(&events).map_err(|e| RanchError::InvalidInput {
            reason: format!("Could not serialise the activity log: {e}"),
        });
    }
    let mut out = String::from(
        "timestamp,session_id,client,tool,kind,connection,status,duration_ms,approval_id,error,target\n",
    );
    for e in &events {
        let cells = [
            e.timestamp.as_str(),
            e.session_id.as_str(),
            e.client_hint.as_deref().unwrap_or(""),
            e.tool.as_str(),
            e.kind.as_str(),
            e.connection_name.as_deref().unwrap_or(""),
            e.status.as_str(),
            &e.duration_ms.to_string(),
            e.approval_id.as_deref().unwrap_or(""),
            e.error.as_deref().unwrap_or(""),
            e.target_json.as_deref().unwrap_or(""),
        ]
        .map(csv_cell);
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    Ok(out)
}
