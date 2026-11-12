//! A memory kept on InOrbit's memory service (`iohr.memory.v1.MemoryService`, RFC 0002): the
//! request bodies, the source tag that scopes an entry, and the plan that compares the local
//! files with what the service holds.
//!
//! The service is immutable: an entry has an id, a kind, a text, a source and a creation time,
//! and there is no update and no version. So the "base version" of a memory is kept here, in
//! the local file (`remote_id`, `remote_version` = the remote creation time, `remote_sha` = the
//! hash of the kind and text when it was last synced), and the check is made on the client:
//!
//! - local changed = the file's hash differs from `remote_sha`;
//! - remote changed = the remote entry's hash differs from `remote_sha`;
//! - remote gone = no remote entry has `remote_id`.
//!
//! Replacing a changed memory is create-then-delete, which is not atomic: the new id is written
//! to the file before the old entry is deleted, so an interrupted sync can leave a duplicate on
//! the service, never lose a memory. A conflict is reported and nothing is overwritten.
//!
//! The service does not filter by source, so the filter runs here, after the answer.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{Value, json};

use super::store::{Entry, Kind, Scope, sha_of};

/// What an entry's source reads for a project: `project:<slug>`.
#[must_use]
pub fn source_tag(slug: &str) -> String {
    format!("project:{slug}")
}

/// The source of an entry made outside any project.
pub const SOURCE_CLI: &str = "cli";

/// The source of a person's own (user scope) memories.
pub const SOURCE_USER: &str = "user";

/// The source tag for memories of `scope`; `slug` is the project's.
#[must_use]
pub fn source_for(scope: Scope, slug: &str) -> String {
    match scope {
        Scope::Project => source_tag(slug),
        Scope::User => SOURCE_USER.to_owned(),
        Scope::Account => SOURCE_CLI.to_owned(),
    }
}

/// The body of `POST /v1/memory/entries`.
#[must_use]
pub fn remember_body(kind: Kind, text: &str, source: &str) -> Value {
    json!({ "kind": kind.api(), "text": text, "source": source })
}

/// The body of `POST /v1/memory/recall`.
#[must_use]
pub fn recall_body(query: &str, k: u32) -> Value {
    json!({ "query": query, "k": k })
}

/// One entry as the service holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Remote {
    pub id: String,
    pub kind: Kind,
    pub text: String,
    pub source: String,
    pub created_at: String,
}

impl Remote {
    /// An entry from the service's JSON; `None` without an id.
    #[must_use]
    pub fn from_value(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let id = s("id");
        if id.is_empty() {
            return None;
        }
        Some(Self {
            id,
            kind: Kind::from_api(&s("kind")),
            text: s("text"),
            source: s("source"),
            created_at: s("created_at"),
        })
    }

    /// The hash of its kind and text, the same as a local file's.
    #[must_use]
    pub fn sha(&self) -> String {
        sha_of(self.kind, &self.text)
    }
}

/// What one sync step does, or finds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    /// A local memory the service does not have: create it.
    Push { id: String },
    /// A local memory the service already holds under another file: record the link only.
    Link { id: String, remote_id: String },
    /// The same on both sides.
    InSync { id: String },
    /// Changed here, unchanged there: create the new entry, then delete the old one.
    Replace { id: String, remote_id: String },
    /// Changed there, unchanged here: reported, never pulled over a file.
    RemoteNewer { id: String, remote_id: String },
    /// The service no longer has it, and the file is as it was synced: reported.
    RemoteGone { id: String },
    /// Both sides changed, or the service dropped a memory that changed here: stop.
    Conflict { id: String, why: String },
    /// On the service, in this scope's source, with no file here.
    RemoteOnly { remote_id: String },
}

impl Action {
    /// The name a person reads.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Push { .. } => "push",
            Self::Link { .. } => "link",
            Self::InSync { .. } => "in-sync",
            Self::Replace { .. } => "replace",
            Self::RemoteNewer { .. } => "remote-newer",
            Self::RemoteGone { .. } => "remote-gone",
            Self::Conflict { .. } => "conflict",
            Self::RemoteOnly { .. } => "remote-only",
        }
    }

    /// The memory (or remote entry) it is about.
    #[must_use]
    pub fn subject(&self) -> &str {
        match self {
            Self::Push { id }
            | Self::Link { id, .. }
            | Self::InSync { id }
            | Self::Replace { id, .. }
            | Self::RemoteNewer { id, .. }
            | Self::RemoteGone { id }
            | Self::Conflict { id, .. } => id,
            Self::RemoteOnly { remote_id } => remote_id,
        }
    }

    /// Whether the sync would change something on either side.
    #[must_use]
    pub fn changes(&self) -> bool {
        matches!(
            self,
            Self::Push { .. } | Self::Link { .. } | Self::Replace { .. }
        )
    }
}

