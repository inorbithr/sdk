use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

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

    /// The profile this entry belongs to.
    #[must_use]
    pub fn profile(&self) -> &ProfileName {
        &self.profile
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
    /// A [`QuietStore`] call that the store would not answer without asking the person
    /// (a Keychain prompt) or inside its budget. Nothing was shown and nothing changed.
    #[error(
        "the credential store would need to ask you before this iohr may read its key; \
         run `iohr whoami` in a terminal and choose Always Allow"
    )]
    WouldAsk,
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

    /// A secret kept under another service than iohr's own sign-ins, such as an
    /// extension's secret reference (service `hr.inorbit.iohr.ext`, the reference as the
    /// account); `None` when there is none. Read only.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store cannot be read.
    pub fn get_named(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Redacted<String>>, StoreError> {
        let entry = self.inner.build(service, account, None).map_err(failed)?;
        match entry.get_password() {
            Ok(s) => Ok(Some(Redacted::new(s))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }

    fn entry(&self, key: &EntryKey) -> Result<Entry, StoreError> {
        self.inner.build(SERVICE, &key.user(), None).map_err(failed)
    }
}

fn failed(e: keyring_core::Error) -> StoreError {
    match e {
        keyring_core::Error::NoStorageAccess(p) => StoreError::Unavailable(p.to_string()),
        keyring_core::Error::PlatformFailure(p) if would_ask(p.as_ref()) => StoreError::WouldAsk,
        other => StoreError::Failed(other.to_string()),
    }
}

/// Whether a platform error is the Keychain saying it would have shown a prompt:
/// `errSecInteractionNotAllowed` (-25308), which is what a call returns while
/// [`QuietStore`] has switched the user interface off, or `errSecUserCanceled` (-128).
#[cfg(target_os = "macos")]
fn would_ask(e: &(dyn std::error::Error + 'static)) -> bool {
    e.downcast_ref::<security_framework::base::Error>()
        .is_some_and(|e| matches!(e.code(), -25308 | -128))
}

#[cfg(not(target_os = "macos"))]
fn would_ask(_: &(dyn std::error::Error + 'static)) -> bool {
    false
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

/// How long a [`QuietStore`] call may take: a Keychain read that has not answered by then
/// is waiting on a prompt (or a locked keychain), and the prompt segment must not wait.
const QUIET_BUDGET: std::time::Duration = std::time::Duration::from_millis(1500);

/// How long a [`QuietStore`] write may take. A write follows a refresh that already
/// rotated the refresh token, so giving up on it would lose the new one and end the
/// sign-in; it only runs after a read the store allowed, so it is normally as quick.
const QUIET_WRITE_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

/// How many [`QuietStore`] calls may run at once on one gate. A call that outlives its
/// budget keeps its thread until the store answers; the cap bounds those threads, and a
/// call made while the gate is full fails at once without starting one. Four leaves room
/// for the terminal panel's reads (asks, auto mode, the coordinator's picture) and one
/// action at the same moment.
const QUIET_IN_FLIGHT: usize = 4;

/// A store for paths that must never block on, or show, a credential prompt: the zsh
/// prompt's `asks count` and the terminal panel's reads. Reads and writes go to the inner
/// store as usual; where the store would have asked the person, or has not answered within
/// the budget, the call fails with [`StoreError::WouldAsk`].
///
/// On macOS, building one calls `SecKeychain::disable_user_interaction`
/// (`SecKeychainSetUserInteractionAllowed(false)`), so the Keychain returns
/// `errSecInteractionNotAllowed` instead of drawing its dialog. The setting holds while the
/// store lives
/// (<https://docs.rs/security-framework/3.7.0/security_framework/os/macos/keychain/struct.SecKeychain.html#method.disable_user_interaction>).
/// The budget is the backstop for a store that waits without a dialog. A call past its
/// budget cannot be stopped: it stays on its own thread until the store answers, and holds
/// one of the gate's four places until then. While every place is taken, a
/// get, set or delete fails at once with [`StoreError::WouldAsk`] and starts no thread, so
/// a long-running process (the terminal panel reads every 20 s) never piles up threads
/// waiting on a stuck Keychain. Writes get a longer budget (10 s): they save a refresh
/// token the server has already rotated.
///
/// Every store made with [`new`](Self::new) shares one gate for the process, because they
/// all wrap the one OS store and a call stuck in it holds the others back too.
/// [`with_budgets`](Self::with_budgets) gives the store a gate of its own.
pub struct QuietStore {
    inner: Arc<dyn Store>,
    budget: std::time::Duration,
    write_budget: std::time::Duration,
    gate: Arc<Gate>,
    #[cfg(target_os = "macos")]
    _no_ui: Option<security_framework::os::macos::keychain::KeychainUserInteractionLock>,
}

impl fmt::Debug for QuietStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuietStore")
            .field("budget", &self.budget)
            .field("write_budget", &self.write_budget)
            .finish_non_exhaustive()
    }
}

/// The calls of one or more [`QuietStore`]s that have not returned yet.
#[derive(Debug)]
struct Gate {
    in_flight: AtomicUsize,
    limit: usize,
}

impl Gate {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            in_flight: AtomicUsize::new(0),
            limit,
        })
    }

    /// A place for one call, or `None` while every place is taken.
    fn enter(self: &Arc<Self>) -> Option<Place> {
        let mut n = self.in_flight.load(Ordering::Acquire);
        while n < self.limit {
            match self.in_flight.compare_exchange_weak(
                n,
                n + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Place(Arc::clone(self))),
                Err(now) => n = now,
            }
        }
        None
    }
}

