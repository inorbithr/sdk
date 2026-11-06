//! A permission rule in Claude Code's grammar: `Tool` or `Tool(specifier)`, for example
//! `Bash(ls:*)`, `Bash(git status)`, `Read(./src/**)`, `WebFetch(domain:velebit.ai)`
//! (<https://code.claude.com/docs/en/permissions>, "Permission rule syntax"). `Bash(ls:*)`
//! and `Bash(ls *)` both mean the command `ls` with any arguments, matched at a word
//! boundary (so not `lsof`); `:*` is only valid at the end.

use std::fmt;
use std::path::Path;

use serde_json::Value;

use super::{glob, path};

/// The tools whose specifier is a path.
const FILE_TOOLS: &[&str] = &["Read", "Edit", "Write", "Grep", "Glob", "NotebookEdit"];

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    tool: String,
    spec: Option<String>,
}

impl Rule {
    /// Reads a rule, or says why it is not one.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (tool, spec) = match text.split_once('(') {
            Some((tool, rest)) => {
                let spec = rest
                    .strip_suffix(')')
                    .ok_or_else(|| format!("`{text}` is missing its closing `)`"))?
                    .trim();
                if spec.is_empty() {
                    return Err(format!("`{text}` has nothing inside the brackets"));
                }
                (tool.trim(), Some(spec.to_owned()))
            }
            None => (text, None),
        };
        let valid = tool.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(format!(
                "`{text}` is not a rule: write a tool, or a tool with a specifier, like `Bash(ls:*)`"
            ));
        }
        if let Some(s) = &spec
            && let Some(at) = s.find(":*")
            && at + 2 != s.len()
        {
            return Err(format!("`:*` is only valid at the end of `{text}`"));
        }
        Ok(Self {
            tool: tool.to_owned(),
            spec,
        })
    }

    pub(crate) fn tool(&self) -> &str {
        &self.tool
    }

    pub(crate) fn spec(&self) -> Option<&str> {
        self.spec.as_deref()
    }

    /// The tool and the specifier, from parts the code built itself.
    pub(crate) fn of(tool: &str, spec: impl Into<String>) -> Self {
        Self {
            tool: tool.to_owned(),
            spec: Some(spec.into()),
        }
    }

    pub(crate) fn is_file_tool(&self) -> bool {
        FILE_TOOLS.contains(&self.tool.as_str())
    }

    /// The rule names a whole tool, or every command: too broad to allow by hand.
    pub(crate) fn is_blanket(&self) -> bool {
        self.tool == "Bash"
            && self
                .spec
                .as_deref()
                .is_none_or(|s| matches!(s, "*" | ":*" | "**"))
    }

    /// Whether this rule covers one simple shell command (a part of a line).
    pub(crate) fn matches_command(&self, command: &str) -> bool {
        if self.tool != "Bash" {
            return false;
        }
        let Some(spec) = self.spec.as_deref() else {
            return true;
        };
        let command = command.trim();
        let prefix = spec
            .strip_suffix(":*")
            .or_else(|| spec.strip_suffix(" *").filter(|p| !p.contains('*')));
        match prefix {
            Some(p) => command == p || command.strip_prefix(p).is_some_and(|r| r.starts_with(' ')),
            None if spec.contains('*') => glob::wild(spec, command),
            None => command == spec,
        }
    }

    /// Whether this rule covers a call of a tool other than the shell.
    pub(crate) fn matches_call(
        &self,
        tool: &str,
        input: &Value,
        cwd: &Path,
        home: Option<&Path>,
    ) -> bool {
        if self.tool != tool || tool == "Bash" {
            return false;
        }
        let Some(spec) = self.spec.as_deref() else {
            return true;
        };
        if self.is_file_tool() {
            let paths = paths_of(input, cwd);
            return !paths.is_empty()
                && paths
                    .iter()
                    .all(|p| path::matches(spec, &path::resolve(p, cwd, home), cwd, home));
        }
        if tool == "WebFetch"
            && let Some(domain) = spec.strip_prefix("domain:")
        {
            return host_of(input).is_some_and(|h| {
                let h = h.to_ascii_lowercase();
                let d = domain.trim().to_ascii_lowercase();
                h == d || h.ends_with(&format!(".{d}"))
            });
        }
        false
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.spec {
            Some(s) => write!(f, "{}({s})", self.tool),
            None => f.write_str(&self.tool),
        }
    }
}

/// The paths a file tool call names. Grep and Glob with no path search the folder itself.
pub(crate) fn paths_of(input: &Value, cwd: &Path) -> Vec<String> {
    let mut v: Vec<String> = ["file_path", "notebook_path", "path"]
        .iter()
        .filter_map(|k| input.get(*k)?.as_str().map(str::to_owned))
        .collect();
    if v.is_empty() {
        v.push(crate::paths::slashed(cwd));
    }
    v
}

/// The host of a `WebFetch` call's URL.
pub(crate) fn host_of(input: &Value) -> Option<String> {
    let url = input.get("url")?.as_str()?;
    url::Url::parse(url).ok()?.host_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn r(s: &str) -> Rule {
        Rule::parse(s).unwrap()
    }

    #[test]
    fn rules_round_trip() {
        for s in [
            "Bash(ls:*)",
            "Bash(git status)",
            "Read(./src/**)",
            "WebFetch(domain:velebit.ai)",
            "WebSearch",
        ] {
            assert_eq!(r(s).to_string(), s);
        }
    }

    #[test]
    fn bad_rules_say_why() {
        for s in ["", "Bash(", "Bash()", "Bash(ls:*x)", "(ls)", "No sh", "1x"] {
            assert!(Rule::parse(s).is_err(), "{s:?}");
        }
    }

    #[test]
    fn a_command_prefix_stops_at_a_word() {
        assert!(r("Bash(ls:*)").matches_command("ls"));
        assert!(r("Bash(ls:*)").matches_command("ls -la"));
        assert!(!r("Bash(ls:*)").matches_command("lsof -i"));
        assert!(r("Bash(ls *)").matches_command("ls -la"));
        assert!(r("Bash(git status)").matches_command("git status"));
        assert!(!r("Bash(git status)").matches_command("git status -s"));
        assert!(r("Bash(git * main)").matches_command("git push origin main"));
        assert!(!r("Bash(ls:*)").matches_command("rm ls"));
    }

    #[test]
    fn file_and_web_rules() {
        let cwd = Path::new("/work/proj");
        let read = |p: &str| json!({ "file_path": p });
        assert!(r("Read(./src/**)").matches_call("Read", &read("src/a.rs"), cwd, None));
        assert!(!r("Read(./src/**)").matches_call("Read", &read("lib/a.rs"), cwd, None));
        assert!(!r("Read(./src/**)").matches_call("Edit", &read("src/a.rs"), cwd, None));
        let web = json!({ "url": "https://docs.velebit.ai/x" });
        assert!(r("WebFetch(domain:velebit.ai)").matches_call("WebFetch", &web, cwd, None));
        assert!(!r("WebFetch(domain:evil.com)").matches_call("WebFetch", &web, cwd, None));
        assert!(!r("WebFetch(domain:velebit.ai)").matches_call(
            "WebFetch",
            &json!({ "url": "https://notvelebit.ai" }),
            cwd,
            None
        ));
        assert!(r("WebSearch").matches_call("WebSearch", &json!({}), cwd, None));
    }

    #[test]
    fn blanket_shell_rules_are_named() {
        assert!(r("Bash").is_blanket());
        assert!(r("Bash(*)").is_blanket());
        assert!(!r("Bash(ls:*)").is_blanket());
        assert!(!r("Read").is_blanket());
    }
}
