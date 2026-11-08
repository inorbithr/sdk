//! Where rules live and where decisions are written down. Config files, no database:
//! `<project>/.inorbit/permissions.toml` (committed with the project, or not) and
//! `permissions.toml` in the iohr config folder (the person's own, every project). Each is
//! `allow = ["Bash(ls:*)"]` and `deny = [...]`. The audit trail is `<project>/.inorbit/
//! permissions.log`, one JSON line per decision: when, which tool, which rule, the outcome and
//! who. It never holds what a command printed or what a file said.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use toml_edit::{Array, DocumentMut, Item, Value};

use super::engine::{Place, RuleSet};
use super::{rule::Rule, secrets};

/// File name of both rule files.
pub(crate) const FILE: &str = "permissions.toml";
/// The project's folder and the audit log in it.
const DIR: &str = ".inorbit";
const LOG: &str = "permissions.log";

/// Which file a rule is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Project,
    User,
}

impl Scope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

/// Which list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Allow,
    Deny,
}

impl Kind {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// The two files of one project, found from a folder.
#[derive(Debug, Clone)]
pub(crate) struct Files {
    /// Where the project's file goes: its root, or the folder when it is not in one.
    pub(crate) root: PathBuf,
    pub(crate) user_dir: PathBuf,
}

impl Files {
    pub(crate) fn find(cwd: &Path, user_dir: &Path) -> Self {
        Self {
            root: crate::memory::find_root(cwd).unwrap_or_else(|| cwd.to_path_buf()),
            user_dir: user_dir.to_path_buf(),
        }
    }

    pub(crate) fn path(&self, scope: Scope) -> PathBuf {
        match scope {
            Scope::Project => self.root.join(DIR).join(FILE),
            Scope::User => self.user_dir.join(FILE),
        }
    }

    pub(crate) fn log_path(&self) -> PathBuf {
        self.root.join(DIR).join(LOG)
    }
}

/// One rule and where it came from.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) scope: Scope,
    pub(crate) kind: Kind,
    pub(crate) rule: Rule,
}

fn read_doc(path: &Path) -> Result<DocumentMut, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => t
            .parse::<DocumentMut>()
            .map_err(|e| format!("{} is not valid TOML: {}", path.display(), e.message())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(format!("cannot read {}: {}", path.display(), e.kind())),
    }
}

fn strings(doc: &DocumentMut, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Every rule in both files. A rule that does not parse is an error: dropping a deny
/// quietly would be unsafe, so the caller treats it as "no asking" and says so.
pub(crate) fn entries(files: &Files) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for scope in [Scope::Project, Scope::User] {
        let path = files.path(scope);
        let doc = read_doc(&path)?;
        for kind in [Kind::Deny, Kind::Allow] {
            for text in strings(&doc, kind.key()) {
                let rule = Rule::parse(&text)
                    .map_err(|e| format!("{}: {} list: {e}", path.display(), kind.key()))?;
                out.push(Entry { scope, kind, rule });
            }
        }
    }
    Ok(out)
}

/// The rules as the engine reads them.
pub(crate) fn rule_set(files: &Files) -> Result<RuleSet, String> {
    let mut set = RuleSet::default();
    for e in entries(files)? {
        match e.kind {
            Kind::Allow => set.allow.push(e.rule),
            Kind::Deny => set.deny.push(e.rule),
        }
    }
    Ok(set)
}

/// Why an allow rule is refused, if it is.
pub(crate) fn refuse_allow(rule: &Rule, at: &Place) -> Option<String> {
    if rule.is_blanket() {
        return Some(format!(
            "`{rule}` would allow every command; name one, like `Bash(ls:*)`"
        ));
    }
    let home = at.home.as_deref();
    let hits = match rule.spec() {
        Some(spec) if rule.is_file_tool() => secrets::pattern_hits_secret(spec, &at.cwd, home),
        Some(spec) if rule.tool() == "Bash" => super::shell::split(spec)
            .parts
            .iter()
            .any(|p| secrets::secret_word(&p.words, &at.cwd, home).is_some()),
        _ => false,
    };
    hits.then(|| {
        format!(
            "`{rule}` could open files that are never allowed (keys, `.env`, credentials); \
             no rule or answer can allow them"
        )
    })
}

