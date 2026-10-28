//! Connections, environments and groups as the sidebar lists them, and the
//! selectors that say where a connection goes.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::connections::JsonValue;
use crate::domain::hierarchy::{self, default_env_name_for_color};

#[derive(Debug, Serialize, Deserialize, Type, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub environment_id: String,
    pub group_id: String,
    pub name: String,
    /// The kind of backend that serves it (`bullmq-redis`).
    pub backend: String,
    /// Where that backend is reached, as it describes it: for
    /// `bullmq-redis`, a `BullmqRedisEndpoint`. Never holds a secret.
    #[specta(type = JsonValue)]
    pub endpoint: serde_json::Value,
    pub env_color: String,
    pub env_name: String,
    pub env_label: String,
    pub group_name: String,
    pub created_at: String,
    /// MCP read-only override: `Some(true)` = agents can
    /// only read, `Some(false)` = agents may write (subject to approval),
    /// `None` = follow the global default. Visible to agents too, so they
    /// can tell up-front that a write would be refused.
    pub mcp_readonly: Option<bool>,
    /// The connection is folded in the sidebar (its queues
    /// hidden). UI state, persisted so it survives a restart.
    pub sidebar_collapsed: bool,
}

/// agent-readable shape of an environment row. The UI today
/// derives the env list from `list_connections` join data; MCP needs a
/// direct fetch so an agent can enumerate environments without listing
/// every connection.
#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: String,
    pub group_id: String,
    /// Lets an agent (and the "Move to…" menu) say
    /// "Lumen / Production" without a second lookup.
    pub group_name: String,
    pub label: String,
    pub color_token: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Type, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub display_order: i32,
    pub collapsed: bool,
    /// Number of connections under this group (across all its environments).
    /// Lets the sidebar decide between "delete immediately" and "confirm the
    /// cascade" without a second round-trip.
    pub connection_count: i32,
}

/// Which group a connection goes into. `New` is find-or-create: a name that
/// already exists (case-insensitively) resolves to that group, so a retried
/// save can never produce two "Lumen" groups.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GroupSelector {
    Existing { id: String },
    New { name: String },
}

impl GroupSelector {
    pub fn named(name: &str) -> Self {
        Self::New {
            name: name.to_string(),
        }
    }
}

/// Which environment a connection goes into. `New` is find-or-create by label
/// within the group; when the label already exists, the existing environment
/// (and its color) wins. The color is an attribute of the environment and
/// is **never** used to pick one.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EnvironmentSelector {
    Existing {
        id: String,
    },
    #[serde(rename_all = "camelCase")]
    New {
        name: String,
        color_token: String,
    },
}

impl EnvironmentSelector {
    pub fn named(name: &str, color_token: &str) -> Self {
        Self::New {
            name: name.to_string(),
            color_token: color_token.to_string(),
        }
    }

    /// A new environment carrying the palette's default name for `color_token`
    /// ("Production", "Staging", "Local", "Custom 3"…), what the form
    /// pre-fills when the user picks a swatch before typing a name.
    pub fn for_color(color_token: &str) -> Self {
        Self::named(&default_env_name_for_color(color_token), color_token)
    }
}

#[derive(Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MoveConnectionResult {
    /// `true` when the move took the connection into a Production-colored
    /// environment and it was not locked yet; the UI tells the user.
    pub newly_locked: bool,
}

impl From<hierarchy::Connection> for Connection {
    fn from(v: hierarchy::Connection) -> Self {
        Self {
            id: v.id,
            environment_id: v.environment_id,
            group_id: v.group_id,
            name: v.name,
            backend: v.backend.as_str().to_string(),
            endpoint: v.endpoint.into_json(),
            env_color: v.env_color,
            env_name: v.env_name,
            env_label: v.env_label,
            group_name: v.group_name,
            created_at: v.created_at,
            mcp_readonly: v.mcp_readonly,
            sidebar_collapsed: v.sidebar_collapsed,
        }
    }
}

impl From<hierarchy::Environment> for Environment {
    fn from(v: hierarchy::Environment) -> Self {
        Self {
            id: v.id,
            group_id: v.group_id,
            group_name: v.group_name,
            label: v.label,
            color_token: v.color_token,
            name: v.name,
        }
    }
}

impl From<hierarchy::Group> for Group {
    fn from(v: hierarchy::Group) -> Self {
        Self {
            id: v.id,
            name: v.name,
            display_order: v.display_order,
            collapsed: v.collapsed,
            connection_count: v.connection_count,
        }
    }
}

impl From<hierarchy::MoveConnectionResult> for MoveConnectionResult {
    fn from(v: hierarchy::MoveConnectionResult) -> Self {
        Self {
            newly_locked: v.newly_locked,
        }
    }
}

impl From<GroupSelector> for hierarchy::GroupSelector {
    fn from(v: GroupSelector) -> Self {
        match v {
            GroupSelector::Existing { id } => Self::Existing { id },
            GroupSelector::New { name } => Self::New { name },
        }
    }
}

impl From<EnvironmentSelector> for hierarchy::EnvironmentSelector {
    fn from(v: EnvironmentSelector) -> Self {
        match v {
            EnvironmentSelector::Existing { id } => Self::Existing { id },
            EnvironmentSelector::New { name, color_token } => Self::New { name, color_token },
        }
    }
}
