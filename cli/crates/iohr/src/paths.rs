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
/// or the person typed). `p` itself when it does not exist.
#[must_use]
pub(crate) fn real(p: &Path) -> PathBuf {
    let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if cfg!(windows)
        && let Some(plain) = c.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && plain.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(plain);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_joined_by_a_slash() {
        let p = Path::new("a").join("b").join("c.md");
        assert_eq!(slashed(&p), "a/b/c.md");
        assert_eq!(slashed(Path::new("")), "");
    }
}