/// Adds `rule` to a list of one file. Returns false when it was already there. An allow rule
/// that would open a secret file is refused. Comments and order in the file are kept.
pub(crate) fn add(
    files: &Files,
    scope: Scope,
    kind: Kind,
    rule: &Rule,
    at: &Place,
) -> Result<bool, String> {
    if kind == Kind::Allow
        && let Some(why) = refuse_allow(rule, at)
    {
        return Err(why);
    }
    let path = files.path(scope);
    let mut doc = read_doc(&path)?;
    let text = rule.to_string();
    if strings(&doc, kind.key()).contains(&text) {
        return Ok(false);
    }
    let item = doc
        .entry(kind.key())
        .or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let Some(list) = item.as_array_mut() else {
        return Err(format!(
            "`{}` in {} is not a list",
            kind.key(),
            path.display()
        ));
    };
    list.push(text);
    list.set_trailing_comma(true);
    write(&path, &doc.to_string())?;
    Ok(true)
}

/// Removes `rule` from every list it is in, in the files `scope` names (`None` for both).
/// Returns how many entries went.
pub(crate) fn remove(files: &Files, scope: Option<Scope>, rule: &Rule) -> Result<usize, String> {
    let mut gone = 0;
    for s in [Scope::Project, Scope::User] {
        if scope.is_some_and(|only| only != s) {
            continue;
        }
        let path = files.path(s);
        let mut doc = read_doc(&path)?;
        let text = rule.to_string();
        let mut changed = false;
        for kind in [Kind::Allow, Kind::Deny] {
            if let Some(list) = doc.get_mut(kind.key()).and_then(Item::as_array_mut) {
                let before = list.len();
                list.retain(|v| v.as_str() != Some(text.as_str()));
                if list.len() != before {
                    gone += before - list.len();
                    changed = true;
                }
            }
        }
        if changed {
            write(&path, &doc.to_string())?;
        }
    }
    Ok(gone)
}

/// Of `texts` (rules as the status row's record wrote them), the ones still saved as allow
/// rules in the project file: what Undo may take back. Text that does not parse or is no
/// longer there is left out, so Undo is offered only where it does something.
pub(crate) fn saved_project_allows(files: &Files, texts: &[String]) -> Vec<Rule> {
    let Ok(all) = entries(files) else {
        return Vec::new();
    };
    texts
        .iter()
        .filter_map(|t| Rule::parse(t).ok())
        .filter(|r| {
            all.iter()
                .any(|e| e.scope == Scope::Project && e.kind == Kind::Allow && e.rule == *r)
        })
        .collect()
}

/// Takes `rules` back out of the project file: the same [`remove`] that `iohr permissions
/// remove --project` calls. Returns how many entries went.
pub(crate) fn take_back(files: &Files, rules: &[Rule]) -> Result<usize, String> {
    let mut gone = 0;
    for rule in rules {
        gone += remove(files, Some(Scope::Project), rule)?;
    }
    Ok(gone)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot make {}: {}", dir.display(), e.kind()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {}", path.display(), e.kind()))
}

/// One line of the audit log.
#[derive(Debug, Serialize)]
struct Line<'a> {
    time: String,
    tool: &'a str,
    rule: &'a str,
    outcome: &'a str,
    who: String,
    /// For a folder granted: where the yes came from (`flag`, `env`, `config`, `trust` or
    /// `card`). Absent on a tool decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    /// For a folder granted: its path, never anything inside it. Absent on a tool decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
}

