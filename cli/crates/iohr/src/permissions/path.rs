//! Paths as rules see them: made absolute and tidied without touching the disk (a rule is
//! about what a command names, so `./a/../.env` is `.env`), then matched against a pattern
//! the way Claude Code reads a `Read(...)` rule
//! (<https://code.claude.com/docs/en/permissions>, "Read & Edit"): `./x` and `x/**` are
//! relative to the project folder, `~/x` to home, `/x` absolute, and a bare `*.pem` or
//! `**/.env` matches at any depth.

use std::path::{Component, Path, PathBuf};

use super::glob;

/// A path made absolute, with the same path relative to the folder when it is inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) abs: String,
    pub(crate) rel: Option<String>,
}

/// Resolves `raw` against `cwd` (and `~` against `home`), folding `.` and `..`.
#[must_use]
pub(crate) fn resolve(raw: &str, cwd: &Path, home: Option<&Path>) -> Resolved {
    let joined: PathBuf = if raw == "~" || raw.starts_with("~/") {
        match home {
            Some(h) => h.join(raw.trim_start_matches('~').trim_start_matches('/')),
            None => PathBuf::from(raw),
        }
    } else if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        cwd.join(raw)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    let abs = crate::paths::slashed(&out);
    let rel = out
        .strip_prefix(cwd)
        .ok()
        .map(crate::paths::slashed)
        .filter(|r| !r.is_empty());
    Resolved { abs, rel }
}

/// Whether `pattern` matches the resolved path.
#[must_use]
pub(crate) fn matches(pattern: &str, at: &Resolved, cwd: &Path, home: Option<&Path>) -> bool {
    if pattern == "~" || pattern.starts_with("~/") {
        let abs = resolve(pattern, cwd, home).abs;
        return glob::path(&abs, &at.abs);
    }
    if let Some(p) = pattern.strip_prefix("//") {
        return glob::path(&format!("/{p}"), &at.abs);
    }
    if pattern.starts_with('/') {
        return glob::path(pattern, &at.abs);
    }
    let (pattern, anchored) = match pattern.strip_prefix("./") {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    // A pattern with no slash matches a name at any depth, as in a gitignore.
    let any_depth;
    let pattern = if anchored || pattern.contains('/') {
        pattern
    } else {
        any_depth = format!("**/{pattern}");
        &any_depth
    };
    at.rel.as_deref().is_some_and(|r| glob::path(pattern, r))
        || (pattern.starts_with("**/") && glob::path(pattern, &at.abs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        PathBuf::from("/work/proj")
    }

    #[test]
    fn dots_are_folded() {
        let r = resolve("./a/../.env", &cwd(), None);
        assert_eq!(r.abs, "/work/proj/.env");
        assert_eq!(r.rel.as_deref(), Some(".env"));
        let r = resolve("../other/x", &cwd(), None);
        assert_eq!(r.abs, "/work/other/x");
        assert_eq!(r.rel, None);
        let r = resolve("~/.ssh/id_rsa", &cwd(), Some(Path::new("/home/me")));
        assert_eq!(r.abs, "/home/me/.ssh/id_rsa");
    }

    #[test]
    fn patterns_read_as_claude_reads_them() {
        let home = Some(Path::new("/home/me"));
        let at = |p: &str| resolve(p, &cwd(), home);
        assert!(matches("./src/**", &at("src/a/b.rs"), &cwd(), home));
        assert!(!matches("./src/**", &at("lib/a.rs"), &cwd(), home));
        assert!(matches("*.pem", &at("deep/dir/a.pem"), &cwd(), home));
        assert!(matches("~/.ssh/**", &at("~/.ssh/id_rsa"), &cwd(), home));
        assert!(matches("/etc/hosts", &at("/etc/hosts"), &cwd(), home));
        assert!(matches("**/.env", &at("/elsewhere/.env"), &cwd(), home));
        assert!(!matches("src/**", &at("/elsewhere/src/a"), &cwd(), home));
    }
}
