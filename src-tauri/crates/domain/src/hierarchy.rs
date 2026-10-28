//! The sidebar hierarchy's rules: what a group or environment may be
//! called, the colours an environment may take, how a full reorder is
//! checked, and the name a copied connection gets.

use std::collections::HashSet;

use crate::endpoint::{BackendKind, Endpoint};
use crate::errors::RanchError;

/// Longest accepted group / environment name, in characters (after trimming).
pub const HIERARCHY_NAME_MAX_CHARS: usize = 64;

/// The curated palette, shared verbatim with `palette.ts`.
pub const ENV_COLOR_TOKENS: &[&str] = &[
    "env-production",
    "env-staging",
    "env-local",
    "env-custom-1",
    "env-custom-2",
    "env-custom-3",
    "env-custom-4",
    "env-custom-5",
    "env-custom-6",
    "env-custom-7",
];

/// Trim + validate a group or environment name.
pub fn normalize_name(kind: &str, raw: &str) -> Result<String, RanchError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} name cannot be empty"),
        });
    }
    if name.chars().count() > HIERARCHY_NAME_MAX_CHARS {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} name cannot exceed {HIERARCHY_NAME_MAX_CHARS} characters"),
        });
    }
    Ok(name.to_string())
}

pub fn validate_color_token(token: &str) -> Result<(), RanchError> {
    if ENV_COLOR_TOKENS.contains(&token) {
        Ok(())
    } else {
        Err(RanchError::InvalidInput {
            reason: format!("Unknown environment color: {token}"),
        })
    }
}

/// Palette default names; mirrors `ENV_COLOR_PALETTE` in `palette.ts`.
pub fn default_env_name_for_color(color_token: &str) -> String {
    match color_token {
        "env-production" => "Production".to_string(),
        "env-staging" => "Staging".to_string(),
        "env-local" => "Local".to_string(),
        other => match other.strip_prefix("env-custom-") {
            Some(n) => format!("Custom {n}"),
            None => "Environment".to_string(),
        },
    }
}

/// Verify `ordered_ids` exactly matches the current set fetched from the DB
/// (no missing, no extra, no duplicates). Returns `InvalidInput` on any
/// mismatch so the frontend can show a clear error and re-fetch the truth.
pub fn validate_full_set(
    kind: &str,
    current: &[(String,)],
    ordered_ids: &[String],
) -> Result<(), RanchError> {
    let current_set: HashSet<&str> = current.iter().map(|(s,)| s.as_str()).collect();
    let provided_set: HashSet<&str> = ordered_ids.iter().map(|s| s.as_str()).collect();

    if provided_set.len() != ordered_ids.len() {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} ordered_ids contains duplicates"),
        });
    }
    if current_set != provided_set {
        return Err(RanchError::InvalidInput {
            reason: format!("{kind} ordered_ids does not match current set"),
        });
    }
    Ok(())
}

/// Compute a non-colliding name for a duplicated connection. Strips an
/// existing " (copy)" / " (copy N)" suffix to avoid `"foo (copy) (copy)"`,
/// then appends " (copy)", " (copy 2)", " (copy 3)", ... as needed.
pub fn next_duplicate_name(source: &str, existing_names: &[String]) -> String {
    let base = strip_copy_suffix(source);
    let first = format!("{base} (copy)");
    if !existing_names.iter().any(|x| x == &first) {
        return first;
    }
    let mut n = 2_u32;
    loop {
        let candidate = format!("{base} (copy {n})");
        if !existing_names.iter().any(|x| x == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn strip_copy_suffix(name: &str) -> &str {
    if let Some(stripped) = name.strip_suffix(" (copy)") {
        return stripped;
    }
    // " (copy 2)" / " (copy 3)" / ...
    if let Some(idx) = name.rfind(" (copy ") {
        let tail = &name[idx + " (copy ".len()..];
        if let Some(num_end) = tail.strip_suffix(')') {
            if num_end.parse::<u32>().is_ok() {
                return &name[..idx];
            }
        }
    }
    name
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub id: String,
    pub environment_id: String,
    pub group_id: String,
    pub name: String,
    /// The backend that serves the connection.
    pub backend: BackendKind,
    /// Where that backend is reached, as it describes it. Never holds a
    /// secret: those are in the secret store under the connection's id.
    pub endpoint: Endpoint,
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
#[derive(Debug)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupSelector {
    Existing { id: String },
    New { name: String },
}

/// Which environment a connection goes into. `New` is find-or-create by label
/// within the group; when the label already exists, the existing environment
/// (and its color) wins. The color is an attribute of the environment and
/// is **never** used to pick one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentSelector {
    Existing { id: String },
    New { name: String, color_token: String },
}

#[derive(Debug, PartialEq, Eq)]
pub struct MoveConnectionResult {
    /// `true` when the move took the connection into a Production-colored
    /// environment and it was not locked yet; the UI tells the user.
    pub newly_locked: bool,
}

impl GroupSelector {
    pub fn named(name: &str) -> Self {
        Self::New {
            name: name.to_string(),
        }
    }
}

impl EnvironmentSelector {
    pub fn named(name: &str, color_token: &str) -> Self {
        Self::New {
            name: name.to_string(),
            color_token: color_token.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_duplicate_name_first_copy() {
        assert_eq!(next_duplicate_name("foo", &[]), "foo (copy)");
    }

    #[test]
    fn next_duplicate_name_second_copy() {
        let existing = vec!["foo".to_string(), "foo (copy)".to_string()];
        assert_eq!(next_duplicate_name("foo", &existing), "foo (copy 2)");
    }

    #[test]
    fn next_duplicate_name_skips_gaps_and_finds_next_free() {
        let existing = vec![
            "foo".to_string(),
            "foo (copy)".to_string(),
            "foo (copy 2)".to_string(),
            "foo (copy 4)".to_string(),
        ];
        assert_eq!(next_duplicate_name("foo", &existing), "foo (copy 3)");
    }

    #[test]
    fn next_duplicate_name_strips_existing_copy_suffix() {
        // Duplicating "foo (copy)" gives "foo (copy 2)", NOT "foo (copy) (copy)".
        let existing = vec!["foo (copy)".to_string()];
        assert_eq!(next_duplicate_name("foo (copy)", &existing), "foo (copy 2)");
    }

    #[test]
    fn next_duplicate_name_strips_numbered_copy_suffix() {
        // Duplicating "foo (copy 3)" treats "foo" as the base.
        let existing = vec!["foo".to_string(), "foo (copy 3)".to_string()];
        assert_eq!(next_duplicate_name("foo (copy 3)", &existing), "foo (copy)");
    }

    #[test]
    fn strip_copy_suffix_passes_through_unrelated_suffixes() {
        assert_eq!(strip_copy_suffix("foo bar"), "foo bar");
        assert_eq!(strip_copy_suffix("foo (copy)"), "foo");
        assert_eq!(strip_copy_suffix("foo (copy 5)"), "foo");
        // Non-numeric suffix is preserved (not a Ranch-generated copy name).
        assert_eq!(strip_copy_suffix("foo (copy x)"), "foo (copy x)");
    }
}