/// How a decision came about.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// A rule allowed it.
    AllowedByRule,
    /// The person said yes once.
    AllowedOnce,
    /// `auto` mode: no rule decided it and nobody was asked.
    AllowedAuto,
    /// The person said always: a rule was written.
    AllowedAlways,
    /// A rule or the secret list refused it.
    DeniedByRule,
    /// The person said no.
    DeniedByPerson,
    /// Nobody could answer (no terminal, `rules` mode, Ctrl-C, no answer in time).
    DeniedNoAnswer,
}

#[cfg(unix)]
impl Outcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AllowedByRule => "allowed by rule",
            Self::AllowedOnce => "allowed once",
            Self::AllowedAuto => "allowed, auto mode",
            Self::AllowedAlways => "allowed always",
            Self::DeniedByRule => "denied by rule",
            Self::DeniedByPerson => "denied by person",
            Self::DeniedNoAnswer => "denied, no answer",
        }
    }
}

/// Appends one decision. `rule` is the rule that decided it, or the one an "always" wrote, or
/// the tool's own words for a call no rule names; a text that looks like a secret is not kept.
/// A failure to write is not the ask's failure: it is returned for the caller to mention.
#[cfg(unix)]
pub(crate) fn audit(files: &Files, tool: &str, rule: &str, outcome: Outcome) -> Result<(), String> {
    let rule = if crate::memory::text::has_secret(rule) {
        "(redacted)"
    } else {
        rule
    };
    append(
        files,
        &Line {
            time: crate::front::now(),
            tool,
            rule,
            outcome: outcome.as_str(),
            who: who(),
            source: None,
            path: None,
        },
    )
}

/// Records that a folder was given to a run to read: one line with the `source` of the yes
/// (`flag`, `env`, `config`, `trust` or `card`) and the folder's `path`, never anything in it.
/// A failure to write is returned for the caller to mention, never the ask's failure.
///
/// # Errors
///
/// The log could not be written.
pub(crate) fn audit_folder(files: &Files, source: &str, path: &Path) -> Result<(), String> {
    let path = path.display().to_string();
    let path: String = path
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    append(
        files,
        &Line {
            time: crate::front::now(),
            tool: "folder",
            rule: "folder granted",
            outcome: "allowed",
            who: who(),
            source: Some(source),
            path: Some(&path),
        },
    )
}

/// Who the line says did it: the login name, or `local`.
fn who() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty() && u.chars().all(|c| c.is_ascii_graphic()))
        .unwrap_or_else(|| "local".to_owned())
}

