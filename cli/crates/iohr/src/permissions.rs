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
        pub(crate) fn mode(&self) -> super::Mode {
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

/// How `iohr ask` handles a tool call no rule decides (PRD 0197). The first three are the modes
/// a person switches between (`--mode`, shift+tab in the `?` hook, [`CYCLE`]); `deny` and `rules`
/// are older `--permissions` words that stay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Rules first, then ask the person at the terminal before each command (the default at
    /// a terminal). Called `prompt` before modes existed; that word still works.
    Manual,
    /// Read only: Io reads files and answers with a plan, changes nothing and runs nothing
    /// (Claude `--permission-mode plan`; Codex `--sandbox read-only`, which has no plan mode).
    Plan,
    /// Commands no rule decides run without asking. A deny rule and the secret list still
    /// win, and a command iohr cannot read to check them is refused.
    Auto,
    /// No shell at all: Io reads files and answers, as before.
    Deny,
    /// Rules only: what no rule allows is refused, and said after the answer (the default
    /// without a terminal).
    Rules,
}

/// The modes of the switch, in the order shift+tab goes through them (and back to the first
/// after the last). The zsh hook gets this ring from [`Mode::ring`], so the order lives here only.
pub(crate) const CYCLE: [Mode; 3] = [Mode::Manual, Mode::Plan, Mode::Auto];

/// The words of one layer (the flags and environment, or the config file) that name a mode:
/// `mode` is `--mode` / `[ask] mode`, `permissions` is `--permissions` / `[ask] permissions`.
/// Inside a layer `mode` wins; a flag layer beats the config layer.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Words<'a> {
    pub(crate) mode: Option<&'a str>,
    pub(crate) permissions: Option<&'a str>,
}

