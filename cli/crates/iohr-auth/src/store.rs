use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use keyring_core::{CredentialStore, Entry};

use crate::config::{ProfileName, write_private};
use crate::secret::Redacted;

/// The service name every `iohr` entry carries in the OS credential store.
const SERVICE: &str = "hr.inorbit.iohr";

/// Where one secret lives: a profile *and* its account (SR-24).
///
/// Keying by both means two accounts never share an entry, which is the bug that let
/// another CLI return the wrong account's token (cli/cli#12885).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryKey {
    profile: ProfileName,
    account: String,
}

impl EntryKey {
    /// The entry for `profile` on `account`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] when the account id is empty, longer than 128 bytes, or
    /// holds anything but ASCII letters, digits, `-` and `_`.
    pub fn new(profile: ProfileName, account: &str) -> Result<Self, StoreError> {
        let ok = (1..=128).contains(&account.len())
            && account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(StoreError::Invalid(
                "an account id has 1 to 128 letters, digits, '-' or '_'",
            ));
        }
        Ok(Self {
            profile,
            account: account.to_owned(),
        })
    }

    fn user(&self) -> String {
        format!("{}@{}", self.profile, self.account)
    }
}

/// Why the credential store failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// There is no usable credential store on this machine.
    #[error(
        "no credential store is available here ({0}); use IOHR_TOKEN for this session, or sign in with --insecure-storage to keep the secret in an owner-only file"
    )]
    Unavailable(String),
    /// The store refused or failed the operation.
    #[error("the credential store failed: {0}")]
    Failed(String),
    /// A value the store cannot take.
    #[error("{0}")]
    Invalid(&'static str),
    /// Reading or writing the file store failed.
    #[error("cannot {action} {path}: {source}")]
    Io {
        /// What was being done.
        action: &'static str,
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
}

/// A place to keep secrets between runs.
///
/// Calls block; async callers run them on a blocking thread.
pub trait Store: Send + Sync {
    /// The secret at `key`, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store cannot be read.
    fn get(&self, key: &EntryKey) -> Result<Option<Redacted<String>>, StoreError>;

    /// Keeps `secret` at `key`, replacing what was there.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store cannot be written.
    fn set(&self, key: &EntryKey, secret: &Redacted<String>) -> Result<(), StoreError>;

    /// Removes the secret at `key`; `true` when there was one.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store cannot be written.
    fn delete(&self, key: &EntryKey) -> Result<bool, StoreError>;
}

/// The operating system's credential store: macOS Keychain, Windows Credential
/// Manager, or the Secret Service (GNOME Keyring, KWallet) on Linux.
pub struct KeyringStore {
    inner: Arc<CredentialStore>,
}

impl fmt::Debug for KeyringStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringStore")
            .field("vendor", &self.inner.vendor())
            .finish()
    }
}

impl KeyringStore {
    /// Opens this platform's credential store.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unavailable`] when the platform has none or it cannot be reached,
    /// for example a Linux session without a Secret Service.
    pub fn open() -> Result<Self, StoreError> {
        let unavailable = |e: keyring_core::Error| StoreError::Unavailable(e.to_string());
        #[cfg(target_os = "macos")]
        let inner: Arc<CredentialStore> =
            apple_native_keyring_store::keychain::Store::new().map_err(unavailable)?;
        #[cfg(target_os = "windows")]
        let inner: Arc<CredentialStore> =
            windows_native_keyring_store::Store::new().map_err(unavailable)?;
        #[cfg(target_os = "linux")]
        let inner: Arc<CredentialStore> =
            zbus_secret_service_keyring_store::Store::new().map_err(unavailable)?;
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let inner: Arc<CredentialStore> = {
            let _ = unavailable;
            return Err(StoreError::Unavailable(
                "this operating system has no supported store".into(),
            ));
        };
        Ok(Self { inner })
    }

    fn entry(&self, key: &EntryKey) -> Result<Entry, StoreError> {
        self.inner.build(SERVICE, &key.user(), None).map_err(failed)
    }
}

fn failed(e: keyring_core::Error) -> StoreError {
    match e {
        keyring_core::Error::NoStorageAccess(p) => StoreError::Unavailable(p.to_string()),
        other => StoreError::Failed(other.to_string()),
    }
}

impl Store for KeyringStore {
    fn get(&self, key: &EntryKey) -> Result<Option<Redacted<String>>, StoreError> {
        match self.entry(key)?.get_password() {
            Ok(s) => Ok(Some(Redacted::new(s))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }

    fn set(&self, key: &EntryKey, secret: &Redacted<String>) -> Result<(), StoreError> {
        self.entry(key)?
            .set_password(secret.expose())
            .map_err(failed)
    }

    fn delete(&self, key: &EntryKey) -> Result<bool, StoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(e) => Err(failed(e)),
        }
    }
}