/// Compare `local` with `remote` (already narrowed to this scope's source) one memory at a
/// time. Pure: it reads nothing and changes nothing.
#[must_use]
pub fn plan(local: &[Entry], remote: &[Remote]) -> Vec<Action> {
    let by_id: HashMap<&str, &Remote> = remote.iter().map(|r| (r.id.as_str(), r)).collect();
    let claimed: HashSet<&str> = local
        .iter()
        .filter_map(|e| e.remote_id.as_deref())
        .collect();
    // Unclaimed remote entries by hash, so a file whose twin is already there is linked, not
    // pushed again.
    let mut unclaimed: HashMap<String, &Remote> = HashMap::new();
    for r in remote.iter().filter(|r| !claimed.contains(r.id.as_str())) {
        unclaimed.entry(r.sha()).or_insert(r);
    }
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for e in local {
        let id = e.id.clone();
        let Some(rid) = e.remote_id.clone() else {
            match unclaimed.get(&e.sha()).filter(|r| !used.contains(&r.id)) {
                Some(r) => {
                    used.insert(r.id.clone());
                    out.push(Action::Link {
                        id,
                        remote_id: r.id.clone(),
                    });
                }
                None => out.push(Action::Push { id }),
            }
            continue;
        };
        let base = e.remote_sha.clone().unwrap_or_default();
        let local_changed = e.sha() != base;
        match by_id.get(rid.as_str()) {
            None if local_changed => out.push(Action::Conflict {
                id,
                why: "changed here, and the service no longer has the entry".to_owned(),
            }),
            None => out.push(Action::RemoteGone { id }),
            Some(r) => {
                let remote_changed = r.sha() != base;
                match (local_changed, remote_changed) {
                    (false, false) => out.push(Action::InSync { id }),
                    (true, false) => out.push(Action::Replace { id, remote_id: rid }),
                    (false, true) => out.push(Action::RemoteNewer { id, remote_id: rid }),
                    (true, true) => out.push(Action::Conflict {
                        id,
                        why: "changed here and on the service since the last sync".to_owned(),
                    }),
                }
            }
        }
    }
    for r in remote {
        if !claimed.contains(r.id.as_str()) && !used.contains(&r.id) {
            out.push(Action::RemoteOnly {
                remote_id: r.id.clone(),
            });
        }
    }
    out
}

/// The entries whose source is `source`, in their order.
#[must_use]
pub fn keep(all: Vec<Remote>, source: &str) -> Vec<Remote> {
    all.into_iter().filter(|r| r.source == source).collect()
}

/// The file's link fields after a sync with `remote`: its id, creation time and hash.
pub fn link(e: &mut Entry, remote: &Remote) {
    e.remote_id = Some(remote.id.clone());
    e.remote_version = Some(remote.created_at.clone());
    e.remote_sha = Some(remote.sha());
}

