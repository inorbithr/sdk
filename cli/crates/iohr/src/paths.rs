//! How a path is written when it is shown to a person or kept in a record (JSON, a hook's
//! file, a template listing): with `/` on every system, so the output is the same on every
//! machine. A path handed to the operating system stays native.

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

#[cfg(test)]
mod tests {
    use super::*;

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
