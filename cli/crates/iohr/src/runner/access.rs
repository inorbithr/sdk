//! How git and `gh` sign in to GitHub for one job (RFC 0120, the runner's git access; the
//! RFC 0147 containment ask).
//!
//! The platform mints a short-lived GitHub App installation token per job lease
//! (`MintRunnerGithubToken`): the job's one repository, contents and pull requests write,
//! for about an hour. The runner keeps it in memory only and hands it to one child at a
//! time: git through a credential helper given on that command's line (`-c`), which reads
//! it from [`TOKEN_VAR`] set on that child only, and `gh` through `GH_TOKEN` on that child
//! only. It is never written to the mirror's configuration or any file, never logged, never
//! in a `Debug` or an error, and never in the model's or a check's environment
//! ([`super::env::contained`]).
//!
//! Only when the platform says the GitHub App is not installed for the account, or is too
//! old to mint a token, does the runner use the host's own `gh` sign-in, and it says why.
//! Every other refusal (a lease that is not live, a repository the installation does not
//! list, a permission it lacks, GitHub unavailable) is an error: nothing is fetched or
//! pushed.

use std::fmt;
use std::time::Duration;

use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use zeroize::Zeroizing;

/// The variable the credential helper reads; set on one git child at a time, never on
/// the runner itself.
pub(crate) const TOKEN_VAR: &str = "IOHR_GIT_TOKEN";
/// A token with less than this left is asked for again before it is used.
pub(crate) const REFRESH_BEFORE: Duration = Duration::from_mins(10);
/// The one credential key the runner sets, per command: GitHub over https only, so a
/// remote rewritten to another host never gets the token.
const HELPER_KEY: &str = "credential.https://github.com.helper";
/// The host's own `gh` sign-in, as git's helper.
const HOST_HELPER: &str = "!gh auth git-credential";
/// Answers `get` with the token from [`TOKEN_VAR`]; `store` and `erase` do nothing. The
/// token is never on a command line, only in that one child's environment.
const TOKEN_HELPER: &str = "!f() { test \"$1\" = get && printf 'username=x-access-token\\npassword=%s\\n' \"$IOHR_GIT_TOKEN\"; }; f";
/// What the platform answers when the account has no GitHub App installation.
pub(crate) const NOT_INSTALLED: &str = "not_installed";

/// An installation token: its value never prints.
#[derive(Clone)]
pub(crate) struct Token {
    value: Zeroizing<String>,
    expires_at: OffsetDateTime,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("value", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// How one job's git and `gh` calls sign in to GitHub.
#[derive(Clone, Debug)]
pub(crate) enum Access {
    /// The lease's own installation token.
    Token(Token),
    /// The host's `gh` sign-in, and why (the platform has no GitHub App for the account,
    /// or no such call).
    Host(String),
}

/// What `MintRunnerGithubToken` answered, as it came: a token, or a reason with a
/// sentence for a person.
#[derive(Clone, Default)]
pub(crate) struct Minted {
    token: Zeroizing<String>,
    pub(crate) expires_at: String,
    pub(crate) repository: String,
    pub(crate) reason: String,
    pub(crate) detail: String,
}

impl fmt::Debug for Minted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Minted")
            .field(
                "token",
                &if self.token.is_empty() {
                    ""
                } else {
                    "[redacted]"
                },
            )
            .field("expires_at", &self.expires_at)
            .field("repository", &self.repository)
            .field("reason", &self.reason)
            .field("detail", &self.detail)
            .finish()
    }
}

