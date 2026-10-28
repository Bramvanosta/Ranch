//! The production defaults: a connection in an environment with the
//! production colour is locked for humans and read-only for agents. They
//! apply when a connection is created there or moved there; leaving
//! production never relaxes them, and an explicit lock choice wins.

/// The one color that triggers the production safety defaults.
pub const PRODUCTION_COLOR_TOKEN: &str = "env-production";

/// Whether an environment of this color carries the production defaults.
pub fn is_production(color_token: &str) -> bool {
    color_token == PRODUCTION_COLOR_TOKEN
}

/// The lock and agent settings a new connection starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewConnectionSafety {
    pub locked: bool,
    /// `Some(true)` in production; `None` elsewhere, where the connection
    /// follows the global agent default.
    pub mcp_readonly: Option<bool>,
}

/// The settings of a connection created in an environment of this color:
/// an explicit lock choice wins, otherwise production locks.
pub fn for_new_connection(color_token: &str, explicit_lock: Option<bool>) -> NewConnectionSafety {
    let production = is_production(color_token);
    NewConnectionSafety {
        locked: explicit_lock.unwrap_or(production),
        mcp_readonly: production.then_some(true),
    }
}

/// Whether moving a connection applies the production defaults: it changes
/// environment and lands in a production-colored one.
pub fn applies_on_move(from_environment_id: &str, to_environment_id: &str, to_color: &str) -> bool {
    from_environment_id != to_environment_id && is_production(to_color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_production_color_carries_the_defaults() {
        assert!(is_production("env-production"));
        assert!(!is_production("env-staging"));
        assert!(!is_production("env-custom-1"));
    }

    #[test]
    fn a_new_production_connection_is_locked_and_read_only_for_agents() {
        assert_eq!(
            for_new_connection("env-production", None),
            NewConnectionSafety {
                locked: true,
                mcp_readonly: Some(true),
            }
        );
        assert_eq!(
            for_new_connection("env-local", None),
            NewConnectionSafety {
                locked: false,
                mcp_readonly: None,
            }
        );
    }

    #[test]
    fn an_explicit_lock_choice_wins_but_agents_stay_read_only() {
        let s = for_new_connection("env-production", Some(false));
        assert!(!s.locked);
        assert_eq!(s.mcp_readonly, Some(true));
        assert!(for_new_connection("env-staging", Some(true)).locked);
    }

    #[test]
    fn only_a_move_into_production_applies_them() {
        assert!(applies_on_move("env-a", "env-b", "env-production"));
        assert!(!applies_on_move("env-a", "env-a", "env-production"));
        assert!(!applies_on_move("env-a", "env-b", "env-staging"));
    }
}
