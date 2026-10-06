//! The redaction rules: what a public document may not carry, one line at a time.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::bytes::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::Error;

/// A rule as `rules.json` and a config's `patterns` write it.
#[derive(Debug, Clone, Deserialize)]
pub struct Pattern {
    /// The rule id a hit reports.
    pub id: String,
    /// The regular expression, in the subset JavaScript and this crate read alike.
    pub re: String,
    /// What a hit means, for the reader.
    pub why: String,
    /// Whether case is ignored.
    #[serde(default)]
    pub ignore_case: bool,
}

/// A lab's own config, `docs/lab/redaction.json`. Unknown keys (a `note`) are ignored.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    /// Any name under one of these, but `www.`, is an internal host.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Names no rule can infer, matched as whole words in any case.
    #[serde(default)]
    pub words: Vec<String>,
    /// Rules of the lab's own; one with a generic rule's id joins it.
    #[serde(default)]
    pub patterns: Vec<Pattern>,
    /// When present, a document's `lab` must be one of these.
    #[serde(default)]
    pub labs: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RulesFile {
    rules: Vec<Pattern>,
}

/// The rules of a rules file (`{"rules": [...]}`, the shape of `rules.json` and of the
/// platform's `docs/lab/strict.json`); other keys, such as a `note`, are ignored.
///
/// # Errors
///
/// [`Error::Json`] when `text` is not a rules file.
pub fn parse_rules(text: &str) -> Result<Vec<Pattern>, Error> {
    Ok(serde_json::from_str::<RulesFile>(text)?.rules)
}

/// One hit: the line (from 1), the rule, the text that matched and why it is refused.
/// A strict rule's hit ([`Redaction::with_strict`]) carries no text: what it matched
/// is never printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The line, from 1.
    pub line: usize,
    /// The rule id.
    pub rule: String,
    /// The text that matched.
    pub text: String,
    /// Why it is refused.
    pub why: String,
}

#[derive(Debug)]
struct Rule {
    id: String,
    re: Regex,
    why: String,
    /// A strict rule: its hits never carry what matched.
    quiet: bool,
}

/// The compiled rules for one lab: the generic ones and its config.
#[derive(Debug)]
pub struct Redaction {
    rules: Vec<Rule>,
    domains: Vec<Regex>,
    labs: Option<Vec<String>>,
}

static FENCE: LazyLock<Regex> = LazyLock::new(|| compile(r"^\s*```", false));
static MARKER: LazyLock<Regex> = LazyLock::new(|| compile(r"\[REDACTED:\s*([^\]]*)\]", false));

/// A rule's pattern with JavaScript's meaning: `\b`, `\d` and `\s` over ASCII.
fn build(re: &str, ignore_case: bool) -> Result<Regex, regex::Error> {
    RegexBuilder::new(re)
        .unicode(false)
        .case_insensitive(ignore_case)
        .build()
}

#[allow(clippy::expect_used)]
fn compile(re: &str, ignore_case: bool) -> Regex {
    build(re, ignore_case).expect("a pattern of this crate's own compiles")
}

/// The rules compiled, in order: a pattern with an earlier one's id and case joins it,
/// so a line still gives one hit per rule; the first of an id keeps its place.
fn merge<'a>(patterns: impl Iterator<Item = &'a Pattern>, quiet: bool) -> Result<Vec<Rule>, Error> {
    let mut order: Vec<(String, bool)> = Vec::new();
    let mut merged: BTreeMap<(String, bool), Pattern> = BTreeMap::new();
    for p in patterns {
        let key = (p.id.clone(), p.ignore_case);
        if let Some(prev) = merged.get_mut(&key) {
            prev.re = format!("(?:{})|(?:{})", prev.re, p.re);
        } else {
            order.push(key.clone());
            merged.insert(key, p.clone());
        }
    }
    let mut out = Vec::new();
    for key in order {
        let Some(p) = merged.remove(&key) else {
            continue;
        };
        let re = build(&p.re, p.ignore_case).map_err(|source| Error::Pattern {
            id: p.id.clone(),
            source,
        })?;
        out.push(Rule {
            id: p.id,
            re,
            why: p.why,
            quiet,
        });
    }
    Ok(out)
}

