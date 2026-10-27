//! Resolution notes from agents.
//!
//! An agent that diagnosed a failure cause leaves a note on it (MCP
//! `annotate_cause`): what is wrong and the steps to fix it. The note is
//! keyed like the cause — connection, queue, signature — and shown next to it
//! (Failed › By cause, the overview's attention list) and in Agents › Notes.
//! A note changes no queue state: no approval, but AI features must be on
//! (checked for every tool) and the connection writable by agents.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;

use crate::core::bullmq_schema::QueueRef;
use crate::core::errors::RanchError;

pub const MAX_TEXT_CHARS: usize = 4_000;
pub const MAX_STEPS: usize = 20;
pub const MAX_STEP_CHARS: usize = 500;
pub const MAX_JOBS_READ: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CauseNote {
    pub id: String,
    pub connection_id: String,
    /// `None` once the connection was removed.
    pub connection_name: Option<String>,
    pub queue_ref: QueueRef,
    /// The cause, as `list_failure_causes` names it.
    pub signature: String,
    pub text: String,
    pub steps: Vec<String>,
    /// The agent client that wrote it (`None` = written in Ranch).
    pub client_hint: Option<String>,
    pub created_at: String,
    /// The jobs the agent read to write it.
    pub jobs_read: Vec<String>,
    pub dismissed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateCauseArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub signature: String,
    pub text: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub jobs_read: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListCauseNotesArgs {
    /// One connection, or every one.
    #[serde(default)]
    pub connection_id: Option<String>,
    /// Dismissed notes are hidden unless asked for.
    #[serde(default)]
    pub include_dismissed: bool,
}

fn invalid(reason: impl Into<String>) -> RanchError {
    RanchError::InvalidInput {
        reason: reason.into(),
    }
}

/// Store a note. `client_hint` is the agent client (MCP) or `None` (Ranch).
pub async fn annotate(
    pool: &SqlitePool,
    args: AnnotateCauseArgs,
    client_hint: Option<String>,
) -> Result<CauseNote, RanchError> {
    let text = args.text.trim().to_string();
    if text.is_empty() {
        return Err(invalid("A note needs some text."));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(invalid(format!(
            "A note is at most {MAX_TEXT_CHARS} characters."
        )));
    }
    let steps: Vec<String> = args
        .steps
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if steps.len() > MAX_STEPS || steps.iter().any(|s| s.chars().count() > MAX_STEP_CHARS) {
        return Err(invalid(format!(
            "At most {MAX_STEPS} steps of {MAX_STEP_CHARS} characters."
        )));
    }
    if args.jobs_read.len() > MAX_JOBS_READ {
        return Err(invalid(format!("At most {MAX_JOBS_READ} job ids.")));
    }
    if args.queue_ref.name.trim().is_empty() {
        return Err(invalid("The queue is required."));
    }
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM connections WHERE id = ?")
        .bind(&args.connection_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(RanchError::ConnectionNotFound {
            id: args.connection_id,
        });
    }

    let id = uuid::Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO cause_notes (id, connection_id, queue_prefix, queue_name, signature, text,
                                  steps_json, client_hint, created_at, jobs_read_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&args.connection_id)
    .bind(&args.queue_ref.prefix)
    .bind(&args.queue_ref.name)
    .bind(&args.signature)
    .bind(&text)
    .bind(serde_json::to_string(&steps).unwrap_or_else(|_| "[]".into()))
    .bind(&client_hint)
    .bind(&created_at)
    .bind(serde_json::to_string(&args.jobs_read).unwrap_or_else(|_| "[]".into()))
    .execute(pool)
    .await?;
    let notes = list_where(pool, "n.id = ?", Some(&id)).await?;
    notes
        .into_iter()
        .next()
        .ok_or_else(|| invalid("The note could not be read back."))
}

/// Newest first.
pub async fn list(
    pool: &SqlitePool,
    args: ListCauseNotesArgs,
) -> Result<Vec<CauseNote>, RanchError> {
    let dismissed = if args.include_dismissed {
        "1 = 1"
    } else {
        "n.dismissed_at IS NULL"
    };
    match args.connection_id {
        Some(id) => {
            list_where(
                pool,
                &format!("{dismissed} AND n.connection_id = ?"),
                Some(&id),
            )
            .await
        }
        None => list_where(pool, dismissed, None).await,
    }
}

async fn list_where(
    pool: &SqlitePool,
    condition: &str,
    bind: Option<&str>,
) -> Result<Vec<CauseNote>, RanchError> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
    );
    let sql = format!(
        "SELECT n.id, n.connection_id, c.name, n.queue_prefix, n.queue_name, n.signature, n.text,
                n.steps_json, n.client_hint, n.created_at, n.jobs_read_json, n.dismissed_at
           FROM cause_notes n
           LEFT JOIN connections c ON c.id = n.connection_id
          WHERE {condition}
          ORDER BY n.created_at DESC
          LIMIT 500"
    );
    let mut query = sqlx::query_as::<_, Row>(&sql);
    if let Some(value) = bind {
        query = query.bind(value);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                connection_id,
                connection_name,
                prefix,
                name,
                signature,
                text,
                steps,
                client_hint,
                created_at,
                jobs_read,
                dismissed_at,
            )| CauseNote {
                id,
                connection_id,
                connection_name,
                queue_ref: QueueRef { prefix, name },
                signature,
                text,
                steps: serde_json::from_str(&steps).unwrap_or_default(),
                client_hint,
                created_at,
                jobs_read: serde_json::from_str(&jobs_read).unwrap_or_default(),
                dismissed_at,
            },
        )
        .collect())
}

/// Hide a note (it stays listed under "Show dismissed").
pub async fn dismiss(pool: &SqlitePool, id: &str) -> Result<(), RanchError> {
    sqlx::query("UPDATE cause_notes SET dismissed_at = ? WHERE id = ? AND dismissed_at IS NULL")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
