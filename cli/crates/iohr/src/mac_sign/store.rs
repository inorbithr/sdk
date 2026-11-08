//! What this machine keeps about its signing key: the key's id and public key (`key.json`) and,
//! on a Mac, the enclave's opaque blob (`key.blob`), in `<config dir>/approvals-key`, 0700.
//! The folder is made by the same helper the config and the token files use.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The key the service knows this machine by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Enrolled {
    /// `apk_...`, from the service.
    pub(crate) key_id: String,
    /// base64url of the 65-byte point.
    pub(crate) public_key: String,
    pub(crate) label: String,
}

/// The folder for the key's files.
pub(crate) fn dir(config_dir: &Path) -> PathBuf {
    config_dir.join("approvals-key")
}

/// The enclave's blob.
pub(crate) fn blob(dir: &Path) -> PathBuf {
    dir.join("key.blob")
}

fn record(dir: &Path) -> PathBuf {
    dir.join("key.json")
}

/// The enrolled key, if this machine has one.
///
/// # Errors
/// The file is there but is not ours, is writable by others, or is damaged.
pub(crate) fn load(dir: &Path) -> Result<Option<Enrolled>, String> {
    use std::io::Read as _;
    let path = record(dir);
    let Some(mut file) = crate::runner::private::open_trusted(&path, false)? else {
        return Ok(None);
    };
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e.kind()))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|_| format!("{} is damaged; run `iohr asks key remove`", path.display()))
}

/// Keeps `key` (replacing any earlier record).
///
/// # Errors
/// The folder or the file cannot be written.
pub(crate) fn save(dir: &Path, key: &Enrolled) -> Result<(), String> {
    use std::io::Write as _;
    iohr_auth::create_private_dir(dir)
        .map_err(|e| format!("cannot make {}: {}", dir.display(), e.kind()))?;
    let path = record(dir);
    let text = serde_json::to_string_pretty(key).map_err(|_| "cannot write the key record")?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o
        .open(&path)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e.kind()))?;
    f.write_all(text.as_bytes())
        .map_err(|e| format!("cannot write {}: {}", path.display(), e.kind()))
}

/// Forgets the key: its record and its blob. Nothing is left that could sign.
///
/// # Errors
/// A file is there and cannot be removed.
pub(crate) fn remove(dir: &Path) -> Result<(), String> {
    for path in [record(dir), blob(dir)] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {}", path.display(), e.kind())),
        }
    }
    Ok(())
}
