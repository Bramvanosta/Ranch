//! Seals as HMAC-SHA256 tags, keyed by a secret in the OS keychain.
//!
//! On macOS the keychain item is readable by Ranch's own binaries and asks
//! the user for anything else; on Linux the Secret Service lets any process
//! of the user read it, as it does the Redis passwords: there is no boundary
//! to keep there.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::application::ports::sealer::{Check, Sealer};
use crate::application::ports::SecretStore;
use crate::domain::errors::RanchError;

/// The keychain account of the key (service `dev.ranch.app`).
pub const KEY_ACCOUNT: &str = "ranch::integrity";

pub type Key = [u8; 32];

pub struct HmacSealer {
    key: Option<Key>,
    /// Whether this process checks seals. Only `ranch-mcp` does: it is the
    /// one that must not take the database's word.
    checks: bool,
}

impl HmacSealer {
    /// The app: seal with `key` (none when the keychain could not give
    /// it), check nothing.
    pub fn for_app(key: Option<Key>) -> Self {
        Self { key, checks: false }
    }

    /// `ranch-mcp`: seal with `key` and check every seal. Without a key,
    /// every check is `Unverifiable`.
    pub fn for_agents(key: Option<Key>) -> Self {
        Self { key, checks: true }
    }

    /// Seal nothing, check nothing: a process with no key and nothing to
    /// enforce (Ranch server, a test's throwaway database).
    pub fn unsealed() -> Self {
        Self::for_app(None)
    }
}

impl Sealer for HmacSealer {
    fn seal(&self, parts: &[&str]) -> Option<String> {
        self.key
            .as_ref()
            .map(|k| hex(&mac(k, parts).finalize().into_bytes()))
    }

    fn check(&self, parts: &[&str], seal: Option<&str>) -> Check {
        if !self.checks {
            return Check::Valid;
        }
        let Some(k) = self.key.as_ref() else {
            return Check::Unverifiable;
        };
        match seal.and_then(unhex) {
            Some(bytes) if mac(k, parts).verify_slice(&bytes).is_ok() => Check::Valid,
            _ => Check::Invalid,
        }
    }
}

fn mac(key: &Key, parts: &[&str]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    // Length-prefixed parts: ("ab", "c") and ("a", "bc") seal differently.
    for part in parts {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part.as_bytes());
    }
    mac
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The key, created (and stored) when the keychain has none. `true` when it
/// was just created.
pub fn load_or_create(keychain: &dyn SecretStore) -> Result<(Key, bool), RanchError> {
    if let Some(key) = load(keychain)? {
        return Ok((key, false));
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| RanchError::InvalidInput {
        reason: format!("no randomness for the integrity key: {e}"),
    })?;
    keychain.set(KEY_ACCOUNT, &hex(&key))?;
    Ok((key, true))
}

/// The key, if the keychain holds one.
pub fn load(keychain: &dyn SecretStore) -> Result<Option<Key>, RanchError> {
    Ok(keychain
        .get(KEY_ACCOUNT)?
        .and_then(|text| unhex(&text))
        .and_then(|bytes| Key::try_from(bytes).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::sealer::{connection_parts, setting_parts};

    #[test]
    fn seals_are_hex_and_bound_to_every_part() {
        let key = [7u8; 32];
        let a = hex(&mac(&key, &["ab", "c"]).finalize().into_bytes());
        let b = hex(&mac(&key, &["a", "bc"]).finalize().into_bytes());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b, "parts are length-prefixed");
        assert_eq!(unhex(&a).map(|v| v.len()), Some(32));
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex("abc"), None);
        assert_ne!(
            connection_parts("c1", None, 1, "bullmq-redis", "{}"),
            connection_parts("c1", Some(0), 1, "bullmq-redis", "{}"),
            "null and false differ"
        );
        let endpoint = |host: &str| format!(r#"{{"host":"{host}","port":6379}}"#);
        let (staging, prod) = (endpoint("staging"), endpoint("prod"));
        assert_ne!(
            mac(
                &key,
                &connection_parts("c1", None, 0, "bullmq-redis", &staging)
            )
            .finalize()
            .into_bytes(),
            mac(
                &key,
                &connection_parts("c1", None, 0, "bullmq-redis", &prod)
            )
            .finalize()
            .into_bytes(),
            "the seal covers where the connection points"
        );
        assert_ne!(setting_parts("k", None), setting_parts("k", Some("")));
    }

    #[test]
    fn the_key_is_created_once_and_read_back() {
        let kc = crate::adapters::secrets::InMemoryKeychain::new();
        assert_eq!(load(&kc).unwrap(), None);
        let (key, created) = load_or_create(&kc).unwrap();
        assert!(created);
        let (again, created) = load_or_create(&kc).unwrap();
        assert!(!created);
        assert_eq!(key, again);
    }
}
