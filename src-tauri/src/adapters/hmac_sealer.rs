//! Seals as HMAC-SHA256 tags, keyed by a secret in the OS keychain.
//!
//! The key has a keychain service of its own (`OsKeychain::for_integrity_key`),
//! apart from the connections' passwords: no entry a connection names can
//! be it. Earlier releases kept it beside the passwords
//! ([`LEGACY_KEY_ACCOUNT`]); the app moves it on its first run.
//!
//! On macOS the keychain item is readable by Ranch's own binaries and asks
//! the user for anything else; on Linux the Secret Service lets any process
//! of the user read it, as it does the Redis passwords: there is no boundary
//! to keep there.

use std::sync::RwLock;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::application::ports::sealer::{Check, Sealer};
use crate::application::ports::SecretStore;
use crate::domain::errors::RanchError;

/// The keychain account of the key, in its own service.
pub const KEY_ACCOUNT: &str = "integrity-key";

/// Where earlier releases kept the key: the passwords' service
/// (`dev.ranch.app`). Read when the key is not in its own service yet.
pub const LEGACY_KEY_ACCOUNT: &str = "ranch::integrity";

pub type Key = [u8; 32];

/// Looks for the key again.
pub type Reload = Box<dyn Fn() -> Option<Key> + Send + Sync>;

pub struct HmacSealer {
    key: RwLock<Option<Key>>,
    /// Whether this process checks seals. Only `ranch-mcp` does: it is the
    /// one that must not take the database's word.
    checks: bool,
    /// The app, when the keychain could not give the key at launch (locked,
    /// say): asked again at each seal, so what the user saves once it opens
    /// is sealed.
    reload: Option<Reload>,
}

impl HmacSealer {
    /// The app: seal with `key` (none when the keychain could not give
    /// it), check nothing.
    pub fn for_app(key: Option<Key>) -> Self {
        Self {
            key: RwLock::new(key),
            checks: false,
            reload: None,
        }
    }

    /// The app without its key yet: `reload` is asked for it at each seal
    /// until it gives one.
    pub fn for_app_reloading(reload: Reload) -> Self {
        Self {
            key: RwLock::new(None),
            checks: false,
            reload: Some(reload),
        }
    }

    /// `ranch-mcp`: seal with `key` and check every seal. Without a key,
    /// every check is `Unverifiable`.
    pub fn for_agents(key: Option<Key>) -> Self {
        Self {
            key: RwLock::new(key),
            checks: true,
            reload: None,
        }
    }

    /// Seal nothing, check nothing: a process with no key and nothing to
    /// enforce (Ranch server, a test's throwaway database).
    pub fn unsealed() -> Self {
        Self::for_app(None)
    }

    fn key(&self) -> Option<Key> {
        let known = *self.key.read().unwrap_or_else(|e| e.into_inner());
        if known.is_some() {
            return known;
        }
        let found = self.reload.as_ref().and_then(|reload| reload())?;
        tracing::info!("integrity key read from the keychain: seals resume");
        *self.key.write().unwrap_or_else(|e| e.into_inner()) = Some(found);
        Some(found)
    }
}

impl Sealer for HmacSealer {
    fn seal(&self, parts: &[&str]) -> Option<String> {
        self.key()
            .map(|k| hex(&mac(&k, parts).finalize().into_bytes()))
    }

