//! The decision: allow, deny or ask, for one tool call.
//!
//! Order (the same as Claude Code's, <https://code.claude.com/docs/en/agent-sdk/permissions>:
//! deny beats allow): the secret-file list first (nothing allows it), then deny rules, then
//! allow rules, else ask. A shell line is cut into its simple commands
//! ([`super::shell`]); it is allowed only when every one of them is, and a line with a
//! substitution, a variable or a write redirect is never allowed by a rule, because a prefix
//! rule cannot see what it will run.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::rule::{Rule, paths_of};
use super::{secrets, shell};

/// What a call is judged against.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleSet {
    pub(crate) allow: Vec<Rule>,
    pub(crate) deny: Vec<Rule>,
}

/// Where the call runs: rules with `./` and `~/` paths read against these.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    pub(crate) cwd: PathBuf,
    pub(crate) home: Option<PathBuf>,
}

impl Place {
    /// The folder a run starts in.
    pub(crate) fn at(cwd: &Path) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            home: etcetera::home_dir().ok(),
        }
    }

    fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// A rule allows it (the first one that does).
    Allow(String),
    /// A rule, or the secret list, refuses it.
    Deny(String),
    /// No rule covers it: a person decides.
    Ask,
}

/// What the secret list says when it refuses.
pub(crate) const SECRET_RULE: &str = "secret files";

/// Judges one call of `tool` with `input`.
#[must_use]
pub(crate) fn decide(rules: &RuleSet, tool: &str, input: &Value, at: &Place) -> Verdict {
    if tool == "Bash" {
        return decide_shell(rules, input.get("command").and_then(Value::as_str), at);
    }
    if is_file_tool(tool)
        && paths_of(input, &at.cwd)
            .iter()
            .any(|p| secrets::is_secret_path(p, &at.cwd, at.home()))
    {
        return Verdict::Deny(SECRET_RULE.to_owned());
    }
    if let Some(r) = rules.deny.iter().find(|r| {
        r.matches_call(tool, input, &at.cwd, at.home()) || (r.tool() == tool && r.spec().is_none())
    }) {
        return Verdict::Deny(r.to_string());
    }
    match rules
        .allow
        .iter()
        .find(|r| r.matches_call(tool, input, &at.cwd, at.home()))
    {
        Some(r) => Verdict::Allow(r.to_string()),
        None => Verdict::Ask,
    }
}

fn is_file_tool(tool: &str) -> bool {
    ["Read", "Edit", "Write", "Grep", "Glob", "NotebookEdit"].contains(&tool)
}

fn decide_shell(rules: &RuleSet, command: Option<&str>, at: &Place) -> Verdict {
    let Some(command) = command else {
        return Verdict::Ask;
    };
    let line = shell::split(command);
    if line.parts.is_empty() {
        return Verdict::Ask;
    }
    if line
        .parts
        .iter()
        .any(|p| secrets::secret_word(&p.words, &at.cwd, at.home()).is_some())
    {
        return Verdict::Deny(SECRET_RULE.to_owned());
    }
    for part in &line.parts {
        if let Some(r) = rules.deny.iter().find(|r| r.matches_command(&part.text)) {
            return Verdict::Deny(r.to_string());
        }
    }
    if line.opaque {
        return Verdict::Ask;
    }
    let mut used: Vec<String> = Vec::new();
    for part in &line.parts {
        match rules.allow.iter().find(|r| r.matches_command(&part.text)) {
            Some(r) => {
                let r = r.to_string();
                if !used.contains(&r) {
                    used.push(r);
                }
            }
            None => return Verdict::Ask,
        }
    }
    Verdict::Allow(used.join(" "))
}

/// Commands whose arguments can run or write more than they name: an "always" for one of these
/// is the exact command, never the command word.
const EXACT_ONLY: &[&str] = &[
    "rm",
    "rmdir",
    "mv",
    "cp",
    "dd",
    "chmod",
    "chown",
    "ln",
    "sudo",
    "su",
    "doas",
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "eval",
    "exec",
    "source",
    ".",
    "xargs",
    "env",
    "find",
    "awk",
    "sed",
    "perl",
    "python",
    "python3",
    "node",
    "ruby",
    "curl",
    "wget",
    "ssh",
    "scp",
    "rsync",
    "nc",
    "kill",
    "killall",
    "pkill",
    "tee",
    "time",
    "nohup",
    "nice",
    "watch",
    "timeout",
    "open",
    "osascript",
    "tar",
    "unzip",
    "git push",
    "npm run",
    "npx",
    "make",
    "docker run",
    "docker exec",
    "kubectl delete",
    "kubectl exec",
];