impl Minted {
    /// The response body of `MintRunnerGithubToken`.
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        Self {
            token: Zeroizing::new(s("token")),
            expires_at: s("expires_at"),
            repository: s("repository"),
            reason: s("reason"),
            detail: s("detail"),
        }
    }

    #[cfg(test)]
    pub(crate) fn token(token: &str, expires_at: &str, repository: &str) -> Self {
        Self {
            token: Zeroizing::new(token.to_owned()),
            expires_at: expires_at.to_owned(),
            repository: repository.to_owned(),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn refused(reason: &str, detail: &str) -> Self {
        Self {
            reason: reason.to_owned(),
            detail: detail.to_owned(),
            ..Self::default()
        }
    }

    /// The access this answer gives for `repository`, or why there is none. A token for
    /// another repository, or one that could break the helper's lines, is refused; an
    /// expiry that does not parse counts as now, so the token is asked for again before
    /// its next use.
    pub(crate) fn access(self, repository: &str, now: OffsetDateTime) -> Result<Access, String> {
        if self.token.is_empty() {
            if self.reason == NOT_INSTALLED {
                return Ok(Access::Host(format!(
                    "the platform has no GitHub App installation for this account ({})",
                    self.detail_or("not installed")
                )));
            }
            return Err(format!(
                "no GitHub access for this job: {} ({})",
                if self.reason.is_empty() {
                    "no reason given"
                } else {
                    self.reason.as_str()
                },
                self.detail_or("no detail")
            ));
        }
        if !self.repository.eq_ignore_ascii_case(repository) {
            return Err(format!(
                "the platform's token is for {:?}, not this job's {repository:?}; not used",
                self.repository
            ));
        }
        if self
            .token
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("the platform's token is malformed; not used".into());
        }
        let expires_at = OffsetDateTime::parse(&self.expires_at, &Rfc3339).unwrap_or(now);
        Ok(Access::Token(Token {
            value: self.token,
            expires_at,
        }))
    }

    fn detail_or<'a>(&'a self, or: &'a str) -> &'a str {
        if self.detail.is_empty() {
            or
        } else {
            &self.detail
        }
    }
}

impl Access {
    /// The host's sign-in, and why.
    pub(crate) fn host(why: impl Into<String>) -> Self {
        Self::Host(why.into())
    }

    /// Good for the calls about to be made: the host's sign-in always, a token with more
    /// than [`REFRESH_BEFORE`] left.
    pub(crate) fn fresh(&self, now: OffsetDateTime) -> bool {
        match self {
            Self::Host(_) => true,
            Self::Token(t) => t.expires_at - now > REFRESH_BEFORE,
        }
    }

    /// What every git call that reaches GitHub is given before its own arguments: every
    /// helper of every configuration file dropped, then the one helper for
    /// `https://github.com`, and no hooks (a hook would inherit the token's variable).
    pub(crate) fn git_args(&self) -> [String; 6] {
        let helper = match self {
            Self::Token(_) => TOKEN_HELPER,
            Self::Host(_) => HOST_HELPER,
        };
        [
            "-c".into(),
            "credential.helper=".into(),
            "-c".into(),
            format!("{HELPER_KEY}={helper}"),
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
        ]
    }

