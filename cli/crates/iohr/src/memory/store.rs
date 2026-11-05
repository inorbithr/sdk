//! The local memory store: one Markdown file per memory, in a folder (RFC 0205, ADR 0216).
//! No database. The folder is the project's `.inorbit/memory/` (committed), or the person's
//! own notes on this machine.
//!
//! A file is front matter plus the text:
//!
//! ```text
//! ---
//! id: use-sqlx-for-queries-3f9a1c
//! kind: fact
//! scope: project
//! created: 2026-11-05T10:00:00Z
//! updated: 2026-11-05T10:00:00Z
//! checked: 2026-11-05
//! why: the billing service moved off diesel
//! pinned: false
//! refs: [src/db.rs, "cmd:cargo nextest run"]
//! ---
//! Use sqlx for queries.
//! ```
//!
//! The keys `remote_id`, `remote_version` and `remote_sha` appear only once a memory has been
//! synced; they stay on this machine ([`LOCAL_KEYS`]). Written files go to disk through a
//! temporary file and a rename, so a crash never leaves half a memory. Text is never logged.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::front;

use super::text;

/// The project's memory folder, relative to its root.
pub const PROJECT_DIR: &str = ".inorbit/memory";

/// The front matter keys that stay on this machine: the sync bookkeeping, never sent.
pub const LOCAL_KEYS: [&str; 3] = ["remote_id", "remote_version", "remote_sha"];

/// The longest memory, in characters. Memories are short facts, not documents.
pub const MAX_TEXT: usize = 2000;

/// What a memory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Fact,
    Preference,
    Episode,
}

impl Kind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Episode => "episode",
        }
    }

    /// The kind a word names; a missing or unknown word is a fact.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "preference" => Self::Preference,
            "episode" => Self::Episode,
            _ => Self::Fact,
        }
    }

    /// The memory service's name for the kind.
    #[must_use]
    pub fn api(self) -> &'static str {
        match self {
            Self::Fact => "KIND_FACT",
            Self::Preference => "KIND_PREFERENCE",
            Self::Episode => "KIND_EPISODE",
        }
    }

    /// The kind the memory service's name stands for.
    #[must_use]
    pub fn from_api(s: &str) -> Self {
        match s {
            "KIND_PREFERENCE" => Self::Preference,
            "KIND_EPISODE" => Self::Episode,
            _ => Self::Fact,
        }
    }
}

/// Where a memory lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The repository's `.inorbit/memory/`, committed with the code.
    Project,
    /// This person's notes on this machine, in every project.
    User,
    /// The account's memory service only; needs sync on.
    Account,
}

impl Scope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Account => "account",
        }
    }

    /// The scope a word names.
    ///
    /// # Errors
    /// [`Error::Usage`] for a word that is none of `project`, `user`, `account`.
    pub fn parse(s: &str) -> Result<Self, Error> {
        match s.trim() {
            "project" => Ok(Self::Project),
            "user" => Ok(Self::User),
            "account" => Ok(Self::Account),
            other => Err(Error::Usage(format!(
                "scope is `project`, `user` or `account`, not `{other}`"
            ))),
        }
    }
}

/// One memory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub created: String,
    pub updated: String,
    /// The day (`YYYY-MM-DD`, UTC) the fact was last checked against its source or its
    /// person; `None` for a file written before the key existed. `iohr memory check` flags a
    /// missing or old one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<String>,
    pub why: String,
    pub pinned: bool,
    /// Paths (relative to the project root) and commands (`cmd:...`) the memory is about.
    pub refs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_sha: Option<String>,
    pub text: String,
}

impl Entry {
    /// A new memory made now.
    #[must_use]
    pub fn new(text: &str, kind: Kind, why: &str, refs: Vec<String>, pinned: bool) -> Self {
        let now = front::now();
        Self {
            id: make_id(text),
            kind,
            created: now.clone(),
            updated: now,
            checked: Some(front::today()),
            why: why.trim().to_owned(),
            pinned,
            refs,
            remote_id: None,
            remote_version: None,
            remote_sha: None,
            text: text.trim().to_owned(),
        }
    }

    /// The fingerprint of what a push sends: kind and text, in hex. Bookkeeping, dates and
    /// refs never change it.
    #[must_use]
    pub fn sha(&self) -> String {
        sha_of(self.kind, &self.text)
    }