/// Secrets in owner-only files, one per entry: mode 0600 in a 0700 directory on Unix,
/// the user's profile directory on Windows. Chosen only with `--insecure-storage`
/// (SR-24), for machines without a credential store.
#[derive(Debug, Clone)]
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    /// A file store in `dir`.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(key: &EntryKey) -> String {
        format!("{}.{}", key.profile, key.account)
    }
}

impl Store for FileStore {
    fn get(&self, key: &EntryKey) -> Result<Option<Redacted<String>>, StoreError> {
        let path = self.dir.join(Self::file(key));
        match fs::read_to_string(&path) {
            Ok(s) => Ok(Some(Redacted::new(s))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StoreError::Io {
                action: "read",
                path,
                source,
            }),
        }
    }

    fn set(&self, key: &EntryKey, secret: &Redacted<String>) -> Result<(), StoreError> {
        let name = Self::file(key);
        write_private(&self.dir, &name, secret.expose().as_bytes()).map_err(|source| {
            StoreError::Io {
                action: "write",
                path: self.dir.join(&name),
                source,
            }
        })
    }

    fn delete(&self, key: &EntryKey) -> Result<bool, StoreError> {
        let path = self.dir.join(Self::file(key));
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StoreError::Io {
                action: "remove",
                path,
                source,
            }),
        }
    }
}

/// Secrets in process memory only, for tests.
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<EntryKey, Redacted<String>>>,
}

impl Store for MemoryStore {
    fn get(&self, key: &EntryKey) -> Result<Option<Redacted<String>>, StoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned())
    }

    fn set(&self, key: &EntryKey, secret: &Redacted<String>) -> Result<(), StoreError> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, key: &EntryKey) -> Result<bool, StoreError> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key)
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryKey, FileStore, KeyringStore, MemoryStore, Store, StoreError};
    use crate::Redacted;

    fn key(profile: &str, account: &str) -> EntryKey {
        EntryKey::new(profile.parse().unwrap(), account).unwrap()
    }

    fn round_trip(store: &dyn Store) {
        let a = key("work", "acc_1");
        let b = key("work", "acc_2");
        assert!(store.get(&a).unwrap().is_none());
        store.set(&a, &Redacted::new("one".into())).unwrap();
        store.set(&b, &Redacted::new("two".into())).unwrap();
        assert_eq!(store.get(&a).unwrap().unwrap().expose(), "one");
        assert_eq!(
            store.get(&b).unwrap().unwrap().expose(),
            "two",
            "one entry per account"
        );
        assert!(store.delete(&a).unwrap());
        assert!(!store.delete(&a).unwrap());
        assert!(store.get(&a).unwrap().is_none());
        assert!(store.delete(&b).unwrap());
    }

    #[test]
    fn account_ids_cannot_escape_a_file_name() {
        for bad in ["", "../x", "a/b", "a.b", "a b", &"a".repeat(129)] {
            assert!(EntryKey::new("p".parse().unwrap(), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn memory_store() {
        round_trip(&MemoryStore::default());
    }

    #[test]
    fn file_store() {
        let dir = tempfile::tempdir().unwrap();
        round_trip(&FileStore::new(dir.path().join("secrets")));
    }

    #[cfg(unix)]
    #[test]
    fn file_store_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path().join("secrets"));
        store
            .set(&key("p", "acc"), &Redacted::new("s".into()))
            .unwrap();
        let mode = std::fs::metadata(dir.path().join("secrets/p.acc"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    /// Runs against the real OS store. Where the machine has none, or it is locked, the
    /// test says so and returns: an absent store is reported, never faked. CI sets
    /// `IOHR_TEST_REQUIRE_STORE`, which turns that into a failure.
    #[test]
    fn os_credential_store() {
        let skip = |why: &str| {
            let required = std::env::var("IOHR_TEST_REQUIRE_STORE").is_ok_and(|v| !v.is_empty());
            assert!(!required, "credential store required: {why}");
            eprintln!("os_credential_store: skipped, {why}");
        };
        match KeyringStore::open() {
            Ok(store) => {
                let k = key("iohr-test", &format!("t{}", std::process::id()));
                let _ = store.delete(&k);
                match store.set(&k, &Redacted::new("probe".into())) {
                    Err(StoreError::Unavailable(why)) => {
                        skip(&format!("store locked or absent: {why}"));
                    }
                    other => {
                        other.unwrap();
                        assert_eq!(store.get(&k).unwrap().unwrap().expose(), "probe");
                        assert!(store.delete(&k).unwrap());
                    }
                }
            }
            Err(e) => skip(&e.to_string()),
        }
    }
}