/// Appends `line` to the project's `permissions.log`.
fn append(files: &Files, line: &Line<'_>) -> Result<(), String> {
    let path = files.log_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.kind().to_string())?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.kind().to_string())?;
    let mut text = serde_json::to_string(line).map_err(|e| e.to_string())?;
    text.push('\n');
    f.write_all(text.as_bytes())
        .map_err(|e| e.kind().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(d: &Path) -> Files {
        Files {
            root: d.join("proj"),
            user_dir: d.join("cfg"),
        }
    }

    fn at() -> Place {
        Place {
            cwd: PathBuf::from("/work/proj"),
            home: Some(PathBuf::from("/home/me")),
        }
    }

    #[test]
    fn rules_are_written_and_read_back_from_both_files() {
        let d = tempfile::tempdir().unwrap();
        let f = files(d.path());
        let ls = Rule::parse("Bash(ls:*)").unwrap();
        assert!(add(&f, Scope::Project, Kind::Allow, &ls, &at()).unwrap());
        assert!(!add(&f, Scope::Project, Kind::Allow, &ls, &at()).unwrap());
        let rm = Rule::parse("Bash(rm:*)").unwrap();
        add(&f, Scope::User, Kind::Deny, &rm, &at()).unwrap();
        let set = rule_set(&f).unwrap();
        assert_eq!(set.allow, vec![ls.clone()]);
        assert_eq!(set.deny, vec![rm.clone()]);
        let all = entries(&f).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(remove(&f, None, &ls).unwrap(), 1);
        assert_eq!(remove(&f, None, &ls).unwrap(), 0);
        assert_eq!(rule_set(&f).unwrap().allow.len(), 0);
    }

    #[test]
    fn a_hand_edited_file_keeps_its_comments() {
        let d = tempfile::tempdir().unwrap();
        let f = files(d.path());
        let p = f.path(Scope::Project);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "# mine\nallow = [\"Bash(ls:*)\"]\n").unwrap();
        add(
            &f,
            Scope::Project,
            Kind::Allow,
            &Rule::parse("Bash(pwd)").unwrap(),
            &at(),
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# mine"), "{text}");
        assert!(text.contains("Bash(pwd)") && text.contains("Bash(ls:*)"));
    }

    #[test]
    fn a_rule_that_would_open_a_secret_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = files(d.path());
        for r in [
            "Read(**/.env)",
            "Read(./.env)",
            "Read(~/.ssh/**)",
            "Edit(./.env.local)",
            "Bash(cat .env)",
            "Bash(cat ~/.aws/credentials)",
            "Bash",
            "Bash(*)",
        ] {
            let rule = Rule::parse(r).unwrap();
            assert!(
                add(&f, Scope::Project, Kind::Allow, &rule, &at()).is_err(),
                "{r} must be refused"
            );
        }
        assert!(!f.path(Scope::Project).exists(), "nothing was written");
        // The same rules are fine as denials.
        let deny = Rule::parse("Read(**/.env)").unwrap();
        assert!(add(&f, Scope::Project, Kind::Deny, &deny, &at()).unwrap());
        // And ordinary ones are fine to allow.
        for r in [
            "Read(./src/**)",
            "Read(./**)",
            "Bash(ls:*)",
            "WebFetch(domain:velebit.ai)",
        ] {
            assert!(
                add(
                    &f,
                    Scope::Project,
                    Kind::Allow,
                    &Rule::parse(r).unwrap(),
                    &at()
                )
                .is_ok(),
                "{r}"
            );
        }
    }

    #[test]
    fn a_bad_rule_in_a_file_is_an_error_not_a_silent_drop() {
        let d = tempfile::tempdir().unwrap();
        let f = files(d.path());
        let p = f.path(Scope::User);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "deny = [\"Bash(\"]\n").unwrap();
        assert!(rule_set(&f).unwrap_err().contains("deny"));
    }

    #[cfg(unix)]
    #[test]
    fn the_audit_line_has_no_content() {
        let d = tempfile::tempdir().unwrap();
        let f = files(d.path());
        audit(&f, "Bash", "Bash(ls:*)", Outcome::AllowedOnce).unwrap();
        audit(
            &f,
            "Bash",
            "Bash(curl -H 'Authorization: Bearer sk-ant-api03-AAAAAAAAAAAAAAAAAAAA')",
            Outcome::DeniedByPerson,
        )
        .unwrap();
        let text = std::fs::read_to_string(f.log_path()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["tool"], "Bash");
        assert_eq!(lines[0]["rule"], "Bash(ls:*)");
        assert_eq!(lines[0]["outcome"], "allowed once");
        assert!(lines[0]["time"].as_str().unwrap().contains('T'));
        assert!(lines[0]["who"].as_str().is_some());
        assert!(!text.contains("sk-ant"), "{text}");
    }

    #[test]
    fn a_granted_folder_is_one_line_with_its_source_and_path_and_nothing_else() {
        let d = tempfile::tempdir().unwrap();
        let f = files(d.path());
        audit_folder(&f, "flag", Path::new("/srv/docs")).unwrap();
        audit_folder(&f, "trust", Path::new("/srv/we\nird")).unwrap();
        let text = std::fs::read_to_string(f.log_path()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(lines[0]["source"], "flag");
        assert_eq!(lines[0]["path"], "/srv/docs");
        assert_eq!(lines[0]["outcome"], "allowed");
        assert_eq!(lines[1]["source"], "trust");
        assert_eq!(
            lines[1]["path"], "/srv/we ird",
            "a control character is a space"
        );
    }
}