    /// The variable that git child gets, if any: ([`TOKEN_VAR`], the token).
    pub(crate) fn git_env(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Token(t) => Some((TOKEN_VAR, t.value.as_str())),
            Self::Host(_) => None,
        }
    }

    /// `GH_TOKEN` for that one `gh` child, if any.
    pub(crate) fn gh_token(&self) -> Option<&str> {
        match self {
            Self::Token(t) => Some(t.value.as_str()),
            Self::Host(_) => None,
        }
    }

    /// A line for the runner's log: which access, never the token.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Token(t) => format!(
                "the lease's GitHub App token (one repository, until {})",
                t.expires_at.format(&Rfc3339).unwrap_or_default()
            ),
            Self::Host(why) => format!("this host's gh sign-in: {why}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    /// A made-up token: the shape of an installation token, never a real one.
    pub(crate) const FAKE: &str = "ghs_fake_runner_token_zebrafinch";

    pub(crate) fn token_for(repository: &str) -> Access {
        let later = OffsetDateTime::now_utc() + Duration::from_hours(1);
        Minted::token(FAKE, &later.format(&Rfc3339).unwrap(), repository)
            .access(repository, OffsetDateTime::now_utc())
            .unwrap()
    }

    #[test]
    fn a_token_never_prints_in_a_debug_or_a_log_line() {
        let a = token_for("acme/app");
        let m = Minted::token(FAKE, "2026-10-24T12:00:00Z", "acme/app");
        for text in [
            format!("{a:?}"),
            format!("{m:?}"),
            a.describe(),
            a.git_args().join(" "),
        ] {
            assert!(!text.contains(FAKE), "{text}");
            assert!(!text.contains("zebrafinch"), "{text}");
        }
        assert_eq!(a.git_env(), Some((TOKEN_VAR, FAKE)));
        assert_eq!(a.gh_token(), Some(FAKE));
    }

    #[test]
    fn only_not_installed_falls_back_to_the_host_and_says_why() {
        let now = OffsetDateTime::now_utc();
        let host = Minted::refused(NOT_INSTALLED, "no installation")
            .access("acme/app", now)
            .unwrap();
        assert!(matches!(&host, Access::Host(why) if why.contains("no installation")));
        assert_eq!(host.git_env(), None);
        assert_eq!(host.gh_token(), None);
        assert!(host.git_args()[3].ends_with("!gh auth git-credential"));
        for reason in [
            "not_listed",
            "needs_permission",
            "rate_limited",
            "unavailable",
            "invalid_name",
            "",
        ] {
            let e = Minted::refused(reason, "d").access("acme/app", now);
            assert!(e.is_err(), "{reason}");
        }
    }

    #[test]
    fn a_token_for_another_repository_or_a_malformed_one_is_not_used() {
        let now = OffsetDateTime::now_utc();
        let e = Minted::token(FAKE, "2026-10-24T12:00:00Z", "acme/other")
            .access("acme/app", now)
            .unwrap_err();
        assert!(!e.contains(FAKE));
        assert!(
            Minted::token("ghs_a\npassword=x", "", "acme/app")
                .access("acme/app", now)
                .is_err()
        );
        assert!(
            Minted::token(FAKE, "", "ACME/App")
                .access("acme/app", now)
                .is_ok()
        );
    }

    #[test]
    fn a_token_is_asked_for_again_ten_minutes_before_it_expires() {
        let now = OffsetDateTime::now_utc();
        let at = |mins: i64| {
            let t = now + time::Duration::minutes(mins);
            Minted::token(FAKE, &t.format(&Rfc3339).unwrap(), "acme/app")
                .access("acme/app", now)
                .unwrap()
        };
        assert!(at(60).fresh(now));
        assert!(!at(9).fresh(now));
        assert!(!at(-1).fresh(now));
        // An expiry that does not parse counts as now.
        let odd = Minted::token(FAKE, "soon", "acme/app")
            .access("acme/app", now)
            .unwrap();
        assert!(!odd.fresh(now));
        assert!(Access::host("x").fresh(now));
    }

    /// `git credential fill` with `access`'s arguments and variable, a host configuration
    /// that names its own helpers, and `host`: what git would send.
    fn fill(access: &Access, host: &str) -> (bool, String) {
        let d = tempfile::tempdir().unwrap();
        let global = d.path().join("gitconfig");
        std::fs::write(
            &global,
            "[credential]\n\thelper = \"!f() { echo password=host-secret; }; f\"\n\
             [credential \"https://github.com\"]\n\thelper = \"!f() { echo password=host-secret; }; f\"\n",
        )
        .unwrap();
        let mut c = std::process::Command::new("git");
        c.args(access.git_args())
            .args(["credential", "fill"])
            .current_dir(d.path())
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove(TOKEN_VAR)
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .envs(access.git_env())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = c.spawn().unwrap();
        {
            use std::io::Write as _;
            let mut i = child.stdin.take().unwrap();
            write!(i, "protocol=https\nhost={host}\npath=acme/app.git\n\n").unwrap();
        }
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    #[test]
    fn git_gets_the_token_for_github_only_and_never_a_host_helper() {
        let a = token_for("acme/app");
        let (ok, got) = fill(&a, "github.com");
        assert!(ok, "{got}");
        assert!(got.contains(&format!("password={FAKE}")), "the token");
        assert!(got.contains("username=x-access-token"));
        assert!(!got.contains("host-secret"), "a host helper answered");
        let (ok, got) = fill(&a, "elsewhere.example");
        assert!(!ok);
        assert!(!got.contains(FAKE) && !got.contains("host-secret"));
    }

    #[test]
    fn the_helper_answers_get_with_the_token_from_its_own_variable_only() {
        let a = token_for("acme/app");
        let args = a.git_args();
        assert_eq!(args[1], "credential.helper=");
        assert!(args[3].starts_with("credential.https://github.com.helper=!f()"));
        assert!(args[3].contains("$IOHR_GIT_TOKEN"));
        assert_eq!(args[5], "core.hooksPath=/dev/null");
    }
}
