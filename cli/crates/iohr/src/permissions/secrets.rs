//! The files an ask may never open, whatever a rule or an answer says.
//!
//! One list for every path that runs a model: the `?` card (`headless.rs`), an editing
//! `iohr ask` (`ask.rs`) and the permission broker. They were two lists that had drifted
//! apart; this is their union. Rules are Claude Code `Read` deny rules (gitignore patterns,
//! <https://code.claude.com/docs/en/permissions>); Claude applies a `Read` deny to Grep,
//! Glob and to the shell commands it recognises as reading. The broker checks the same
//! patterns itself, for the file tools and for every word of a shell command, before any
//! allow rule is looked at, and `permissions allow` refuses a rule that would name one.

use std::path::Path;

use super::path::resolve;

/// The deny rules, as Claude Code writes them.
pub(crate) const READ_RULES: &[&str] = &[
    "Read(./.env)",
    "Read(./.env.*)",
    "Read(**/.env)",
    "Read(**/.env.*)",
    "Read(**/*.pem)",
    "Read(**/*.key)",
    "Read(**/*.p12)",
    "Read(**/*.pfx)",
    "Read(**/id_rsa*)",
    "Read(**/id_ed25519*)",
    "Read(**/id_ecdsa*)",
    "Read(**/.netrc)",
    "Read(**/.npmrc)",
    "Read(**/.pypirc)",
    "Read(**/.ssh/**)",
    "Read(**/.aws/**)",
    "Read(**/.gnupg/**)",
    "Read(**/.kube/**)",
    "Read(**/.docker/config.json)",
    "Read(**/*secret*)",
    "Read(**/*credential*)",
    "Read(**/*.tfstate)",
    "Read(~/.ssh/**)",
    "Read(~/.aws/**)",
    "Read(~/.config/gh/**)",
    "Read(~/.inorbit/**)",
];

/// The pattern inside each rule.
fn patterns() -> impl Iterator<Item = &'static str> {
    READ_RULES
        .iter()
        .filter_map(|r| r.strip_prefix("Read(")?.strip_suffix(')'))
}

/// Whether `raw`, a path as written (relative to `cwd`, or `~/...`), is a secret file.
#[must_use]
pub(crate) fn is_secret_path(raw: &str, cwd: &Path, home: Option<&Path>) -> bool {
    // The path may be a folder (`Grep` of `~/.ssh`): what is under it counts too.
    [raw.to_owned(), format!("{}/x", raw.trim_end_matches('/'))]
        .iter()
        .any(|r| {
            let at = resolve(r, cwd, home);
            patterns().any(|p| super::path::matches(p, &at, cwd, home))
        })
}

/// Whether `pattern`, the inside of a `Read(...)`/`Edit(...)` allow rule, names a secret file:
/// it is one of the list's own patterns, or a path the list covers. A broad pattern (`./**`)
/// is not refused here: the list still wins over it whenever a call is judged.
#[must_use]
pub(crate) fn pattern_hits_secret(pattern: &str, cwd: &Path, home: Option<&Path>) -> bool {
    let p = pattern.trim();
    patterns().any(|s| s == p) || is_secret_path(p, cwd, home)
}

/// A path the words of a shell command name that is a secret, if any.
#[must_use]
pub(crate) fn secret_word<'a>(
    words: &'a [String],
    cwd: &Path,
    home: Option<&Path>,
) -> Option<&'a str> {
    words.iter().find_map(|w| {
        // `--file=.env` names the path after the `=`.
        let w = w.rsplit_once('=').map_or(w.as_str(), |(_, v)| v);
        (!w.is_empty() && !w.starts_with('-') && is_secret_path(w, cwd, home)).then_some(w)
    })
}