/// A new local memory from a remote one (for `--pull`): the id comes from the text, the dates
/// from the service.
#[must_use]
pub fn pulled(remote: &Remote) -> Entry {
    let mut e = Entry::new(&remote.text, remote.kind, "", Vec::new(), false);
    e.created.clone_from(&remote.created_at);
    e.updated.clone_from(&remote.created_at);
    link(&mut e, remote);
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(text: &str) -> Entry {
        Entry::new(text, Kind::Fact, "why", Vec::new(), false)
    }

    fn remote(id: &str, text: &str) -> Remote {
        Remote {
            id: id.into(),
            kind: Kind::Fact,
            text: text.into(),
            source: "project:x".into(),
            created_at: "2026-11-01T00:00:00Z".into(),
        }
    }

    fn synced(text: &str, r: &Remote) -> Entry {
        let mut e = local(text);
        link(&mut e, r);
        e
    }

    #[test]
    fn the_bodies_carry_kind_text_source_and_the_query() {
        let b = remember_body(Kind::Preference, "use sqlx", "project:x");
        assert_eq!(b["kind"], "KIND_PREFERENCE");
        assert_eq!(b["text"], "use sqlx");
        assert_eq!(b["source"], "project:x");
        assert_eq!(recall_body("timeouts", 25)["k"], 25);
        assert_eq!(source_for(Scope::Project, "billing"), "project:billing");
        assert_eq!(source_for(Scope::User, "billing"), "user");
    }

    #[test]
    fn the_filter_keeps_only_the_sources_entries_in_order() {
        let mut a = remote("a", "1");
        let mut b = remote("b", "2");
        let mut c = remote("c", "3");
        a.source = "project:x".into();
        b.source = "other".into();
        c.source = "project:x".into();
        let ids: Vec<String> = keep(vec![a, b, c], "project:x")
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["a", "c"]);
    }

    #[test]
    fn a_new_memory_is_pushed_and_one_whose_twin_is_there_is_linked() {
        let a = local("a fact");
        let b = local("another fact");
        let twin = remote("r1", "another fact");
        let p = plan(&[a.clone(), b.clone()], &[twin]);
        assert_eq!(p[0], Action::Push { id: a.id });
        assert_eq!(
            p[1],
            Action::Link {
                id: b.id,
                remote_id: "r1".into()
            }
        );
        assert_eq!(p.len(), 2, "the twin is not remote-only: {p:?}");
    }

    #[test]
    fn unchanged_both_sides_is_in_sync_and_a_local_edit_is_replaced() {
        let r = remote("r1", "a fact");
        let same = synced("a fact", &r);
        assert_eq!(
            plan(std::slice::from_ref(&same), std::slice::from_ref(&r)),
            [Action::InSync {
                id: same.id.clone()
            }]
        );
        let mut edited = same.clone();
        edited.text = "a fact, edited".into();
        assert_eq!(
            plan(&[edited.clone()], &[r]),
            [Action::Replace {
                id: edited.id,
                remote_id: "r1".into()
            }]
        );
    }

    #[test]
    fn a_remote_change_alone_is_reported_not_pulled() {
        let was = remote("r1", "a fact");
        let e = synced("a fact", &was);
        let now = remote("r1", "a fact, changed there");
        assert_eq!(
            plan(std::slice::from_ref(&e), &[now]),
            [Action::RemoteNewer {
                id: e.id,
                remote_id: "r1".into()
            }]
        );
    }

    #[test]
    fn both_changed_and_a_dropped_edit_are_conflicts_and_a_dropped_copy_is_gone() {
        let was = remote("r1", "a fact");
        let mut both = synced("a fact", &was);
        both.text = "edited here".into();
        let now = remote("r1", "edited there");
        assert!(matches!(
            plan(&[both], &[now]).as_slice(),
            [Action::Conflict { .. }]
        ));
        let mut dropped = synced("a fact", &was);
        dropped.text = "edited here".into();
        assert!(matches!(
            plan(&[dropped], &[]).as_slice(),
            [Action::Conflict { .. }]
        ));
        let gone = synced("a fact", &was);
        assert!(matches!(
            plan(&[gone], &[]).as_slice(),
            [Action::RemoteGone { .. }]
        ));
    }

    #[test]
    fn entries_only_the_service_has_are_remote_only() {
        let p = plan(&[], &[remote("r9", "elsewhere")]);
        assert_eq!(
            p,
            [Action::RemoteOnly {
                remote_id: "r9".into()
            }]
        );
        assert!(!p[0].changes());
    }

    #[test]
    fn a_pulled_entry_is_linked_and_in_sync() {
        let r = remote("r9", "elsewhere");
        let e = pulled(&r);
        assert_eq!(
            plan(std::slice::from_ref(&e), &[r]),
            [Action::InSync { id: e.id }]
        );
    }

    #[test]
    fn from_value_needs_an_id_and_reads_the_wire_names() {
        let v =
            json!({"id":"r1","kind":"KIND_PREFERENCE","text":"t","source":"s","created_at":"c"});
        let r = Remote::from_value(&v).expect("entry");
        assert_eq!((r.kind, r.created_at.as_str()), (Kind::Preference, "c"));
        assert!(Remote::from_value(&json!({"text":"t"})).is_none());
    }
}
