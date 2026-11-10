//! The one way this program opens a local `DuckDB` file: the knowledge base
//! (`.inorbit/crawl/kb.duckdb`, claims and evidence included) and the session host's queue.
//! RFC 0218 section 9 lists what a store of what a project knows must do before it holds
//! claims; this is where each of those is done, once, so no opener can forget one.
//!
//! - **Owner-only folder.** The folder is made `0700`. A folder that exists and is open to
//!   others, belongs to another user or is a link is refused, naming `chmod 700`, never
//!   changed behind the person's back. The file itself is made `0600`.
//! - **One writer.** An exclusive, non-blocking `flock` on `<file>.lock`, held as long as the
//!   [`Db`] lives; a killed process frees it, and a recycled process id cannot fool anyone. A
//!   second writer is refused with a message, not left to wait or to fail inside `DuckDB`.
//!   Readers hold the lock shared: many readers, or one writer, never both (`DuckDB` allows
//!   one read-write process, or many read-only ones, never a mix: "Concurrency",
//!   <https://duckdb.org/docs/current/connect/concurrency>).
//! - **Crash-safe writes.** `DuckDB` writes ahead and replays its log on the next open;
//!   [`Db::checkpoint`] folds the log into the file once a write is done. Callers write in
//!   one transaction, so a crash leaves the file as it was or as it will be, never half.
//! - **A size cap.** A file over [`Limits::max_bytes`] is refused for writing, and
//!   [`Db::ensure_room`] says so after a write, so a crawl that grows without end stops at
//!   the cap; memory and temporary files are capped in the engine too.
//! - **No network listener, no way out.** The engine opens no socket (the bundled build has
//!   none), and the connection turns off external access (files and URLs read from SQL),
//!   extension loading and installing, community and unsigned extensions, then locks the
//!   configuration, so no statement can turn them back on.

use std::fs::{File, OpenOptions};
use std::ops::Deref;
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use duckdb::{AccessMode, Config, Connection};
use rustix::fs::{FlockOperation, flock};

/// What an opener asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// The one writer: makes the folder and the file, applies migrations.
    Write,
    /// A reader: the file must exist; many at once, none while a writer holds it.
    Read,
}

/// The caps on one open file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The file's size, in bytes, above which it is not opened for writing.
    pub(crate) max_bytes: u64,
    /// The engine's memory, as `DuckDB` says it (`"1GB"`).
    pub(crate) memory: &'static str,
    /// The engine's temporary files (spill), as `DuckDB` says it.
    pub(crate) temp: &'static str,
}

impl Limits {
    /// The caps of every store the program keeps: 2 GiB of file, 1 GB of memory and spill.
    /// A project's knowledge is megabytes; a file this large is a runaway, not a crawl.
    pub(crate) const STORE: Self = Self {
        max_bytes: 2 * 1024 * 1024 * 1024,
        memory: "1GB",
        temp: "1GB",
    };
}

/// Why a file was not opened. The text is what the person reads.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OpenError {
    #[error("{0} is not a folder (a link or a file): it must be a folder of your own")]
    NotAFolder(PathBuf),
    #[error("{0} belongs to another user ({1}), not to you ({2})")]
    NotYours(PathBuf, u32, u32),
    #[error("{0} has mode {1:o} and is open to others; it must be 700 (chmod 700 {0})")]
    Loose(PathBuf, u32),
    #[error("{0} is in use by another process; {1}")]
    Busy(PathBuf, &'static str),
    #[error("{path} is {size} bytes, over the {cap} byte cap; remove sites you no longer need")]
    TooBig { path: PathBuf, size: u64, cap: u64 },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Db(PathBuf, duckdb::Error),
}

/// An open file: the connection and the lock that keeps it ours.
pub(crate) struct Db {
    conn: Connection,
    // Held for the file's life: the lock goes with the last handle (a clone shares it).
    lock: Arc<File>,
    path: PathBuf,
    access: Access,
    limits: Limits,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.path)
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

impl Deref for Db {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.conn
    }
}

/// Makes `dir` (and what is missing above it) for its owner alone, or checks the one that is
/// there: not a link, ours, closed to others.
///
/// # Errors
///
/// The folder is not ours alone, or cannot be made.
pub(crate) fn private_dir(dir: &Path) -> Result<(), OpenError> {
    let io = |e| OpenError::Io(dir.to_owned(), e);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io)?;
    check_dir(dir)
}

/// The checks of [`private_dir`] on a folder that must already be there.
fn check_dir(dir: &Path) -> Result<(), OpenError> {
    // Not `metadata`: a link is looked at, not followed.
    let meta = std::fs::symlink_metadata(dir).map_err(|e| OpenError::Io(dir.to_owned(), e))?;
    if !meta.is_dir() {
        return Err(OpenError::NotAFolder(dir.to_owned()));
    }
    let owner = rustix::process::geteuid().as_raw();
    if meta.uid() != owner {
        return Err(OpenError::NotYours(dir.to_owned(), meta.uid(), owner));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(OpenError::Loose(dir.to_owned(), mode));
    }
    Ok(())
}

