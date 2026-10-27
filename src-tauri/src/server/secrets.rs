//! The server's secret store: a file in the data volume.
//!
//! A container has no OS keychain. Redis passwords, SSH passwords and pinned
//! host keys go to `<data dir>/secrets.json` (mode 0600), one entry per
//! secret id — the same ids the desktop app gives the OS keychain, through
//! the same [`KeychainAdapter`] trait, so nothing above this file knows the
//! difference.
//!
//! With `RANCH_SECRET_KEY` set, each entry is sealed with XChaCha20-Poly1305
//! (key = SHA-256 of the variable, a fresh random nonce per write, the secret
//! id as associated data so an entry cannot be moved under another id).
//! Without it, entries are stored in clear and the server says so at start-up:
//! the volume is then the only protection.
//!
//! Entry format: `x:<base64(nonce ‖ ciphertext)>` sealed, `p:<base64>` clear.
//! A sealed entry read without the key — or with another key — is an error,
//! never an empty password.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::errors::RanchError;
use crate::core::keychain::KeychainAdapter;

const FILE_NAME: &str = "secrets.json";
const NONCE_LEN: usize = 24;

#[derive(Default, Serialize, Deserialize)]
struct SecretsFile {
    version: u32,
    entries: BTreeMap<String, String>,
}

pub struct FileSecretStore {
    path: PathBuf,
    cipher: Option<XChaCha20Poly1305>,
    // One writer at a time, and reads see whole files.
    lock: Mutex<()>,
}

impl FileSecretStore {
    /// `secret_key`: the value of `RANCH_SECRET_KEY`, if any (empty = none).
    pub fn new(data_dir: &Path, secret_key: Option<&str>) -> Self {
        let cipher = secret_key.filter(|k| !k.is_empty()).map(|k| {
            let key = Sha256::digest(k.as_bytes());
            XChaCha20Poly1305::new(&key)
        });
        Self {
            path: data_dir.join(FILE_NAME),
            cipher,
            lock: Mutex::new(()),
        }
    }

    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    fn load(&self) -> Result<SecretsFile, RanchError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| fail(format!("{} is not readable: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SecretsFile {
                version: 1,
                ..Default::default()
            }),
            Err(e) => Err(fail(format!("cannot read {}: {e}", self.path.display()))),
        }
    }

    /// Write-then-rename: a crash leaves the old file or the new one.
    fn save(&self, file: &SecretsFile) -> Result<(), RanchError> {
        let bytes = serde_json::to_vec_pretty(file).map_err(|e| fail(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        write_private(&tmp, &bytes)
            .and_then(|()| std::fs::rename(&tmp, &self.path))
            .map_err(|e| fail(format!("cannot write {}: {e}", self.path.display())))
    }

    fn seal(&self, id: &str, secret: &str) -> Result<String, RanchError> {
        let Some(cipher) = &self.cipher else {
            return Ok(format!("p:{}", B64.encode(secret.as_bytes())));
        };
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: secret.as_bytes(),
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| fail("could not seal the secret".to_string()))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&sealed);
        Ok(format!("x:{}", B64.encode(blob)))
    }

    fn open(&self, id: &str, entry: &str) -> Result<String, RanchError> {
        let (tag, body) = entry.split_at(entry.len().min(2));
        let bytes = B64
            .decode(body)
            .map_err(|_| fail(format!("the stored secret for {id} is corrupt")))?;
        let clear = match tag {
            "p:" => bytes,
            "x:" => {
                let cipher = self.cipher.as_ref().ok_or_else(|| {
                    fail("this secret is encrypted: start the server with the RANCH_SECRET_KEY it was saved with".to_string())
                })?;
                if bytes.len() < NONCE_LEN {
                    return Err(fail(format!("the stored secret for {id} is corrupt")));
                }
                let (nonce, sealed) = bytes.split_at(NONCE_LEN);
                cipher
                    .decrypt(
                        XNonce::from_slice(nonce),
                        Payload {
                            msg: sealed,
                            aad: id.as_bytes(),
                        },
                    )
                    .map_err(|_| {
                        fail("cannot decrypt this secret: RANCH_SECRET_KEY is not the key it was saved with".to_string())
                    })?
            }
            _ => return Err(fail(format!("the stored secret for {id} is corrupt"))),
        };
        String::from_utf8(clear).map_err(|_| fail(format!("the stored secret for {id} is corrupt")))
    }
}

