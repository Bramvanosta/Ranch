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
