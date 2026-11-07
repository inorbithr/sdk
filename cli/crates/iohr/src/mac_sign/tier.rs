//! Which asks this Mac may sign, and which stay on the phone (RFC 0113, ADR 0215).
//!
//! A key held by one Mac behind Touch ID is weaker than the phone's: a program running as the
//! same user can start the prompt. So what is hard to take back stays phone-only: a merge, a
//! roll, a migration, an auto-approve rule. The rule fails closed: a kind this file does not
//! name goes to the phone, and so does an ask whose title or words say one of those things.
//! Wrongly sending an ask to the phone costs a tap; wrongly signing one here costs more.
//!
//! The service does not enforce the tier yet (it cannot tell a Mac's key from a phone's);
//! this is the CLI's own refusal, and the same list is what the service should check.

use serde_json::Value;

/// What the Mac may do with an ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tier {
    /// Sign here after Touch ID.
    Mac,
    /// Phone only; the text says why, in a few words that follow "this is".
    Phone(&'static str),
}

/// The kinds a Mac may sign, besides the words below: a review, a document, a decision, a
/// command run. A pull request is a merge; a roll is a roll.
const MAC_KINDS: &[&str] = &["review", "document", "decision", "run"];

/// The ask's own text that says it is a merge, a roll, a migration or an auto-approve rule,
/// by the word's start (`roll` is also `rollback` and `rolling`; `scroll` does not start so).
const WORDS: &[(&str, &str)] = &[
    ("merg", "a merge"),
    ("roll", "a roll"),
    ("migrat", "a migration"),
    ("autoapprov", "an auto-approve rule"),
];

/// Where an ask goes.
pub(crate) fn classify(ask: &Value) -> Tier {
    let kind = text(ask, "kind");
    match kind {
        "pull_request" => return Tier::Phone("a merge"),
        "roll" => return Tier::Phone("a roll"),
        k if !MAC_KINDS.contains(&k) => return Tier::Phone("a kind this Mac does not sign"),
        _ => {}
    }
    let payload = ask.get("payload").unwrap_or(&Value::Null);
    if payload
        .get("migrations")
        .and_then(Value::as_array)
        .is_some_and(|m| !m.is_empty())
    {
        return Tier::Phone("a migration");
    }
    for field in ["title", "summary", "approve_means"] {
        if let Some(why) = said(text(ask, field)) {
            return Tier::Phone(why);
        }
    }
    for field in ["title", "kind", "name", "summary"] {
        if let Some(why) = said(text(payload, field)) {
            return Tier::Phone(why);
        }
    }
    for option in payload
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if let Some(why) = said(option) {
            return Tier::Phone(why);
        }
    }
    Tier::Mac
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The first sensitive thing `s` says: its words, lower-cased and cut at anything that is not a
/// letter or a digit; `auto-approve` is read as one word.
fn said(s: &str) -> Option<&'static str> {
    let lower = s.to_lowercase().replace("auto-approve", "autoapprove").replace("auto approve", "autoapprove");
    lower
        .split(|c: char| !c.is_alphanumeric())
        .find_map(|w| WORDS.iter().find(|(start, _)| w.starts_with(start)).map(|(_, why)| *why))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ask(kind: &str, title: &str) -> Value {
        json!({"kind": kind, "title": title, "summary": "", "payload": {}})
    }

    #[test]
    fn a_merge_a_roll_a_migration_and_an_auto_approve_rule_stay_on_the_phone() {
        assert_eq!(classify(&ask("pull_request", "Fix a typo")), Tier::Phone("a merge"));
        assert_eq!(classify(&ask("roll", "Roll protocol")), Tier::Phone("a roll"));
        let mut roll_like = ask("decision", "Ship it");
        roll_like["payload"] = json!({"migrations": ["20261101_x.sql"]});
        assert_eq!(classify(&roll_like), Tier::Phone("a migration"));
        assert_eq!(classify(&ask("decision", "Migrate the cases tables")), Tier::Phone("a migration"));
        assert_eq!(classify(&ask("decision", "Merge PR 12 into main")), Tier::Phone("a merge"));
        assert_eq!(classify(&ask("decision", "Roll back the edge")), Tier::Phone("a roll"));
        assert_eq!(
            classify(&ask("decision", "Turn on Auto-Approve for docs PRs")),
            Tier::Phone("an auto-approve rule")
        );
        assert_eq!(
            classify(&ask("review", "autoapprove")),
            Tier::Phone("an auto-approve rule")
        );
    }

    #[test]
    fn the_words_are_found_in_the_summary_the_means_and_the_options_too() {
        let mut a = ask("document", "Plan");
        a["summary"] = json!("then merge it");
        assert_eq!(classify(&a), Tier::Phone("a merge"));
        let mut b = ask("document", "Plan");
        b["approve_means"] = json!("rolls the edge");
        assert_eq!(classify(&b), Tier::Phone("a roll"));
        let mut c = ask("decision", "Pick");
        c["payload"] = json!({"options": ["Keep", "Migrate now"]});
        assert_eq!(classify(&c), Tier::Phone("a migration"));
    }

    #[test]
    fn a_kind_nobody_named_fails_closed() {
        assert!(matches!(classify(&ask("", "x")), Tier::Phone(_)));
        assert!(matches!(classify(&ask("runner_enrollment", "x")), Tier::Phone(_)));
        assert!(matches!(classify(&Value::Null), Tier::Phone(_)));
    }

    #[test]
    fn a_review_a_document_a_decision_and_a_run_may_be_signed_here() {
        for kind in ["review", "document", "decision", "run"] {
            assert_eq!(classify(&ask(kind, "Plan for the scroll bar")), Tier::Mac, "{kind}");
        }
    }
}