/// One call's place in its [`Gate`], given back when the call returns (or panics).
struct Place(Arc<Gate>);

impl Drop for Place {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The gate every [`QuietStore::new`] shares.
static OS_STORE_GATE: LazyLock<Arc<Gate>> = LazyLock::new(|| Gate::new(QUIET_IN_FLIGHT));

impl QuietStore {
    /// `inner` with the Keychain's dialogs off, reads bounded to 1.5 s and writes to 10 s,
    /// on the process's one gate.
    #[must_use]
    pub fn new(inner: Arc<dyn Store>) -> Self {
        Self::on_gate(
            inner,
            QUIET_BUDGET,
            QUIET_WRITE_BUDGET,
            Arc::clone(&OS_STORE_GATE),
        )
    }

    /// As [`new`](Self::new), with other budgets for reads and for writes, on a gate of its
    /// own.
    #[must_use]
    pub fn with_budgets(
        inner: Arc<dyn Store>,
        budget: std::time::Duration,
        write_budget: std::time::Duration,
    ) -> Self {
        Self::on_gate(inner, budget, write_budget, Gate::new(QUIET_IN_FLIGHT))
    }

    fn on_gate(
        inner: Arc<dyn Store>,
        budget: std::time::Duration,
        write_budget: std::time::Duration,
        gate: Arc<Gate>,
    ) -> Self {
        Self {
            inner,
            budget,
            write_budget,
            gate,
            // When the switch fails the budget still bounds the wait.
            #[cfg(target_os = "macos")]
            _no_ui: security_framework::os::macos::keychain::SecKeychain::disable_user_interaction(
            )
            .ok(),
        }
    }

    fn within<T: Send + 'static>(
        &self,
        budget: std::time::Duration,
        f: impl FnOnce(&dyn Store) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        // Every place taken: earlier calls are still waiting on the store, so this one
        // would too. Fail without starting another thread.
        let place = self.gate.enter().ok_or(StoreError::WouldAsk)?;
        let (tx, rx) = std::sync::mpsc::channel();
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("iohr-quiet-store".into())
            .spawn(move || {
                let done = f(inner.as_ref());
                // Given back before the answer, so a caller that has it can call again.
                drop(place);
                let _ = tx.send(done);
            })
            .map_err(|e| StoreError::Failed(format!("cannot start the store call: {e}")))?;
        match rx.recv_timeout(budget) {
            Ok(done) => done,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(StoreError::WouldAsk),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(StoreError::Failed(
                "the credential store call stopped".into(),
            )),
        }
    }
}

