//! Asking the binary whether it is signed in on the person's subscription (RFC 0120,
//! "Provider checks, every turn"). The runner never reads `.credentials.json`, the
//! keychain or `auth.json`: it runs `claude auth status` and reads what it prints.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// What `claude auth status` said, reduced to what the runner may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthState {
    /// `authMethod: claude.ai` and `apiProvider: firstParty`: the subscription.
    Subscription,
    /// Not signed in: the person runs `claude` and `/login`.
    SignedOut,
    /// Signed in some other way (an API key, Bedrock, Vertex, a token): refused.
    Refused(String),
}

impl AuthState {
    pub(crate) fn signed_in(&self) -> bool {
        matches!(self, Self::Subscription)
    }
}

/// Reads `claude auth status` output. Anything not plainly the subscription is refused;
/// an unreadable answer is never taken as a pass.
pub(crate) fn parse_auth_status(text: &str) -> AuthState {
    let Ok(v) = serde_json::from_str::<Value>(text.trim()) else {
        return AuthState::Refused("claude auth status did not answer in JSON".into());
    };
    let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
    let logged_in = v
        .get("loggedIn")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| !get("authMethod").is_empty());
    if !logged_in {
        return AuthState::SignedOut;
    }
    match (get("authMethod"), get("apiProvider")) {
        ("claude.ai", "firstParty") => AuthState::Subscription,
        (m, p) => AuthState::Refused(format!(
            "sign-in method \"{m}\" on provider \"{p}\" is not the subscription (claude.ai, firstParty)"
        )),
    }
}

/// The first `name` file on `PATH`, as an absolute path: the binary as installed.
pub(crate) fn find_on_path(name: &str, path: Option<&OsString>) -> Option<PathBuf> {
    let path = path?;
    std::env::split_paths(path)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| is_exe(p))
}

#[cfg(unix)]
fn is_exe(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_exe(p: &Path) -> bool {
    p.is_file() || p.with_extension("exe").is_file() || p.with_extension("cmd").is_file()
}

/// SHA-256 (lowercase hex) over the account's salt, a newline and the identity line as
/// the binary printed it (the salt comes with `CreateRunnerChallenge`).
pub(crate) fn identity_sha256(salt: &str, line: &str) -> String {
    super::claude::hex(&Sha256::digest(
        format!("{salt}\n{}", line.trim()).as_bytes(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    #[test]
    fn auth_status_table() {
        let cases = [
            (
                r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"a@b.c"}"#,
                AuthState::Subscription,
            ),
            (r#"{"loggedIn":false}"#, AuthState::SignedOut),
            (
                r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#,
                AuthState::Refused(String::new()),
            ),
            (
                r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"bedrock"}"#,
                AuthState::Refused(String::new()),
            ),
            (
                r#"{"loggedIn":true,"authMethod":"oauth_token","apiProvider":"firstParty"}"#,
                AuthState::Refused(String::new()),
            ),
            ("Logged in as someone", AuthState::Refused(String::new())),
            ("", AuthState::Refused(String::new())),
        ];
        for (text, want) in cases {
            let got = parse_auth_status(text);
            match want {
                AuthState::Refused(_) => assert!(matches!(got, AuthState::Refused(_)), "{text}"),
                other => assert_eq!(got, other, "{text}"),
            }
        }
    }

    #[test]
    fn identity_is_a_lowercase_sha256() {
        let h = identity_sha256("salt", "line\n");
        assert_eq!(h.len(), 64);
        assert!(
            h.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_eq!(h, identity_sha256("salt", "line"));
        // sha256("salt\nline"), and another salt is another hash.
        assert_eq!(
            h,
            "2ddda023dfab174ef508faa656c099f1cd248fd6d23d514a7e5cf678bfaf92d3"
        );
        assert_ne!(h, identity_sha256("other", "line"));
    }

    #[test]
    fn path_lookup_takes_absolute_entries_only() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path =
            std::env::join_paths([PathBuf::from("relative"), dir.path().to_owned()]).unwrap();
        assert_eq!(find_on_path("claude", Some(&path)), Some(bin));
        assert_eq!(find_on_path("claude", None), None);
    }
}
