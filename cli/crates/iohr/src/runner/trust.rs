//! What this machine lets a coordinator turn do, whatever the turn asks (SR-35): the tool
//! ceiling, the repository allow-list and the OS sandbox mode. Each comes from the
//! `[runner]` table (or a profile's), is read at start and again on SIGHUP, and is applied
//! by the runner itself, so a coordinator can ask for less than the ceiling, never more.
//!
//! - The ceiling ([`Ceiling`]): the most tools a turn gets. What the turn asks for is cut
//!   down to it ([`Ceiling::narrow`]); what falls outside is dropped and logged by name.
//!   The default is the file tools and no shell ([`DEFAULT_TOOLS`]). `coordinator` keeps
//!   whatever the coordinator asks for, the behaviour before the ceiling existed.
//! - The allow-list ([`Repositories`]): the only repositories a job turn is checked out
//!   and run in, as `owner/name` or `owner/*`. Empty (the default) refuses every one.
//! - The sandbox mode ([`super::sandbox::Mode`]).

use super::sandbox;

/// The tools a turn gets at most when `tool_ceiling` is unset: read and search files,
/// and edit and write them inside the turn's checkout. No shell and no web: those are the
/// tools that run code or reach other machines, and the coordinator never asks for them
/// today, so the default takes nothing away from a turn that worked before.
pub(crate) const DEFAULT_TOOLS: &[&str] = &["Read", "Grep", "Glob", "Edit", "Write"];

/// The `tool_ceiling` value that keeps the coordinator's own tool list.
pub(crate) const COORDINATOR: &str = "coordinator";
/// The `tool_ceiling` value that names [`DEFAULT_TOOLS`].
pub(crate) const DEFAULT: &str = "default";

/// The most tools a turn may get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ceiling {
    /// Whatever the coordinator asks for, unchanged (`tool_ceiling = "coordinator"`).
    Coordinator,
    /// At most these entries: a tool name (`Read`) or a Claude Code permission rule
    /// (`Bash(git status:*)`).
    Tools(Vec<String>),
}

impl Default for Ceiling {
    fn default() -> Self {
        Self::Tools(DEFAULT_TOOLS.iter().map(|t| (*t).to_owned()).collect())
    }
}

/// A turn's tools after the ceiling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Narrowed {
    /// What the turn gets, in the order asked.
    pub(crate) kept: Vec<String>,
    /// What it asked for and does not get.
    pub(crate) dropped: Vec<String>,
}

impl Narrowed {
    /// The turn asked for tools and the ceiling left it none: it must not start.
    pub(crate) fn nothing_left(&self) -> bool {
        self.kept.is_empty() && !self.dropped.is_empty()
    }

    /// The tool names of what is kept, once each, for `--tools` (built-in tools only:
    /// an MCP tool is not one of Claude Code's built-in set).
    pub(crate) fn tool_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for k in &self.kept {
            let name = tool_name(k);
            if !name.starts_with("mcp__") && !out.iter().any(|o| o == name) {
                out.push(name.to_owned());
            }
        }
        out
    }
}

/// The tool an entry is for: `Bash` for `Bash(git status:*)`.
pub(crate) fn tool_name(entry: &str) -> &str {
    entry.split_once('(').map_or(entry, |(n, _)| n).trim()
}

/// Whether `entry` is a tool name (`Read`, `mcp__server__tool`) or a permission rule on
/// one (`Bash(git status:*)`): letters, digits and `_`, starting with a letter, and a rule
/// in parentheses that closes at the end.
pub(crate) fn is_tool_entry(entry: &str) -> bool {
    let (name, rule) = match entry.split_once('(') {
        Some((n, r)) => (n, Some(r)),
        None => (entry, None),
    };
    let name_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let rule_ok =
        rule.is_none_or(|r| r.len() > 1 && r.ends_with(')') && !r[..r.len() - 1].contains(')'));
    name_ok && rule_ok && !entry.starts_with('-')
}

