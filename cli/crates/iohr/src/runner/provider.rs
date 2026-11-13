//! Asking the binary how it is signed in (RFC 0120, "Provider checks, every turn"; RFC
//! 0214): on an Anthropic API key, or on the person's own Claude plan. The runner never
//! reads `.credentials.json`, the keychain or `auth.json`: it runs `claude auth status`
//! and reads what it prints. Which of the two a turn may run on is the [`AuthPolicy`].

use std::ffi::OsString;
use std::path::PathBuf;

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
/// the person's own plan, `api` an API key alone, `plan` the person's own plan alone.
/// The runner's local setting and the account's policy (sent with each turn) must both
/// allow a sign-in: the server can narrow a runner, never widen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthPolicy {
    Any,
    Plan,
    Api,
}

impl AuthPolicy {
    /// `any`, `plan` or `api`; anything else is `None` (config) or, from the server,
    /// read as `api` by [`AuthPolicy::from_server`].
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "any" => Some(Self::Any),
            "plan" => Some(Self::Plan),
            "api" => Some(Self::Api),
            _ => None,
        }
    }

    /// The account's policy for one provider, as the turn carries it: absent is no
    /// narrowing, a value the runner does not know never allows the person's own plan.
    pub(crate) fn from_server(v: Option<&str>) -> Option<Self> {
        v.map(|s| Self::parse(s).unwrap_or(Self::Api))
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Plan => "plan",
            Self::Api => "api",
        }
    }

    /// The default when the runner's config says nothing: Claude runs on an API key
    /// only (the person's own plan is turned on in local config, never by default),
    /// Codex takes either, and a provider the runner does not know takes a key only.
    pub(crate) fn default_for(provider: &str) -> Self {
        match provider {
            "codex" => Self::Any,
            _ => Self::Api,
        }
    }

    /// May a turn run on this sign-in under this policy?
    pub(crate) fn allows(self, auth: &AuthState) -> bool {
        match auth {
            AuthState::ApiKey => self != Self::Plan,
            AuthState::Subscription => self != Self::Api,
            AuthState::SignedOut | AuthState::Refused(_) => false,
        }
    }

    /// Whether a stored API key goes to the provider: not under `plan`, where the
    /// turn runs on the person's own sign-in.
    pub(crate) fn uses_key(self) -> bool {
        self != Self::Plan
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
    /// does not say): `(outcome, detail)`. A sign-in runs only where both the local setting
    /// and the account allow it; the person's own plan also needs the personal-use
    /// notice accepted, and takes only work this person assigned.
    pub(crate) fn refusal(
        &self,
        provider: &str,
        auth: &AuthState,
        server: Option<&str>,
        (creator, me): (&str, &str),
    ) -> Option<(&'static str, String)> {
        match auth {
            AuthState::ApiKey | AuthState::Subscription => {
                let local = self.policy(provider);
                let narrowed = AuthPolicy::from_server(server).filter(|a| !a.allows(auth));
                let refused = if local.allows(auth) {
                    narrowed.map(|a| (a, format!("the account's policy is {}", a.as_str())))
                } else {
                    Some((local, format!("{provider}_auth = {}", local.as_str())))
                };
                if let Some((policy, whose)) = refused {
                    Some((
                        "refused_provider",
                        if policy == AuthPolicy::Plan {
                            format!(
                                "{provider} turns here run on your own plan only ({whose}); sign in \
with `claude`, then `/login`"
                            )
                        } else {
                            format!(
                                "{provider} turns here run on an API key only ({whose}); store one \
with `iohr runner api-key set`"
                            )
                        },
                    ))
                } else if *auth == AuthState::ApiKey {
                    None
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

/// The first `name` file on `PATH`, as an absolute path: the binary as installed
/// ([`crate::ask::find_on_path`]).
pub(crate) fn find_on_path(name: &str, path: Option<&OsString>) -> Option<PathBuf> {
    crate::ask::find_on_path(name, path?)
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
    fn auth_policy_is_any_plan_or_api_and_unknown_never_allows_the_plan() {
        use AuthPolicy::{Any, Api, Plan};
        assert_eq!(AuthPolicy::default_for("claude"), Api);
        assert_eq!(AuthPolicy::default_for("codex"), Any);
        assert_eq!(AuthPolicy::default_for("gemini"), Api);
        for p in [Any, Plan, Api] {
            assert_eq!(AuthPolicy::parse(p.as_str()), Some(p));
        }
        assert_eq!(AuthPolicy::from_server(None), None);
        assert_eq!(AuthPolicy::from_server(Some("plan")), Some(Plan));
        assert_eq!(AuthPolicy::from_server(Some("subscription_ok")), Some(Api));
        assert_eq!(AuthPolicy::parse("API"), None);
        assert_eq!(AuthPolicy::parse("api_only"), None);
        let (key, sub) = (AuthState::ApiKey, AuthState::Subscription);
        assert!(Any.allows(&key) && Any.allows(&sub));
        assert!(Api.allows(&key) && !Api.allows(&sub));
        assert!(!Plan.allows(&key) && Plan.allows(&sub));
        for p in [Any, Plan, Api] {
            assert!(!p.allows(&AuthState::SignedOut));
            assert!(!p.allows(&AuthState::Refused(String::new())));
        }
        assert!(Any.uses_key() && Api.uses_key() && !Plan.uses_key());
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
        // Nothing configured: api, so the person's own plan never runs.
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
        // The account narrows it, and a value the runner does not know never allows it.
        assert_eq!(out(&ok, Some("plan"), me), None);
        let (_, why) = ok.refusal("claude", &sub, Some("api"), me).unwrap();
        assert!(why.contains("the account's policy is api"), "{why}");
        assert_eq!(out(&ok, Some("whatever"), me), Some("refused_provider"));
        // An API key runs under api or any, whatever the acceptance or the creator.
        let key = AuthState::ApiKey;
        let boss = ("usr_boss", "usr_me");
        assert!(none.refusal("claude", &key, Some("api"), boss).is_none());
        assert!(ok.refusal("claude", &key, Some("any"), boss).is_none());
        // Under plan, locally or from the account, an API key does not run.
        let plan = LocalAuth {
            claude: AuthPolicy::Plan,
            ..ok
        };
        let (o, why) = plan.refusal("claude", &key, None, me).unwrap();
        assert_eq!(o, "refused_provider");
        assert!(why.contains("claude_auth = plan"), "{why}");
        let (_, why) = ok.refusal("claude", &key, Some("plan"), me).unwrap();
        assert!(why.contains("the account's policy is plan"), "{why}");
        assert_eq!(out(&plan, None, me), None);
        // Plan here and api on the account leave nothing to run on.
        assert_eq!(out(&plan, Some("api"), me), Some("refused_provider"));
        assert!(plan.refusal("claude", &key, Some("api"), me).is_some());
        assert_eq!(
            ok.refusal("claude", &AuthState::SignedOut, None, me)
                .map(|(o, _)| o),
            Some("login_expired")
        );
        // Every provider has the switch: codex is any unless set.
        assert_eq!(none.policy("codex"), AuthPolicy::Any);
        assert_eq!(none.policy("gemini"), AuthPolicy::Api);
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
        // Windows finds a program by its PATHEXT extension, so the file there is claude.exe.
        let file = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let bin = dir.path().join(file);
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path =
            std::env::join_paths([PathBuf::from("relative"), dir.path().to_owned()]).unwrap();
        // The lookup returns the extension as PATHEXT spells it (`.EXE`); Windows paths
        // ignore case, so the comparison does too.
        let lower = |p: &std::path::Path| p.to_string_lossy().to_lowercase();
        assert_eq!(
            find_on_path("claude", Some(&path)).as_deref().map(lower),
            Some(lower(&bin))
        );
        assert_eq!(find_on_path("claude", None), None);
    }
}
