//! `iohr-ext.lock`: which extensions, at which digest, signed by whom.
//!
//! The same format serves two places: the machine's own record in the data directory,
//! and a file a team commits so that `iohr ext sync` installs exactly the same set
//! elsewhere.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::manifest::{check_name, check_version};
use super::oci::Digest;

/// The file name.
pub const FILE: &str = "iohr-ext.lock";
const VERSION: u32 = 1;
const MAX_LOCK: u64 = 1024 * 1024;

/// One pinned extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Entry {
    /// Its name, the command it adds.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The digest of its image index: every platform's artifact, by content.
    pub digest: String,
    /// Who signed it: the release workflow (without the tag) or `key:sha256:...`.
    pub signer: String,
}

impl Entry {
    /// An entry.
    #[must_use]
    pub fn new(name: &str, version: &str, digest: &Digest, signer: String) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
            digest: digest.as_str().to_owned(),
            signer,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Wire {
    version: u32,
    #[serde(default, rename = "extension")]
    extensions: Vec<Entry>,
}

/// A lock file's contents, by extension name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lock {
    /// The entries.
    pub entries: BTreeMap<String, Entry>,
}

/// Why a lock file cannot be used.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LockError(pub String);

impl Lock {
    /// Reads lock-file text.
    ///
    /// # Errors
    ///
    /// [`LockError`] when it is not a version 1 lock with valid entries.
    pub fn parse(text: &str) -> Result<Self, LockError> {
        let w: Wire = toml::from_str(text)
            .map_err(|e| LockError(format!("not a valid {FILE}: {}", e.message())))?;
        if w.version != VERSION {
            return Err(LockError(format!(
                "{FILE} version {} is from a newer iohr; update iohr",
                w.version
            )));
        }
        let mut entries = BTreeMap::new();
        for e in w.extensions {
            check_name(&e.name).map_err(|m| LockError(format!("{FILE}: {m}")))?;
            check_version(&e.version).map_err(|m| LockError(format!("{FILE}: {m}")))?;
            Digest::parse(&e.digest).map_err(|m| LockError(format!("{FILE}: {m}")))?;
            if e.signer.is_empty() {
                return Err(LockError(format!("{FILE}: {} names no signer", e.name)));
            }
            if entries.insert(e.name.clone(), e).is_some() {
                return Err(LockError(format!("{FILE} lists an extension twice")));
            }
        }
        Ok(Self { entries })
    }

    /// The file's text: sorted by name, so it diffs well.
    #[must_use]
    pub fn render(&self) -> String {
        let w = Wire {
            version: VERSION,
            extensions: self.entries.values().cloned().collect(),
        };
        let body = toml::to_string_pretty(&w).unwrap_or_default();
        format!(
            "# Extensions of the iohr command line, pinned by digest and signer.\n\
             # Written by `iohr ext`; `iohr ext sync` installs exactly this set.\n\n{body}"
        )
    }

    /// Reads `path`; a missing file is an empty lock.
    ///
    /// # Errors
    ///
    /// [`LockError`] when the file cannot be read or is not a lock.
    pub fn load(path: &Path) -> Result<Self, LockError> {
        match std::fs::metadata(path) {
            Ok(m) if m.len() > MAX_LOCK => {
                return Err(LockError(format!("{} is too large", path.display())));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(LockError(format!("cannot read {}: {e}", path.display()))),
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| LockError(format!("cannot read {}: {e}", path.display())))?;
        Self::parse(&text).map_err(|e| LockError(format!("{}: {e}", path.display())))
    }

    /// Writes `path` in one step (a temporary file, then a rename).
    ///
    /// # Errors
    ///
    /// [`LockError`] when it cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), LockError> {
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
        let tmp = path.with_extension("lock.tmp");
        let write = || -> std::io::Result<()> {
            if let Some(d) = dir {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&tmp, self.render())?;
            std::fs::rename(&tmp, path)
        };
        write().map_err(|e| LockError(format!("cannot write {}: {e}", path.display())))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::{Entry, Lock};
    use crate::ext::oci::Digest;

    #[test]
    fn round_trips_and_checks_entries() {
        let mut lock = Lock::default();
        let e = Entry::new(
            "agent",
            "0.1.0",
            &Digest::of(b"x"),
            "https://github.com/inorbithr/agent/.github/workflows/release.yml".into(),
        );
        lock.entries.insert("agent".into(), e);
        let text = lock.render();
        assert!(text.contains("[[extension]]"), "{text}");
        assert_eq!(Lock::parse(&text).unwrap(), lock);

        for bad in [
            "version = 2",
            "version = 1\n[[extension]]\nname = \"ext\"\nversion = \"1.0.0\"\ndigest = \"sha256:00\"\nsigner = \"s\"",
            "version = 1\n[[extension]]\nname = \"agent\"\nversion = \"1.0.0\"\ndigest = \"md5:00\"\nsigner = \"s\"",
        ] {
            assert!(Lock::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(super::FILE);
        assert!(Lock::load(&path).unwrap().entries.is_empty());
        Lock::default().save(&path).unwrap();
        assert!(Lock::load(&path).unwrap().entries.is_empty());
    }
}
