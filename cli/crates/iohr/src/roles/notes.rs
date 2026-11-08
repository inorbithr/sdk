//! What a Role keeps in the project between its turns (RFC 0213, S2): a memory area and a
//! handoff note, one of each per Role.
//!
//! - The memory area is `.inorbit/memory/<role>/`, a folder of the same memory files
//!   `iohr memory` keeps (so one parser, [`Store`]). The project store skips a sub-folder, so
//!   a Role's lessons never show up as the project's. A Role's turn is told its newest ones.
//! - The handoff note is `.inorbit/handoff/<role>.md`, written when a Role's turn ends: what
//!   it was asked and what it answered, so the next turn of that Role starts from it. It is
//!   [`crate::handoff`]'s idea (a short note on disk, read into the next prompt) per Role.
//!
//! Both are the person's own files in their project folder: nothing here logs, sends or
//! traces them. A Role name that is not a safe folder name keeps nothing and reads nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::front;
use crate::memory::store::{PROJECT_DIR, Scope, Store};

/// The folder of the handoff notes, under the project root.
pub(crate) const HANDOFF_DIR: &str = ".inorbit/handoff";
/// The most memories of the area a turn is told, newest first.
const MEMORY_COUNT: usize = 8;
/// The most characters of the area one turn carries.
const MEMORY_MAX: usize = 2_000;
/// The most characters of the question and of the answer kept in the note.
const ASKED_MAX: usize = 800;
const SAID_MAX: usize = 2_000;
/// The most of a note a turn takes.
const READ_MAX: usize = 3_000;

/// A Role name that is safe as a folder and a file name: lower-case letters, digits, `-` and
/// `_`, short, never starting with a dot. Anything else (`../x`, `a/b`, an empty name) is not.
#[must_use]
pub(crate) fn valid_role(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// The Role's memory area.
#[must_use]
pub(crate) fn memory_dir(root: &Path, role: &str) -> Option<PathBuf> {
    valid_role(role).then(|| root.join(PROJECT_DIR).join(role))
}

/// The Role's handoff note.
#[must_use]
pub(crate) fn handoff_file(root: &Path, role: &str) -> Option<PathBuf> {
    valid_role(role).then(|| root.join(HANDOFF_DIR).join(format!("{role}.md")))
}

fn clip(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_owned();
    }
    let mut c: String = t.chars().take(max).collect();
    c.push_str(" ...");
    c
}

fn one_line(s: &str, max: usize) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clip(&s.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// What the Role's earlier turns left, as text for its next one: its handoff note and the
/// newest memories of its area (pinned first). Empty when it has neither.
#[must_use]
pub(crate) fn read(root: &Path, role: &str) -> String {
    let mut out = String::new();
    if let Some(note) = handoff_file(root, role)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| clip(&t, READ_MAX))
        .filter(|t| !t.is_empty())
    {
        let _ = write!(
            out,
            "Your last turn in this project left this handoff note \
             ({HANDOFF_DIR}/{role}.md):\n\n{note}\n\n"
        );
    }
    if let Some(dir) = memory_dir(root, role) {
        let mut mem = Store::new(dir, Scope::Project).load().unwrap_or_default();
        mem.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.updated.cmp(&a.updated))
        });
        let mut lines = String::new();
        for e in mem.iter().take(MEMORY_COUNT) {
            let line = format!("- {}\n", one_line(&e.text, 300));
            if lines.len() + line.len() > MEMORY_MAX {
                break;
            }
            lines.push_str(&line);
        }
        if !lines.is_empty() {
            let _ = write!(
                out,
                "What you kept in your memory area ({PROJECT_DIR}/{role}/):\n{lines}"
            );
        }
    }
    out.trim_end().to_owned()
}

