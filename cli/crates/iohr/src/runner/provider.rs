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

/// The canonical identity line of a subscription sign-in, from `claude auth status`:
///
/// `authMethod=claude.ai apiProvider=firstParty orgId=<orgId> email=<email>`
///
/// with `orgId` and `email` trimmed and lowercased. Only the fields that name the seat go
/// in: the sign-in method and provider (always the subscription's, as nothing else is
/// accepted), the organisation and the account's email. Fields that change on a token
/// refresh or with the host (`loginExpiresAt`, `configDirectory`, `projectsDirectory`),
/// or that the person may change without changing seat (`orgName`, `subscriptionType`),
/// stay out, so one seat gives one line on every machine and after every refresh.
///
/// `None` when the answer is not plainly a subscription sign-in naming both an
/// organisation and an email: such an answer names no seat and is never hashed.
pub(crate) fn identity_line(text: &str) -> Option<String> {
    if parse_auth_status(text) != AuthState::Subscription {
        return None;
    }
    let v = serde_json::from_str::<Value>(text.trim()).ok()?;
    let field = |k: &str| {
        let s = v.get(k)?.as_str()?.trim().to_lowercase();
        (!s.is_empty() && !s.contains(char::is_whitespace)).then_some(s)
    };
    let org = field("orgId")?;
    let email = field("email").filter(|e| e.contains('@'))?;
    Some(format!(
        "authMethod=claude.ai apiProvider=firstParty orgId={org} email={email}"
    ))
}

/// SHA-256 (lowercase hex) over the account's salt, a newline and the identity line
/// ([`identity_line`]; the salt comes with `CreateRunnerChallenge`).
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

    /// A salt drawn at runtime, as the real one comes from the account.
    fn salt() -> String {
        let mut b = [0_u8; 16];
        getrandom::fill(&mut b).unwrap();
        super::super::claude::hex(&b)
    }

    #[test]
    fn identity_is_a_lowercase_sha256() {
        let (salt, other) = (salt(), salt());
        assert_ne!(salt, other);
        let h = identity_sha256(&salt, "line\n");
        assert_eq!(h.len(), 64);
        assert!(
            h.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_eq!(h, identity_sha256(&salt, "line"));
        // sha256("<salt>\nline"), and another salt is another hash.
        let want = super::super::claude::hex(&Sha256::digest(format!("{salt}\nline").as_bytes()));
        assert_eq!(h, want);
        assert_ne!(h, identity_sha256(&other, "line"));
    }

    /// A subscription sign-in as `claude auth status` prints it, with every field.
    fn status(email: &str, org: &str, expires: &str, config: &str, projects: &str) -> String {
        serde_json::json!({
            "analyticsDisabled": false,
            "apiProvider": "firstParty",
            "authMethod": "claude.ai",
            "configDirectory": config,
            "email": email,
            "loggedIn": true,
            "loginExpiresAt": expires,
            "orgId": org,
            "orgName": "Example",
            "projectsDirectory": projects,
            "subscriptionType": "max",
        })
        .to_string()
    }

    const ORG: &str = "00000000-0000-4000-8000-00000000000a";

    #[test]
    fn identity_line_is_the_seat_alone() {
        let line = identity_line(&status(
            "seat@example.com",
            ORG,
            "2026-11-05T10:00:00Z",
            "/home/a/.claude",
            "/home/a/.claude/projects",
        ))
        .unwrap();
        assert_eq!(
            line,
            format!(
                "authMethod=claude.ai apiProvider=firstParty orgId={ORG} email=seat@example.com"
            )
        );
    }

    #[test]
    fn identity_hash_is_stable_across_refresh_and_hosts() {
        let salt = salt();
        let hash = |text: &str| identity_sha256(&salt, &identity_line(text).unwrap());
        let base = hash(&status(
            "seat@example.com",
            ORG,
            "2026-11-05T10:00:00Z",
            "/home/a/.claude",
            "/home/a/.claude/projects",
        ));
        // A token refresh, another host, another config directory, the email's case.
        for other in [
            status(
                "seat@example.com",
                ORG,
                "2026-11-06T10:00:00Z",
                "/home/a/.claude",
                "/home/a/.claude/projects",
            ),
            status(
                "seat@example.com",
                ORG,
                "2026-11-05T10:00:00Z",
                "/Users/b/.config/claude",
                "/home/a/.claude/projects",
            ),
            status(
                "seat@example.com",
                ORG,
                "2026-11-05T10:00:00Z",
                "/home/a/.claude",
                "/srv/projects",
            ),
            status(
                "Seat@Example.COM",
                ORG,
                "2026-11-05T10:00:00Z",
                "/home/a/.claude",
                "/home/a/.claude/projects",
            ),
            status(
                " seat@example.com ",
                &ORG.to_uppercase(),
                "2026-11-07T00:00:00Z",
                "/x",
                "/y",
            ),
        ] {
            assert_eq!(hash(&other), base, "{other}");
        }
        // Another seat: another email, or the same email in another organisation.
        for other in [
            status(
                "other@example.com",
                ORG,
                "2026-11-05T10:00:00Z",
                "/home/a/.claude",
                "/home/a/.claude/projects",
            ),
            status(
                "seat@example.com",
                "00000000-0000-4000-8000-00000000000b",
                "2026-11-05T10:00:00Z",
                "/home/a/.claude",
                "/home/a/.claude/projects",
            ),
        ] {
            assert_ne!(hash(&other), base, "{other}");
        }
    }

    #[test]
    fn identity_line_fails_closed() {
        let ok = status("seat@example.com", ORG, "t", "/c", "/p");
        assert!(identity_line(&ok).is_some());
        let without = |k: &str| {
            let mut v: Value = serde_json::from_str(&ok).unwrap();
            v.as_object_mut().unwrap().remove(k);
            v.to_string()
        };
        let with = |k: &str, x: Value| {
            let mut v: Value = serde_json::from_str(&ok).unwrap();
            v[k] = x;
            v.to_string()
        };
        for text in [
            String::new(),
            "Logged in as seat@example.com".to_owned(),
            r#"{"loggedIn":false}"#.to_owned(),
            without("email"),
            without("orgId"),
            with("email", "".into()),
            with("email", "not-an-email".into()),
            with("email", "a b@example.com".into()),
            with("email", Value::Null),
            with("orgId", " ".into()),
            with("orgId", 7.into()),
            with("authMethod", "api_key".into()),
            with("apiProvider", "bedrock".into()),
            with("loggedIn", false.into()),
        ] {
            assert_eq!(identity_line(&text), None, "{text}");
        }
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