fn said(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

impl Mode {
    pub(crate) const NAMES: &'static str = "manual, plan or auto";
    /// What `--permissions` takes: the three modes and the two older words.
    pub(crate) const PERMISSION_NAMES: &'static str =
        "manual (or prompt), plan, auto, deny or rules";

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::Deny => "deny",
            Self::Rules => "rules",
        }
    }

    /// The mode of one of the three names (`--mode`, `[ask] mode`, the hook's variable).
    pub(crate) fn parse(word: &str) -> Option<Self> {
        CYCLE.into_iter().find(|m| m.name() == word.trim())
    }

    /// The mode of a `--permissions` word: the three names, `prompt` for manual, `deny`, `rules`.
    fn parse_permissions(word: &str) -> Option<Self> {
        match word.trim() {
            "prompt" => Some(Self::Manual),
            "deny" => Some(Self::Deny),
            "rules" => Some(Self::Rules),
            w => Self::parse(w),
        }
    }

    /// The name to show for this mode: only the three of the switch have one.
    pub(crate) fn shown(self) -> Option<&'static str> {
        CYCLE.contains(&self).then(|| self.name())
    }

    /// The ring as the words the zsh hook cycles through, in order.
    pub(crate) fn ring() -> String {
        CYCLE.map(Self::name).join(" ")
    }

    /// Whether Io may run a command at all in this mode.
    pub(crate) fn runs_commands(self) -> bool {
        matches!(self, Self::Manual | Self::Auto | Self::Rules)
    }

    /// The mode of a run. The flag layer (`--mode`, `IOHR_ASK_MODE`, `--permissions`,
    /// `IOHR_ASK_PERMISSIONS`) beats `[ask]`; inside a layer `mode` beats `permissions`; with
    /// nothing named it is `manual` at a terminal and `rules` without one. `manual` without a
    /// terminal to ask on is `rules`: nobody can answer, and no answer is no. `auto` and `plan`
    /// ask nobody, so they hold without a terminal.
    pub(crate) fn choose(
        flag: Words<'_>,
        config: Words<'_>,
        terminal: bool,
    ) -> Result<Self, String> {
        let named = if let Some(w) = said(flag.mode) {
            Some(
                Self::parse(w)
                    .ok_or_else(|| format!("--mode {w:?} is not one of {}", Self::NAMES))?,
            )
        } else if let Some(w) = said(flag.permissions) {
            Some(Self::parse_permissions(w).ok_or_else(|| {
                format!(
                    "--permissions {w:?} is not one of {}",
                    Self::PERMISSION_NAMES
                )
            })?)
        } else if let Some(w) = said(config.mode) {
            Some(
                Self::parse(w)
                    .ok_or_else(|| format!("[ask] mode {w:?} is not one of {}", Self::NAMES))?,
            )
        } else if let Some(w) = said(config.permissions) {
            Some(Self::parse_permissions(w).ok_or_else(|| {
                format!(
                    "[ask] permissions {w:?} is not one of {}",
                    Self::PERMISSION_NAMES
                )
            })?)
        } else {
            None
        };
        Ok(match named {
            Some(Self::Manual) | None if terminal => Self::Manual,
            Some(Self::Manual) | None => Self::Rules,
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

    fn flag<'a>(mode: Option<&'a str>, permissions: Option<&'a str>) -> Words<'a> {
        Words { mode, permissions }
    }

    #[test]
    fn the_default_is_manual_at_a_terminal_and_rules_without_one() {
        let none = Words::default();
        assert_eq!(Mode::choose(none, none, true), Ok(Mode::Manual));
        assert_eq!(Mode::choose(none, none, false), Ok(Mode::Rules));
        assert_eq!(
            Mode::choose(flag(None, Some("prompt")), none, false),
            Ok(Mode::Rules)
        );
        assert_eq!(
            Mode::choose(flag(Some("manual"), None), none, false),
            Ok(Mode::Rules)
        );
    }

    #[test]
    fn plan_and_auto_hold_without_a_terminal() {
        let none = Words::default();
        for (word, mode) in [("plan", Mode::Plan), ("auto", Mode::Auto)] {
            assert_eq!(Mode::choose(flag(Some(word), None), none, false), Ok(mode));
            assert_eq!(Mode::choose(flag(Some(word), None), none, true), Ok(mode));
        }
    }

    #[test]
    fn permissions_words_map_onto_the_modes_and_the_older_two_stay() {
        let none = Words::default();
        for (word, mode) in [
            ("prompt", Mode::Manual),
            ("manual", Mode::Manual),
            ("plan", Mode::Plan),
            ("auto", Mode::Auto),
            ("deny", Mode::Deny),
            ("rules", Mode::Rules),
        ] {
            assert_eq!(
                Mode::choose(flag(None, Some(word)), none, true),
                Ok(mode),
                "{word}"
            );
        }
    }

    #[test]
    fn the_flag_beats_the_config_and_mode_beats_permissions_in_a_layer() {
        let cfg = flag(Some("auto"), Some("deny"));
        assert_eq!(
            Mode::choose(flag(None, Some("rules")), cfg, true),
            Ok(Mode::Rules)
        );
        assert_eq!(
            Mode::choose(flag(Some("plan"), Some("rules")), cfg, true),
            Ok(Mode::Plan)
        );
        assert_eq!(Mode::choose(Words::default(), cfg, true), Ok(Mode::Auto));
        assert_eq!(
            Mode::choose(Words::default(), flag(None, Some("rules")), true),
            Ok(Mode::Rules)
        );
        assert_eq!(
            Mode::choose(flag(Some(" "), None), Words::default(), true),
            Ok(Mode::Manual)
        );
    }

    #[test]
    fn a_bad_word_is_refused_and_names_where_it_came_from() {
        let none = Words::default();
        let e = Mode::choose(flag(Some("yolo"), None), none, true).unwrap_err();
        assert!(
            e.contains("--mode") && e.contains("manual, plan or auto"),
            "{e}"
        );
        let e = Mode::choose(flag(Some("deny"), None), none, true).unwrap_err();
        assert!(e.contains("--mode"), "--mode takes only the three: {e}");
        let e = Mode::choose(flag(None, Some("yolo")), none, true).unwrap_err();
        assert!(
            e.contains("--permissions") && e.contains("deny or rules"),
            "{e}"
        );
        let e = Mode::choose(none, flag(Some("yolo"), None), true).unwrap_err();
        assert!(e.contains("[ask] mode"), "{e}");
    }

    #[test]
    fn the_cycle_goes_manual_plan_auto_and_the_hook_gets_that_order() {
        assert_eq!(CYCLE, [Mode::Manual, Mode::Plan, Mode::Auto]);
        assert_eq!(Mode::ring(), "manual plan auto");
        // The ring words parse back to the same modes, in order.
        let back: Vec<_> = Mode::ring().split(' ').filter_map(Mode::parse).collect();
        assert_eq!(back, CYCLE);
        // Only the three of the switch have a name to show; the older words have none.
        assert_eq!(Mode::Plan.shown(), Some("plan"));
        assert_eq!(Mode::Deny.shown(), None);
        assert_eq!(Mode::Rules.shown(), None);
    }

    #[test]
    fn only_plan_and_deny_run_no_commands() {
        assert!(Mode::Manual.runs_commands() && Mode::Auto.runs_commands());
        assert!(Mode::Rules.runs_commands());
        assert!(!Mode::Plan.runs_commands() && !Mode::Deny.runs_commands());
    }
}