/// Commands that name a verb first: `git status`, not `git`.
const WITH_VERB: &[&str] = &[
    "git",
    "cargo",
    "npm",
    "pnpm",
    "yarn",
    "docker",
    "kubectl",
    "gh",
    "mise",
    "go",
    "make",
    "brew",
    "pip",
    "pip3",
    "uv",
    "rustup",
    "helm",
    "terraform",
    "systemctl",
    "journalctl",
];

/// The narrowest rule that covers this call, to offer as "always", or none when no safe rule
/// does (a line with a substitution or a redirect, a secret file, a call with no path).
#[must_use]
pub(crate) fn suggest(tool: &str, input: &Value, at: &Place) -> Option<Vec<Rule>> {
    match tool {
        "Bash" => {
            let command = input.get("command")?.as_str()?;
            let line = shell::split(command);
            if line.opaque || line.parts.is_empty() {
                return None;
            }
            let mut out: Vec<Rule> = Vec::new();
            for part in &line.parts {
                if secrets::secret_word(&part.words, &at.cwd, at.home()).is_some() {
                    return None;
                }
                let rule = shell_rule(&part.text, &part.words)?;
                if !out.contains(&rule) {
                    out.push(rule);
                }
            }
            Some(out)
        }
        "WebFetch" => {
            let host = super::rule::host_of(input)?;
            Some(vec![Rule::of("WebFetch", format!("domain:{host}"))])
        }
        "Read" | "Grep" | "Glob" => {
            let p = paths_of(input, &at.cwd).into_iter().next()?;
            if secrets::is_secret_path(&p, &at.cwd, at.home()) {
                return None;
            }
            let at_p = super::path::resolve(&p, &at.cwd, at.home());
            let dir = Path::new(&at_p.abs).parent()?;
            let spec = match dir.strip_prefix(&at.cwd) {
                Ok(rel) if rel.as_os_str().is_empty() => "./**".to_owned(),
                Ok(rel) => format!("./{}/**", crate::paths::slashed(rel)),
                Err(_) => format!("{}/**", crate::paths::slashed(dir)),
            };
            Some(vec![Rule::of(tool, spec)])
        }
        "Edit" | "Write" | "NotebookEdit" => {
            let p = paths_of(input, &at.cwd).into_iter().next()?;
            if secrets::is_secret_path(&p, &at.cwd, at.home()) {
                return None;
            }
            let at_p = super::path::resolve(&p, &at.cwd, at.home());
            let spec = at_p.rel.map_or(at_p.abs, |r| format!("./{r}"));
            Some(vec![Rule::of(tool, spec)])
        }
        _ => None,
    }
}