impl Ceiling {
    /// Cuts `asked` down to the ceiling. An entry is kept when the ceiling names its
    /// tool bare (`Read` keeps `Read` and `Read(src/**)`) or names the entry exactly. A
    /// bare tool asked for when the ceiling only holds rules on it (`Bash` against
    /// `Bash(git status:*)`) becomes those rules. Anything else is dropped.
    pub(crate) fn narrow(&self, asked: &[String]) -> Narrowed {
        let Self::Tools(ceiling) = self else {
            return Narrowed {
                kept: asked.to_vec(),
                dropped: Vec::new(),
            };
        };
        let mut out = Narrowed::default();
        let keep = |e: &str, out: &mut Narrowed| {
            if !out.kept.iter().any(|k| k == e) {
                out.kept.push(e.to_owned());
            }
        };
        for a in asked {
            let a = a.trim();
            let name = tool_name(a);
            if ceiling.iter().any(|c| c == name || c == a) {
                keep(a, &mut out);
                continue;
            }
            let rules: Vec<&String> = ceiling
                .iter()
                .filter(|c| c.as_str() != tool_name(c) && tool_name(c) == name)
                .collect();
            if a == name && !rules.is_empty() {
                for r in rules {
                    keep(r, &mut out);
                }
            } else {
                out.dropped.push(a.to_owned());
            }
        }
        out
    }

    /// Whether this is a ceiling at all (not `coordinator`).
    pub(crate) fn restricts(&self) -> bool {
        matches!(self, Self::Tools(_))
    }

    /// How `iohr runner config show` and the start line name it.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Coordinator => "coordinator (what the coordinator asks for)".to_owned(),
            Self::Tools(t) if t.is_empty() => "no tools".to_owned(),
            Self::Tools(t) => t.join(", "),
        }
    }
}

/// The repositories a job turn may be checked out and run in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Repositories(pub(crate) Vec<String>);

impl Repositories {
    /// Whether `repository` (`owner/name`) is on the list: named exactly, or its owner
    /// named with `owner/*`. GitHub names are compared without regard to case.
    pub(crate) fn allows(&self, repository: &str) -> bool {
        let Some((owner, _)) = repository.split_once('/') else {
            return false;
        };
        self.0.iter().any(|p| match p.split_once('/') {
            Some((o, "*")) => o.eq_ignore_ascii_case(owner),
            Some(_) => p.eq_ignore_ascii_case(repository),
            None => false,
        })
    }

    /// Why `repository` is refused, for the turn's report and the log.
    pub(crate) fn refusal(&self, repository: &str) -> String {
        if self.0.is_empty() {
            format!(
                "repository {repository} is not on this runner's allow-list, which is empty: \
                 add it to `repositories` in the [runner] table of config.toml (\"owner/name\" or \"owner/*\") \
                 and send the runner SIGHUP"
            )
        } else {
            format!(
                "repository {repository} is not on this runner's allow-list ({}): \
                 add it to `repositories` in the [runner] table of config.toml and send the runner SIGHUP",
                self.0.join(", ")
            )
        }
    }

    /// How `iohr runner config show` and the start line name it.
    pub(crate) fn describe(&self) -> String {
        if self.0.is_empty() {
            "none (every repository is refused)".to_owned()
        } else {
            self.0.join(", ")
        }
    }
}

/// Whether `pattern` is an allow-list entry: `owner/name` or `owner/*`, each part made of
/// the characters GitHub allows.
pub(crate) fn is_repository_pattern(pattern: &str) -> bool {
    let part = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    match pattern.split_once('/') {
        Some((owner, "*")) => part(owner),
        Some((owner, name)) => part(owner) && part(name),
        None => false,
    }
}

/// The three together, as one reload sends them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Trust {
    pub(crate) ceiling: Ceiling,
    pub(crate) repositories: Repositories,
    pub(crate) sandbox: sandbox::Mode,
}

