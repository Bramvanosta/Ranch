//! Audit log commands.
//!
//! Two read-only Tauri commands surface the audit log to the frontend.
//! Neither command exposes a write path; the only writer is
//! `adapters::sqlite::audit::record`, reached through the write pipeline's
//! `AuditLog` port, or directly by the lock toggles (`commands::safety`). There is no UI affordance, no MCP tool, no
//! Tauri command for deletion; purging audit data requires explicit
//! filesystem action on `ranch.db`.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::app_state::AppState;
use crate::contract::audit as wire;
use crate::contract::errors::RanchErrorDto;
use crate::domain::audit::{AuditAction, AuditLogEntry, AuditOutcome, AuditSource, AuditTarget};
use crate::domain::errors::RanchError;

const LIST_MAX_LIMIT: u64 = 1000;
const EXPORT_HARD_CAP: u64 = 10_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListAuditLogEntriesArgs {
    /// Inclusive lower bound on `created_at` (ISO-8601 string compare).
    pub from: Option<String>,
    /// Inclusive upper bound on `created_at`.
    pub to: Option<String>,
    pub connection_id: Option<String>,
    /// camelCase action discriminator (e.g. `"retryJob"`). `None` = all.
    pub action: Option<String>,
    /// `"ui"` | `"agent"` | `None` = all.
    pub source: Option<String>,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportAuditLogArgs {
    pub from: Option<String>,
    pub to: Option<String>,
    pub connection_id: Option<String>,
    pub action: Option<String>,
    pub source: Option<String>,
    /// `"json"` | `"csv"`.
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub content: String,
    pub suggested_filename: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_audit_log_entries(
    args: ListAuditLogEntriesArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<wire::AuditLogEntry>, RanchErrorDto> {
    list_audit_log_entries_inner(args, state.db.pool())
        .await
        .map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn export_audit_log(
    args: ExportAuditLogArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ExportResult, RanchErrorDto> {
    export_audit_log_inner(args, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn list_audit_log_entries_inner(
    args: ListAuditLogEntriesArgs,
    pool: &SqlitePool,
) -> Result<Vec<wire::AuditLogEntry>, RanchError> {
    let entries = read_audit_log(args, pool).await?;
    Ok(entries.into_iter().map(Into::into).collect())
}

/// The rows `list_audit_log_entries` shows, as the core reads them.
pub async fn read_audit_log(
    args: ListAuditLogEntriesArgs,
    pool: &SqlitePool,
) -> Result<Vec<AuditLogEntry>, RanchError> {
    let limit = args.limit.clamp(1, LIST_MAX_LIMIT);
    let offset = args.offset;
    fetch_entries(pool, &args.into_filter(), limit, offset).await
}

pub async fn export_audit_log_inner(
    args: ExportAuditLogArgs,
    pool: &SqlitePool,
) -> Result<ExportResult, RanchError> {
    let filter = args.to_filter();
    let entries = fetch_entries(pool, &filter, EXPORT_HARD_CAP, 0).await?;

    let (content, ext) = match args.format.as_str() {
        "json" => (
            serde_json::to_string_pretty(
                &entries
                    .iter()
                    .cloned()
                    .map(wire::AuditLogEntry::from)
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| RanchError::InvalidInput {
                reason: format!("JSON serialization failed: {e}"),
            })?,
            "json",
        ),
        "csv" => (entries_to_csv(&entries)?, "csv"),
        other => {
            return Err(RanchError::InvalidInput {
                reason: format!("Unsupported export format: {other:?} (expected 'json' or 'csv')"),
            });
        }
    };

    let suggested_filename = build_filename(&filter, ext);
    Ok(ExportResult {
        content,
        suggested_filename,
    })
}

/// Internal filter struct shared between list + export to keep query
/// building in one place.
#[derive(Debug, Clone, Default)]
struct Filter {
    from: Option<String>,
    to: Option<String>,
    connection_id: Option<String>,
    action: Option<String>,
    source: Option<String>,
}

impl ListAuditLogEntriesArgs {
    fn into_filter(self) -> Filter {
        Filter {
            from: self.from,
            to: self.to,
            connection_id: self.connection_id,
            action: self.action,
            source: self.source,
        }
    }
}

impl ExportAuditLogArgs {
    fn to_filter(&self) -> Filter {
        Filter {
            from: self.from.clone(),
            to: self.to.clone(),
            connection_id: self.connection_id.clone(),
            action: self.action.clone(),
            source: self.source.clone(),
        }
    }
}

async fn fetch_entries(
    pool: &SqlitePool,
    filter: &Filter,
    limit: u64,
    offset: u64,
) -> Result<Vec<AuditLogEntry>, RanchError> {
    // Build the WHERE clause via positional binds; never string-interp
    // user input into SQL.
    // The group and environment are joined, not snapshotted: they answer
    // "which of the three connections called `Lumen Studios` is this?",
    // and a renamed group should read as its current name. A deleted
    // connection leaves them NULL and the row keeps its name snapshot.
    let mut sql = String::from(
        "SELECT a.id, a.created_at, a.source, a.operator_id, a.agent_client, \
         a.agent_transport, a.action, a.target_kind, a.target_payload, a.outcome, \
         a.outcome_reason, a.connection_id, a.connection_name, a.env_color, \
         a.approval_id, g.name AS group_name, e.label AS environment_name \
         FROM audit_log_entries a \
         LEFT JOIN connections c ON c.id = a.connection_id \
         LEFT JOIN environments e ON e.id = c.environment_id \
         LEFT JOIN groups g ON g.id = e.group_id \
         WHERE 1=1",
    );
    if filter.from.is_some() {
        sql.push_str(" AND a.created_at >= ?");
    }
    if filter.to.is_some() {
        sql.push_str(" AND a.created_at <= ?");
    }
    if filter.connection_id.is_some() {
        sql.push_str(" AND a.connection_id = ?");
    }
    if filter.action.is_some() {
        sql.push_str(" AND a.action = ?");
    }
    if filter.source.is_some() {
        sql.push_str(" AND a.source = ?");
    }
    sql.push_str(" ORDER BY a.created_at DESC, a.id DESC LIMIT ? OFFSET ?");

    let mut q = sqlx::query(&sql);
    if let Some(v) = &filter.from {
        q = q.bind(v);
    }
    if let Some(v) = &filter.to {
        q = q.bind(v);
    }
    if let Some(v) = &filter.connection_id {
        q = q.bind(v);
    }
    if let Some(v) = &filter.action {
        q = q.bind(v);
    }
    if let Some(v) = &filter.source {
        q = q.bind(v);
    }
    q = q.bind(limit as i64).bind(offset as i64);

    let rows = q.fetch_all(pool).await.map_err(RanchError::database)?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        // One row that does not read (written by another version, or by
        // hand) is left out, not the whole page with it.
        match decode_entry(&row) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                let id: Option<String> = row.try_get("id").ok();
                tracing::warn!(error = %e, ?id, "unreadable audit log row left out");
            }
        }
    }
    Ok(entries)
}

fn decode_entry(row: &SqliteRow) -> Result<AuditLogEntry, RanchError> {
    let id: String = row.try_get("id").map_err(RanchError::database)?;
    let created_at: String = row.try_get("created_at").map_err(RanchError::database)?;
    let source_str: String = row.try_get("source").map_err(RanchError::database)?;
    let operator_id: String = row.try_get("operator_id").map_err(RanchError::database)?;
    let agent_client: Option<String> = row.try_get("agent_client").map_err(RanchError::database)?;
    let agent_transport: Option<String> = row
        .try_get("agent_transport")
        .map_err(RanchError::database)?;
    let action_str: String = row.try_get("action").map_err(RanchError::database)?;
    let target_payload: String = row
        .try_get("target_payload")
        .map_err(RanchError::database)?;
    let outcome_str: String = row.try_get("outcome").map_err(RanchError::database)?;
    let outcome_reason: Option<String> = row
        .try_get("outcome_reason")
        .map_err(RanchError::database)?;
    let connection_id: Option<String> =
        row.try_get("connection_id").map_err(RanchError::database)?;
    let connection_name: Option<String> = row
        .try_get("connection_name")
        .map_err(RanchError::database)?;
    let env_color: Option<String> = row.try_get("env_color").map_err(RanchError::database)?;
    let approval_id: Option<String> = row.try_get("approval_id").map_err(RanchError::database)?;
    let group_name: Option<String> = row.try_get("group_name").map_err(RanchError::database)?;
    let environment_name: Option<String> = row
        .try_get("environment_name")
        .map_err(RanchError::database)?;

    let source =
        AuditSource::try_from_str(&source_str).ok_or_else(|| RanchError::InvalidInput {
            reason: format!("Unknown audit source in DB: {source_str:?}"),
        })?;
    let action =
        AuditAction::try_from_str(&action_str).ok_or_else(|| RanchError::InvalidInput {
            reason: format!("Unknown audit action in DB: {action_str:?}"),
        })?;
    let outcome =
        AuditOutcome::try_from_str(&outcome_str).ok_or_else(|| RanchError::InvalidInput {
            reason: format!("Unknown audit outcome in DB: {outcome_str:?}"),
        })?;
    let target: AuditTarget =
        serde_json::from_str(&target_payload).map_err(|e| RanchError::InvalidInput {
            reason: format!("Could not parse target_payload as AuditTarget: {e}"),
        })?;

    Ok(AuditLogEntry {
        id,
        created_at,
        source,
        operator_id,
        agent_client,
        agent_transport,
        action,
        target,
        outcome,
        outcome_reason,
        connection_id,
        connection_name,
        env_color,
        group_name,
        environment_name,
        approval_id,
    })
}

/// A cell a spreadsheet will not run: outcome reasons carry Redis error text
/// and names are user- or agent-written, and `=HYPERLINK(…)` in a cell is a
/// formula once the CSV is opened. Same guard as the agent activity export.
fn formula_safe(value: &str) -> String {
    if matches!(
        value.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    ) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

fn entries_to_csv(entries: &[AuditLogEntry]) -> Result<String, RanchError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "created_at",
        "source",
        "operator_id",
        "action",
        "target_kind",
        "target_payload",
        "outcome",
        "outcome_reason",
        "connection_id",
        "connection_name",
        "group_name",
        "environment_name",
    ])
    .map_err(|e| RanchError::InvalidInput {
        reason: format!("CSV write failed: {e}"),
    })?;
    for e in entries {
        let target_payload = serde_json::to_string(&e.target).unwrap_or_default();
        let cells = [
            e.created_at.as_str(),
            e.source.as_str(),
            &e.operator_id,
            e.action.as_str(),
            e.target.kind_str_for_csv(),
            &target_payload,
            e.outcome.as_str(),
            e.outcome_reason.as_deref().unwrap_or(""),
            e.connection_id.as_deref().unwrap_or(""),
            e.connection_name.as_deref().unwrap_or(""),
            e.group_name.as_deref().unwrap_or(""),
            e.environment_name.as_deref().unwrap_or(""),
        ];
        wtr.write_record(cells.map(formula_safe))
            .map_err(|err| RanchError::InvalidInput {
                reason: format!("CSV write failed: {err}"),
            })?;
    }
    let bytes = wtr.into_inner().map_err(|e| RanchError::InvalidInput {
        reason: format!("CSV finalize failed: {e}"),
    })?;
    String::from_utf8(bytes).map_err(|e| RanchError::InvalidInput {
        reason: format!("CSV UTF-8 conversion failed: {e}"),
    })
}

fn build_filename(filter: &Filter, ext: &str) -> String {
    fn iso_date(opt: &Option<String>, fallback: &str) -> String {
        opt.as_deref()
            .and_then(|s| s.get(..10))
            .unwrap_or(fallback)
            .to_string()
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let from = iso_date(&filter.from, &today);
    let to = iso_date(&filter.to, &today);
    format!("ranch-audit-{from}-to-{to}.{ext}")
}

/// Tiny adapter so `entries_to_csv` can read the `target_kind` column-
/// style string from the in-memory enum without reaching into private
/// methods of `AuditTarget`. Keeps `kind_str` on the enum itself.
trait CsvTargetKind {
    fn kind_str_for_csv(&self) -> &'static str;
}

impl CsvTargetKind for AuditTarget {
    fn kind_str_for_csv(&self) -> &'static str {
        self.kind_str()
    }
}

#[cfg(test)]
mod tests {
    use super::formula_safe;

    #[test]
    fn csv_cells_never_start_a_formula() {
        assert_eq!(formula_safe("=HYPERLINK(\"x\")"), "'=HYPERLINK(\"x\")");
        assert_eq!(formula_safe("+1"), "'+1");
        assert_eq!(formula_safe("-1"), "'-1");
        assert_eq!(formula_safe("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(formula_safe("\tx"), "'\tx");
        assert_eq!(formula_safe("emails"), "emails");
        assert_eq!(formula_safe(""), "");
    }
}