/// Writes the Role's handoff note after a turn: when, what it was asked and what it answered.
/// Through a temporary file and a rename, so a reader never sees half a note.
///
/// # Errors
/// A message saying what could not be written (never the text).
pub(crate) fn record(root: &Path, role: &str, asked: &str, said: &str) -> Result<(), String> {
    let path =
        handoff_file(root, role).ok_or_else(|| format!("`{role}` is not a safe Role name"))?;
    let dir = path.parent().ok_or("no folder for the note")?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create {}: {}", dir.display(), e.kind()))?;
    let text = format!(
        "# Handoff: {role}\n\nWritten by iohr when the Role's last turn ended ({}). The next \
         turn of this Role reads it; it is overwritten by each turn.\n\n## Asked\n\n{}\n\n\
         ## Answered\n\n{}\n",
        front::now(),
        clip(asked, ASKED_MAX),
        clip(said, SAID_MAX),
    );
    let tmp = dir.join(format!(".{role}.tmp"));
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("cannot write {HANDOFF_DIR}/{role}.md: {}", e.kind()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::{Entry, Kind};

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn only_a_safe_name_is_a_folder() {
        for ok in ["security", "rust-engineer", "a_b", "x1"] {
            assert!(valid_role(ok), "{ok}");
        }
        for bad in ["", "../x", "a/b", ".hidden", "Up", "a b", &"x".repeat(65)] {
            assert!(!valid_role(bad), "{bad}");
            assert!(memory_dir(Path::new("/p"), bad).is_none());
            assert!(handoff_file(Path::new("/p"), bad).is_none());
        }
    }

    #[test]
    fn a_turn_leaves_a_note_the_next_one_reads_and_one_role_never_reads_anothers() {
        let d = tmp();
        record(
            d.path(),
            "security",
            "is the key safe?",
            "Yes, it is in the vault.",
        )
        .unwrap();
        let note = read(d.path(), "security");
        assert!(
            note.contains("is the key safe?") && note.contains("in the vault"),
            "{note}"
        );
        assert!(note.contains(".inorbit/handoff/security.md"));
        assert_eq!(read(d.path(), "rust-engineer"), "");
        // Overwritten, not appended; no temporary file left.
        record(d.path(), "security", "second?", "second answer").unwrap();
        let note = read(d.path(), "security");
        assert!(note.contains("second answer") && !note.contains("in the vault"));
        let left: Vec<_> = std::fs::read_dir(d.path().join(HANDOFF_DIR))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["security.md"]);
    }

    #[test]
    fn a_name_that_climbs_out_writes_nothing() {
        let d = tmp();
        assert!(record(d.path(), "../../evil", "q", "a").is_err());
        assert!(!d.path().join(".inorbit").exists());
        assert_eq!(read(d.path(), "../../evil"), "");
    }

    #[test]
    fn a_long_answer_is_cut() {
        let d = tmp();
        record(d.path(), "security", &"q".repeat(5000), &"a".repeat(20_000)).unwrap();
        let on_disk =
            std::fs::read_to_string(d.path().join(HANDOFF_DIR).join("security.md")).unwrap();
        assert!(on_disk.chars().count() < ASKED_MAX + SAID_MAX + 400);
        assert!(read(d.path(), "security").chars().count() <= READ_MAX + 400);
    }

    #[test]
    fn the_memory_area_is_the_roles_own_and_the_project_store_skips_it() {
        let d = tmp();
        let area = Store::new(memory_dir(d.path(), "security").unwrap(), Scope::Project);
        let mut e = Entry::new("Keys rotate every 90 days.", Kind::Fact, "", vec![], false);
        e.id = "rotate".into();
        area.save(&e).unwrap();
        let said = read(d.path(), "security");
        assert!(said.contains("Keys rotate every 90 days."), "{said}");
        assert!(said.contains(".inorbit/memory/security/"));
        assert!(Store::project(d.path()).load().unwrap().is_empty());
        assert_eq!(read(d.path(), "rust-engineer"), "");
    }

    #[test]
    fn the_area_is_told_newest_first_and_within_its_budget() {
        let d = tmp();
        let area = Store::new(memory_dir(d.path(), "security").unwrap(), Scope::Project);
        for i in 0..30 {
            let mut e = Entry::new(
                &format!("fact number {i} {}", "x".repeat(250)),
                Kind::Fact,
                "",
                vec![],
                false,
            );
            e.id = format!("m{i}");
            e.updated = format!("2026-11-08T10:00:{i:02}Z");
            area.save(&e).unwrap();
        }
        let said = read(d.path(), "security");
        assert!(said.contains("fact number 29"));
        assert!(!said.contains("fact number 0 "));
        assert!(said.lines().filter(|l| l.starts_with("- ")).count() <= MEMORY_COUNT);
        assert!(said.len() < MEMORY_MAX + 300);
    }
}