/// The lock file beside `path`.
fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Opens `path` under [`Limits::STORE`].
///
/// # Errors
///
/// See [`open_with`].
pub(crate) fn open(path: &Path, access: Access) -> Result<Db, OpenError> {
    open_with(path, access, Limits::STORE)
}

/// Opens `path`: checks the folder, takes the lock, applies the caps, locks the engine's
/// configuration. A writer makes the folder and the file when they are not there.
///
/// # Errors
///
/// An [`OpenError`], with the reason in words: the folder is loose, another process holds the
/// file, the file is over its cap, or the engine refuses it.
pub(crate) fn open_with(path: &Path, access: Access, limits: Limits) -> Result<Db, OpenError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    match access {
        Access::Write => private_dir(dir)?,
        Access::Read => check_dir(dir)?,
    }
    let lock = take_lock(path, access)?;
    let io = |e| OpenError::Io(path.to_owned(), e);
    if access == Access::Write {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        if size > limits.max_bytes {
            return Err(OpenError::TooBig {
                path: path.to_owned(),
                size,
                cap: limits.max_bytes,
            });
        }
    }
    let db = |e| OpenError::Db(path.to_owned(), e);
    let mut config = Config::default()
        .access_mode(if access == Access::Read {
            AccessMode::ReadOnly
        } else {
            AccessMode::ReadWrite
        })
        .map_err(db)?
        .enable_external_access(false)
        .map_err(db)?
        .enable_autoload_extension(false)
        .map_err(db)?
        .max_memory(limits.memory)
        .map_err(db)?;
    for (key, value) in [
        ("autoinstall_known_extensions", "false"),
        ("allow_community_extensions", "false"),
        ("max_temp_directory_size", limits.temp),
    ] {
        config = config.with(key, value).map_err(db)?;
    }
    let conn = Connection::open_with_flags(path, config).map_err(db)?;
    // After this no statement can switch any of the above back on.
    conn.execute_batch("SET lock_configuration = true")
        .map_err(db)?;
    if access == Access::Write {
        // `DuckDB` makes the file with the process umask; the folder is 0700 so nobody got
        // in between, and from here the file is the owner's alone.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    }
    Ok(Db {
        conn,
        lock: Arc::new(lock),
        path: path.to_owned(),
        access,
        limits,
    })
}

/// Takes the lock beside `path`: exclusive for a writer, shared for a reader.
fn take_lock(path: &Path, access: Access) -> Result<File, OpenError> {
    let lock = lock_path(path);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock)
        .map_err(|e| OpenError::Io(lock.clone(), e))?;
    let (op, why) = match access {
        Access::Write => (
            FlockOperation::NonBlockingLockExclusive,
            "only one process writes it at a time; wait for that one to finish",
        ),
        Access::Read => (
            FlockOperation::NonBlockingLockShared,
            "a writer holds it right now; try again when it has finished",
        ),
    };
    match flock(&file, op) {
        Ok(()) => Ok(file),
        Err(e) if e == rustix::io::Errno::WOULDBLOCK => Err(OpenError::Busy(path.to_owned(), why)),
        Err(e) => Err(OpenError::Io(lock, e.into())),
    }
}

impl Db {
    /// Another connection to the same file, for another thread. It shares the lock, which is
    /// let go when the last of them is.
    ///
    /// # Errors
    ///
    /// The engine cannot clone the connection.
    pub(crate) fn handle(&self) -> Result<Self, OpenError> {
        Ok(Self {
            conn: self
                .conn
                .try_clone()
                .map_err(|e| OpenError::Db(self.path.clone(), e))?,
            lock: self.lock.clone(),
            path: self.path.clone(),
            access: self.access,
            limits: self.limits,
        })
    }

    /// Folds the write-ahead log into the file, so a finished write is in the file itself.
    /// A reader has nothing to fold.
    ///
    /// # Errors
    ///
    /// The engine could not checkpoint.
    pub(crate) fn checkpoint(&self) -> Result<(), OpenError> {
        if self.access == Access::Read {
            return Ok(());
        }
        self.conn
            .execute_batch("CHECKPOINT")
            .map_err(|e| OpenError::Db(self.path.clone(), e))
    }

