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
    "Read(**/.git-credentials)",
    "Read(**/.zsh_history)",
    "Read(**/.bash_history)",
    "Read(**/Library/Keychains/**)",
    "Read(**/*secret*)",
    "Read(**/*credential*)",
    "Read(**/*.tfstate)",
    "Read(~/.ssh/**)",
    "Read(~/.aws/**)",
    "Read(~/.config/gh/**)",
    "Read(~/Library/Keychains/**)",
    "Read(~/.inorbit/**)",
    // Cloud and password-manager sign-ins (gcloud, Azure, DigitalOcean, 1Password).
    "Read(**/.config/gcloud/**)",
    "Read(**/.azure/**)",
    "Read(**/.config/doctl/**)",
    "Read(**/.config/op/**)",
    "Read(**/.op/**)",
    "Read(**/.1password/**)",
    "Read(**/Library/Application Support/1Password/**)",
    "Read(**/Library/Group Containers/*1password*/**)",
    // Browser profiles hold cookies, saved logins and history (Chrome, Chromium, Brave, Edge,
    // Arc, Firefox, Safari).
    "Read(**/Library/Application Support/Google/Chrome/**)",
    "Read(**/Library/Application Support/BraveSoftware/**)",
    "Read(**/Library/Application Support/Microsoft Edge/**)",
    "Read(**/Library/Application Support/Arc/**)",
    "Read(**/Library/Application Support/Firefox/**)",
    "Read(**/Library/Safari/**)",
    "Read(**/.config/google-chrome/**)",
    "Read(**/.config/chromium/**)",
    "Read(**/.config/BraveSoftware/**)",
    "Read(**/.config/microsoft-edge/**)",
    "Read(**/.mozilla/**)",
];

/// The folders under the home folder that hold secrets, as home-relative paths: each is one
/// of [`READ_RULES`]' `**` folders (a test keeps the two lists in step). A folder that is one
/// of them, inside one or above one (`~/.config`, `~/Library`, the home folder, `/Users`) is
/// too broad to hand over whole ([`crate::access::broad_or_secret_bearing`]).
pub(crate) const SECRET_DIRS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".azure",
    ".op",
    ".1password",
    ".mozilla",
    ".inorbit",
    ".config/gh",
    ".config/gcloud",
    ".config/doctl",
    ".config/op",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/microsoft-edge",
    "Library/Keychains",
    "Library/Safari",
    "Library/Application Support/1Password",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Application Support/Firefox",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychains_git_credentials_and_shell_history_are_secret_wherever_they_are() {
        let home = Path::new("/Users/me");
        let cwd = Path::new("/Users/me/work");
        for p in [
            "~/Library/Keychains/login.keychain-db",
            "~/Library/Keychains",
            "/Users/me/Library/Keychains/login.keychain-db",
            "~/.git-credentials",
            "../.git-credentials",
            "~/.zsh_history",
            "~/.bash_history",
            "/Users/other/.zsh_history",
        ] {
            assert!(is_secret_path(p, cwd, Some(home)), "{p}");
        }
        assert!(!is_secret_path(
            "~/Library/Preferences/x.plist",
            cwd,
            Some(home)
        ));
        assert!(!is_secret_path("~/Downloads/notes.txt", cwd, Some(home)));
        // The broker's allow-rule check refuses a rule that names one.
        assert!(pattern_hits_secret("~/.zsh_history", cwd, Some(home)));
        assert!(pattern_hits_secret("**/.git-credentials", cwd, Some(home)));
        // Every list entry is passed to Claude as a `Read(...)` deny.
        assert!(READ_RULES.contains(&"Read(~/Library/Keychains/**)"));
    }

    #[test]
    fn every_secret_folder_is_covered_by_a_deny_rule() {
        let home = Path::new("/Users/me");
        for d in SECRET_DIRS {
            let p = format!("~/{d}");
            assert!(is_secret_path(&p, home, Some(home)), "{p}");
        }
        for p in [
            "~/.config/gcloud/credentials.db",
            "~/.azure/accessTokens.json",
            "~/.config/doctl/config.yaml",
            "~/Library/Group Containers/2BUA8C4S2C.com.1password/x",
            "~/Library/Application Support/Google/Chrome/Default/Cookies",
            "~/Library/Safari/History.db",
            "/home/me/.mozilla/firefox/x.default/logins.json",
        ] {
            assert!(is_secret_path(p, home, Some(home)), "{p}");
        }
        assert!(!is_secret_path(
            "~/Library/Application Support/Code/x",
            home,
            Some(home)
        ));
    }
}
