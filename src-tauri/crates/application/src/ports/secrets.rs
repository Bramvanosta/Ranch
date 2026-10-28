//! Where secrets are kept: a connection's password, its SSH password and
//! the SSH host key it pinned, each under a name of its own. Secrets never
//! go into SQLite.
//!
//! The adapters are the `ranch-secrets` crate's: the OS keychain, the
//! server's secrets file, and an in-memory store for tests.

use ranch_domain::errors::RanchError;

pub trait SecretStore: Send + Sync {
    fn set(&self, connection_id: &str, secret: &str) -> Result<(), RanchError>;
    fn get(&self, connection_id: &str) -> Result<Option<String>, RanchError>;
    fn delete(&self, connection_id: &str) -> Result<(), RanchError>;
}

/// The name a connection's password is kept under: its id. The other names
/// are `{id}::…` (its SSH password, its host key pins) or the store's own
/// (`ranch::integrity`, the key of the seals), so an id with `::` in it
/// would name another secret, and names none.
pub fn password_entry(connection_id: &str) -> Result<&str, RanchError> {
    if connection_id.is_empty() || connection_id.contains("::") {
        return Err(RanchError::InvalidInput {
            reason: format!("\"{connection_id}\" is not a connection id"),
        });
    }
    Ok(connection_id)
}

#[cfg(test)]
mod tests {
    use super::password_entry;

    #[test]
    fn a_password_entry_names_no_other_secret() {
        assert_eq!(password_entry("0192f3a4-c1").unwrap(), "0192f3a4-c1");
        assert!(password_entry("").is_err());
        assert!(password_entry("ranch::integrity").is_err());
        assert!(password_entry("c1::ssh").is_err());
        assert!(password_entry("c1::ssh::hostkey").is_err());
    }
}