    fn check(&self, parts: &[&str], seal: Option<&str>) -> Check {
        if !self.checks {
            return Check::Valid;
        }
        let Some(k) = self.key() else {
            return Check::Unverifiable;
        };
        match seal.and_then(unhex) {
            Some(bytes) if mac(&k, parts).verify_slice(&bytes).is_ok() => Check::Valid,
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

/// The app's key: read from its own service (`store`), else moved there
/// from where earlier releases kept it (`legacy`, the passwords' service),
/// else created there. `true` when it was just created.
pub fn load_or_create(
    store: &dyn SecretStore,
    legacy: &dyn SecretStore,
) -> Result<(Key, bool), RanchError> {
    if let Some(key) = read(store, KEY_ACCOUNT)? {
        return Ok((key, false));
    }
    if let Some(key) = read(legacy, LEGACY_KEY_ACCOUNT)? {
        move_out_of_legacy(store, legacy, &key);
        return Ok((key, false));
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| RanchError::InvalidInput {
        reason: format!("no randomness for the integrity key: {e}"),
    })?;
    store.set(KEY_ACCOUNT, &hex(&key))?;
    Ok((key, true))
}

/// The key, if the keychain holds one: in its own service, or where
/// earlier releases kept it (`ranch-mcp` started before the app moved it).
pub fn load(store: &dyn SecretStore, legacy: &dyn SecretStore) -> Result<Option<Key>, RanchError> {
    match read(store, KEY_ACCOUNT)? {
        Some(key) => Ok(Some(key)),
        None => read(legacy, LEGACY_KEY_ACCOUNT),
    }
}

fn read(store: &dyn SecretStore, account: &str) -> Result<Option<Key>, RanchError> {
    Ok(store
        .get(account)?
        .and_then(|text| unhex(&text))
        .and_then(|bytes| Key::try_from(bytes).ok()))
}

/// Copy the key to its own service, then forget the old entry. A failure
/// leaves it where it was, in use: the next run tries again.
fn move_out_of_legacy(store: &dyn SecretStore, legacy: &dyn SecretStore, key: &Key) {
    if let Err(e) = store.set(KEY_ACCOUNT, &hex(key)) {
        tracing::warn!(error = %e, "could not move the integrity key to its own keychain service");
        return;
    }
    match read(store, KEY_ACCOUNT) {
        Ok(Some(copy)) if copy == *key => {}
        _ => {
            tracing::warn!("the integrity key's copy does not read back; the old entry is kept");
            return;
        }
    }
    match legacy.delete(LEGACY_KEY_ACCOUNT) {
        Ok(()) => tracing::info!("integrity key moved to its own keychain service"),
        Err(e) => tracing::warn!(error = %e, "could not delete the integrity key's old entry"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::secrets::InMemoryKeychain;
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
        let store = InMemoryKeychain::new();
        let passwords = InMemoryKeychain::new();
        assert_eq!(load(&store, &passwords).unwrap(), None);
        let (key, created) = load_or_create(&store, &passwords).unwrap();
        assert!(created);
        let (again, created) = load_or_create(&store, &passwords).unwrap();
        assert!(!created);
        assert_eq!(key, again);
        assert_eq!(passwords.get(LEGACY_KEY_ACCOUNT).unwrap(), None);
    }

    #[test]
    fn an_earlier_releases_key_moves_to_its_own_service() {
        let store = InMemoryKeychain::new();
        let passwords = InMemoryKeychain::new();
        let key = [9u8; 32];
        passwords.set(LEGACY_KEY_ACCOUNT, &hex(&key)).unwrap();

        // ranch-mcp reads it where it is, and moves nothing.
        assert_eq!(load(&store, &passwords).unwrap(), Some(key));
        assert!(passwords.get(LEGACY_KEY_ACCOUNT).unwrap().is_some());

        // The app keeps it (no new key: the seals hold) and moves it.
        let (moved, created) = load_or_create(&store, &passwords).unwrap();
        assert!(!created);
        assert_eq!(moved, key);
        assert_eq!(passwords.get(LEGACY_KEY_ACCOUNT).unwrap(), None);
        assert_eq!(load(&store, &passwords).unwrap(), Some(key));
    }

    #[test]
    fn a_reloading_sealer_seals_once_the_key_is_there() {
        let store = std::sync::Arc::new(InMemoryKeychain::new());
        let reader = std::sync::Arc::clone(&store);
        let sealer = HmacSealer::for_app_reloading(Box::new(move || {
            read(reader.as_ref(), KEY_ACCOUNT).ok().flatten()
        }));
        assert_eq!(sealer.seal(&["a"]), None);
        store.set(KEY_ACCOUNT, &hex(&[3u8; 32])).unwrap();
        assert_eq!(
            sealer.seal(&["a"]),
            HmacSealer::for_app(Some([3u8; 32])).seal(&["a"])
        );
    }
}
