//! The environment a provider child process gets: an explicit allowlist of the runner's
//! own (RFC 0120, "Provider checks, every turn"; study 0021 conditions 1, 2 and 10;
//! study 0023 constraint 4).
//!
//! The runner runs a model only on the person's own subscription. Only the variables
//! `claude` needs to run and to find its own sign-in are passed on (`PATH`, `HOME`, the
//! locale, the temp directory, `CLAUDE_CONFIG_DIR` and the XDG directories). Everything
//! else is dropped, so a variable nobody thought of (`GH_TOKEN`, `AZURE_API_KEY`, a
//! `FOO_SECRET`) never reaches the model's tools. On top of the allowlist, the old
//! denylist and a name pattern (`*TOKEN*`, `*KEY*`, `*SECRET*`, `*PASSWORD*`) stay as a
//! backstop, so an allowed name can never carry a credential either. There is no
//! pass-through: the RFC's one server exception (`login.pass_through`) is a token, and
//! the owner's rule is no token at all, so it is not built.

use std::ffi::OsString;

/// The only variables passed on, by exact name (compared in upper case: Windows is
/// case-blind). `claude` finds its subscription sign-in through `HOME` (the macOS
/// keychain, `~/.claude`), `CLAUDE_CONFIG_DIR` and the XDG directories.
pub(crate) const ALLOW_EXACT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "TZ",
    "LANG",
    "LANGUAGE",
    "CLAUDE_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    // Where `cargo` builds: not a secret, and the runner sets it per repository.
    "CARGO_TARGET_DIR",
    // Windows.
    "USERPROFILE",
    "USERNAME",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

/// Allowed by prefix: the locale.
pub(crate) const ALLOW_PREFIX: &[&str] = &["LC_"];

/// A name containing one of these is a credential, allowed or not.
const SECRET_PARTS: &[&str] = &["TOKEN", "KEY", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL"];

/// Variables removed by exact name (compared in upper case: Windows is case-blind).
pub(crate) const DENY_EXACT: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_API_KEY",
    "GEMINI_API_KEY",
    "CLOUD_ML_REGION",
];

/// Variables removed by prefix: whole families, so a new member is covered too.
pub(crate) const DENY_PREFIX: &[&str] = &[
    "ANTHROPIC_",
    "CLAUDE_CODE_USE_",
    "CLAUDE_CODE_SKIP_",
    "CLAUDE_CODE_OAUTH",
    "CLAUDE_CODE_API",
    "AWS_",
    "AZURE_OPENAI_",
    "OPENAI_",
    "CODEX_",
    "GOOGLE_CLOUD_",
    "GOOGLE_GENAI_",
    "GCLOUD_",
    "VERTEX_",
    // The CLI's own credentials must not reach a model's tools either.
    "IOHR_TOKEN",
    "IOHR_EXT_REGISTRY_AUTH",
];

/// Whether `name` must not reach the child: not on the allowlist, or a credential or
/// routing variable.
pub(crate) fn is_denied(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    let allowed =
        ALLOW_EXACT.contains(&up.as_str()) || ALLOW_PREFIX.iter().any(|p| up.starts_with(p));
    !allowed
        || DENY_EXACT.contains(&up.as_str())
        || DENY_PREFIX.iter().any(|p| up.starts_with(p))
        || SECRET_PARTS.iter().any(|p| up.contains(p))
}

/// The allowlisted part of `env`: `PATH`, `HOME`, the locale and the few others
/// `claude` needs. A name that is not UTF-8 is dropped.
pub(crate) fn scrub<I>(env: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    env.into_iter()
        .filter(|(k, _)| k.to_str().is_some_and(|n| !is_denied(n)))
        .collect()
}

/// True when nothing but allowed variables is in `env`; what the runner tells
/// `AcceptLease`.
pub(crate) fn is_clean(env: &[(OsString, OsString)]) -> bool {
    env.iter()
        .all(|(k, _)| k.to_str().is_some_and(|n| !is_denied(n)))
}

/// Where the model's and a check's `gh` looks for a sign-in: a directory that does not
/// exist and only an administrator could make, so `gh` there is signed out.
pub(crate) const NO_GH_CONFIG: &str = "/nonexistent/iohr-gh";