#[cfg(test)]
impl Repositories {
    pub(crate) fn of(p: &[&str]) -> Self {
        Self(p.iter().map(|s| (*s).to_owned()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn the_default_ceiling_keeps_the_file_tools_and_drops_the_shell_and_the_web() {
        let n = Ceiling::default().narrow(&v(&["Read", "Edit", "Bash", "WebFetch", "Grep"]));
        assert_eq!(n.kept, v(&["Read", "Edit", "Grep"]));
        assert_eq!(n.dropped, v(&["Bash", "WebFetch"]));
        assert!(!n.nothing_left());
        assert_eq!(n.tool_names(), v(&["Read", "Edit", "Grep"]));
    }

    #[test]
    fn coordinator_keeps_everything_asked() {
        let asked = v(&["Read", "Bash", "WebFetch"]);
        let n = Ceiling::Coordinator.narrow(&asked);
        assert_eq!(n.kept, asked);
        assert_eq!(n.dropped, Vec::<String>::new());
        assert!(!Ceiling::Coordinator.restricts());
    }

    #[test]
    fn a_bare_tool_in_the_ceiling_keeps_rules_on_it_and_a_rule_keeps_only_itself() {
        let c = Ceiling::Tools(v(&["Read", "Bash(git status:*)"]));
        let n = c.narrow(&v(&[
            "Read(src/**)",
            "Bash(git status:*)",
            "Bash(rm -rf:*)",
        ]));
        assert_eq!(n.kept, v(&["Read(src/**)", "Bash(git status:*)"]));
        assert_eq!(n.dropped, v(&["Bash(rm -rf:*)"]));
        assert_eq!(n.tool_names(), v(&["Read", "Bash"]));
    }

    #[test]
    fn a_bare_tool_asked_against_rules_on_it_becomes_those_rules() {
        let c = Ceiling::Tools(v(&["Bash(git status:*)", "Bash(git diff:*)"]));
        let n = c.narrow(&v(&["Bash"]));
        assert_eq!(n.kept, v(&["Bash(git status:*)", "Bash(git diff:*)"]));
        assert_eq!(n.dropped, Vec::<String>::new());
    }

    #[test]
    fn a_turn_whose_every_tool_is_outside_has_nothing_left() {
        let n = Ceiling::default().narrow(&v(&["Bash", "WebFetch"]));
        assert_eq!(n.kept, Vec::<String>::new());
        assert!(n.nothing_left());
        // A turn that asked for nothing has nothing to lose.
        assert!(!Ceiling::default().narrow(&[]).nothing_left());
        // An empty ceiling leaves nothing of anything.
        assert!(
            Ceiling::Tools(Vec::new())
                .narrow(&v(&["Read"]))
                .nothing_left()
        );
    }

    #[test]
    fn mcp_tools_stay_out_of_the_built_in_names() {
        let c = Ceiling::Tools(v(&["Read", "mcp__docs__search"]));
        let n = c.narrow(&v(&["mcp__docs__search", "Read"]));
        assert_eq!(n.kept.len(), 2);
        assert_eq!(n.tool_names(), v(&["Read"]));
    }

    #[test]
    fn tool_entries_are_names_or_closed_rules() {
        for ok in ["Read", "Bash(git status:*)", "mcp__a__b", "Read(./src/**)"] {
            assert!(is_tool_entry(ok), "{ok}");
        }
        for bad in [
            "",
            "-p",
            "--tools",
            "Bash(",
            "Bash()",
            "1Read",
            "Read)x",
            "Bash(a)b)",
            "Re ad",
        ] {
            assert!(!is_tool_entry(bad), "{bad}");
        }
    }

    #[test]
    fn the_allow_list_names_repositories_exactly_or_by_owner() {
        let r = Repositories::of(&["acme/app", "tools/*"]);
        assert!(r.allows("acme/app"));
        assert!(r.allows("Acme/App"));
        assert!(r.allows("tools/anything"));
        assert!(!r.allows("acme/other"));
        assert!(!r.allows("evil/app"));
        assert!(!r.allows("acme"));
    }

    #[test]
    fn an_empty_allow_list_refuses_everything_and_says_how_to_add_one() {
        let r = Repositories::default();
        assert!(!r.allows("acme/app"));
        let why = r.refusal("acme/app");
        assert!(
            why.contains("acme/app") && why.contains("empty") && why.contains("repositories"),
            "{why}"
        );
        let why = Repositories::of(&["acme/web"]).refusal("acme/app");
        assert!(why.contains("acme/web"), "{why}");
    }

    #[test]
    fn allow_list_patterns_are_owner_name_or_owner_star() {
        for ok in ["acme/app", "acme/*", "a-b/c.d_e"] {
            assert!(is_repository_pattern(ok), "{ok}");
        }
        for bad in [
            "acme", "*/*", "*", "acme/", "/app", "acme/a/b", "acme/..", "acme/a b", "acme/ap*",
        ] {
            assert!(!is_repository_pattern(bad), "{bad}");
        }
    }
}
