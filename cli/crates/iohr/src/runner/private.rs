//! Files this runner trusts to say what it may do: the pinned device keys and the record
//! of decisions already carried out (`runner/approvals`). A file that another user owns,
//! or that group or world can write, could be changed by them, so it is refused rather
//! than read.
//!
//! The directory is made 0700 by [`iohr_auth::create_private_dir`] (the one helper the
//! config and the token files use); the file is opened once and its owner and mode are
//! read from that handle, so what is checked is what is read.

use std::fs::File;
use std::path::Path;

/// Opens `path` and refuses it unless this user owns it and only this user can write it
/// (no group or world write bit), and its directory too.
///
/// Without `append`: read only; a file (or directory) that is not there is `Ok(None)`.
/// With `append`: the directory and the file are made (0700 and 0600) when missing, an
/// existing one is made private too, a link at the file's name is refused, and the
/// handle can be read and appended to.
///
/// # Errors
/// The file or its directory cannot be opened, belongs to another user, or is writable
/// by group or world. The text names the path, never the content.
pub(crate) fn open_trusted(path: &Path, append: bool) -> Result<Option<File>, String> {
    let fail = |e: &std::io::Error| format!("cannot open {}: {}", path.display(), e.kind());
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if append && let Some(dir) = dir {
        iohr_auth::create_private_dir(dir).map_err(|e| fail(&e))?;
    }
    // The directory first: nothing is made in one another user could change.
    if let Some(dir) = dir {
        match std::fs::metadata(dir) {
            Ok(meta) => check(&meta, dir, "directory")?,
            Err(e) if !append && e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(fail(&e)),
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    if append {
        o.append(true).create(true);
        // Mode 0600 as it is made, and never through a link at the name.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            o.mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed());
        }
    }
    let file = match o.open(path) {
        Ok(f) => f,
        Err(e) if !append && e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(fail(&e)),
    };
    let meta = file.metadata().map_err(|e| fail(&e))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    check(&meta, path, "file")?;
    // A file kept from before with a group or world read bit is made 0600 too.
    #[cfg(unix)]
    if append {
        use std::os::unix::fs::PermissionsExt as _;
        if meta.permissions().mode() & 0o077 != 0 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| fail(&e))?;
        }
    }
    Ok(Some(file))
}

/// Whether `meta` (of `path`) is owned by this user and not writable by group or world.
#[cfg(unix)]
fn check(meta: &std::fs::Metadata, path: &Path, what: &str) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;
    let me = rustix::process::geteuid().as_raw();
    if meta.uid() != me {
        return Err(format!(
            "{} {} is owned by another user (uid {}, not {me}); refused",
            what,
            path.display(),
            meta.uid()
        ));
    }
    let mode = meta.mode() & 0o7777;
    if mode & 0o022 != 0 {
        return Err(format!(
            "{} {} is writable by group or world (mode {mode:04o}); refused, run chmod go-w on it",
            what,
            path.display()
        ));
    }
    Ok(())
}

/// Other systems keep access in ACLs, which this check does not read.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps, reason = "same signature as the unix check")]
fn check(_: &std::fs::Metadata, _: &Path, _: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    fn mode(p: &Path, m: u32) {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    }

    #[test]
    fn a_file_only_its_owner_can_write_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("keys");
        std::fs::write(&p, "x").unwrap();
        for m in [0o600, 0o644, 0o444, 0o400] {
            mode(&p, m);
            assert!(open_trusted(&p, false).unwrap().is_some(), "{m:o}");
        }
        assert!(
            open_trusted(&dir.path().join("none"), false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_group_or_world_writable_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("keys");
        std::fs::write(&p, "x").unwrap();
        for m in [0o620, 0o602, 0o666, 0o660, 0o606] {
            mode(&p, m);
            let e = open_trusted(&p, false).unwrap_err();
            assert!(e.contains("writable by group or world"), "{m:o}: {e}");
        }
    }

    #[test]
    fn a_group_or_world_writable_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("keys");
        std::fs::write(&p, "x").unwrap();
        mode(dir.path(), 0o777);
        let e = open_trusted(&p, false).unwrap_err();
        assert!(e.contains("directory") && e.contains("writable"), "{e}");
        mode(dir.path(), 0o700);
    }

    #[test]
    fn a_directory_is_not_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("keys");
        std::fs::create_dir(&p).unwrap();
        assert!(
            open_trusted(&p, false)
                .unwrap_err()
                .contains("regular file")
        );
    }

    #[test]
    fn appending_makes_the_file_and_directory_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("b").join("seen");
        assert!(open_trusted(&p, true).unwrap().is_some());
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(p.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn another_user_s_file_is_refused() {
        // /etc/hosts belongs to root, and a test run as root has nothing to refuse.
        if rustix::process::geteuid().is_root() || !Path::new("/etc/hosts").exists() {
            return;
        }
        let e = open_trusted(Path::new("/etc/hosts"), false).unwrap_err();
        assert!(e.contains("owned by another user"), "{e}");
    }

    #[test]
    fn appending_never_follows_a_link_and_tightens_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("elsewhere");
        std::fs::write(&elsewhere, "kept").unwrap();
        let link = dir.path().join("seen");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert!(open_trusted(&link, true).is_err());
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "kept");

        let p = dir.path().join("kept");
        std::fs::write(&p, "x").unwrap();
        mode(&p, 0o644);
        mode(dir.path(), 0o755);
        assert!(open_trusted(&p, true).unwrap().is_some());
        let m = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(m(&p), 0o600);
        assert_eq!(m(dir.path()), 0o700);
    }
}