/// Set on top of the scrubbed environment for the provider child and a check (the
/// RFC 0147 containment ask): neither inherits the host's GitHub sign-in. `gh` gets a
/// configuration directory that does not exist (an empty `GH_CONFIG_DIR` would mean the
/// default one, the host's login), git drops every credential helper of every
/// configuration file (`credential.helper` set to empty, the last and so the winning
/// entry) and never prompts. `GH_TOKEN` and `GITHUB_TOKEN` are already gone: the
/// allowlist drops them. This is containment, not a sandbox: a process that can run any
/// program as this user can still reach what this user can.
pub(crate) fn contained() -> [(&'static str, &'static str); 7] {
    [
        ("GH_CONFIG_DIR", NO_GH_CONFIG),
        ("GH_PROMPT_DISABLED", "1"),
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "credential.helper"),
        ("GIT_CONFIG_VALUE_0", ""),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    #[test]
    fn the_containment_signs_gh_and_git_out_and_carries_no_credential() {
        let c = contained();
        let get = |k: &str| c.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        assert_eq!(get("GH_CONFIG_DIR"), Some(NO_GH_CONFIG));
        assert_ne!(NO_GH_CONFIG, "");
        assert!(!std::path::Path::new(NO_GH_CONFIG).exists());
        assert_eq!(get("GIT_CONFIG_KEY_0"), Some("credential.helper"));
        assert_eq!(get("GIT_CONFIG_VALUE_0"), Some(""));
        assert_eq!(get("GH_TOKEN"), None);
        assert_eq!(get("GITHUB_TOKEN"), None);
    }

    /// With the containment, git asks no helper for GitHub, even when the host's own
    /// configuration names one.
    #[test]
    fn git_under_the_containment_asks_no_credential_helper() {
        let d = tempfile::tempdir().unwrap();
        let global = d.path().join("gitconfig");
        std::fs::write(
            &global,
            "[credential]\n\thelper = \"!f() { echo password=host-secret; }; f\"\n\
             [credential \"https://github.com\"]\n\thelper = \"!f() { echo password=host-secret; }; f\"\n",
        )
        .unwrap();
        let mut c = std::process::Command::new("git");
        c.args(["credential", "fill"])
            .env_clear()
            .envs(scrub(std::env::vars_os()))
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(contained())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = c.spawn().unwrap();
        {
            use std::io::Write as _;
            let mut i = child.stdin.take().unwrap();
            i.write_all(b"protocol=https\nhost=github.com\nusername=x\n\n")
                .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(!text.contains("host-secret"), "{text}");
        assert!(!out.status.success(), "{text}");
    }

    /// With the containment, `gh` is signed out (skipped where `gh` is not installed).
    #[test]
    fn gh_under_the_containment_is_signed_out() {
        let Ok(out) = std::process::Command::new("gh")
            .args(["auth", "status"])
            .env_clear()
            .envs(scrub(std::env::vars_os()))
            .envs(contained())
            .output()
        else {
            return;
        };
        assert!(!out.status.success());
        let both = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!both.contains("Logged in"), "{both}");
    }

    fn pairs(names: &[&str]) -> Vec<(OsString, OsString)> {
        names.iter().map(|n| ((*n).into(), "x".into())).collect()
    }

    fn names(env: &[(OsString, OsString)]) -> Vec<String> {
        env.iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn every_credential_and_routing_variable_is_removed() {
        let denied = [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_VERTEX_PROJECT_ID",
            "ANTHROPIC_FOUNDRY_API_KEY",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_PROFILE",
            "AWS_REGION",
            "AWS_BEARER_TOKEN_BEDROCK",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
            "CLOUD_ML_REGION",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "CODEX_API_KEY",
            "AZURE_OPENAI_API_KEY",
            "AZURE_OPENAI_ENDPOINT",
            "IOHR_TOKEN",
            "IOHR_TOKEN_CI",
            // Windows and shells are case-blind.
            "anthropic_api_key",
            "Anthropic_Base_Url",
            "aws_secret_access_key",
        ];
        let out = scrub(pairs(&denied));
        assert!(out.is_empty(), "left behind: {:?}", names(&out));
        assert!(is_clean(&out));
    }

    #[test]
    fn what_claude_needs_is_kept() {
        let kept = [
            "PATH",
            "HOME",
            "USER",
            "LANG",
            "TERM",
            "TMPDIR",
            "XDG_CONFIG_HOME",
        ];
        let mixed: Vec<&str> = kept
            .iter()
            .copied()
            .chain(["ANTHROPIC_API_KEY", "AWS_REGION"])
            .collect();
        let out = scrub(pairs(&mixed));
        assert_eq!(names(&out), kept);
        assert!(is_clean(&out));
    }

    #[test]
    fn only_the_allowlist_passes_so_unlisted_credentials_are_dropped() {
        let parent = pairs(&[
            "PATH",
            "HOME",
            "LC_ALL",
            "CLAUDE_CONFIG_DIR",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "AZURE_API_KEY",
            "FOO_SECRET",
            "DB_PASSWORD",
            "SSH_AUTH_SOCK",
            "NPM_CONFIG__AUTH",
            "gh_token",
        ]);
        let out = scrub(parent.clone());
        assert_eq!(names(&out), ["PATH", "HOME", "LC_ALL", "CLAUDE_CONFIG_DIR"]);
        assert!(is_clean(&out));
        assert!(!is_clean(&parent));
        // An allowed name that looks like a credential is still dropped.
        assert!(is_denied("LC_API_KEY") && is_denied("HOME_TOKEN"));
    }

    #[test]
    fn a_dirty_environment_is_not_clean() {
        assert!(!is_clean(&pairs(&["PATH", "ANTHROPIC_BASE_URL"])));
    }

    #[test]
    fn the_listed_names_are_all_denied() {
        for n in DENY_EXACT {
            assert!(is_denied(n), "{n}");
        }
    }
}
