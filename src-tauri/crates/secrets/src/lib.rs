//! Where Ranch keeps secrets: the adapters of the `SecretStore` port
//! (`application::ports::secrets`).
//!
//! - `OsKeychain` (feature `os-keychain`, the desktop app and `ranch-mcp`):
//!   the OS keychain, through the `keyring` crate.
//! - `FileSecretStore` (feature `file`, Ranch server): a file in the data
//!   volume, sealed with `RANCH_SECRET_KEY` when it is set.
//! - `InMemoryKeychain`: for tests.
//!
//! A crate of its own: it reaches the rest of Ranch through the port alone,
//! and each shell enables the store it needs, so neither pulls the other's
//! dependencies (libdbus on Linux for the keychain, a cipher for the file).

// The stores answer `RanchError`; see the domain crate's note on
// `result_large_err`.
#![allow(clippy::result_large_err)]

#[cfg(feature = "file")]
mod file;
mod keychain;

#[cfg(feature = "file")]
pub use file::FileSecretStore;
pub use keychain::InMemoryKeychain;
#[cfg(feature = "os-keychain")]
pub use keychain::OsKeychain;
