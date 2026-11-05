//! `iohr ask` with the local backend (PRD 0088.1): start the developer's own agent in the
//! current folder with the question. The question and the answer are never logged, traced or
//! written by iohr; they go to the agent's argv and the agent's own terminal, nothing else.

use std::path::{Path, PathBuf};

pub(crate) mod stream;

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

fn pick<'a>(flag: Option<&'a str>, configured: Option<&'a str>, default: &'a str) -> &'a str {
    [flag, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or(default)
}

/// The model: the flag, else `[ask] model`, else for Claude `haiku` (the small, fast tier). Codex
/// has no default here: with none named it uses its own configured model.
///
/// # Errors
/// A value that is empty after trimming, has a space, or starts with `-` (it would read as a flag).
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
        (None, Agent::Claude) => "haiku",
        (None, Agent::Codex) => return Ok(None),
    };
    if model.starts_with('-') || model.contains(char::is_whitespace) {
        return Err(format!(
            "`model` is `{model}`; give a model name such as haiku"
        ));
    }
    Ok(Some(model.to_owned()))
}

/// How the thinking is drawn: the flag, else `[ask] thinking`, else `show`.
///
/// # Errors
/// A value that is not `show` or `collapse`.
pub(crate) fn choose_thinking(
    flag: Option<&str>,
    configured: Option<&str>,
) -> Result<stream::Thinking, String> {
    match pick(flag, configured, "show") {
        "show" => Ok(stream::Thinking::Show),
        "collapse" => Ok(stream::Thinking::Collapse),
        other => Err(format!("`thinking` is `{other}`; one of: show, collapse")),
    }
}

/// The arguments that run `agent` once on `question`, streamed as JSON lines, with no session
/// kept. One list for the real start and for `--dry-run`, so the dry run shows exactly what
/// would run. The question is the last argument, after `--`, so it can never read as a flag.
///
/// Claude (flags checked against `claude --help`): `-p` prints and exits, `--output-format
/// stream-json` needs `--verbose`, `--include-partial-messages` streams pieces as they
/// arrive, `--no-session-persistence` keeps nothing on disk, `--permission-mode acceptEdits`
/// lets it edit. Codex (<https://developers.openai.com/codex/cli/reference>): `exec --json`
/// prints JSON lines, `--ephemeral` keeps no session, `--sandbox` is `read-only` unless edits
/// are allowed, `--skip-git-repo-check` lets it answer in any folder.
#[must_use]
pub(crate) fn argv(
    agent: Agent,
    model: Option<&str>,
    allow_edits: bool,
    question: &str,
) -> Vec<String> {
    let mut a: Vec<String> = match agent {
        Agent::Claude => [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--no-session-persistence",
        ]
        .map(str::to_owned)
        .to_vec(),
        Agent::Codex => ["exec", "--json", "--ephemeral", "--skip-git-repo-check"]
            .map(str::to_owned)
            .to_vec(),
    };
    if let Some(m) = model {
        a.extend(["--model".to_owned(), m.to_owned()]);
    }
    match (agent, allow_edits) {
        (Agent::Claude, true) => {
            a.extend(["--permission-mode".to_owned(), "acceptEdits".to_owned()]);
        }
        (Agent::Claude, false) => {}
        (Agent::Codex, edits) => a.extend([
            "--sandbox".to_owned(),
            if edits {
                "workspace-write"
            } else {
                "read-only"
            }
            .to_owned(),
        ]),
    }
    a.extend(["--".to_owned(), question.to_owned()]);
    a
}

/// `program` and `args` as one line a shell reads back the same way.
#[must_use]
pub(crate) fn shell_line(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(|w| {
            if !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
            {
                w.to_owned()
            } else {
                format!("'{}'", w.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

/// The agent to start without being told which (`iohr project init`): the one the config
/// names, if any, else `claude`, then `codex`, whichever is on `path`. `Ok(None)`: none found.
///
/// # Errors
/// A configured value that is not `claude` or `codex`.
pub(crate) fn detect_agent(
    configured: Option<&str>,
    path: &std::ffi::OsStr,
) -> Result<Option<(Agent, PathBuf)>, String> {
    if configured.is_some_and(|c| !c.trim().is_empty()) {
        let agent = choose_agent(None, configured)?;
        return Ok(find_on_path(agent.name(), path).map(|b| (agent, b)));
    }
    Ok([Agent::Claude, Agent::Codex]
        .into_iter()
        .find_map(|a| find_on_path(a.name(), path).map(|b| (a, b))))
}

/// The one line printed on stderr before the agent starts (AI Act Art. 50). The wording
/// follows the `ext-ask` example's label.
#[must_use]
pub(crate) fn label(agent: Agent) -> String {
    label_for("iohr ask", "AI answer", agent)
}

/// The same label for another command: `command` names it, `what` says what the AI makes.
#[must_use]
pub(crate) fn label_for(command: &str, what: &str, agent: Agent) -> String {
    format!(
        "{command}: {what} -- generated by an AI model ({}) under your own subscription; the folder goes to that provider. Check it before you rely on it.",
        agent.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_model_defaults_to_haiku_for_claude_and_to_nothing_for_codex() {
        let m = |a, f, c| choose_model(a, f, c);
        assert_eq!(m(Agent::Claude, None, None), Ok(Some("haiku".into())));
        assert_eq!(
            m(Agent::Claude, None, Some("sonnet")),
            Ok(Some("sonnet".into()))
        );
        assert_eq!(
            m(Agent::Claude, Some("opus"), Some("sonnet")),
            Ok(Some("opus".into()))
        );
        assert_eq!(m(Agent::Codex, None, None), Ok(None));
        assert!(m(Agent::Claude, Some("--x"), None).is_err());
        assert!(m(Agent::Claude, Some("a b"), None).is_err());
    }

    #[test]
    fn the_argv_has_no_session_and_no_edits_unless_asked() {
        let a = argv(Agent::Claude, Some("haiku"), false, "q");
        assert!(a.contains(&"--no-session-persistence".to_owned()));
        assert!(!a.contains(&"--permission-mode".to_owned()));
        assert_eq!(a[a.len() - 2..], ["--", "q"]);
        let e = argv(Agent::Claude, Some("haiku"), true, "q");
        assert!(
            e.windows(2)
                .any(|w| w == ["--permission-mode", "acceptEdits"])
        );
        assert!(
            argv(Agent::Codex, None, false, "q")
                .windows(2)
                .any(|w| w == ["--sandbox", "read-only"])
        );
    }

    #[test]
    fn the_shell_line_quotes_what_a_shell_would_split() {
        assert_eq!(
            shell_line("claude", &["-p".into(), "it's a b".into()]),
            "claude -p 'it'\\''s a b'"
        );
    }

    #[test]
    fn detection_takes_the_configured_agent_else_whichever_is_on_the_path() {
        let d = std::env::temp_dir();
        let path = d.as_os_str();
        assert!(detect_agent(Some("bogus"), path).is_err());
        assert!(
            detect_agent(None, std::ffi::OsStr::new(""))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_label_names_the_agent_and_the_provider() {
        let l = label(Agent::Claude);
        assert!(l.contains("(claude)") && l.contains("goes to that provider"));
        assert!(!l.contains('\n'));
    }
}
