//! Resolution notes agents leave on a failure cause, as the UI and the
//! MCP tools read and write them.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::queues::QueueId;
use crate::core::cause_notes;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateCauseArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub queue_name: String,
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

impl From<cause_notes::CauseNote> for CauseNote {
    fn from(v: cause_notes::CauseNote) -> Self {
        Self {
            id: v.id,
            connection_id: v.connection_id,
            connection_name: v.connection_name,
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            signature: v.signature,
            text: v.text,
            steps: v.steps,
            client_hint: v.client_hint,
            created_at: v.created_at,
            jobs_read: v.jobs_read,
            dismissed_at: v.dismissed_at,
        }
    }
}

impl From<AnnotateCauseArgs> for cause_notes::NewCauseNote {
    fn from(v: AnnotateCauseArgs) -> Self {
        Self {
            connection_id: v.connection_id,
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            signature: v.signature,
            text: v.text,
            steps: v.steps,
            jobs_read: v.jobs_read,
        }
    }
}

impl From<ListCauseNotesArgs> for cause_notes::CauseNoteFilter {
    fn from(v: ListCauseNotesArgs) -> Self {
        Self {
            connection_id: v.connection_id,
            include_dismissed: v.include_dismissed,
        }
    }
}
