//! The document checks: file name, front matter, status log, duplicate numbers, and
//! redaction on public documents read as an uncleared reader sees them (classified spans
//! cut), and the classified markers themselves on every document. The rule ids and the messages are the site's own.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::classified::{AccessLevel, cut, parse_classified};
use crate::redaction::{Hit, Redaction};

/// What a document is, by the folder it sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A decision, in `rfcs/` (or any folder but `studies/`).
    Rfc,
    /// A measurement, in `studies/`.
    Study,
}

impl Kind {
    /// `studies` holds studies; any other folder holds RFCs.
    #[must_use]
    pub fn of_folder(name: &str) -> Self {
        if name == "studies" {
            Self::Study
        } else {
            Self::Rfc
        }
    }

    fn statuses(self) -> &'static [&'static str] {
        match self {
            Self::Rfc => &["open", "decided", "superseded"],
            Self::Study => &["running", "measured", "published"],
        }
    }
}

/// One problem: where (a line from 1, or the whole file), which rule, what to fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The line, from 1; `None` for the whole file.
    pub line: Option<usize>,
    /// The rule id, stable across the site and this command.
    pub rule: String,
    /// What is wrong, for the reader.
    pub message: String,
}

impl Finding {
    fn new(line: Option<usize>, rule: &str, message: impl Into<String>) -> Self {
        Self {
            line,
            rule: rule.into(),
            message: message.into(),
        }
    }
}

const REQUIRED: [&str; 6] = ["title", "status", "date", "public", "summary", "lab"];
const OPTIONAL: [&str; 7] = [
    "supersedes",
    "rfc",
    "headline",
    "headline_note",
    "audience",
    // The pre-publication review (the platform's docs/lab/review-checklist.md): the
    // day it was done and who did it.
    "reviewed",
    "reviewer",
];

#[allow(clippy::expect_used)]
fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a pattern of this crate's own compiles")
}

static FILE_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d{4})-([a-z0-9-]+)\.md$"));
static KEY_VALUE: LazyLock<Regex> = LazyLock::new(|| re(r"^([a-z_]+):\s*(.*)$"));
static DATE: LazyLock<Regex> = LazyLock::new(|| re(r"^\d{4}-\d{2}-\d{2}$"));
static LOG_HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^##\s+Status log\s*$"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^#{1,2}\s"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| re(r"^- (\d{4}-\d{2}-\d{2}):\s*(.*)$"));
static RUN_ON: LazyLock<Regex> = LazyLock::new(|| re(r"^\s+\S"));

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Bool(bool),
}

/// The four-digit number of a well-named file.
fn number(name: &str) -> Option<&str> {
    FILE_NAME
        .captures(name)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
}

/// Every finding for one document named `name`, of `kind`, against `redaction`.
#[must_use]
pub fn check_document(name: &str, text: &str, kind: Kind, redaction: &Redaction) -> Vec<Finding> {
    let mut out = Vec::new();
    if number(name).is_none() {
        out.push(Finding::new(
            None,
            "file-name",
            "the file name must be NNNN-slug.md",
        ));
        return out;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        out.push(Finding::new(
            Some(1),
            "front-matter",
            "the file must start with a `---` front matter block",
        ));
        return out;
    }
    let Some((front, close, key_lines)) = front_matter(&lines, &mut out) else {
        return out;
    };
    let i = close;
    let body_start = i + 2;
    let body = &lines[i + 1..];

    for key in REQUIRED {
        if !front.contains_key(key) {
            out.push(Finding::new(
                None,
                "missing-key",
                format!("missing `{key}`"),
            ));
        }
    }
    if !matches!(front.get("public"), Some(Value::Bool(_))) {
        out.push(Finding::new(
            None,
            "public",
            "`public` must be true or false",
        ));
    }
    let text_of = |key: &str| match front.get(key) {
        Some(Value::Text(t)) => Some(t.as_str()),
        _ => None,
    };
    if !text_of("status").is_some_and(|s| kind.statuses().contains(&s)) {
        out.push(Finding::new(
            None,
            "status",
            format!("`status` must be one of {}", kind.statuses().join(", ")),
        ));
    }
    if !text_of("date").is_some_and(|d| DATE.is_match(d)) {
        out.push(Finding::new(None, "date", "`date` must be YYYY-MM-DD"));
    }
    if let Some(labs) = redaction.labs()
        && !text_of("lab").is_some_and(|l| labs.iter().any(|x| x == l))
    {
        out.push(Finding::new(
            None,
            "lab",
            format!("`lab` must be one of {}", labs.join(", ")),
        ));
    }
    if text_of("reviewed").is_some_and(|d| !DATE.is_match(d)) {
        out.push(Finding::new(
            key_lines.get("reviewed").copied(),
            "reviewed",
            "`reviewed` must be YYYY-MM-DD",
        ));
    }
    status_log(body, body_start, &mut out);

    // Classified spans, on every document: a marker that does not parse is a finding,
    // never a guess, and the front matter carries none (it is read before anything is
    // cut).
    let classified = parse_classified(text);
    for p in &classified.problems {
        out.push(Finding::new(Some(p.line), "classified", p.message.clone()));
    }
    for s in &classified.spans {
        if s.line < body_start {
            out.push(Finding::new(
                Some(s.line),
                "classified",
                "the front matter carries no classified span",
            ));
        }
    }

    // Redaction, for public documents only (a draft is read by its own people): front
    // matter values and the body, as a reader without clearance sees them. What a
    // classified span withholds is not checked; its reason is. A cut keeps the line
    // count, so the lines are the source's. When the markers do not parse, the text
    // is read as it is.
    if front.get("public") == Some(&Value::Bool(true)) {
        let seen = if classified.problems.is_empty() {
            cut(text, AccessLevel::Public).unwrap_or_else(|_| text.to_owned())
        } else {
            text.to_owned()
        };
        let seen: Vec<&str> = seen.split('\n').collect();
        let split = (body_start - 1).min(seen.len());
        for h in redaction.scan(&seen[..split]) {
            out.push(hit(&h, 0));
        }
        for h in redaction.scan(&seen[split..]) {
            out.push(hit(&h, body_start - 1));
        }
    }
    out
}

