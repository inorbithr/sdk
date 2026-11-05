//! A project's memory over `iohr.memory.v1.MemoryService` (RFC 0002): the request bodies, the
//! source tag that scopes an entry to a project, and the filter that shows only that project's
//! entries.
//!
//! The service keeps one memory per person and does not filter by source, so the filter runs
//! here, after the answer. Nothing is stored on this machine.

use serde_json::{Value, json};

/// What an entry's source reads for a project: `project:<slug>`.
#[must_use]
pub fn source_tag(slug: &str) -> String {
    format!("project:{slug}")
}

/// The source of an entry made outside any project.
pub const SOURCE_CLI: &str = "cli";

/// The longest entry the CLI sends, in characters. Entries are short facts, not documents.
pub const MAX_TEXT: usize = 2000;

/// The body of `POST /v1/memory/entries`.
#[must_use]
pub fn remember_body(text: &str, preference: bool, source: &str) -> Value {
    json!({
        "kind": if preference { "KIND_PREFERENCE" } else { "KIND_FACT" },
        "text": text,
        "source": source,
    })
}

/// The body of `POST /v1/memory/recall`.
#[must_use]
pub fn recall_body(query: &str, k: u32) -> Value {
    json!({ "query": query, "k": k })
}

/// How many entries to ask the service for when `want` are wanted after the filter: more,
/// because the service does not filter by source and the others' entries take places.
#[must_use]
pub fn ask_for(want: u32, filtering: bool) -> u32 {
    if filtering {
        want.saturating_mul(5).clamp(want, 100)
    } else {
        want
    }
}

/// The entries whose source is `source`, in their order; all of them when `source` is `None`.
#[must_use]
pub fn keep(entries: Vec<Value>, source: Option<&str>) -> Vec<Value> {
    let Some(source) = source else {
        return entries;
    };
    entries
        .into_iter()
        .filter(|e| e.get("source").and_then(Value::as_str) == Some(source))
        .collect()
}

/// An entry's kind as a person reads it.
#[must_use]
pub fn kind_name(kind: &str) -> &'static str {
    match kind {
        "KIND_PREFERENCE" => "preference",
        "KIND_EPISODE" => "episode",
        _ => "fact",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, source: &str) -> Value {
        json!({ "id": id, "kind": "KIND_FACT", "text": "t", "source": source })
    }

    #[test]
    fn the_tag_is_project_and_slug() {
        assert_eq!(source_tag("billing-api"), "project:billing-api");
    }

    #[test]
    fn remember_sends_kind_text_and_the_project_source() {
        let b = remember_body("use sqlx", false, &source_tag("x"));
        assert_eq!(b["kind"], "KIND_FACT");
        assert_eq!(b["text"], "use sqlx");
        assert_eq!(b["source"], "project:x");
        assert_eq!(remember_body("a", true, "cli")["kind"], "KIND_PREFERENCE");
    }

    #[test]
    fn recall_sends_the_query_and_k() {
        let b = recall_body("timeouts", 25);
        assert_eq!(b["query"], "timeouts");
        assert_eq!(b["k"], 25);
    }

    #[test]
    fn the_filter_keeps_only_the_projects_entries_in_order() {
        let all = vec![
            entry("a", "project:x"),
            entry("b", "workbench"),
            entry("c", "project:y"),
            entry("d", "project:x"),
        ];
        let ids = |v: Vec<Value>| {
            v.iter()
                .map(|e| e["id"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(keep(all.clone(), Some("project:x"))), ["a", "d"]);
        assert_eq!(ids(keep(all.clone(), None)).len(), 4);
        assert_eq!(keep(all, Some("project:z")).len(), 0);
    }

    #[test]
    fn a_filtered_recall_asks_for_more_than_it_shows() {
        assert_eq!(ask_for(10, false), 10);
        assert_eq!(ask_for(10, true), 50);
        assert_eq!(ask_for(60, true), 100);
    }

    #[test]
    fn kinds_read_plainly() {
        assert_eq!(kind_name("KIND_PREFERENCE"), "preference");
        assert_eq!(kind_name("KIND_FACT"), "fact");
    }
}
