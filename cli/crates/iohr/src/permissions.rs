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
pub(crate) mod trust;
/// Windows has no unix sockets: the live ask is not offered, and `iohr ask` stays read-only.
/// No [`Session`] is ever made here (`commands/ask.rs` starts none off unix), so the type is
/// empty: the callers that read a session compile, and nothing else of the broker exists.
#[cfg(not(unix))]
pub(crate) mod broker {
    /// A running broker, which this system cannot have.
    pub(crate) enum Session {}

    /// A call the rules refused, which only a running broker records.
    pub(crate) enum Denied {}

    use super::Settled;

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
        pub(crate) fn settled(&self) -> Vec<Settled> {
            match *self {}
        }
    }

    pub(crate) fn denied_lines(_: &[Denied], _: &str) -> Vec<String> {
        Vec::new()
    }
}

use std::fmt;

use rule::Rule;

/// What the person chose for one ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grant {
    /// Allowed this time; nothing saved.
    Once,
    /// Allowed, and a rule saved in the project file.
    Always,
    /// Not allowed.
    No,
}

/// The result of one ask the person answered, kept so the answer card it led to can show it
/// (the ask is answered while the model still runs; the card is drawn after). Plain words
/// first; the rule and the file are detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settled {
    pub(crate) grant: Grant,
    /// What was allowed or refused, short (`ls`, `git status`, `read /tmp/x`).
    pub(crate) subject: String,
    /// The rules saved, as written in the rule file; empty unless `grant` is `Always`.
    pub(crate) rules: Vec<String>,
    /// The project rule file, relative to the folder; empty unless `grant` is `Always`.
    pub(crate) file: String,
    /// Why "always" could not be saved, when it could not (the grant is then `Once`).
    pub(crate) not_saved: Option<String>,
}

impl Settled {
    /// The one status line, in plain words.
    pub(crate) fn sentence(&self) -> String {
        let s = &self.subject;
        match (self.grant, &self.not_saved) {
            (Grant::Always, _) => format!("{s} allowed in this project from now on"),
            (Grant::Once, Some(why)) => {
                format!("{s} allowed this time (could not save the rule: {why})")
            }
            (Grant::Once, None) => format!("{s} allowed this time"),
            (Grant::No, _) => format!("{s} not allowed"),
        }
    }

    /// The rule and the file, for the dim line under the sentence; only for a saved rule.
    pub(crate) fn detail(&self) -> Option<String> {
        (self.grant == Grant::Always && !self.rules.is_empty())
            .then(|| format!("{} \u{b7} {}", self.rules.join(", "), self.file))
    }

    /// The command that takes a saved rule back, for output with no action row.
    pub(crate) fn undo_command(&self, bin: &str) -> Option<String> {
        (self.grant == Grant::Always && !self.rules.is_empty()).then(|| {
            self.rules
                .iter()
                .map(|r| format!("{bin} permissions remove '{r}'"))
                .collect::<Vec<_>>()
                .join(" && ")
        })
    }

    /// The structured form `--json` carries, never the sentence.
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "grant": match self.grant {
                Grant::Once => "once",
                Grant::Always => "always",
                Grant::No => "no",
            },
            "subject": self.subject,
            "rules": self.rules,
            "file": (!self.file.is_empty()).then_some(&self.file),
            "saved": self.grant == Grant::Always,
            "not_saved": self.not_saved,
        })
    }
}

/// What a saved rule lets through, in a few words: a command prefix rule (`Bash(ls:*)`) is
/// the command (`ls`), several are joined, any other rule is written as it is.
pub(crate) fn rules_subject(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|r| match (r.tool(), r.spec()) {
            ("Bash", Some(spec)) => spec.strip_suffix(":*").unwrap_or(spec).to_owned(),
            _ => r.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

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

    fn settled(grant: Grant, rules: &[&str], not_saved: Option<&str>) -> Settled {
        Settled {
            grant,
            subject: "ls".into(),
            rules: rules.iter().map(|r| (*r).to_owned()).collect(),
            file: ".inorbit/permissions.toml".into(),
            not_saved: not_saved.map(str::to_owned),
        }
    }

    #[test]
    fn a_result_is_a_plain_sentence_with_the_rule_and_the_file_apart() {
        let a = settled(Grant::Always, &["Bash(ls:*)"], None);
        assert_eq!(a.sentence(), "ls allowed in this project from now on");
        assert_eq!(
            a.detail().as_deref(),
            Some("Bash(ls:*) \u{b7} .inorbit/permissions.toml")
        );
        assert_eq!(
            a.undo_command("iohr").as_deref(),
            Some("iohr permissions remove 'Bash(ls:*)'")
        );
        assert_eq!(a.json()["grant"], "always");
        assert_eq!(a.json()["rules"][0], "Bash(ls:*)");
        let once = settled(Grant::Once, &[], None);
        assert_eq!(once.sentence(), "ls allowed this time");
        assert!(once.detail().is_none() && once.undo_command("iohr").is_none());
        let unsaved = settled(Grant::Once, &[], Some("read-only folder"));
        assert!(
            unsaved
                .sentence()
                .contains("could not save the rule: read-only folder")
        );
        assert_eq!(settled(Grant::No, &[], None).sentence(), "ls not allowed");
        let mut no = settled(Grant::No, &[], None);
        no.file.clear();
        assert!(no.json()["file"].is_null());
        assert_eq!(no.json()["saved"], false);
    }

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