impl KeychainAdapter for FileSecretStore {
    fn set(&self, connection_id: &str, secret: &str) -> Result<(), RanchError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.load()?;
        file.entries
            .insert(connection_id.to_string(), self.seal(connection_id, secret)?);
        self.save(&file)
    }

    fn get(&self, connection_id: &str) -> Result<Option<String>, RanchError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        match self.load()?.entries.get(connection_id) {
            Some(entry) => self.open(connection_id, entry).map(Some),
            None => Ok(None),
        }
    }

    fn delete(&self, connection_id: &str) -> Result<(), RanchError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.load()?;
        if file.entries.remove(connection_id).is_some() {
            self.save(&file)?;
        }
        Ok(())
    }
}

fn fail(reason: String) -> RanchError {
    RanchError::Keychain(reason)
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_store_round_trips_and_says_it_is_clear() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSecretStore::new(dir.path(), None);
        assert!(!store.is_encrypted());
        assert_eq!(store.get("c1").unwrap(), None);
        store.set("c1", "hunter2").unwrap();
        store.set("c2", "pässwörd ✓").unwrap();
        assert_eq!(store.get("c1").unwrap().as_deref(), Some("hunter2"));
        assert_eq!(store.get("c2").unwrap().as_deref(), Some("pässwörd ✓"));
        store.delete("c1").unwrap();
        assert_eq!(store.get("c1").unwrap(), None);
        // Deleting what is not there is not an error (the trait's contract).
        store.delete("nope").unwrap();
    }

    #[test]
    fn sealed_store_never_writes_the_secret_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        FileSecretStore::new(dir.path(), Some("correct horse"))
            .set("c1", "hunter2")
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(!on_disk.contains("hunter2"));
        assert!(!on_disk.contains(&B64.encode("hunter2")));
        assert!(on_disk.contains("\"x:"));

        let reopened = FileSecretStore::new(dir.path(), Some("correct horse"));
        assert_eq!(reopened.get("c1").unwrap().as_deref(), Some("hunter2"));
    }

    #[test]
    fn a_wrong_or_missing_key_is_an_error_not_an_empty_password() {
        let dir = tempfile::tempdir().unwrap();
        FileSecretStore::new(dir.path(), Some("right"))
            .set("c1", "hunter2")
            .unwrap();
        let wrong = FileSecretStore::new(dir.path(), Some("wrong")).get("c1");
        assert!(matches!(wrong, Err(RanchError::Keychain(m)) if m.contains("RANCH_SECRET_KEY")));
        let none = FileSecretStore::new(dir.path(), None).get("c1");
        assert!(matches!(none, Err(RanchError::Keychain(m)) if m.contains("RANCH_SECRET_KEY")));
    }

    #[test]
    fn a_sealed_entry_cannot_be_moved_under_another_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSecretStore::new(dir.path(), Some("k"));
        store.set("c1", "hunter2").unwrap();
        let path = dir.path().join(FILE_NAME);
        let moved = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\"c1\"", "\"c2\"");
        std::fs::write(&path, moved).unwrap();
        assert!(store.get("c2").is_err());
    }

    #[test]
    fn clear_entries_stay_readable_once_a_key_is_set() {
        let dir = tempfile::tempdir().unwrap();
        FileSecretStore::new(dir.path(), None)
            .set("c1", "old")
            .unwrap();
        let keyed = FileSecretStore::new(dir.path(), Some("k"));
        assert_eq!(keyed.get("c1").unwrap().as_deref(), Some("old"));
        // …and are sealed the next time they are written.
        keyed.set("c1", "new").unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(on_disk.contains("\"x:"));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        FileSecretStore::new(dir.path(), None)
            .set("c1", "s")
            .unwrap();
        let mode = std::fs::metadata(dir.path().join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
