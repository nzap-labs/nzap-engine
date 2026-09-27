//! Secret storage for the Google refresh token.
//!
//! The OS keychain is preferred (Windows Credential Manager, macOS Keychain,
//! Secret Service on Linux). Where none is available (minimal Linux
//! installs, CI) secrets fall back to a JSON file readable only by the user,
//! and the UI surfaces that with [`StorageKind::File`].

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::error::{Error, Result};

/// Where secrets actually end up — shown on the Account page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    Keychain,
    File,
    Memory,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    fn kind(&self) -> StorageKind;
}

/// Pick the keychain when it works, otherwise the 0600 file at `fallback`.
pub fn default_store(service: &str, fallback: PathBuf) -> Arc<dyn SecretStore> {
    #[cfg(feature = "keychain")]
    {
        if let Some(store) = KeychainStore::probe(service) {
            return Arc::new(store);
        }
        tracing::warn!("No usable OS keychain; storing secrets in a user-only file");
    }
    #[cfg(not(feature = "keychain"))]
    let _ = service;
    Arc::new(FileStore::new(fallback))
}

// ---------------------------------------------------------------------------
// Keychain
// ---------------------------------------------------------------------------

#[cfg(feature = "keychain")]
pub struct KeychainStore {
    service: String,
}

#[cfg(feature = "keychain")]
impl KeychainStore {
    /// A store for `service`, or `None` when the platform keychain cannot be
    /// reached (no Secret Service daemon, locked-down session, …).
    pub fn probe(service: &str) -> Option<Self> {
        let entry = keyring::Entry::new(service, "nzap-probe").ok()?;
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Some(Self {
                service: service.to_owned(),
            }),
            Err(error) => {
                tracing::info!("Keychain unavailable: {error}");
                None
            }
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key)
            .map_err(|error| Error::Io(format!("Keychain error: {error}")))
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeychainStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(Error::Io(format!("Keychain read failed: {error}"))),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|error| Error::Io(format!("Keychain write failed: {error}")))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(Error::Io(format!("Keychain delete failed: {error}"))),
        }
    }

    fn kind(&self) -> StorageKind {
        StorageKind::Keychain
    }
}

// ---------------------------------------------------------------------------
// File fallback
// ---------------------------------------------------------------------------

/// A JSON map written atomically with owner-only permissions.
pub struct FileStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn read_map(&self) -> Result<BTreeMap<String, String>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error.into()),
        }
    }

    fn write_map(&self, map: &BTreeMap<String, String>) -> Result<()> {
        if map.is_empty() {
            return match fs::remove_file(&self.path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            };
        }
        write_private(&self.path, serde_json::to_string_pretty(map)?.as_bytes())
    }
}

impl SecretStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().map_err(|_| poisoned())?;
        Ok(self.read_map()?.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| poisoned())?;
        let mut map = self.read_map()?;
        map.insert(key.to_owned(), value.to_owned());
        self.write_map(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| poisoned())?;
        let mut map = self.read_map()?;
        if map.remove(key).is_some() {
            self.write_map(&map)?;
        }
        Ok(())
    }

    fn kind(&self) -> StorageKind {
        StorageKind::File
    }
}

// ---------------------------------------------------------------------------
// In-memory (tests)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<BTreeMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.map.lock().map_err(|_| poisoned())?.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.map
            .lock()
            .map_err(|_| poisoned())?
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.map.lock().map_err(|_| poisoned())?.remove(key);
        Ok(())
    }

    fn kind(&self) -> StorageKind {
        StorageKind::Memory
    }
}

fn poisoned() -> Error {
    Error::internal("A secret-store lock was poisoned.")
}

/// Write `bytes` to `path` atomically (temp file + rename) with 0600
/// permissions on Unix. On Windows the per-user profile ACL applies.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(store: &dyn SecretStore) {
        assert_eq!(store.get("a").ok().flatten(), None);
        store.set("a", "1").unwrap();
        store.set("b", "2").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("1"));
        store.delete("a").unwrap();
        store.delete("missing").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(store.get("b").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn memory_store_round_trip() {
        exercise(&MemoryStore::default());
    }

    #[test]
    fn file_store_round_trip_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("secrets.json");
        let store = FileStore::new(path.clone());
        exercise(&store);
        assert_eq!(store.kind(), StorageKind::File);
        store.delete("b").unwrap();
        assert!(!path.exists(), "an empty store removes its file");
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        FileStore::new(path.clone()).set("k", "v").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