/// A redaction hit as a finding, its line moved by `shift`. A strict rule's hit has no
/// text and says only why.
fn hit(h: &Hit, shift: usize) -> Finding {
    let message = if h.text.is_empty() {
        format!("({})", h.why)
    } else {
        format!("\"{}\" ({})", h.text, h.why)
    };
    Finding::new(Some(h.line + shift), &h.rule, message)
}

/// The `key: value` lines between the two `---`: the values, the index of the closing
/// line, and the line (from 1) each key first appears on; `None` when it never closes.
#[allow(clippy::type_complexity)]
fn front_matter(
    lines: &[&str],
    out: &mut Vec<Finding>,
) -> Option<(BTreeMap<String, Value>, usize, BTreeMap<String, usize>)> {
    let mut front: BTreeMap<String, Value> = BTreeMap::new();
    let mut key_lines: BTreeMap<String, usize> = BTreeMap::new();
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i];
        if line.trim() == "---" {
            break;
        }
        let Some(c) = KEY_VALUE.captures(line) else {
            out.push(Finding::new(
                Some(i + 1),
                "front-matter",
                format!("not a `key: value` line: {line}"),
            ));
            i += 1;
            continue;
        };
        let key = c.get(1).map_or("", |m| m.as_str());
        let mut value = c.get(2).map_or("", |m| m.as_str()).trim();
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value = &value[1..value.len() - 1];
        }
        if !REQUIRED.contains(&key) && !OPTIONAL.contains(&key) {
            out.push(Finding::new(
                Some(i + 1),
                "unknown-key",
                format!("unknown key `{key}`"),
            ));
        }
        let value = match value {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            v => Value::Text(v.to_owned()),
        };
        front.insert(key.to_owned(), value);
        key_lines.entry(key.to_owned()).or_insert(i + 1);
        i += 1;
    }
    if i >= lines.len() {
        out.push(Finding::new(
            None,
            "front-matter",
            "the front matter never closes",
        ));
        return None;
    }
    Some((front, i, key_lines))
}

/// The `## Status log` bullets: each starts `- YYYY-MM-DD:`, its text may run on over
/// indented lines; any other bullet there is a finding.
fn status_log(body: &[&str], body_start: usize, out: &mut Vec<Finding>) {
    let Some(start) = body.iter().position(|l| LOG_HEADING.is_match(l)) else {
        return;
    };
    let mut bullets = 0usize;
    for (i, line) in body.iter().enumerate().skip(start + 1) {
        if HEADING.is_match(line) {
            break;
        }
        if BULLET.is_match(line) {
            bullets += 1;
        } else if RUN_ON.is_match(line) && bullets > 0 {
            // the previous bullet's text, running on
        } else if line.starts_with("- ") {
            out.push(Finding::new(
                Some(body_start + i),
                "status-log",
                "a status log bullet must start with `- YYYY-MM-DD:`",
            ));
        }
    }
}

/// The findings that need the whole folder: two files with one number. In name order,
/// the first keeps the number and each later one is the finding.
#[must_use]
pub fn check_folder<'a>(names: &[&'a str]) -> BTreeMap<&'a str, Vec<Finding>> {
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    let mut first: BTreeMap<&str, &str> = BTreeMap::new();
    let mut out: BTreeMap<&str, Vec<Finding>> = BTreeMap::new();
    for name in sorted {
        let Some(n) = number(name) else { continue };
        match first.get(n) {
            Some(earlier) => out.entry(name).or_default().push(Finding::new(
                None,
                "duplicate-number",
                format!("number {n} is already {earlier}"),
            )),
            None => {
                first.insert(n, name);
            }
        }
    }
    out
}
