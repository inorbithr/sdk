//! `iohr-ext.lock`: which extensions, at which digest, signed by whom.
//!
//! The same format serves two places: the machine's own record in the data directory,
//! and a file a team commits so that `iohr ext sync` installs exactly the same set
//! elsewhere.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::manifest::{check_name, check_privileges, check_version};
use super::oci::Digest;

/// The file name.
pub const FILE: &str = "iohr-ext.lock";
/// The lock format without privileges, which every `iohr` with extensions reads.
const VERSION: u32 = 1;
/// The lock format once an entry names privileges (SR-32). An `iohr` from before
/// privileges refuses it instead of installing a privileged extension without showing
/// them.
const VERSION_PRIVILEGES: u32 = 2;
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
    /// The privileges the person confirmed for its system service (SR-32). `sync`
    /// refuses an artifact that declares any other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub privileges: Vec<String>,
}

impl Entry {
    /// An entry.
    #[must_use]
    pub fn new(
        name: &str,
        version: &str,
        digest: &Digest,
        signer: String,
        privileges: Vec<String>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
            digest: digest.as_str().to_owned(),
            signer,
            privileges,
        }
    }

    /// The privileges in `declared` that this entry does not hold: what an artifact asks
    /// for beyond what was confirmed.
    #[must_use]
    pub fn unconfirmed<'a>(&self, declared: &'a [String]) -> Vec<&'a str> {
        declared
            .iter()
            .filter(|p| !self.privileges.contains(p))
            .map(String::as_str)
            .collect()
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
        if w.version != VERSION && w.version != VERSION_PRIVILEGES {
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
            check_privileges(&e.privileges)
                .map_err(|m| LockError(format!("{FILE}: {}: {m}", e.name)))?;
            if entries.insert(e.name.clone(), e).is_some() {
                return Err(LockError(format!("{FILE} lists an extension twice")));
            }
        }
        Ok(Self { entries })
    }

    /// The file's text: sorted by name, so it diffs well.
    #[must_use]
    pub fn render(&self) -> String {
        let privileged = self.entries.values().any(|e| !e.privileges.is_empty());
        let w = Wire {
            version: if privileged {
                VERSION_PRIVILEGES
            } else {
                VERSION
            },
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
            Vec::new(),
        );
        lock.entries.insert("agent".into(), e);
        let text = lock.render();
        assert!(text.contains("[[extension]]"), "{text}");
        // Without privileges the file is the one every earlier iohr wrote and reads.
        assert!(text.contains("version = 1\n"), "{text}");
        assert!(!text.contains("privileges"), "{text}");
        assert_eq!(Lock::parse(&text).unwrap(), lock);

        for bad in [
            "version = 3",
            "version = 1\n[[extension]]\nname = \"ext\"\nversion = \"1.0.0\"\ndigest = \"sha256:00\"\nsigner = \"s\"",
            "version = 1\n[[extension]]\nname = \"agent\"\nversion = \"1.0.0\"\ndigest = \"md5:00\"\nsigner = \"s\"",
        ] {
            assert!(Lock::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn privileges_are_recorded_checked_and_raise_the_format() {
        let mut lock = Lock::default();
        let e = Entry::new(
            "capture",
            "0.1.0",
            &Digest::of(b"x"),
            "key:sha256:00".into(),
            vec!["CAP_BPF".into(), "CAP_PERFMON".into()],
        );
        assert_eq!(
            e.unconfirmed(&["CAP_BPF".into(), "CAP_NET_ADMIN".into()]),
            ["CAP_NET_ADMIN"]
        );
        assert_eq!(e.unconfirmed(&["CAP_PERFMON".into()]), [] as [&str; 0]);
        lock.entries.insert("capture".into(), e);
        let text = lock.render();
        // An iohr from before privileges refuses this file rather than ignore them.
        assert!(text.contains("version = 2\n"), "{text}");
        assert!(
            text.contains("privileges = [\n    \"CAP_BPF\",\n    \"CAP_PERFMON\",\n]"),
            "{text}"
        );
        assert_eq!(Lock::parse(&text).unwrap(), lock);

        let entry = |p: &str| {
            format!(
                "version = 2\n[[extension]]\nname = \"capture\"\nversion = \"1.0.0\"\n\
                 digest = \"{}\"\nsigner = \"s\"\nprivileges = {p}\n",
                Digest::of(b"x")
            )
        };
        assert!(Lock::parse(&entry("[\"CAP_BPF\"]")).is_ok());
        for bad in [
            "[\"CAP_ROOT\"]",
            "[\"CAP_BPF\", \"CAP_BPF\"]",
            "\"CAP_BPF\"",
        ] {
            assert!(Lock::parse(&entry(bad)).is_err(), "{bad}");
        }
        assert!(Lock::parse("version = 3").is_err());
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