    /// The file's text, for the scope it lives in.
    #[must_use]
    pub fn render(&self, scope: Scope) -> String {
        let mut lines = vec![
            format!("id: {}", front::quote(&self.id)),
            format!("kind: {}", self.kind.as_str()),
            format!("scope: {}", scope.as_str()),
            format!("created: {}", front::quote(&self.created)),
            format!("updated: {}", front::quote(&self.updated)),
            format!("why: {}", front::quote(&self.why)),
            format!("pinned: {}", self.pinned),
            format!("refs: {}", front::list_value(&self.refs)),
        ];
        if let Some(c) = &self.checked {
            lines.insert(5, format!("checked: {c}"));
        }
        for (k, v) in [
            ("remote_id", &self.remote_id),
            ("remote_version", &self.remote_version),
            ("remote_sha", &self.remote_sha),
        ] {
            if let Some(v) = v {
                lines.push(format!("{k}: {}", front::quote(v)));
            }
        }
        format!("---\n{}\n---\n{}\n", lines.join("\n"), self.text)
    }

    /// The memory in a file's text, or `None` for a text with no front matter or no id.
    #[must_use]
    pub fn parse(text: &str, stem: &str) -> Option<Self> {
        let (_, body) = front::split(text)?;
        let id = front::get(text, "id")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| stem.to_owned());
        let some = |k: &str| front::get(text, k).filter(|s| !s.is_empty());
        let created = some("created").unwrap_or_default();
        Some(Self {
            id,
            kind: Kind::parse(&front::get(text, "kind").unwrap_or_default()),
            updated: some("updated").unwrap_or_else(|| created.clone()),
            created,
            checked: some("checked"),
            why: front::get(text, "why").unwrap_or_default(),
            pinned: front::get(text, "pinned").is_some_and(|p| p.trim() == "true"),
            refs: front::get_list(text, "refs"),
            remote_id: some("remote_id"),
            remote_version: some("remote_version"),
            remote_sha: some("remote_sha"),
            text: body.trim().to_owned(),
        })
    }
}

/// The SHA-256 of a memory's kind and text, in hex.
#[must_use]
pub fn sha_of(kind: Kind, text: &str) -> String {
    front::sha_hex(&format!("{}\n{}", kind.as_str(), text.trim()))
}

/// An id for a memory: a few of its words and a short hash of its normal form, so the same
/// text always gets the same id. Letters, digits and dashes only.
#[must_use]
pub fn make_id(text: &str) -> String {
    let norm = text::normalize(text);
    let head: String = norm.chars().take(40).collect();
    let words = crate::atlas::slug(&head);
    let words = words.trim_matches('-');
    let hash: String = front::sha_hex(&norm).chars().take(6).collect();
    if words.is_empty() {
        format!("memory-{hash}")
    } else {
        format!("{words}-{hash}")
    }
}

/// Whether `id` is safe as a file name: lower-case letters, digits and dashes.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A folder of memories.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
    pub scope: Scope,
}

impl Store {
    #[must_use]
    pub fn new(dir: PathBuf, scope: Scope) -> Self {
        Self { dir, scope }
    }

    /// The project store under `root`.
    #[must_use]
    pub fn project(root: &Path) -> Self {
        Self::new(root.join(PROJECT_DIR), Scope::Project)
    }

    /// The person's own store on this machine: `~/.inorbit/memory/notes/`, apart from every
    /// other file under `~/.inorbit/memory/`.
    ///
    /// # Errors
    /// [`Error::Failed`] when there is no home folder.
    pub fn user() -> Result<Self, Error> {
        let home = etcetera::home_dir()
            .map_err(|_| Error::Failed("cannot find the home folder for user memory".into()))?;
        Ok(Self::new(
            home.join(".inorbit").join("memory").join("notes"),
            Scope::User,
        ))
    }

    /// The file of the memory `id`.
    #[must_use]
    pub fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.md"))
    }

    /// Every memory in the folder, oldest first. A file that is not a memory (the README) is
    /// skipped.
    ///
    /// # Errors
    /// [`Error::Failed`] when the folder exists and cannot be read.
    pub fn load(&self) -> Result<Vec<Entry>, Error> {
        let read = match std::fs::read_dir(&self.dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(Error::Failed(format!(
                    "cannot read {}: {e}",
                    self.dir.display()
                )));
            }
        };
        let mut out = Vec::new();
        for item in read.flatten() {
            let path = item.path();
            let Some(stem) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".md"))
                .filter(|s| valid_id(s))
            else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(e) = Entry::parse(&text, stem) {
                out.push(e);
            }
        }
        out.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Writes a memory: through a temporary file and a rename.
    ///
    /// # Errors
    /// [`Error::Failed`] for an unsafe id or a failed write.
    pub fn save(&self, e: &Entry) -> Result<(), Error> {
        if !valid_id(&e.id) {
            return Err(Error::Failed("a memory id is letters, digits and -".into()));
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|err| Error::Failed(format!("cannot make {}: {err}", self.dir.display())))?;
        let tmp = self.dir.join(format!(".{}.tmp", e.id));
        std::fs::write(&tmp, e.render(self.scope))
            .and_then(|()| std::fs::rename(&tmp, self.file(&e.id)))
            .map_err(|err| Error::Failed(format!("cannot write memory {}: {err}", e.id)))
    }

    /// Deletes a memory; `false` when there was none.
    ///
    /// # Errors
    /// [`Error::Usage`] for an unsafe id, [`Error::Failed`] when the delete fails.
    pub fn remove(&self, id: &str) -> Result<bool, Error> {
        if !valid_id(id) {
            return Err(Error::Usage(
                "a memory id is lower-case letters, digits and -".into(),
            ));
        }
        match std::fs::remove_file(self.file(id)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::Failed(format!("cannot delete memory {id}: {e}"))),
        }
    }
}

