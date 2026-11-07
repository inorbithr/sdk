//! Where a project's session host keeps its files, and the lock that says one is running.
//!
//! Everything for a project lives in one folder below the data directory, named by a hash of
//! the project's real path: `lock` (held with `flock` for as long as the host lives, so a
//! killed host frees it and a recycled process id cannot fool anyone), `sock` (the unix socket
//! of [`super::link`]), `queue.duckdb` and `host.log`. The names are short on purpose: a unix
//! socket path is limited to about 100 bytes.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use rustix::fs::{FlockOperation, flock};
use sha2::{Digest as _, Sha256};

/// The files of one project's host.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    pub(crate) dir: PathBuf,
    pub(crate) sock: PathBuf,
    pub(crate) lock: PathBuf,
    pub(crate) db: PathBuf,
    pub(crate) log: PathBuf,
}

impl Paths {
    /// The files of the host for `project`, below `data` (the iohr data directory).
    pub(crate) fn for_project(data: &Path, project: &Path) -> Self {
        let real = crate::paths::real(project);
        let hash = Sha256::digest(real.to_string_lossy().as_bytes());
        let mut name = String::new();
        for b in hash.iter().take(5) {
            // Writing to a String cannot fail.
            let _ = write!(name, "{b:02x}");
        }
        Self::in_dir(data.join("session").join(name))
    }

    /// The files of a host kept in `dir`.
    pub(crate) fn in_dir(dir: PathBuf) -> Self {
        Self {
            sock: dir.join("sock"),
            lock: dir.join("lock"),
            db: dir.join("queue.duckdb"),
            log: dir.join("host.log"),
            dir,
        }
    }

    /// Makes the folder, for the owner alone (a session's socket is a way into the host).
    pub(crate) fn create(&self) -> std::io::Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
    }
}

/// The lock of a running host: held until dropped, or until the process ends.
#[derive(Debug)]
pub(crate) struct HostLock {
    _file: File,
}

/// Takes the lock, or says another host holds it (`Ok(None)`).
pub(crate) fn take(paths: &Paths) -> std::io::Result<Option<HostLock>> {
    paths.create()?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&paths.lock)?;
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(HostLock { _file: file })),
        Err(e) if e == rustix::io::Errno::WOULDBLOCK => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_project_is_one_folder_and_two_projects_are_two() {
        let data = tempfile::tempdir().unwrap();
        let a = Paths::for_project(data.path(), Path::new("/projects/a"));
        let again = Paths::for_project(data.path(), Path::new("/projects/a"));
        let b = Paths::for_project(data.path(), Path::new("/projects/b"));
        assert_eq!(a.dir, again.dir);
        assert_ne!(a.dir, b.dir);
        assert!(a.dir.starts_with(data.path().join("session")));
        // A unix socket path must fit in `sockaddr_un` (104 bytes on macOS).
        assert!(a.sock.as_os_str().len() < 100, "{}", a.sock.display());
    }

    #[test]
    fn the_lock_is_held_by_one_and_freed_when_it_drops() {
        let data = tempfile::tempdir().unwrap();
        let p = Paths::in_dir(data.path().join("h"));
        let first = take(&p).unwrap();
        assert!(first.is_some());
        assert!(take(&p).unwrap().is_none(), "a second host must be refused");
        drop(first);
        assert!(take(&p).unwrap().is_some(), "a dropped lock is free again");
    }
}