    /// Whether the file is still under its size cap.
    ///
    /// # Errors
    ///
    /// [`OpenError::TooBig`] when it is not.
    pub(crate) fn ensure_room(&self) -> Result<(), OpenError> {
        let size = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        if size > self.limits.max_bytes {
            return Err(OpenError::TooBig {
                path: self.path.clone(),
                size,
                cap: self.limits.max_bytes,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn private_tmp() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        t
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_writer_makes_the_folder_and_the_file_for_its_owner_alone() {
        let t = private_tmp();
        let file = t.path().join("a/b/kb.duckdb");
        let db = open(&file, Access::Write).unwrap();
        db.execute_batch("CREATE TABLE t(x INTEGER); INSERT INTO t VALUES (1)")
            .unwrap();
        db.checkpoint().unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(&lock_path(&file)), 0o600);
        db.ensure_room().unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_folder_open_to_others_is_refused_and_left_as_it_is() {
        let t = private_tmp();
        let dir = t.path().join("crawl");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.join("kb.duckdb");
        for access in [Access::Write, Access::Read] {
            let e = open(&file, access).unwrap_err();
            assert!(matches!(e, OpenError::Loose(_, 0o755)), "{e}");
            assert!(e.to_string().contains("chmod 700"), "{e}");
        }
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o755,
            "never changed behind the person's back"
        );
        assert!(!file.exists(), "nothing was made in it");
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_link_in_place_of_the_folder_is_refused() {
        let t = private_tmp();
        let real = t.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let e = open(&link.join("kb.duckdb"), Access::Write).unwrap_err();
        assert!(matches!(e, OpenError::NotAFolder(_)), "{e}");
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_second_writer_is_refused_and_a_reader_waits_for_no_one_but_the_writer() {
        let t = private_tmp();
        let file = t.path().join("kb.duckdb");
        let first = open(&file, Access::Write).unwrap();
        for access in [Access::Write, Access::Read] {
            let e = open(&file, access).unwrap_err();
            assert!(matches!(e, OpenError::Busy(..)), "{e}");
        }
        drop(first);
        let r1 = open(&file, Access::Read).unwrap();
        let r2 = open(&file, Access::Read).unwrap();
        let e = open(&file, Access::Write).unwrap_err();
        assert!(
            matches!(e, OpenError::Busy(..)),
            "no writer while readers hold it: {e}"
        );
        drop((r1, r2));
        open(&file, Access::Write).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_clone_keeps_the_lock_until_the_last_handle_goes() {
        let t = private_tmp();
        let file = t.path().join("kb.duckdb");
        let a = open(&file, Access::Write).unwrap();
        let b = a.handle().unwrap();
        drop(a);
        assert!(matches!(
            open(&file, Access::Write).unwrap_err(),
            OpenError::Busy(..)
        ));
        drop(b);
        open(&file, Access::Write).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_file_over_the_cap_is_not_opened_for_writing_and_a_write_that_passes_it_is_said() {
        let t = private_tmp();
        let file = t.path().join("kb.duckdb");
        let tiny = Limits {
            max_bytes: 1,
            ..Limits::STORE
        };
        let db = open_with(&file, Access::Write, Limits::STORE).unwrap();
        db.execute_batch("CREATE TABLE t(x INTEGER); INSERT INTO t VALUES (1)")
            .unwrap();
        db.checkpoint().unwrap();
        drop(db);
        let e = open_with(&file, Access::Write, tiny).unwrap_err();
        assert!(matches!(e, OpenError::TooBig { cap: 1, .. }), "{e}");
        assert!(e.to_string().contains("cap"), "{e}");
        // A reader still reads what is there.
        open_with(&file, Access::Read, tiny).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_statement_cannot_reach_out_or_switch_the_guards_off() {
        let t = private_tmp();
        let db = open(&t.path().join("kb.duckdb"), Access::Write).unwrap();
        for sql in [
            "SET enable_external_access = true",
            "SET autoinstall_known_extensions = true",
            "SET allow_community_extensions = true",
            "SET lock_configuration = false",
            "INSTALL httpfs",
            "LOAD httpfs",
            "COPY (SELECT 1) TO 'leak.csv'",
            "SELECT * FROM read_csv('https://example.invalid/x.csv')",
        ] {
            assert!(db.execute_batch(sql).is_err(), "{sql} must be refused");
        }
        assert!(!Path::new("leak.csv").exists());
        db.execute_batch("CREATE TABLE ok(x INTEGER)").unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
    fn a_write_that_fails_halfway_leaves_nothing_and_a_finished_one_survives_a_reopen() {
        let t = private_tmp();
        let file = t.path().join("kb.duckdb");
        {
            let db = open(&file, Access::Write).unwrap();
            db.execute_batch("CREATE TABLE t(x INTEGER PRIMARY KEY)")
                .unwrap();
            let tx = db.unchecked_transaction().unwrap();
            tx.execute_batch("INSERT INTO t VALUES (1)").unwrap();
            assert!(tx.execute_batch("INSERT INTO t VALUES (1)").is_err());
            drop(tx); // rolled back
            let n: i64 = db
                .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0);
            db.execute_batch("INSERT INTO t VALUES (2)").unwrap();
            // No checkpoint: the log is replayed on the next open.
        }
        let db = open(&file, Access::Read).unwrap();
        let n: i64 = db.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
    }
}
