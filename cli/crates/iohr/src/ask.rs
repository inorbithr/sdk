//! `iohr ask` with the local backend (PRD 0088.1): start the developer's own agent in the
//! current folder with the question. The question and the answer are never logged, traced or
//! written by iohr; they go to the agent's argv and the agent's own terminal, nothing else.

use std::path::{Path, PathBuf};

/// Where the answer comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Via {
    /// The developer's own agent, under their own subscription.
    Local,
    /// Io over the coordinator: reserved, not built yet.
    Io,
}

/// Which local agent to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Agent {
    /// Claude Code.
    Claude,
    /// The Codex CLI: reserved, not built yet.
    Codex,
}

impl Via {
    /// The name in the config and the flag.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Io => "io",
        }
    }
}

impl Agent {
    /// The name in the config, the flag and on PATH.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The engine Io runs on, and where, for the label's via line.
    #[must_use]
    pub(crate) fn display(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code (local)",
            Self::Codex => "Codex (local)",
        }
    }

    /// What to do when the binary is missing.
    #[must_use]
    pub(crate) fn install_hint(self) -> &'static str {
        match self {
            Self::Claude => {
                "install Claude Code with `npm install -g @anthropic-ai/claude-code` (https://docs.claude.com/en/docs/claude-code/setup), then run `claude` once to sign in"
            }
            Self::Codex => "install the Codex CLI with `npm install -g @openai/codex`",
        }
    }
}

/// The backend: the flag, else `[ask] via`, else `local`.
///
/// # Errors
/// A value that is not `local` or `io`.
pub(crate) fn choose_via(flag: Option<&str>, configured: Option<&str>) -> Result<Via, String> {
    match pick(flag, configured, "local") {
        "local" => Ok(Via::Local),
        "io" => Ok(Via::Io),
        other => Err(format!("`via` is `{other}`; one of: local, io")),
    }
}

/// The agent: the flag, else `[ask] agent`, else `claude`.
///
/// # Errors
/// A value that is not `claude` or `codex`.
pub(crate) fn choose_agent(flag: Option<&str>, configured: Option<&str>) -> Result<Agent, String> {
    match pick(flag, configured, "claude") {
        "claude" => Ok(Agent::Claude),
        "codex" => Ok(Agent::Codex),
        other => Err(format!("`agent` is `{other}`; one of: claude, codex")),
    }
}

/// The model a quick question uses when none is named: the small, fast tier, the cheapest that
/// answers.
pub(crate) const DEFAULT_MODEL: &str = "haiku";

/// The model: the flag (or `IOHR_ASK_MODEL`), else `[ask] model`, else for Claude `haiku`. Codex
/// has no default here: with none named it uses its own configured model. The one reading of
/// the setting, for the card and for the streamed answer alike.
///
/// # Errors
/// A value that has a space or starts with `-` (it would read as a flag).
pub(crate) fn choose_model(
    agent: Agent,
    flag: Option<&str>,
    configured: Option<&str>,
) -> Result<Option<String>, String> {
    let named = [flag, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty());
    let model = match (named, agent) {
        (Some(m), _) => m,
        (None, Agent::Claude) => DEFAULT_MODEL,
        (None, Agent::Codex) => return Ok(None),
    };
    if model.starts_with('-') || model.contains(char::is_whitespace) {
        return Err(format!(
            "`model` is `{model}`; give a model name such as haiku"
        ));
    }
    Ok(Some(model.to_owned()))
}

fn pick<'a>(flag: Option<&'a str>, configured: Option<&'a str>, default: &'a str) -> &'a str {
    [flag, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or(default)
}

/// The executable `name` on the `PATH` value `path`, if any.
#[must_use]
pub(crate) fn find_on_path(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file() || p.with_extension("exe").is_file() || p.with_extension("cmd").is_file()
}

/// Where the question goes, said on every surface that shows an answer.
pub(crate) const PROVIDER: &str = "under your own subscription; the folder goes to that provider.";
/// The last word of every label.
pub(crate) const CHECK: &str = "Check it before you rely on it.";

/// The one line printed on stderr before the agent starts (AI Act Art. 50). The wording
/// follows the `ext-ask` example's label.
#[must_use]
pub(crate) fn label(agent: Agent) -> String {
    format!(
        "Io · AI answer -- Io running on {}, an AI model, {PROVIDER} {CHECK}",
        agent.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_the_flag_then_the_config_then_haiku() {
        let m = choose_model;
        assert_eq!(
            m(Agent::Claude, None, None).unwrap().as_deref(),
            Some("haiku")
        );
        assert_eq!(
            m(Agent::Claude, None, Some(" opus ")).unwrap().as_deref(),
            Some("opus")
        );
        assert_eq!(
            m(Agent::Claude, Some("sonnet"), Some("opus"))
                .unwrap()
                .as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            m(Agent::Claude, Some(""), Some("opus")).unwrap().as_deref(),
            Some("opus"),
            "an empty flag is not a choice"
        );
        assert_eq!(m(Agent::Codex, None, None).unwrap(), None);
        assert_eq!(
            m(Agent::Codex, None, Some("gpt-5")).unwrap().as_deref(),
            Some("gpt-5")
        );
        assert!(m(Agent::Claude, Some("--help"), None).is_err());
        assert!(m(Agent::Claude, None, Some("two words")).is_err());
    }

    #[test]
    fn flag_then_config_then_default() {
        assert_eq!(
            choose_agent(Some("codex"), Some("claude")),
            Ok(Agent::Codex)
        );
        assert_eq!(choose_agent(None, Some("codex")), Ok(Agent::Codex));
        assert_eq!(choose_agent(Some(" "), None), Ok(Agent::Claude));
        assert_eq!(choose_via(None, None), Ok(Via::Local));
        assert_eq!(choose_via(Some("io"), Some("local")), Ok(Via::Io));
        assert!(choose_via(Some("cloud"), None).is_err());
        assert!(choose_agent(None, Some("x")).is_err());
    }

    #[test]
    fn the_label_names_the_agent_and_the_provider() {
        let l = label(Agent::Claude);
        assert!(
            l.starts_with("Io · AI answer")
                && l.contains("Claude Code (local)")
                && l.contains("goes to that provider")
        );
        assert!(!l.contains('\n'));
    }
}