impl Store for QuietStore {
    fn get(&self, key: &EntryKey) -> Result<Option<Redacted<String>>, StoreError> {
        let key = key.clone();
        self.within(self.budget, move |s| s.get(&key))
    }

    fn set(&self, key: &EntryKey, secret: &Redacted<String>) -> Result<(), StoreError> {
        let (key, secret) = (key.clone(), secret.clone());
        self.within(self.write_budget, move |s| s.set(&key, &secret))
    }

    fn delete(&self, key: &EntryKey) -> Result<bool, StoreError> {
        let key = key.clone();
        self.within(self.write_budget, move |s| s.delete(&key))
    }
}

/// How long a credential-store call may take before the person is told why it waits.
const SLOW_STORE: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs a credential-store call off the async threads (every store call blocks). A call
/// still running after [`SLOW_STORE`] prints one line on stderr: on a Mac the Keychain is
/// most likely asking whether this `iohr` build may read its item, and a runner under
/// launchd otherwise waits there with nothing in its log (Study 0094.15).
///
/// # Errors
///
/// The call's own error, or [`StoreError::Failed`] when the call panicked.
pub async fn off_thread<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    off_thread_noting(f, SLOW_STORE, || {
        use std::io::Write as _;
        let _ = writeln!(
            io::stderr().lock(),
            "iohr: still waiting on the credential store after {}s; on a Mac the Keychain may be \
             asking to let this iohr read its key: choose Always Allow",
            SLOW_STORE.as_secs()
        );
    })
    .await
}