fn shell_rule(text: &str, words: &[String]) -> Option<Rule> {
    let first = words.first()?;
    let verb = words
        .get(1)
        .filter(|w| WITH_VERB.contains(&first.as_str()) && !w.starts_with('-'));
    let name = match verb {
        Some(v) => format!("{first} {v}"),
        None => first.clone(),
    };
    if EXACT_ONLY.contains(&name.as_str()) || EXACT_ONLY.contains(&first.as_str()) {
        // Quoting is part of the command, so a rule keeps the text as written.
        return Some(Rule::of("Bash", text));
    }
    Some(Rule::of("Bash", format!("{name}:*")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at() -> Place {
        Place {
            cwd: PathBuf::from("/work/proj"),
            home: Some(PathBuf::from("/home/me")),
        }
    }

    fn rules(allow: &[&str], deny: &[&str]) -> RuleSet {
        RuleSet {
            allow: allow.iter().map(|r| Rule::parse(r).unwrap()).collect(),
            deny: deny.iter().map(|r| Rule::parse(r).unwrap()).collect(),
        }
    }

    fn bash(rs: &RuleSet, cmd: &str) -> Verdict {
        decide(rs, "Bash", &json!({ "command": cmd }), &at())
    }

    #[test]
    fn nothing_matching_asks() {
        assert_eq!(bash(&RuleSet::default(), "ls -la"), Verdict::Ask);
    }

    #[test]
    fn an_allow_rule_allows_and_deny_beats_it() {
        let rs = rules(&["Bash(ls:*)"], &[]);
        assert_eq!(bash(&rs, "ls -la"), Verdict::Allow("Bash(ls:*)".into()));
        let rs = rules(&["Bash(ls:*)"], &["Bash(ls -la)"]);
        assert_eq!(bash(&rs, "ls -la"), Verdict::Deny("Bash(ls -la)".into()));
        assert_eq!(bash(&rs, "ls"), Verdict::Allow("Bash(ls:*)".into()));
    }

    #[test]
    fn every_part_of_a_line_must_be_allowed() {
        let rs = rules(&["Bash(ls:*)"], &[]);
        assert_eq!(bash(&rs, "ls && rm -rf x"), Verdict::Ask);
        assert_eq!(bash(&rs, "ls; ls -la"), Verdict::Allow("Bash(ls:*)".into()));
        let rs = rules(&["Bash(ls:*)", "Bash(wc:*)"], &[]);
        assert!(matches!(bash(&rs, "ls | wc -l"), Verdict::Allow(_)));
    }

    #[test]
    fn a_deny_rule_catches_one_part_of_a_line() {
        let rs = rules(&["Bash(ls:*)"], &["Bash(rm:*)"]);
        assert_eq!(bash(&rs, "ls; rm x"), Verdict::Deny("Bash(rm:*)".into()));
    }

    #[test]
    fn substitutions_and_redirects_are_never_allowed_by_a_rule() {
        let rs = rules(&["Bash(ls:*)", "Bash(echo:*)"], &[]);
        for cmd in ["ls $(rm -rf x)", "ls `id`", "echo hi > f", "ls $HOME"] {
            assert_eq!(bash(&rs, cmd), Verdict::Ask, "{cmd}");
        }
    }

    #[test]
    fn secret_files_beat_every_allow_rule() {
        let rs = rules(
            &["Bash(cat:*)", "Read", "Read(**/.env)", "Bash(cat .env)"],
            &[],
        );
        for cmd in [
            "cat .env",
            "cat ./sub/../.env",
            "cat ~/.ssh/id_rsa",
            "ls; cat --file=.env",
        ] {
            assert_eq!(bash(&rs, cmd), Verdict::Deny(SECRET_RULE.into()), "{cmd}");
        }
        for p in [
            ".env",
            "keys/server.pem",
            "/home/me/.aws/credentials",
            "~/.inorbit/x",
        ] {
            assert_eq!(
                decide(&rs, "Read", &json!({ "file_path": p }), &at()),
                Verdict::Deny(SECRET_RULE.into()),
                "{p}"
            );
        }
        // A search of a secret folder is refused the same way.
        assert_eq!(
            decide(&rs, "Grep", &json!({ "path": "/home/me/.ssh" }), &at()),
            Verdict::Deny(SECRET_RULE.into())
        );
    }

    #[test]
    fn file_tools_follow_their_rules() {
        let rs = rules(&["Read(./src/**)"], &["Read(./src/gen/**)"]);
        let read = |p: &str| decide(&rs, "Read", &json!({ "file_path": p }), &at());
        assert!(matches!(read("src/a.rs"), Verdict::Allow(_)));
        assert!(matches!(read("src/gen/a.rs"), Verdict::Deny(_)));
        assert_eq!(read("lib/a.rs"), Verdict::Ask);
    }

    #[test]
    fn suggestions_are_the_narrowest_rule() {
        let s = |tool: &str, input: Value| {
            suggest(tool, &input, &at())
                .map(|v| v.iter().map(ToString::to_string).collect::<Vec<_>>())
        };
        assert_eq!(
            s("Bash", json!({"command":"ls -la"})),
            Some(vec!["Bash(ls:*)".into()])
        );
        assert_eq!(
            s("Bash", json!({"command":"git status -s"})),
            Some(vec!["Bash(git status:*)".into()])
        );
        assert_eq!(
            s("Bash", json!({"command":"cargo test -p iohr"})),
            Some(vec!["Bash(cargo test:*)".into()])
        );
        assert_eq!(
            s("Bash", json!({"command":"rm -rf target"})),
            Some(vec!["Bash(rm -rf target)".into()])
        );
        assert_eq!(
            s("Bash", json!({"command":"git push origin main"})),
            Some(vec!["Bash(git push origin main)".into()])
        );
        assert_eq!(
            s("Bash", json!({"command":"ls | wc -l"})),
            Some(vec!["Bash(ls:*)".into(), "Bash(wc:*)".into()])
        );
        assert_eq!(s("Bash", json!({"command":"ls $(id)"})), None);
        assert_eq!(s("Bash", json!({"command":"cat .env"})), None);
        assert_eq!(
            s("WebFetch", json!({"url":"https://velebit.ai/a"})),
            Some(vec!["WebFetch(domain:velebit.ai)".into()])
        );
        assert_eq!(
            s("Read", json!({"file_path":"/work/proj/src/a.rs"})),
            Some(vec!["Read(./src/**)".into()])
        );
        assert_eq!(s("Read", json!({"file_path":".env"})), None);
    }

    #[test]
    fn a_suggested_rule_covers_the_call_it_came_from() {
        for cmd in [
            "ls -la",
            "git status",
            "cargo test -p iohr",
            "rm -rf target",
            "ls | wc -l",
        ] {
            let input = json!({ "command": cmd });
            let rs = RuleSet {
                allow: suggest("Bash", &input, &at()).unwrap(),
                deny: vec![],
            };
            assert!(
                matches!(decide(&rs, "Bash", &input, &at()), Verdict::Allow(_)),
                "{cmd}"
            );
        }
    }
}