impl Redaction {
    /// The rules from a `rules.json` text and a lab's config.
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when `rules` is not a rules file, [`Error::Pattern`] when a
    /// pattern does not compile.
    pub fn new(rules: &str, config: &Config) -> Result<Self, Error> {
        let generic = parse_rules(rules)?;
        let mut out = merge(generic.iter().chain(&config.patterns), false)?;
        for w in &config.words {
            let re = build(&format!(r"\b{}\b", regex::escape(w)), true).map_err(|source| {
                Error::Pattern {
                    id: "denylist".into(),
                    source,
                }
            })?;
            out.push(Rule {
                id: "denylist".into(),
                re,
                why: format!("\"{w}\" names a piece of the deployment"),
                quiet: false,
            });
        }
        let domains = config
            .domains
            .iter()
            .map(|d| {
                build(&format!(r"\b([a-z0-9-]+)\.{}\b", regex::escape(d)), true).map_err(|source| {
                    Error::Pattern {
                        id: "domain".into(),
                        source,
                    }
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            rules: out,
            domains,
            labs: config.labs.clone(),
        })
    }

    /// These rules added as strict ones (the platform's `docs/lab/strict.json`): checked
    /// like the others, after them, and their hits never carry what matched. Rules with
    /// one id and case join into one, as in [`Redaction::new`].
    ///
    /// # Errors
    ///
    /// [`Error::Pattern`] when a pattern does not compile.
    pub fn with_strict(mut self, rules: &[Pattern]) -> Result<Self, Error> {
        self.rules.extend(merge(rules.iter(), true)?);
        Ok(self)
    }

    /// The labs a document may name, when the config lists them.
    #[must_use]
    pub fn labs(&self) -> Option<&[String]> {
        self.labs.as_deref()
    }

    /// Every hit in `lines`. A fence line itself is skipped; a `redacted` block's body
    /// (the reason) is read like any other line, and so is a marker's reason.
    #[must_use]
    pub fn scan(&self, lines: &[&str]) -> Vec<Hit> {
        let mut hits = Vec::new();
        for (i, raw) in lines.iter().enumerate() {
            if FENCE.is_match(raw.as_bytes()) {
                continue;
            }
            let line = MARKER.replace_all(raw.as_bytes(), &b"$1"[..]);
            // A name under one of the lab's domains other than `www.`: one rule, so the
            // leftmost such name is the line's one hit.
            let mut first: Option<(usize, &[u8])> = None;
            for re in &self.domains {
                for c in re.captures_iter(&line) {
                    let (Some(all), Some(label)) = (c.get(0), c.get(1)) else {
                        continue;
                    };
                    if label.as_bytes().eq_ignore_ascii_case(b"www") {
                        continue;
                    }
                    if first.is_none_or(|(at, _)| all.start() < at) {
                        first = Some((all.start(), all.as_bytes()));
                    }
                    break;
                }
            }
            if let Some((_, text)) = first {
                hits.push(Hit {
                    line: i + 1,
                    rule: "domain".into(),
                    text: String::from_utf8_lossy(text).into_owned(),
                    why: "an internal host name".into(),
                });
            }
            for rule in &self.rules {
                if let Some(m) = rule.re.find(&line) {
                    hits.push(Hit {
                        line: i + 1,
                        rule: rule.id.clone(),
                        text: if rule.quiet {
                            String::new()
                        } else {
                            String::from_utf8_lossy(m.as_bytes()).trim().to_owned()
                        },
                        why: rule.why.clone(),
                    });
                }
            }
        }
        hits
    }
}
