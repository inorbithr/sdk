//! `iohr permissions list|allow|deny|remove|check`: the rules for what Io may run or open
//! when `iohr ask` is used (PRD 0197). Local files only, no sign-in and no network.
//!
//! The rules are Claude Code's own grammar
//! (<https://code.claude.com/docs/en/permissions>): a deny rule beats an allow rule, and the
//! secret-file list ([`crate::permissions::secrets`]) beats both: `allow` refuses a rule that
//! could open such a file.

use std::path::Path;

use serde_json::{Value, json};

use crate::cli::{Global, PermissionScope, PermissionsCommand};
use crate::context::Ctx;
use crate::error::Error;
use crate::output::Out;
use crate::permissions::engine::{Place, Verdict, decide};
use crate::permissions::rule::Rule;
use crate::permissions::store::{self, Files, Kind, Scope};
use crate::ui::Listed;

pub(crate) fn run(g: &Global, cmd: PermissionsCommand, out: Out) -> Result<(), Error> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = Files::find(&cwd, &Ctx::load(g)?.dir);
    let place = Place::at(&cwd);
    match cmd {
        PermissionsCommand::List { scope } => list(&files, only(scope), out),
        PermissionsCommand::Allow { rule, scope } => {
            add(&files, &place, Kind::Allow, &rule, scope, out)
        }
        PermissionsCommand::Deny { rule, scope } => {
            add(&files, &place, Kind::Deny, &rule, scope, out)
        }
        PermissionsCommand::Remove { rule, scope } => remove(&files, &rule, only(scope), out),
        PermissionsCommand::Check { tool, input } => check(&files, &place, &tool, &input, out),
    }
}

/// The one file named, if any.
fn only(s: PermissionScope) -> Option<Scope> {
    match (s.project, s.user) {
        (_, true) => Some(Scope::User),
        (true, false) => Some(Scope::Project),
        (false, false) => None,
    }
}

fn parse(rule: &str) -> Result<Rule, Error> {
    Rule::parse(rule).map_err(Error::Usage)
}

fn list(files: &Files, scope: Option<Scope>, out: Out) -> Result<(), Error> {
    let entries: Vec<_> = store::entries(files)
        .map_err(Error::Failed)?
        .into_iter()
        .filter(|e| scope.is_none_or(|s| s == e.scope))
        .collect();
    if out.json {
        let rows: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "rule": e.rule.to_string(),
                    "kind": e.kind.key(),
                    "scope": e.scope.label(),
                    "file": files.path(e.scope).display().to_string(),
                })
            })
            .collect();
        Out::print_json(&json!({ "rules": rows }));
    } else if entries.is_empty() {
        Out::note("no rules yet; Io asks you before it runs a command");
    } else {
        let rows: Vec<Vec<String>> = entries
            .iter()
            .map(|e| {
                vec![
                    e.kind.key().to_owned(),
                    e.rule.to_string(),
                    e.scope.label().to_owned(),
                ]
            })
            .collect();
        Out::list(
            &["KIND", "RULE", "FILE"],
            &rows,
            &Listed::new("rule", "rules"),
        );
    }
    Ok(())
}

fn add(
    files: &Files,
    place: &Place,
    kind: Kind,
    rule: &str,
    scope: PermissionScope,
    out: Out,
) -> Result<(), Error> {
    let rule = parse(rule)?;
    let scope = only(scope).unwrap_or(Scope::Project);
    let added = store::add(files, scope, kind, &rule, place).map_err(Error::Usage)?;
    let file = files.path(scope);
    if out.json {
        Out::print_json(&json!({
            "rule": rule.to_string(),
            "kind": kind.key(),
            "scope": scope.label(),
            "file": file.display().to_string(),
            "added": added,
        }));
    } else if added {
        Out::note(&format!(
            "{} {rule} in the {} file ({})",
            match kind {
                Kind::Allow => "allowed",
                Kind::Deny => "denied",
            },
            scope.label(),
            shown(&file, &place.cwd)
        ));
    } else {
        Out::note(&format!("{rule} is already in the {} file", scope.label()));
    }
    Ok(())
}

fn remove(files: &Files, rule: &str, scope: Option<Scope>, out: Out) -> Result<(), Error> {
    let rule = parse(rule)?;
    let gone = store::remove(files, scope, &rule).map_err(Error::Failed)?;
    if out.json {
        Out::print_json(&json!({"rule": rule.to_string(), "removed": gone}));
    } else if gone == 0 {
        Out::note(&format!("{rule} was not in a rule file"));
    } else {
        Out::note(&format!("removed {rule}"));
    }
    Ok(())
}

/// The call a tool and one argument stand for.
fn call_of(tool: &str, input: &str) -> Value {
    match tool {
        "Bash" => json!({"command": input}),
        "WebFetch" => json!({"url": input}),
        "Glob" => json!({"pattern": input}),
        "Grep" => json!({"path": input}),
        _ => json!({"file_path": input}),
    }
}

fn check(files: &Files, place: &Place, tool: &str, input: &str, out: Out) -> Result<(), Error> {
    let rules = store::rule_set(files).map_err(Error::Failed)?;
    let verdict = decide(&rules, tool, &call_of(tool, input), place);
    let (word, rule) = match &verdict {
        Verdict::Allow(r) => ("allow", Some(r.as_str())),
        Verdict::Deny(r) => ("deny", Some(r.as_str())),
        Verdict::Ask => ("ask", None),
    };
    if out.json {
        Out::print_json(&json!({"tool": tool, "decision": word, "rule": rule}));
    } else {
        Out::raw(
            match rule {
                Some(r) => format!("{word}  {r}\n"),
                None => format!("{word}  no rule covers it; you would be asked\n"),
            }
            .as_bytes(),
        );
    }
    Ok(())
}

/// A path under the folder as a relative one, for a note.
fn shown(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}
