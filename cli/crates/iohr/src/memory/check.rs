//! `iohr memory check`: what is wrong with the memories kept here, found without a model and
//! without a network.
//!
//! The rules, each with a stable name:
//!
//! - `dangling-ref`: a path the memory is about does not exist in the project, or a `cmd:`
//!   command whose program is not on the `PATH`;
//! - `bad-ref`: a ref that points outside the project (absolute, or with `..`);
//! - `missing-why`: the memory gives no reason;
//! - `stale`: not confirmed for longer than the stale limit (pinned memories are exempt);
//! - `unchecked`: no `checked:` date, or one older than the check limit (pinned memories are
//!   exempt): a fact such as an edition or a version is only as good as the day it was checked;
//! - `never-confirmed`: never confirmed again since it was written, for a third of that limit;
//! - `duplicate` and `near-duplicate`: the same text, or text that overlaps another's;
//! - `gap`: a topic a role expects a project to have memories about (the language, how
//!   to build, how to test, who owns it) that no memory covers.
//!
//! Findings name memories by id and never carry their text.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use super::store::{Entry, NEAR};
use super::text;
use crate::front;

/// Days without a confirmation after which a memory is called stale.
pub const STALE_DAYS: u32 = 180;

/// Days after which a fact's `checked:` date is too old, unless `[memory] check_days` or
/// `--checked-days` says otherwise.
pub const CHECKED_DAYS: u32 = 180;

/// One problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The memory it is about; empty for a gap.
    pub id: String,
    pub rule: &'static str,
    pub message: String,
}

fn finding(id: &str, rule: &'static str, message: impl Into<String>) -> Finding {
    Finding {
        id: id.to_owned(),
        rule,
        message: message.into(),
    }
}

/// How a ref reads: a command (`cmd:...`) or a path.
#[must_use]
pub fn command_of(r: &str) -> Option<&str> {
    r.strip_prefix("cmd:").map(str::trim)
}