async fn off_thread_noting<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
    slow: std::time::Duration,
    note: impl FnOnce(),
) -> Result<T, StoreError> {
    let mut call = tokio::task::spawn_blocking(f);
    let joined = if let Ok(done) = tokio::time::timeout(slow, &mut call).await {
        done
    } else {
        note();
        call.await
    };
    joined.map_err(|e| StoreError::Failed(format!("the credential store call stopped: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::{
        EntryKey, FileStore, KeyringStore, MemoryStore, QuietStore, Store, StoreError,
        off_thread_noting,
    };
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

    /// Runs against the real OS store, and only where `IOHR_TEST_REQUIRE_STORE` is set: CI
    /// on macOS and Windows, and `mise run cli:keyring` on Linux. There a missing or
    /// locked store is a failure. Without the variable the test says so and returns,
    /// because on a developer's Mac the login Keychain can wait on an access prompt
    /// nobody sees and hold the Keychain lock, which stalls every other `iohr` that
    /// reads its credentials, sign-in included.
    #[test]
    fn os_credential_store() {
        let required = std::env::var("IOHR_TEST_REQUIRE_STORE").is_ok_and(|v| !v.is_empty());
        if !required {
            eprintln!(
                "os_credential_store: skipped, the real OS store is tested only where \
                 IOHR_TEST_REQUIRE_STORE is set (CI, mise run cli:keyring)"
            );
            return;
        }
        let store =
            KeyringStore::open().unwrap_or_else(|e| panic!("credential store required: {e}"));
        let k = key("iohr-test", &format!("t{}", std::process::id()));
        let _ = store.delete(&k);
        match store.set(&k, &Redacted::new("probe".into())) {
            Err(StoreError::Unavailable(why)) => {
                panic!("credential store required: store locked or absent: {why}")
            }
            other => other.unwrap(),
        }
        assert_eq!(store.get(&k).unwrap().unwrap().expose(), "probe");
        assert!(store.delete(&k).unwrap());
    }

    #[tokio::test]
    async fn a_slow_store_call_says_why_it_waits_and_still_answers() {
        let noted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let n = std::sync::Arc::clone(&noted);
        let got = off_thread_noting(
            || {
                std::thread::sleep(std::time::Duration::from_millis(200));
                Ok(7)
            },
            std::time::Duration::from_millis(10),
            move || n.store(true, std::sync::atomic::Ordering::SeqCst),
        )
        .await;
        assert_eq!(got.unwrap(), 7);
        assert!(noted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_quick_store_call_says_nothing() {
        let got = off_thread_noting(
            || Err::<(), _>(StoreError::Invalid("no")),
            std::time::Duration::from_secs(5),
            || panic!("a quick call must not print the note"),
        )
        .await;
        assert!(matches!(got, Err(StoreError::Invalid("no"))));
    }

    /// A store whose every call takes `delay`, as a Keychain does behind its dialog.
    struct Slow {
        delay: std::time::Duration,
    }

    impl Store for Slow {
        fn get(&self, _: &EntryKey) -> Result<Option<Redacted<String>>, StoreError> {
            std::thread::sleep(self.delay);
            Ok(Some(Redacted::new("late".into())))
        }
        fn set(&self, _: &EntryKey, _: &Redacted<String>) -> Result<(), StoreError> {
            std::thread::sleep(self.delay);
            Ok(())
        }
        fn delete(&self, _: &EntryKey) -> Result<bool, StoreError> {
            std::thread::sleep(self.delay);
            Ok(true)
        }
    }

    fn quiet(delay_ms: u64, budget_ms: u64) -> std::sync::Arc<QuietStore> {
        let ms = std::time::Duration::from_millis;
        std::sync::Arc::new(QuietStore::with_budgets(
            std::sync::Arc::new(Slow {
                delay: ms(delay_ms),
            }),
            ms(budget_ms),
            ms(budget_ms * 10),
        ))
    }

    /// The prompt path: the store would wait on a dialog, so the call fails inside its
    /// budget, without the "still waiting" line, and the person sees nothing.
    #[tokio::test]
    async fn a_quiet_store_call_fails_fast_and_never_says_it_waits() {
        let store = quiet(2_000, 30);
        let k = key("work", "acc_1");
        let started = std::time::Instant::now();
        let got = off_thread_noting(
            move || store.get(&k),
            // The wait note comes after 300 ms; the budget is 30 ms.
            std::time::Duration::from_millis(300),
            || panic!("the quiet path must never print the wait note"),
        )
        .await;
        assert!(matches!(got, Err(StoreError::WouldAsk)), "{got:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1_000),
            "the call did not wait for the slow store"
        );
    }

    #[test]
    fn a_quiet_store_passes_quick_calls_through() {
        let store = quiet(0, 1_000);
        let k = key("work", "acc_1");
        assert_eq!(store.get(&k).unwrap().unwrap().expose(), "late");
        store.set(&k, &Redacted::new("x".into())).unwrap();
        assert!(store.delete(&k).unwrap());
    }

    /// A write is a refresh token the server already rotated: it waits longer than a read
    /// and is not given up on at the read's budget.
    #[test]
    fn a_quiet_store_gives_a_write_more_time_than_a_read() {
        // The store answers in 100 ms: well past the 20 ms read budget, well inside a
        // 5 s write budget, so a busy test host cannot push the write over its deadline.
        let ms = std::time::Duration::from_millis;
        let store = QuietStore::with_budgets(
            std::sync::Arc::new(Slow { delay: ms(100) }),
            ms(20),
            ms(5_000),
        );
        let k = key("work", "acc_1");
        assert!(matches!(store.get(&k), Err(StoreError::WouldAsk)));
        store.set(&k, &Redacted::new("x".into())).unwrap();
    }

    /// A store whose calls wait until the test lets them go, as a Keychain does when it
    /// holds its lock. `reached` counts the calls that got to it.
    struct Held {
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        reached: std::sync::atomic::AtomicUsize,
    }

    impl Held {
        fn wait(&self) {
            self.reached
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Dropping the sender lets every call go.
            let _ = self.release.lock().unwrap().recv();
        }
    }

    impl Store for Held {
        fn get(&self, _: &EntryKey) -> Result<Option<Redacted<String>>, StoreError> {
            self.wait();
            Ok(Some(Redacted::new("held".into())))
        }
        fn set(&self, _: &EntryKey, _: &Redacted<String>) -> Result<(), StoreError> {
            self.wait();
            Ok(())
        }
        fn delete(&self, _: &EntryKey) -> Result<bool, StoreError> {
            self.wait();
            Ok(true)
        }
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A quiet store whose every gate place is taken by a call stuck in the store, and the
    /// sender that lets those calls go.
    fn stuck_quiet() -> (
        QuietStore,
        std::sync::Arc<Held>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let held = std::sync::Arc::new(Held {
            release: std::sync::Mutex::new(rx),
            reached: std::sync::atomic::AtomicUsize::new(0),
        });
        let ms = std::time::Duration::from_millis;
        let store = QuietStore::with_budgets(
            std::sync::Arc::clone(&held) as std::sync::Arc<dyn Store>,
            ms(50),
            ms(50),
        );
        let k = key("work", "acc_1");
        for _ in 0..super::QUIET_IN_FLIGHT {
            assert!(matches!(store.get(&k), Err(StoreError::WouldAsk)));
        }
        until("every stuck call reaches the store", || {
            held.reached.load(std::sync::atomic::Ordering::SeqCst) == super::QUIET_IN_FLIGHT
        });
        (store, held, tx)
    }

    /// The long-running process: calls stuck in the store take every place, and the next
    /// get, set or delete fails at once without starting a thread or reaching the store.
    #[test]
    fn a_quiet_store_with_calls_stuck_refuses_the_next_without_a_thread() {
        let (store, held, _tx) = stuck_quiet();
        let k = key("work", "acc_1");
        for _ in 0..100 {
            let started = std::time::Instant::now();
            assert!(matches!(store.get(&k), Err(StoreError::WouldAsk)));
            assert!(matches!(
                store.set(&k, &Redacted::new("x".into())),
                Err(StoreError::WouldAsk)
            ));
            assert!(matches!(store.delete(&k), Err(StoreError::WouldAsk)));
            assert!(
                started.elapsed() < std::time::Duration::from_millis(50),
                "a refused call waited for its budget"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            held.reached.load(std::sync::atomic::Ordering::SeqCst),
            super::QUIET_IN_FLIGHT,
            "a refused call reached the store"
        );
        assert_eq!(
            store
                .gate
                .in_flight
                .load(std::sync::atomic::Ordering::SeqCst),
            super::QUIET_IN_FLIGHT,
            "only the stuck calls hold a place"
        );
    }

    /// Once the stuck calls return, their places are free and calls work again.
    #[test]
    fn a_quiet_store_works_again_once_the_stuck_calls_return() {
        let (store, held, tx) = stuck_quiet();
        drop(tx);
        until("the stuck calls give their places back", || {
            store
                .gate
                .in_flight
                .load(std::sync::atomic::Ordering::SeqCst)
                == 0
        });
        let k = key("work", "acc_1");
        assert_eq!(store.get(&k).unwrap().unwrap().expose(), "held");
        store.set(&k, &Redacted::new("x".into())).unwrap();
        assert!(store.delete(&k).unwrap());
        assert_eq!(
            held.reached.load(std::sync::atomic::Ordering::SeqCst),
            super::QUIET_IN_FLIGHT + 3
        );
    }

    /// The prompt, the panel and every other caller build a quiet store per session, so
    /// the bound holds only because they share one gate.
    #[test]
    fn quiet_stores_on_the_os_store_share_one_gate() {
        let a = QuietStore::new(std::sync::Arc::new(MemoryStore::default()));
        let b = QuietStore::new(std::sync::Arc::new(MemoryStore::default()));
        assert!(std::sync::Arc::ptr_eq(&a.gate, &b.gate));
        let own = QuietStore::with_budgets(
            std::sync::Arc::new(MemoryStore::default()),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        );
        assert!(!std::sync::Arc::ptr_eq(&a.gate, &own.gate));
    }

    /// `errSecInteractionNotAllowed` is how the Keychain says "I would have asked".
    #[cfg(target_os = "macos")]
    #[test]
    fn the_keychain_refusing_to_prompt_is_would_ask() {
        use security_framework::base::Error;
        let platform = |code: i32| {
            super::failed(keyring_core::Error::PlatformFailure(Box::new(
                Error::from_code(code),
            )))
        };
        assert!(matches!(platform(-25308), StoreError::WouldAsk));
        assert!(matches!(platform(-128), StoreError::WouldAsk));
        assert!(matches!(platform(-25293), StoreError::Failed(_)));
    }
}
