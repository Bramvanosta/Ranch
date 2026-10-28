//! Resolution notes from agents.
//!
//! An agent that diagnosed a failure cause leaves a note on it (MCP
//! `annotate_cause`): what is wrong and the steps to fix it. The note is
//! keyed like the cause (connection, queue, signature) and shown next to it
//! (Failed › By cause, the overview's attention list) and in Agents › Notes.
//! A note changes no queue state: no approval, but AI features must be on
//! (checked for every tool) and the connection writable by agents.

use sqlx::SqlitePool;

use crate::domain::errors::RanchError;
use crate::domain::queues::QueueId;

pub const MAX_TEXT_CHARS: usize = 4_000;
pub const MAX_STEPS: usize = 20;
pub const MAX_STEP_CHARS: usize = 500;
pub const MAX_JOBS_READ: usize = 200;
/// A failure signature, a queue id or name, a job id.
pub const MAX_KEY_CHARS: usize = 2_000;
/// Open notes per connection: an agent looping on annotate_cause cannot
/// flood the Notes tab (dismissed ones stay, as the record of what was said).
pub const MAX_NOTES_PER_CONNECTION: i64 = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct CauseNote {
    pub id: String,
    pub connection_id: String,
    /// `None` once the connection was removed.
    pub connection_name: Option<String>,
    pub queue_id: QueueId,
    /// The queue's name when the note was written, for display.
    pub queue_name: String,
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

/// A note to store, on the cause of `signature` in a queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCauseNote {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub queue_name: String,
    pub signature: String,
    pub text: String,
    pub steps: Vec<String>,
    pub jobs_read: Vec<String>,
}

/// Which notes to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CauseNoteFilter {
    /// One connection, or every one.
    pub connection_id: Option<String>,
    /// Dismissed notes are hidden unless asked for.
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
    args: NewCauseNote,
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
    if args.queue_id.as_str().trim().is_empty() {
        return Err(invalid("The queue is required."));
    }
    // Everything an agent sends is bounded, not only the text: the
    // signature, the queue, each job id.
    if args.signature.chars().count() > MAX_KEY_CHARS
        || args.queue_id.as_str().chars().count() > MAX_KEY_CHARS
        || args.queue_name.chars().count() > MAX_KEY_CHARS
        || args
            .jobs_read
            .iter()
            .any(|id| id.chars().count() > MAX_KEY_CHARS)
    {
        return Err(invalid(format!(
            "The signature, the queue and each job id are at most {MAX_KEY_CHARS} characters."
        )));
    }
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM connections WHERE id = ?")
        .bind(&args.connection_id)
        .fetch_optional(pool)
        .await
        .map_err(RanchError::database)?;
    if exists.is_none() {
        return Err(RanchError::ConnectionNotFound {
            id: args.connection_id,
        });
    }
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM cause_notes WHERE connection_id = ? AND dismissed_at IS NULL",
    )
    .bind(&args.connection_id)
    .fetch_one(pool)
    .await
    .map_err(RanchError::database)?;
    if count >= MAX_NOTES_PER_CONNECTION {
        return Err(invalid(format!(
            "This connection already holds {MAX_NOTES_PER_CONNECTION} open notes: a human has to dismiss some in Ranch first."
        )));
    }

    let id = uuid::Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO cause_notes (id, connection_id, queue_id, queue_name, signature, text,
                                  steps_json, client_hint, created_at, jobs_read_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&args.connection_id)
    .bind(args.queue_id.as_str())
    .bind(&args.queue_name)
    .bind(&args.signature)
    .bind(&text)
    .bind(serde_json::to_string(&steps).unwrap_or_else(|_| "[]".into()))
    .bind(&client_hint)
    .bind(&created_at)
    .bind(serde_json::to_string(&args.jobs_read).unwrap_or_else(|_| "[]".into()))
    .execute(pool)
    .await
    .map_err(RanchError::database)?;
    let notes = list_where(pool, "n.id = ?", Some(&id)).await?;
    notes
        .into_iter()
        .next()
        .ok_or_else(|| invalid("The note could not be read back."))
}

/// Newest first.
pub async fn list(pool: &SqlitePool, args: CauseNoteFilter) -> Result<Vec<CauseNote>, RanchError> {
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
        "SELECT n.id, n.connection_id, c.name, n.queue_id, n.queue_name, n.signature, n.text,
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
    let rows = query.fetch_all(pool).await.map_err(RanchError::database)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                connection_id,
                connection_name,
                queue_id,
                queue_name,
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
                queue_id: QueueId::new(queue_id),
                queue_name,
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
        .await
        .map_err(RanchError::database)?;
    Ok(())
}
