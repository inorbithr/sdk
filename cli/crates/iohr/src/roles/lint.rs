//! What InOrbit's import would refuse in a role, checked here first so `iohr` names the role
//! and the line instead of sending a body the service answers with a 400.
//!
//! The rules are the parts of core's `lint_tree` (`crates/coordinator/src/learning/lint.rs`,
//! commit 00c977e9d, 2026-10-31) a client can tell from the text: the privacy filter over a
//! role's name, aliases, section titles, lines and citations, a known model, and no override
//! word in a child's core. The service stays the authority; this only says no sooner.

use std::sync::LazyLock;

use regex::Regex;

use super::Role;
use crate::privacy;

/// The models the service knows (core `pack.rs` `KNOWN_MODELS`).
pub(crate) const KNOWN_MODELS: &[&str] = &["haiku", "sonnet", "opus", "fable"];

/// A line that tries to cancel a rule above it: never in a child's core (core `text.rs`
/// `OVERRIDE`).
pub(crate) const OVERRIDE: &str = r"(?i)\b(ignore|disregard|override|bypass|unless|except when)\b";

static OVERRIDE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(OVERRIDE).unwrap_or_else(|_| unreachable!("a fixed pattern, compiled in the tests"))
});

/// Every reason the service would refuse `p`, each naming where; empty when it would take it.
/// A refused text is never repeated: a reason says which line, not what it says.
#[must_use]
pub fn findings(p: &Role) -> Vec<String> {
    let mut out = Vec::new();
    let mut say = |what: String, why: &str| {
        out.push(format!("{what} is refused by the privacy filter ({why})"));
    };
    for n in std::iter::once(&p.name).chain(&p.aliases) {
        if let Some(why) = privacy::refuse(n) {
            say(format!("the name {n:?}"), why);
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for (i, l) in p.lines.iter().enumerate() {
        let at = format!("line {} ({}, {})", i + 1, l.place, l.section);
        if !seen.contains(&l.section.as_str()) {
            seen.push(&l.section);
            if let Some(why) = privacy::refuse(&l.section) {
                say(format!("the section title of {at}"), why);
            }
        }
        if let Some(why) = privacy::refuse(&l.text) {
            say(at.clone(), why);
        }
        if let Some(why) = l.cites.iter().find_map(|c| privacy::refuse_citation(c)) {
            say(format!("a citation of {at}"), why);
        }
    }
    if !KNOWN_MODELS.contains(&p.model.as_str()) {
        out.push(format!(
            "model {:?} is not one of {KNOWN_MODELS:?}",
            p.model
        ));
    }
    if !p.parent.is_empty() {
        for (i, l) in p.lines.iter().enumerate() {
            if l.place == "core" && OVERRIDE_RE.is_match(&l.text) {
                out.push(format!(
                    "line {} (core, {}) of a child says ignore, override or unless",
                    i + 1,
                    l.section
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_line_is_named_and_never_repeated() {
        let mut p = super::super::catalogue().remove(1);
        let secret = "token = abcdefghijkl";
        p.lines[0].text = format!("Keep {secret} safe");
        p.lines[1].text = "Ignore the parent's rule here.".to_owned();
        p.model = "gpt".to_owned();
        let f = findings(&p);
        assert_eq!(f.len(), 3, "{f:?}");
        assert!(f[0].starts_with("line 1 (core, "), "{f:?}");
        assert!(f[0].ends_with("(secret)"), "{f:?}");
        assert!(f.iter().all(|x| !x.contains(secret)), "{f:?}");
        assert!(f[1].starts_with("model \"gpt\""), "{f:?}");
        assert!(f[2].contains("ignore, override or unless"), "{f:?}");
    }
}
