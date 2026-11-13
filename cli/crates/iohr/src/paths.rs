//! How a path is written when it is shown to a person or kept in a record (JSON, a hook's
//! file, a template listing): with `/` on every system, so the output is the same on every
//! machine. A path handed to the operating system stays native. [`find_program`] is the one
//! `PATH` lookup for a program iohr starts.

use std::path::{Path, PathBuf};

/// `p` with `/` between its components, whatever the system's separator is. On unix a `\` is
/// a character of a file name and is left alone.
#[must_use]
pub(crate) fn slashed(p: &Path) -> String {
    let s = p.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '/' {
        s.into_owned()
    } else {
        s.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// `p` as the disk names it: links followed and, on Windows, the short `RUNNER~1` form made
/// long, without the `\\?\` prefix `canonicalize` adds there (so it compares equal to a path git
/// or the person typed). A part that does not exist yet (a folder about to be made) is kept
/// as written, below the nearest folder that does.
#[must_use]
pub(crate) fn real(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p;
    let base = loop {
        if let Ok(c) = at.canonicalize() {
            break c;
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                at = parent;
            }
            _ => return p.to_path_buf(),
        }
    };
    let mut out = plain(base);
    out.extend(rest.iter().rev());
    out
}

/// `c` without the `\\?\` prefix of a canonical Windows path (a drive path only).
fn plain(c: PathBuf) -> PathBuf {
    if cfg!(windows)
        && let Some(p) = c.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && p.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(p);
    }
    c
}

/// The program's full path: itself when it is a path, otherwise the first match in an
/// absolute entry of `path` (a `PATH` value). A relative entry (`.`, an empty one) is
/// skipped, so a program of that name in the project directory is never run. On Windows the
/// match is the name with a `PATHEXT` extension (`mvn.cmd`, never the extensionless `mvn`
/// shell script beside it, which Windows cannot start).
#[must_use]
pub(crate) fn find_program(program: impl AsRef<Path>, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let program = program.as_ref();
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_path_buf());
    }
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| candidates(&dir, program).into_iter().find(|p| runnable(p)))
}

#[cfg(windows)]
fn candidates(dir: &Path, program: &Path) -> Vec<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let named = program.extension().map(|_| dir.join(program));
    named
        .into_iter()
        .chain(exts.split(';').filter(|e| !e.is_empty()).map(|e| {
            let mut name = program.as_os_str().to_owned();
            name.push(e);
            dir.join(name)
        }))
        .collect()
}

#[cfg(not(windows))]
fn candidates(dir: &Path, program: &Path) -> Vec<PathBuf> {
    vec![dir.join(program)]
}

#[cfg(unix)]
fn runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program file named `name` in `dir` that this system can start: executable on unix,
    /// with the `.cmd` extension on Windows.
    fn fake_program(dir: &Path, name: &str) -> PathBuf {
        let file = if cfg!(windows) {
            dir.join(format!("{name}.cmd"))
        } else {
            dir.join(name)
        };
        std::fs::write(&file, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        file
    }

    #[test]
    fn a_program_is_found_in_absolute_path_entries_only() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_program(dir.path(), "claude");
        let path =
            std::env::join_paths([PathBuf::from("relative"), dir.path().to_owned()]).unwrap();
        assert_eq!(find_program("claude", &path), Some(bin));
        assert_eq!(find_program("claude", std::ffi::OsStr::new("")), None);
        assert_eq!(find_program("absent", &path), None);
    }

    #[test]
    fn the_runnable_file_wins_over_an_extensionless_script_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mvn"), "#!/bin/sh\n").unwrap();
        let bin = fake_program(dir.path(), "mvn");
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(find_program("mvn", &path), Some(bin));
    }

    #[test]
    fn a_folder_not_made_yet_is_named_below_the_one_that_exists() {
        let d = tempfile::tempdir().unwrap();
        let made = real(&d.path().join("a").join("b"));
        assert_eq!(made, real(d.path()).join("a").join("b"));
        assert!(!made.exists());
    }

    #[test]
    fn components_are_joined_by_a_slash() {
        let p = Path::new("a").join("b").join("c.md");
        assert_eq!(slashed(&p), "a/b/c.md");
        assert_eq!(slashed(Path::new("")), "");
    }
}