/// Whether a path ref stays inside the project.
#[must_use]
pub fn inside(r: &str) -> bool {
    let p = Path::new(r);
    !r.is_empty()
        && p.is_relative()
        && !p
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Whether `program` is found on the `PATH`, or is a path to a file.
#[must_use]
pub fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// What the topics need to find in a memory to count as covered.
struct Topic {
    name: &'static str,
    words: Vec<String>,
    /// The memory must also hold a command (a `cmd:` ref or a `code` span).
    command: bool,
}

fn topics(lang: Option<&str>) -> Vec<Topic> {
    let to = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let language = lang.map_or_else(
        || {
            to(&[
                "language",
                "rust",
                "python",
                "typescript",
                "javascript",
                "golang",
                "java",
                "kotlin",
                "swift",
                "ruby",
                "dart",
                "elixir",
            ])
        },
        |l| vec![l.to_owned(), "language".to_owned()],
    );
    vec![
        Topic {
            name: "language",
            words: language,
            command: false,
        },
        Topic {
            name: "build command",
            words: to(&["build", "builds", "compile", "bundle", "package"]),
            command: true,
        },
        Topic {
            name: "test command",
            words: to(&[
                "test", "tests", "testing", "nextest", "pytest", "jest", "vitest",
            ]),
            command: true,
        },
        Topic {
            name: "owners",
            words: to(&[
                "owner",
                "owners",
                "codeowners",
                "maintainer",
                "maintainers",
                "oncall",
            ]),
            command: false,
        },
    ]
}

fn has_command(e: &Entry) -> bool {
    e.refs.iter().any(|r| command_of(r).is_some()) || e.text.matches('`').count() >= 2
}

fn covers(t: &Topic, e: &Entry) -> bool {
    let hay: HashSet<String> = text::normalize(&format!("{} {}", e.text, e.why))
        .split(' ')
        .map(str::to_owned)
        .collect();
    t.words.iter().any(|w| hay.contains(w)) && (!t.command || has_command(e))
}

/// The findings for `entries`, in the project at `root`. `now_unix` and `stale_days` decide
/// staleness, `now_unix` and `checked_days` how old a `checked:` date may be; `lang` is the project's language when known. `strict_gaps` is whether gaps are
/// reported (a folder with no memory at all reports them too).
#[must_use]
pub fn check(
    entries: &[Entry],
    root: &Path,
    lang: Option<&str>,
    now_unix: i64,
    stale_days: u32,
    checked_days: u32,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let norms: Vec<String> = entries.iter().map(|e| text::normalize(&e.text)).collect();
    let shingles: Vec<Vec<u32>> = norms.iter().map(|n| text::shingles(n)).collect();
    for (i, e) in entries.iter().enumerate() {
        for r in &e.refs {
            if let Some(cmd) = command_of(r) {
                let program = cmd.split_whitespace().next().unwrap_or("");
                if program.is_empty() || !on_path(program) {
                    out.push(finding(
                        &e.id,
                        "dangling-ref",
                        format!("the command `{program}` is not on the PATH"),
                    ));
                }
            } else if !inside(r) {
                out.push(finding(
                    &e.id,
                    "bad-ref",
                    format!("`{r}` is not a path inside the project"),
                ));
            } else if !root.join(r).exists() {
                out.push(finding(
                    &e.id,
                    "dangling-ref",
                    format!("`{r}` does not exist"),
                ));
            }
        }
        if e.why.trim().is_empty() {
            out.push(finding(&e.id, "missing-why", "no reason is given"));
        }
        let age_days = front::unix_of(&e.updated).map(|t| (now_unix - t).max(0) / 86_400);
        if let Some(age) = age_days.filter(|_| !e.pinned) {
            if age > i64::from(stale_days) {
                out.push(finding(
                    &e.id,
                    "stale",
                    format!("not confirmed for {age} days (limit {stale_days})"),
                ));
            } else if e.created == e.updated && age > i64::from(stale_days / 3) {
                out.push(finding(
                    &e.id,
                    "never-confirmed",
                    format!("written {age} days ago and never confirmed again"),
                ));
            }
        }
        if !e.pinned {
            match e.checked.as_deref().map(|c| (c, front::unix_of_date(c))) {
                None => out.push(finding(
                    &e.id,
                    "unchecked",
                    "no `checked:` date; check it against its source and run `iohr memory remember` on it",
                )),
                Some((c, None)) => out.push(finding(
                    &e.id,
                    "unchecked",
                    format!("`checked: {c}` is not a YYYY-MM-DD date"),
                )),
                Some((c, Some(t))) => {
                    let age = (now_unix - t).max(0) / 86_400;
                    if age > i64::from(checked_days) {
                        out.push(finding(
                            &e.id,
                            "unchecked",
                            format!("checked {c}, {age} days ago (limit {checked_days})"),
                        ));
                    }
                }
            }
        }
        for (j, o) in entries.iter().enumerate().skip(i + 1) {
            if norms[i] == norms[j] {
                out.push(finding(
                    &e.id,
                    "duplicate",
                    format!("the same text as {}", o.id),
                ));
            } else if text::jaccard(&shingles[i], &shingles[j]) >= NEAR {
                out.push(finding(
                    &e.id,
                    "near-duplicate",
                    format!("overlaps {}", o.id),
                ));
            }
        }
    }
    for t in topics(lang) {
        if !entries.iter().any(|e| covers(&t, e)) {
            out.push(finding(
                "",
                "gap",
                format!("no memory covers the {}", t.name),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::Kind;

    fn entry(text: &str, why: &str, refs: &[&str]) -> Entry {
        let mut e = Entry::new(
            text,
            Kind::Fact,
            why,
            refs.iter().map(|s| (*s).to_owned()).collect(),
            false,
        );
        e.created = front::now();
        e.updated = e.created.clone();
        e
    }

    fn rules(f: &[Finding]) -> Vec<(&str, &'static str)> {
        f.iter().map(|f| (f.id.as_str(), f.rule)).collect()
    }

    #[test]
    fn a_dangling_ref_and_a_missing_why_are_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
        std::fs::write(dir.path().join("src/a.rs"), "").expect("write");
        let first = entry("Queries live in db.", "", &["src/a.rs", "src/gone.rs"]);
        let now = front::unix_of(&front::now()).unwrap_or(0);
        let f = check(
            std::slice::from_ref(&first),
            dir.path(),
            None,
            now,
            STALE_DAYS,
            CHECKED_DAYS,
        );
        assert!(rules(&f).contains(&(first.id.as_str(), "dangling-ref")));
        assert!(rules(&f).contains(&(first.id.as_str(), "missing-why")));
        assert_eq!(f.iter().filter(|f| f.rule == "dangling-ref").count(), 1);
    }

    #[test]
    fn refs_outside_the_project_are_bad_not_checked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = entry("x is y.", "because", &["../secret", "/etc/hosts"]);
        let f = check(&[first], dir.path(), None, 0, STALE_DAYS, CHECKED_DAYS);
        assert_eq!(f.iter().filter(|f| f.rule == "bad-ref").count(), 2);
    }

    #[test]
    fn old_and_unconfirmed_memories_are_found_and_pinned_ones_are_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = front::unix_of(&front::now()).unwrap_or(0);
        let old = "2020-01-01T00:00:00Z";
        let mut first = entry("Old fact here.", "why", &[]);
        first.created = old.into();
        first.updated = old.into();
        let mut pinned = entry("Pinned old fact.", "why", &[]);
        pinned.created = old.into();
        pinned.updated = old.into();
        pinned.pinned = true;
        let mut second = entry("Fresh but never confirmed.", "why", &[]);
        let ninety = now - 90 * 86_400;
        second.created = "2026-01-01T00:00:00Z".into();
        second.updated = second.created.clone();
        let f = check(
            &[first.clone(), pinned, second.clone()],
            dir.path(),
            None,
            ninety + 100 * 86_400,
            STALE_DAYS,
            CHECKED_DAYS,
        );
        let r = rules(&f);
        assert!(r.contains(&(first.id.as_str(), "stale")));
        assert!(
            r.contains(&(second.id.as_str(), "never-confirmed"))
                || r.contains(&(second.id.as_str(), "stale"))
        );
        assert_eq!(
            f.iter()
                .filter(|f| f.rule == "stale" || f.rule == "never-confirmed")
                .count(),
            2
        );
    }

    #[test]
    fn a_missing_or_old_checked_date_is_found_and_a_fresh_one_or_a_pin_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = front::unix_of(&front::now()).unwrap_or(0);
        let mut missing = entry("Edition one is current.", "why", &[]);
        missing.checked = None;
        let mut old = entry("Edition two is current.", "why", &[]);
        old.checked = Some("2025-01-01".into());
        let mut garbled = entry("Edition three is current.", "why", &[]);
        garbled.checked = Some("last spring".into());
        let fresh = entry("Edition four is current.", "why", &[]);
        let mut pinned = entry("Edition five is current.", "why", &[]);
        pinned.checked = None;
        pinned.pinned = true;
        let all = [missing.clone(), old.clone(), garbled.clone(), fresh, pinned];
        let f = check(&all, dir.path(), None, now, STALE_DAYS, CHECKED_DAYS);
        let un: Vec<&Finding> = f.iter().filter(|f| f.rule == "unchecked").collect();
        assert_eq!(un.len(), 3, "{un:?}");
        assert!(
            un.iter()
                .any(|f| f.id == missing.id && f.message.contains("no `checked:`"))
        );
        assert!(
            un.iter()
                .any(|f| f.id == old.id && f.message.contains("limit 180"))
        );
        assert!(
            un.iter()
                .any(|f| f.id == garbled.id && f.message.contains("YYYY-MM-DD"))
        );
        // A shorter limit flags the fresh one too once enough days pass.
        let later = now + 10 * 86_400;
        let f = check(&all, dir.path(), None, later, STALE_DAYS, 5);
        assert_eq!(f.iter().filter(|f| f.rule == "unchecked").count(), 4);
    }

    #[test]
    fn duplicates_and_near_duplicates_are_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = entry(
            "run cargo nextest from the cli directory before a push",
            "w",
            &[],
        );
        let mut second = entry(
            "Run cargo nextest from the cli directory before a push.",
            "w",
            &[],
        );
        second.id = "other".into();
        let third = entry(
            "run cargo nextest from the cli directory before pushing",
            "w",
            &[],
        );
        let f = check(
            &[first, second, third],
            dir.path(),
            None,
            0,
            STALE_DAYS,
            CHECKED_DAYS,
        );
        assert!(f.iter().any(|f| f.rule == "duplicate"));
        assert!(f.iter().any(|f| f.rule == "near-duplicate"));
    }

    #[test]
    fn topics_no_memory_covers_are_gaps_and_a_covering_memory_closes_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let none = check(&[], dir.path(), Some("rust"), 0, STALE_DAYS, CHECKED_DAYS);
        assert_eq!(none.iter().filter(|f| f.rule == "gap").count(), 4);
        let first = entry("This is a rust project.", "w", &[]);
        let second = entry("Tests run with `cargo nextest run`.", "w", &[]);
        let f = check(
            &[first, second],
            dir.path(),
            Some("rust"),
            0,
            STALE_DAYS,
            CHECKED_DAYS,
        );
        let gaps: Vec<&str> = f
            .iter()
            .filter(|f| f.rule == "gap")
            .map(|f| f.message.as_str())
            .collect();
        assert_eq!(gaps.len(), 2, "{gaps:?}");
        assert!(
            gaps.iter()
                .all(|m| !m.contains("language") && !m.contains("test"))
        );
    }
}