/// What `remember` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remembered {
    /// A new memory.
    Created {
        entry: Entry,
        similar: Option<String>,
    },
    /// The same text was already kept: its `updated` moved to now (re-confirmed).
    Confirmed(Entry),
}

/// Keeps `text`: a new file, or, when the same text is already there, that file re-confirmed
/// (`updated` is now; a `why` and refs given are added). A near duplicate is kept and named.
///
/// # Errors
/// [`Error::Usage`] for empty, too long or secret-looking text; [`Error::Failed`] from the
/// disk.
pub fn remember(
    store: &Store,
    text: &str,
    kind: Kind,
    why: &str,
    refs: &[String],
    pinned: bool,
) -> Result<Remembered, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Usage(
            "there is nothing to remember: the text is empty".into(),
        ));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(Error::Usage(format!(
            "a memory is a short fact, at most {MAX_TEXT} characters"
        )));
    }
    if text::has_secret(text) || text::has_secret(why) {
        return Err(Error::Usage(
            "that looks like a secret, and memory is committed with the project: nothing was kept"
                .into(),
        ));
    }
    let existing = store.load()?;
    let norm = text::normalize(text);
    if let Some(same) = existing.iter().find(|e| text::normalize(&e.text) == norm) {
        let mut e = same.clone();
        e.updated = front::now();
        e.checked = Some(front::today());
        if !why.trim().is_empty() {
            why.trim().clone_into(&mut e.why);
        }
        for r in refs {
            if !e.refs.contains(r) {
                e.refs.push(r.clone());
            }
        }
        e.pinned |= pinned;
        store.save(&e)?;
        return Ok(Remembered::Confirmed(e));
    }
    let shingles = text::shingles(&norm);
    let similar = existing
        .iter()
        .find(|e| text::jaccard(&shingles, &text::shingles(&text::normalize(&e.text))) >= NEAR)
        .map(|e| e.id.clone());
    let entry = Entry::new(text, kind, why, refs.to_vec(), pinned);
    store.save(&entry)?;
    Ok(Remembered::Created { entry, similar })
}

/// The Jaccard score from which two memories are called near duplicates.
pub const NEAR: f64 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, Store) {
        let t = tempfile::tempdir().expect("tempdir");
        let s = Store::project(t.path());
        (t, s)
    }

    #[test]
    fn a_memory_round_trips_through_its_file() {
        let (_t, s) = temp();
        let r = remember(
            &s,
            "Use sqlx for queries.",
            Kind::Preference,
            "billing left diesel: it is slow",
            &["src/db.rs".into(), "cmd:cargo nextest run".into()],
            true,
        )
        .unwrap();
        let Remembered::Created { entry, .. } = r else {
            panic!("created");
        };
        let loaded = s.load().unwrap();
        assert_eq!(loaded, vec![entry.clone()]);
        assert!(valid_id(&entry.id));
        let text = std::fs::read_to_string(s.dir.join(format!("{}.md", entry.id))).unwrap();
        assert!(text.starts_with("---\nid: "));
        assert!(text.contains("scope: project"));
        assert!(text.ends_with("---\nUse sqlx for queries.\n"));
    }

    #[test]
    fn the_same_text_is_confirmed_not_duplicated() {
        let (_t, s) = temp();
        remember(&s, "Run cargo test.", Kind::Fact, "", &[], false).unwrap();
        let r = remember(&s, "run cargo test", Kind::Fact, "ci does too", &[], false).unwrap();
        assert!(matches!(r, Remembered::Confirmed(_)));
        let all = s.load().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].why, "ci does too");
    }

    #[test]
    fn secrets_and_empty_text_are_refused_and_nothing_is_written() {
        let (_t, s) = temp();
        assert!(remember(&s, "  ", Kind::Fact, "", &[], false).is_err());
        assert!(
            remember(
                &s,
                "key ghp_abcdefghijklmnopqrstuvwxyz012345",
                Kind::Fact,
                "",
                &[],
                false
            )
            .is_err()
        );
        assert_eq!(s.load().unwrap(), Vec::new());
        assert!(!s.dir.exists());
    }

    #[test]
    fn remove_refuses_a_path_and_reports_a_missing_one() {
        let (_t, s) = temp();
        assert!(s.remove("../x").is_err());
        assert!(!s.remove("nope").unwrap());
    }
}
