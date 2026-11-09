//! Asking the binary how it is signed in (RFC 0120, "Provider checks, every turn"; RFC
//! 0214): on an Anthropic API key, or on the person's own Claude plan. The runner never
//! reads `.credentials.json`, the keychain or `auth.json`: it runs `claude auth status`
//! and reads what it prints. Which of the two a turn may run on is the [`AuthPolicy`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// What `claude auth status` said, reduced to what the runner may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthState {
    /// `authMethod: api_key` and `apiProvider: firstParty`: an Anthropic API key, the
    /// one the runner hands claude from the credential store.
    ApiKey,
    /// `authMethod: claude.ai` and `apiProvider: firstParty`: the person's own plan,
    /// for personal use only.
    Subscription,
    /// Not signed in.
    SignedOut,
    /// Signed in some other way (Bedrock, Vertex, a token): refused.
    Refused(String),
}

impl AuthState {
    pub(crate) fn signed_in(&self) -> bool {
        matches!(self, Self::ApiKey | Self::Subscription)
    }

    /// The method the runner reports to the coordinator: `api_key`, `subscription`, or
    /// empty when it is not signed in in a way it may use.
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Subscription => "subscription",
            Self::SignedOut | Self::Refused(_) => "",
        }
    }
}

/// Which sign-in a provider's turns may run on (RFC 0214): `any` takes an API key or
/// the person's own plan, `api_only` takes an API key alone. The runner's local
/// setting and the account's policy (sent with each turn) are both read, and the
/// stricter wins: the server can tighten a runner, never loosen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AuthPolicy {
    Any,
    ApiOnly,
}

impl AuthPolicy {
    /// `any` or `api_only`; anything else is `None` (config) or, from the server,
    /// read as `api_only` by [`AuthPolicy::from_server`].
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "any" => Some(Self::Any),
            "api_only" => Some(Self::ApiOnly),
            _ => None,
        }
    }

    /// The account's policy for one provider, as the turn carries it: absent is no
    /// tightening, a value the runner does not know is the strictest it does.
    pub(crate) fn from_server(v: Option<&str>) -> Option<Self> {
        v.map(|s| Self::parse(s).unwrap_or(Self::ApiOnly))
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::ApiOnly => "api_only",
        }
    }

    /// The stricter of the two.
    pub(crate) fn stricter(self, other: Option<Self>) -> Self {
        other.map_or(self, |o| self.max(o))
    }

    /// The default when the runner's config says nothing: Claude runs on an API key
    /// only (the person's own plan is turned on in local config, never by default),
    /// Codex takes either, and a provider the runner does not know takes a key only.
    pub(crate) fn default_for(provider: &str) -> Self {
        match provider {
            "codex" => Self::Any,
            _ => Self::ApiOnly,
        }
    }

    /// May a turn run on this sign-in under this policy?
    pub(crate) fn allows(self, auth: &AuthState) -> bool {
        match auth {
            AuthState::ApiKey => true,
            AuthState::Subscription => self == Self::Any,
            AuthState::SignedOut | AuthState::Refused(_) => false,
        }
    }
}

/// What this machine says about sign-in (RFC 0214), read from `config.toml` and
/// `runner/personal-use.json` at start and on SIGHUP: each provider's local policy and
/// whether the person accepted the personal-use notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalAuth {
    pub(crate) claude: AuthPolicy,
    pub(crate) codex: AuthPolicy,
    /// The acceptance of the personal-use notice, if there is one.
    pub(crate) personal_use: Option<super::personal_use::Accepted>,
}

impl Default for LocalAuth {
    /// Nothing configured: each provider's default and no acceptance.
    fn default() -> Self {
        Self {
            claude: AuthPolicy::default_for(super::claude::PROVIDER),
            codex: AuthPolicy::default_for("codex"),
            personal_use: None,
        }
    }
}

impl LocalAuth {
    /// The local policy for `provider`.
    pub(crate) fn policy(&self, provider: &str) -> AuthPolicy {
        match provider {
            super::claude::PROVIDER => self.claude,
            "codex" => self.codex,
            other => AuthPolicy::default_for(other),
        }
    }

    /// Whether the personal-use notice is accepted and still holds now.
    pub(crate) fn personal_use_holds(&self) -> bool {
        self.personal_use
            .is_some_and(|a| a.holds(std::time::SystemTime::now()))
    }

