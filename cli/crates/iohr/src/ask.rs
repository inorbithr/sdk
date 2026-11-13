//! `iohr ask` with the local backend (PRD 0088.1): start the developer's own agent in the
//! current folder with the question. The question and the answer are never logged, traced or
//! written by iohr; they go to the agent's argv and the agent's own terminal, nothing else.

use std::path::PathBuf;

use crate::runner::claude;

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

fn pick<'a>(flag: Option<&'a str>, configured: Option<&'a str>, default: &'a str) -> &'a str {
    [flag, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or(default)
}

/// Where a run's model can come from, in the order it is looked for (RFC 0211).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ModelChoice<'a> {
    /// The flag (or its environment variable).
    pub(crate) flag: Option<&'a str>,
    /// The setting in the config file.
    pub(crate) configured: Option<&'a str>,
    /// The model of the Role the run works in (`--role`); Claude only, its names are Claude's.
    pub(crate) role: Option<&'a str>,
    /// What a Claude run uses when nothing above names one; `None` is [`DEFAULT_MODEL`].
    pub(crate) default: Option<&'a str>,
}

/// A model name as a command line can carry it: no space, and not starting with `-` (it would
/// read as a flag).
///
/// # Errors
/// A name that has a space or starts with `-`.
pub(crate) fn model_name(model: &str) -> Result<&str, String> {
    let model = model.trim();
    if model.is_empty() || model.starts_with('-') || model.contains(char::is_whitespace) {
        return Err(format!(
            "`model` is `{model}`; give a model name such as haiku"
        ));
    }
    Ok(model)
}

