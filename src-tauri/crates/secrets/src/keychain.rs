//! The OS keychain, behind the `SecretStore` port
//! (`application::ports::secrets`) that downstream code (commands,
//! connection actors) consumes. Two impls ship here:
//!   - `OsKeychain`: production: real OS keychain via the `keyring` crate
//!     (Apple Keychain on macOS, Credential Manager on Windows, libsecret on
//!     Linux).
//!   - `InMemoryKeychain`: tests only: thread-safe HashMap. Keeps CI green
//!     on Linux runners that lack `secret-service`.
//!
//! Service name = `dev.ranch.app` (matches `tauri.conf.json` identifier).
//! Each connection's credential is stored under that service with the
//! connection's id as the username (and `keychain_ref` column on the
//! `connections` table holds that id for resolution at use time).

use std::collections::HashMap;
use std::sync::Mutex;

use ranch_application::ports::SecretStore;
use ranch_domain::errors::RanchError;

#[cfg(feature = "os-keychain")]
const SERVICE: &str = "dev.ranch.app";
#[cfg(feature = "os-keychain")]
const KEYCHAIN_DOC_URL: &str =
    "https://github.com/Bramvanosta/ranch/blob/main/docs/keychain-setup.md";

/// The OS keychain, and everything that names the `keyring` crate: feature
/// `os-keychain`, which the desktop app and `ranch-mcp` enable. A container has no keychain: the server uses `FileSecretStore`.
#[cfg(feature = "os-keychain")]
mod os {
    use super::{RanchError, SecretStore, KEYCHAIN_DOC_URL, SERVICE};

    /// Real OS-backed keychain via the `keyring` crate.
    pub struct OsKeychain;

    impl OsKeychain {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for OsKeychain {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SecretStore for OsKeychain {
        fn set(&self, connection_id: &str, secret: &str) -> Result<(), RanchError> {
            let entry = keyring::Entry::new(SERVICE, connection_id).map_err(map_unavailable)?;
            entry.set_password(secret).map_err(map_unavailable)
        }

        fn get(&self, connection_id: &str) -> Result<Option<String>, RanchError> {
            let entry = keyring::Entry::new(SERVICE, connection_id).map_err(map_unavailable)?;
            match entry.get_password() {
                Ok(secret) => Ok(Some(secret)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(map_unavailable(e)),
            }
        }

        fn delete(&self, connection_id: &str) -> Result<(), RanchError> {
            let entry = keyring::Entry::new(SERVICE, connection_id).map_err(map_unavailable)?;
            match entry.delete_credential() {
                Ok(()) => Ok(()),
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(map_unavailable(e)),
            }
        }
    }

    /// Map keyring errors that indicate "no usable backend" to the structured
    /// `KeychainUnavailable` variant; everything else flows through `Keychain`.
    fn map_unavailable(err: keyring::Error) -> RanchError {
        use keyring::Error::{NoStorageAccess, PlatformFailure};
        match &err {
            PlatformFailure(_) | NoStorageAccess(_) => RanchError::KeychainUnavailable {
                reason: err.to_string(),
                doc_url: KEYCHAIN_DOC_URL.to_string(),
            },
            _ => RanchError::Keychain(err.to_string()),
        }
    }
}
#[cfg(feature = "os-keychain")]
pub use os::OsKeychain;

/// Test-only in-memory keychain. Same trait, no OS dependency.
pub struct InMemoryKeychain {
    inner: Mutex<HashMap<String, String>>,
}

impl InMemoryKeychain {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryKeychain {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for InMemoryKeychain {
    fn set(&self, connection_id: &str, secret: &str) -> Result<(), RanchError> {
        // `expect` here is acceptable: mutex poisoning means another thread
        // panicked while holding the lock, an irrecoverable state with no
        // sensible fallback for an in-memory test double.
        self.inner
            .lock()
            .expect("inmem keychain mutex poisoned")
            .insert(connection_id.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, connection_id: &str) -> Result<Option<String>, RanchError> {
        Ok(self
            .inner
            .lock()
            .expect("inmem keychain mutex poisoned")
            .get(connection_id)
            .cloned())
    }

    fn delete(&self, connection_id: &str) -> Result<(), RanchError> {
        self.inner
            .lock()
            .expect("inmem keychain mutex poisoned")
            .remove(connection_id);
        Ok(())
    }
}