    /// Why a turn may not run on `auth` for `provider`, with the account's policy for it
    /// (`server`) and the person who created the job (`creator`, empty when the service
    /// does not say): `(outcome, detail)`. An API key always runs; the person's own plan
    /// runs only where both the local setting and the account allow it, after the
    /// personal-use notice was accepted, and only on work this person assigned.
    pub(crate) fn refusal(
        &self,
        provider: &str,
        auth: &AuthState,
        server: Option<&str>,
        (creator, me): (&str, &str),
    ) -> Option<(&'static str, String)> {
        match auth {
            AuthState::ApiKey => None,
            AuthState::Subscription => {
                let policy = self
                    .policy(provider)
                    .stricter(AuthPolicy::from_server(server));
                if !policy.allows(auth) {
                    Some((
                        "refused_provider",
                        format!(
                            "{provider} turns here run on an API key only ({provider}_auth = api_only); \
store one with `iohr runner api-key set`"
                        ),
                    ))
                } else if !self.personal_use_holds() {
                    Some((
                        "refused_provider",
                        "running on your own Claude plan is personal use only; read and accept the \
notice with `iohr runner personal-use`, or store an API key with `iohr runner api-key set`"
                            .to_owned(),
                    ))
                } else if !creator.is_empty() && creator != me {
                    Some((
                        "refused_user",
                        "personal use only: on your own Claude plan this runner takes only work you \
assign yourself"
                            .to_owned(),
                    ))
                } else {
                    None
                }
            }
            AuthState::SignedOut => Some((
                "login_expired",
                format!("{provider} is signed out on this machine"),
            )),
            AuthState::Refused(why) => Some(("refused_provider", why.clone())),
        }
    }
}

/// Reads `claude auth status` output. Only a first-party API key or the person's own
/// plan counts; an unreadable answer is never taken as a pass.
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
        ("api_key", "firstParty") => AuthState::ApiKey,
        ("claude.ai", "firstParty") => AuthState::Subscription,
        (m, p) => AuthState::Refused(format!(
            "sign-in method \"{m}\" on provider \"{p}\" is neither an Anthropic API key nor a Claude plan"
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
                r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty","apiKeySource":"ANTHROPIC_API_KEY"}"#,
                AuthState::ApiKey,
            ),
            (
                r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"bedrock"}"#,
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
    fn auth_policy_stricter_wins_and_unknown_is_api_only() {
        use AuthPolicy::{Any, ApiOnly};
        assert_eq!(AuthPolicy::default_for("claude"), ApiOnly);
        assert_eq!(AuthPolicy::default_for("codex"), Any);
        assert_eq!(AuthPolicy::default_for("gemini"), ApiOnly);
        assert_eq!(Any.stricter(None), Any);
        assert_eq!(Any.stricter(Some(ApiOnly)), ApiOnly);
        assert_eq!(
            ApiOnly.stricter(Some(Any)),
            ApiOnly,
            "the server never loosens"
        );
        assert_eq!(AuthPolicy::from_server(None), None);
        assert_eq!(AuthPolicy::from_server(Some("any")), Some(Any));
        assert_eq!(
            AuthPolicy::from_server(Some("subscription_ok")),
            Some(ApiOnly)
        );
        assert_eq!(AuthPolicy::parse("API_ONLY"), None);
        assert!(ApiOnly.allows(&AuthState::ApiKey));
        assert!(!ApiOnly.allows(&AuthState::Subscription));
        assert!(Any.allows(&AuthState::Subscription));
        assert!(!Any.allows(&AuthState::SignedOut));
        assert!(!Any.allows(&AuthState::Refused(String::new())));
        assert_eq!(AuthState::ApiKey.method(), "api_key");
        assert_eq!(AuthState::Subscription.method(), "subscription");
    }

    #[test]
    fn the_own_plan_runs_only_where_allowed_accepted_and_self_assigned() {
        use super::super::personal_use::Accepted;
        let sub = AuthState::Subscription;
        let me = ("usr_me", "usr_me");
        let out = |l: &LocalAuth, s: Option<&str>, who: (&str, &str)| {
            l.refusal("claude", &sub, s, who).map(|(o, _)| o)
        };
        // Nothing configured: api_only, so the person's own plan never runs.
        let none = LocalAuth::default();
        assert_eq!(out(&none, None, me), Some("refused_provider"));
        assert!(
            none.refusal("claude", &AuthState::ApiKey, None, me)
                .is_none()
        );
        // Turned on locally but not accepted: still refused.
        let on = LocalAuth {
            claude: AuthPolicy::Any,
            ..LocalAuth::default()
        };
        let (_, why) = on.refusal("claude", &sub, None, me).unwrap();
        assert!(why.contains("personal use only"), "{why}");
        // On and accepted: runs on work its person assigned (or work that names nobody).
        let ok = LocalAuth {
            personal_use: Some(Accepted::now()),
            ..on
        };
        assert_eq!(out(&ok, None, me), None);
        assert_eq!(out(&ok, Some("any"), ("", "usr_me")), None);
        assert_eq!(out(&ok, None, ("usr_boss", "usr_me")), Some("refused_user"));
        // The account tightens it, and an unknown value is the strictest.
        assert_eq!(out(&ok, Some("api_only"), me), Some("refused_provider"));
        assert_eq!(out(&ok, Some("whatever"), me), Some("refused_provider"));
        // An API key runs whatever the policy, the acceptance or the creator.
        assert!(
            none.refusal(
                "claude",
                &AuthState::ApiKey,
                Some("api_only"),
                ("usr_boss", "usr_me")
            )
            .is_none()
        );
        assert_eq!(
            ok.refusal("claude", &AuthState::SignedOut, None, me)
                .map(|(o, _)| o),
            Some("login_expired")
        );
        // Every provider has the switch: codex is any unless set.
        assert_eq!(none.policy("codex"), AuthPolicy::Any);
        assert_eq!(none.policy("gemini"), AuthPolicy::ApiOnly);
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