/// The model: the flag (or `IOHR_ASK_MODEL`), else `[ask] model`, else the Role's model, else for
/// Claude `default` (`haiku`, the small, fast tier, unless the caller has its own). The one
/// reading of the setting, for the card and the streamed answer alike. Codex has no default
/// here and takes no Role's model: with none named it uses its own configured model.
///
/// # Errors
/// A value that is empty after trimming, has a space, or starts with `-` (it would read as a flag).
pub(crate) fn choose_model(agent: Agent, c: ModelChoice<'_>) -> Result<Option<String>, String> {
    let named = |list: &[Option<&str>]| {
        list.iter()
            .flatten()
            .map(|v| v.trim())
            .find(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let model = match agent {
        Agent::Claude => named(&[c.flag, c.configured, c.role, c.default])
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        Agent::Codex => match named(&[c.flag, c.configured]) {
            Some(m) => m,
            None => return Ok(None),
        },
    };
    model_name(&model).map(|m| Some(m.to_owned()))
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

/// Files an agent run that may edit is never allowed to read, whatever else it may do: the
/// local environment files, keys and the credential folders of the common tools. Claude Code
/// applies a `Read` deny to its own Read tool and to the shell commands it recognises as
/// reading (`cat` of such a file is refused, checked on 2.1.318). The one list lives in
/// [`crate::permissions::secrets`], shared with the `?` card and the permission broker.
pub(crate) const SECRET_READS: &[&str] = crate::permissions::secrets::READ_RULES;

/// The arguments that run `agent` once on `question`, streamed as JSON lines, with no session
/// kept. One list for the real start and for `--dry-run`, so the dry run shows exactly what
/// would run. The question is the last argument, after `--`, so it can never read as a flag.
///
/// Claude (flags checked against `claude --help`): `-p` prints and exits, `--output-format
/// stream-json` needs `--verbose`, `--include-partial-messages` streams pieces as they
/// arrive, `--no-session-persistence` keeps nothing on disk, `--permission-mode acceptEdits`
/// lets it edit (`--permission-mode plan` in plan mode, which only reads), `--max-turns` bounds the run. An editing run takes `allow`, the commands it may
/// run (each a prefix, `cargo test`, scoped as `Bash(cargo test:*)`), and always `Read`, `Glob`,
/// `Grep`, `Write`, `Edit`, `WebSearch` and `WebFetch` (a documents project needs no shell to
/// fetch a site: `WebFetch` does it); any other command is refused because nobody can be asked in print mode, and
/// `--setting-sources ""` then keeps the person's own settings from allowing more. With nothing in
/// `allow` the shell stays denied outright (`--disallowedTools Bash`: a deny beats an allow,
/// so the two cannot be mixed). Whatever the run may do, `SECRET_READS` stay unreadable. Codex (<https://developers.openai.com/codex/cli/reference>): `exec --json`
/// prints JSON lines, `--ephemeral` keeps no session, `--sandbox` is `read-only` unless edits
/// are allowed, `--skip-git-repo-check` lets it answer in any folder.
///
/// Claude's vector comes from the one builder ([`claude::build`]), which refuses a flag that
/// would change what Claude Code is (invariant 8); that refusal is the error.
pub(crate) fn argv(
    agent: Agent,
    model: Option<&str>,
    (allow_edits, plan): (bool, bool),
    allow: &[String],
    max_turns: Option<u32>,
    role: Option<&str>,
    question: &str,
) -> Result<Vec<String>, String> {
    if agent == Agent::Claude {
        return claude_argv(model, (allow_edits, plan), allow, max_turns, role, question);
    }
    let mut a: Vec<String> = ["exec", "--json", "--ephemeral", "--skip-git-repo-check"]
        .map(str::to_owned)
        .to_vec();
    if let Some(m) = model {
        a.extend(["--model".to_owned(), m.to_owned()]);
    }
    // Codex has no plan mode (checked against its CLI reference): plan is its read-only sandbox.
    a.extend([
        "--sandbox".to_owned(),
        if allow_edits && !plan {
            "workspace-write"
        } else {
            "read-only"
        }
        .to_owned(),
    ]);
    // Codex has no turn limit and no system prompt flag: the caller's time limit is its only
    // bound, and a Role's text (RFC 0211) leads the prompt.
    let question = match role {
        Some(r) => format!("{r}\n\n{question}"),
        None => question.to_owned(),
    };
    a.extend(["--".to_owned(), question]);
    Ok(a)
}

/// Claude's half of [`argv`]: plan mode (which beats `allow_edits`) is Claude's own, so it
/// reads and proposes and changes nothing, with the shell and the secret files denied
/// outright; an editing run accepts edits and takes `allow` and the file and web tools, or
/// denies the shell when `allow` is empty. A Role's text (RFC 0211) is more system prompt
/// (`--append-system-prompt`, <https://code.claude.com/docs/en/cli-reference>), so the
/// question stays the person's own words.
fn claude_argv(
    model: Option<&str>,
    (allow_edits, plan): (bool, bool),
    allow: &[String],
    max_turns: Option<u32>,
    role: Option<&str>,
    question: &str,
) -> Result<Vec<String>, String> {
    let secrets = || SECRET_READS.iter().map(|r| (*r).to_owned());
    let mut call = claude::Call {
        output: claude::Output::Stream,
        model,
        max_turns,
        keep: claude::Keep::Nothing,
        prompt: role.map_or(claude::Prompt::Own, claude::Prompt::Append),
        question: Some(question),
        ..claude::Call::default()
    };
    if plan {
        call.permission_mode = Some("plan");
        call.disallowed = secrets().chain(["Bash".to_owned()]).collect();
    } else if allow_edits {
        call.permission_mode = Some("acceptEdits");
        call.disallowed = secrets().collect();
        if allow.is_empty() {
            call.disallowed.push("Bash".to_owned());
        } else {
            call.setting_sources = Some("");
            call.allowed = allow
                .iter()
                .map(|c| format!("Bash({c}:*)"))
                .chain(
                    [
                        "Read",
                        "Glob",
                        "Grep",
                        "Write",
                        "Edit",
                        "WebSearch",
                        "WebFetch",
                    ]
                    .map(str::to_owned),
                )
                .collect();
        }
    }
    claude::build(&call, &claude::Auth::OwnSignIn)
        .map(|b| b.args)
        .map_err(|e| e.to_string())
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
        return Ok(crate::paths::find_program(agent.name(), path).map(|b| (agent, b)));
    }
    Ok([Agent::Claude, Agent::Codex]
        .into_iter()
        .find_map(|a| crate::paths::find_program(a.name(), path).map(|b| (a, b))))
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
        "Io -- Io running on {}, an AI model, {PROVIDER} {CHECK}",
        agent.display()
    )
}

/// The same label for another command: `command` names it, `what` says what the AI makes.
#[must_use]
pub(crate) fn label_for(command: &str, what: &str, agent: Agent) -> String {
    format!(
        "{command}: {what}, generated by an AI model ({}) {PROVIDER} {CHECK}",
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
        let m = |a, f, c| {
            choose_model(
                a,
                ModelChoice {
                    flag: f,
                    configured: c,
                    ..ModelChoice::default()
                },
            )
        };
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
    fn the_model_order_is_flag_then_config_then_the_role_then_the_default() {
        let all = ModelChoice {
            flag: Some("flag"),
            configured: Some("config"),
            role: Some("role"),
            default: Some("default"),
        };
        let pick = |c: ModelChoice<'_>| choose_model(Agent::Claude, c).unwrap().unwrap();
        assert_eq!(pick(all), "flag");
        assert_eq!(pick(ModelChoice { flag: None, ..all }), "config");
        assert_eq!(
            pick(ModelChoice {
                flag: Some(" "),
                configured: None,
                ..all
            }),
            "role"
        );
        assert_eq!(
            pick(ModelChoice {
                flag: None,
                configured: None,
                role: None,
                ..all
            }),
            "default"
        );
        assert_eq!(
            pick(ModelChoice {
                default: None,
                ..ModelChoice::default()
            }),
            "haiku"
        );
    }

    #[test]
    fn a_roles_model_is_never_used_for_codex_and_a_bad_one_is_refused_for_claude() {
        let c = ModelChoice {
            role: Some("sonnet"),
            default: Some("sonnet"),
            ..ModelChoice::default()
        };
        assert_eq!(choose_model(Agent::Codex, c), Ok(None));
        let named = ModelChoice {
            flag: Some("gpt-x"),
            ..c
        };
        assert_eq!(choose_model(Agent::Codex, named), Ok(Some("gpt-x".into())));
        let bad = ModelChoice {
            role: Some("--evil"),
            ..ModelChoice::default()
        };
        assert!(choose_model(Agent::Claude, bad).is_err());
    }

    /// The rules a vector denies, one `--disallowedTools=<rule>` flag each.
    fn denied(a: &[String]) -> Vec<String> {
        assert!(
            !a.contains(&"--disallowedTools".to_owned()),
            "list form: {a:?}"
        );
        a.iter()
            .filter_map(|x| x.strip_prefix("--disallowedTools="))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_argv_has_no_session_and_no_edits_unless_asked() {
        let a = argv(
            Agent::Claude,
            Some("haiku"),
            (false, false),
            &[],
            None,
            None,
            "q",
        )
        .unwrap();
        assert!(a.contains(&"--no-session-persistence".to_owned()));
        assert!(!a.contains(&"--permission-mode".to_owned()));
        assert_eq!(a[a.len() - 2..], ["--", "q"]);
        let e = argv(
            Agent::Claude,
            Some("haiku"),
            (true, false),
            &[],
            Some(7),
            None,
            "q",
        )
        .unwrap();
        assert!(
            e.windows(2)
                .any(|w| w == ["--permission-mode", "acceptEdits"])
        );
        // With nothing allowed the shell is denied outright, the secret files are unreadable,
        // and the turn bound reaches Claude only.
        let refused = &denied(&e);
        assert!(refused.contains(&"Bash".to_owned()));
        assert!(
            SECRET_READS
                .iter()
                .all(|r| refused.contains(&(*r).to_owned()))
        );
        assert!(!e.contains(&"--allowedTools".to_owned()));
        assert!(!e.contains(&"--setting-sources".to_owned()));
        assert!(e.windows(2).any(|w| w == ["--max-turns", "7"]));
        assert!(!a.contains(&"--max-turns".to_owned()) && denied(&a).is_empty());
        assert!(
            !argv(Agent::Codex, None, (false, false), &[], Some(7), None, "q")
                .unwrap()
                .contains(&"--max-turns".to_owned())
        );
        assert!(
            argv(Agent::Codex, None, (false, false), &[], Some(7), None, "q")
                .unwrap()
                .windows(2)
                .any(|w| w == ["--sandbox", "read-only"])
        );
    }

    #[test]
    fn plan_reads_and_changes_nothing_and_beats_allow_edits() {
        for edits in [false, true] {
            let c = argv(
                Agent::Claude,
                Some("haiku"),
                (edits, true),
                &[],
                Some(7),
                None,
                "q",
            )
            .unwrap();
            assert!(
                c.windows(2).any(|w| w == ["--permission-mode", "plan"]),
                "{c:?}"
            );
            assert!(!c.contains(&"acceptEdits".to_owned()), "{c:?}");
            let denied = &denied(&c);
            assert!(denied.contains(&"Bash".to_owned()));
            assert!(
                SECRET_READS
                    .iter()
                    .all(|r| denied.contains(&(*r).to_owned()))
            );
            assert!(!c.contains(&"--allowedTools".to_owned()));
            assert_eq!(c[c.len() - 2..], ["--", "q"]);
            // Codex has no plan mode: plan is its read-only sandbox, even with edits asked.
            let x = argv(Agent::Codex, None, (edits, true), &[], None, None, "q").unwrap();
            assert!(
                x.windows(2).any(|w| w == ["--sandbox", "read-only"]),
                "{x:?}"
            );
        }
    }

    #[test]
    fn scoped_commands_and_the_web_are_allowed_and_the_rest_of_the_shell_is_not() {
        let allow = ["cargo build".to_owned(), "cargo test".to_owned()];
        let e = argv(
            Agent::Claude,
            Some("haiku"),
            (true, false),
            &allow,
            Some(7),
            None,
            "q",
        )
        .unwrap();
        let at = e.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(
            e[at + 1..at + 10],
            [
                "Bash(cargo build:*)",
                "Bash(cargo test:*)",
                "Read",
                "Glob",
                "Grep",
                "Write",
                "Edit",
                "WebSearch",
                "WebFetch"
            ]
        );
        // No network shell and no blanket shell is ever allowed.
        assert!(!e.iter().any(|x| x.contains("curl") || x == "Bash"));
        // A deny beats an allow, so a scoped allow means no blanket `Bash` deny, and the
        // secret files stay denied; the person's own settings cannot allow more.
        let denied = &denied(&e);
        assert!(!denied.contains(&"Bash".to_owned()));
        // The deny list is exactly the secret reads: never wider tools without it, never
        // narrower.
        assert_eq!(denied[..], *SECRET_READS);
        assert!(
            SECRET_READS
                .iter()
                .all(|r| denied.contains(&(*r).to_owned()))
        );
        assert!(e.windows(2).any(|w| w == ["--setting-sources", ""]));
        assert_eq!(e[e.len() - 2..], ["--", "q"]);
        // Reading runs allow nothing.
        let r = argv(
            Agent::Claude,
            Some("haiku"),
            (false, false),
            &allow,
            None,
            None,
            "q",
        )
        .unwrap();
        assert!(!r.contains(&"--allowedTools".to_owned()));
    }

    #[test]
    fn a_role_is_appended_to_claudes_system_prompt_and_leads_codexs_prompt() {
        let a = argv(
            Agent::Claude,
            Some("haiku"),
            (false, false),
            &[],
            None,
            Some("Role text"),
            "q",
        )
        .unwrap();
        let at = a
            .iter()
            .position(|x| x == "--append-system-prompt")
            .unwrap();
        assert_eq!(a[at + 1], "Role text");
        assert_eq!(a[a.len() - 2..], ["--", "q"]);
        let c = argv(
            Agent::Codex,
            None,
            (false, false),
            &[],
            None,
            Some("Role text"),
            "q",
        )
        .unwrap();
        assert!(!c.contains(&"--append-system-prompt".to_owned()));
        assert_eq!(c[c.len() - 1], "Role text\n\nq");
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
        assert!(
            l.starts_with("Io -- Io running on")
                && l.contains("Claude Code (local)")
                && l.contains("goes to that provider")
        );
        assert!(!l.contains('\n'));
    }
}
