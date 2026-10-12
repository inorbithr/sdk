//! The environment a provider child process gets: the runner's own, with every credential
//! and routing variable removed (RFC 0120, "Provider checks, every turn"; study 0021
//! conditions 1, 2 and 10; study 0023 constraint 4).
//!
//! The runner runs a model only on the person's own subscription. A variable that would
//! send `claude` to an API key, a bearer token, a cloud provider (Bedrock, Vertex,
//! Foundry), another endpoint or paid credits is never passed on. There is no
//! pass-through: the RFC's one server exception (`login.pass_through`) is a token, and
//! the owner's rule is no token at all, so it is not built.

use std::ffi::OsString;

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

/// Whether `name` is a credential or routing variable.
pub(crate) fn is_denied(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    DENY_EXACT.contains(&up.as_str()) || DENY_PREFIX.iter().any(|p| up.starts_with(p))
}

/// `env` without any denied variable. Everything else (`PATH`, `HOME`, the locale) is
/// kept: `claude` needs `HOME` to find its own sign-in.
pub(crate) fn scrub<I>(env: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    env.into_iter()
        .filter(|(k, _)| !k.to_str().is_some_and(is_denied))
        .collect()
}

/// True when nothing denied is left in `env`; what the runner tells `AcceptLease`.
pub(crate) fn is_clean(env: &[(OsString, OsString)]) -> bool {
    !env.iter().any(|(k, _)| k.to_str().is_some_and(is_denied))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

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
