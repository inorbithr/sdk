//! Permission asks for `iohr ask` (PRD 0197): when Io wants to run a command, the terminal
//! asks, and the answer is remembered as a rule if the person says "always".
//!
//! Local only. Nothing here talks to InOrbit. The pieces:
//!
//! - [`rule`], [`engine`], [`shell`], [`path`], [`glob`]: Claude Code's rule grammar and the
//!   allow / deny / ask decision (deny beats allow; the secret list beats everything).
//! - [`secrets`]: the one list of files no rule or answer can open.
//! - [`store`]: the rule files (`.inorbit/permissions.toml`, a user file in the config
//!   folder) and the audit log (`.inorbit/permissions.log`).
//! - [`broker`] and [`server`]: the live ask. Claude Code is started with
//!   `--permission-prompt-tool mcp__iohrperm__approve` and an `--mcp-config` that runs this
//!   same binary as a stdio MCP server (`iohr permission-server`, hidden). The server relays
//!   each question over a private unix socket (in a 0700 folder, removed on exit) to the
//!   broker in the `iohr ask` process, which owns the terminal. No answer means no.
//!
//! Claude Code facts this relies on (checked on 2.1.319 and in the docs):
//! `--permission-prompt-tool` and `--permission-prompts`
//! (<https://code.claude.com/docs/en/cli-reference>), print mode denies what nobody approves
//! and reports `permission_denials` (<https://code.claude.com/docs/en/headless>), the tool's
//! input `{tool_name, input, tool_use_id}` and its `allow` / `deny` result
//! (<https://code.claude.com/docs/en/agent-sdk/permissions> and `.../agent-sdk/typescript`),
//! the rule grammar (<https://code.claude.com/docs/en/permissions>).

#[cfg(unix)]
pub(crate) mod ask_card;
#[cfg(unix)]
pub(crate) mod broker;
pub(crate) mod engine;
pub(crate) mod glob;
pub(crate) mod path;
pub(crate) mod rule;
pub(crate) mod secrets;
#[cfg(unix)]
pub(crate) mod server;
pub(crate) mod shell;
pub(crate) mod store;
/// Windows has no unix sockets: the live ask is not offered, and `iohr ask` stays read-only.
/// No [`Session`] is ever made here (`commands/ask.rs` starts none off unix), so the type is
/// empty: the callers that read a session compile, and nothing else of the broker exists.
#[cfg(not(unix))]
pub(crate) mod broker {
    /// A running broker, which this system cannot have.
    pub(crate) enum Session {}

    /// A call the rules refused, which only a running broker records.
    pub(crate) enum Denied {}

    impl Session {
        pub(crate) fn args(&self) -> Vec<String> {
            match *self {}
        }
        pub(crate) fn waiting(&self) -> (std::time::Duration, bool) {
            match *self {}
        }
        pub(crate) fn denied(&self) -> Vec<Denied> {
            match *self {}
        }
        pub(crate) fn bin(&self) -> &str {
            match *self {}
        }
    }

    pub(crate) fn denied_lines(_: &[Denied], _: &str) -> Vec<String> {
        Vec::new()
    }
}

use std::fmt;

/// How `iohr ask` handles a tool call no rule decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Rules first, then ask the person at the terminal (the default at a terminal).
    Prompt,
    /// No shell at all: Io reads files and answers, as before.
    Deny,
    /// Rules only: what no rule allows is refused, and said after the answer (the default
    /// without a terminal).
    Rules,
}

impl Mode {
    pub(crate) const NAMES: &'static str = "prompt, deny or rules";

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Deny => "deny",
            Self::Rules => "rules",
        }
    }

    /// The flag (or `IOHR_ASK_PERMISSIONS`) wins over `[ask] permissions`; with neither,
    /// `prompt` at a terminal and `rules` without one. `prompt` without a terminal to ask on
    /// is `rules`: nobody can answer, and no answer is no.
    pub(crate) fn choose(
        flag: Option<&str>,
        config: Option<&str>,
        terminal: bool,
    ) -> Result<Self, String> {
        let named = match flag.or(config).map(str::trim) {
            None | Some("") => None,
            Some("prompt") => Some(Self::Prompt),
            Some("deny") => Some(Self::Deny),
            Some("rules") => Some(Self::Rules),
            Some(other) => {
                return Err(format!(
                    "--permissions {other:?} is not one of {}",
                    Self::NAMES
                ));
            }
        };
        Ok(match named {
            Some(Self::Prompt) | None if terminal => Self::Prompt,
            Some(Self::Prompt) | None => Self::Rules,
            Some(m) => m,
        })
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_prompt_at_a_terminal_and_rules_without_one() {
        assert_eq!(Mode::choose(None, None, true), Ok(Mode::Prompt));
        assert_eq!(Mode::choose(None, None, false), Ok(Mode::Rules));
        assert_eq!(Mode::choose(Some("prompt"), None, false), Ok(Mode::Rules));
    }

    #[test]
    fn the_flag_beats_the_config_and_a_bad_word_is_refused() {
        assert_eq!(
            Mode::choose(Some("deny"), Some("rules"), true),
            Ok(Mode::Deny)
        );
        assert_eq!(Mode::choose(None, Some("rules"), true), Ok(Mode::Rules));
        assert_eq!(Mode::choose(None, Some(" "), true), Ok(Mode::Prompt));
        let e = Mode::choose(Some("yolo"), None, true).unwrap_err();
        assert!(e.contains("prompt, deny or rules"), "{e}");
    }
}
