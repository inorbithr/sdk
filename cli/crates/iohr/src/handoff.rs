//! The handoff note of a stopped agent run: `.inorbit/handoff.md` in the project.
//!
//! When the agent `iohr project init` starts stops at its turn or time cap, what it did is on
//! disk but what it meant to do next is in its head. This writes a short note: what was asked,
//! which files were written, and the agent's last words (its own summary of what is left). The
//! next run in the folder (`iohr ask --allow-edits`) reads it into its prompt. It is the first,
//! small step; the full compaction and handoff system comes later and builds on this file.
//!
//! The note is the person's own text and the agent's own words, kept in their project folder
//! and nowhere else: nothing here logs, sends or traces it.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Where the note is, under the project root.
pub(crate) const FILE: &str = ".inorbit/handoff.md";

/// The most characters of the person's request and of the agent's last words kept in the note.
const ASKED_MAX: usize = 1500;
const SAID_MAX: usize = 2500;
/// The most files listed.
const FILES_MAX: usize = 60;
/// The most of a note a prompt takes.
const READ_MAX: usize = 6000;

/// Why the run stopped, in the person's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Turns,
    Time(u64),
}

impl Stop {
    /// A sentence: `it used all its 150 turns`, `it ran out of its 1800 s`.
    pub(crate) fn said(self, turns: u32) -> String {
        match self {
            Stop::Turns => format!("it used all its {turns} turns before it finished"),
            Stop::Time(s) => format!("it did not finish in {s} s and was stopped"),
        }
    }
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

/// The files the run changed or added, as git sees the project (`git status --porcelain
/// -uall`): paths only, never contents. Empty when git is missing or the folder is no repository.
pub(crate) fn changed_files(dir: &Path) -> Vec<String> {
    let Ok(o) = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain", "-uall"])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !o.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|p| p.trim().trim_matches('"').to_owned())
        .filter(|p| !p.is_empty() && p != FILE)
        .collect()
}

/// The text of the note.
pub(crate) fn render(asked: &str, files: &[String], said: &str, why: &str, bin: &str) -> String {
    let mut t = String::from(
        "# Handoff: the agent run stopped before it finished\n\n\
         Written by iohr when the run stopped. A first, small handoff note; read it before \
         you go on, and delete the file when the work is done.\n\n",
    );
    let _ = write!(t, "## Why it stopped\n\n{why}.\n\n");
    let _ = write!(t, "## What was asked\n\n{}\n\n", clip(asked, ASKED_MAX));
    let _ = write!(t, "## What was done\n\n");
    if files.is_empty() {
        t.push_str("No file changes were found.\n\n");
    } else {
        let _ = writeln!(t, "{} files written or changed:\n", files.len());
        for f in files.iter().take(FILES_MAX) {
            let _ = writeln!(t, "- `{f}`");
        }
        if files.len() > FILES_MAX {
            let _ = writeln!(t, "- and {} more", files.len() - FILES_MAX);
        }
        t.push('\n');
    }
    let _ = write!(t, "## What is left\n\nThe agent's last words:\n\n");
    if said.trim().is_empty() {
        t.push_str("(it said nothing after its last tool call; compare what was asked with what was done)\n");
    } else {
        for l in clip(said, SAID_MAX).lines() {
            let _ = writeln!(t, "> {l}");
        }
    }
    let _ = write!(
        t,
        "\n## Continue\n\nRun `{bin} ask --allow-edits \"continue the work\"` in this folder.\n"
    );
    t
}

/// Writes the note under `dir`.
///
/// # Errors
/// A message saying what could not be written.
pub(crate) fn write(
    dir: &Path,
    asked: &str,
    said: &str,
    why: &str,
    bin: &str,
) -> Result<usize, String> {
    let files = changed_files(dir);
    let path = dir.join(FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {}", parent.display(), e.kind()))?;
    }
    std::fs::write(&path, render(asked, &files, said, why, bin))
        .map_err(|e| format!("cannot write {FILE}: {}", e.kind()))?;
    Ok(files.len())
}

/// The note of the project at `root`, when it has one (cut to what a prompt takes).
pub(crate) fn read(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(FILE)).ok()?;
    let t = text.trim();
    (!t.is_empty()).then(|| clip(t, READ_MAX))
}

/// A run that goes on from the note ended well: the note is done with.
pub(crate) fn clear(root: &Path) {
    let _ = std::fs::remove_file(root.join(FILE));
}

/// The words a run that goes on from the note starts with: the note, and how to work in a run
/// that has no shell (`iohr ask --allow-edits` runs none, so no `atlas new`).
pub(crate) fn preface(note: &str) -> String {
    format!(
        "An earlier run in this project stopped before it finished. Its handoff note \
         (.inorbit/handoff.md) follows. Check what is on disk, finish what is left, and keep \
         the same conventions. This run has no shell: write files with Write and Edit; a new \
         document in atlas/ is `atlas/<kind>/NNNN-slug.md`, numbered one above the highest \
         number in that folder, with the same front matter as its neighbours.\n\n{note}\n\n\
         Now: "
    )
}

/// The project root whose note a run goes on from, and the question with the note in front of
/// it. `edits` is false for a run that changes nothing: it takes no note and the question stays.
pub(crate) fn continue_from(edits: bool, question: String) -> (Option<std::path::PathBuf>, String) {
    let root = edits
        .then(|| std::env::current_dir().ok())
        .flatten()
        .and_then(|d| crate::memory::find_root(&d))
        .filter(|r| read(r).is_some());
    match root.as_deref().and_then(read) {
        Some(note) => {
            let q = format!("{}{question}", preface(&note));
            (root, q)
        }
        None => (root, question),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_says_what_was_asked_done_and_left_and_how_to_continue() {
        let t = render(
            "build the site knowledge base",
            &["atlas/prd/0002-a.md".to_owned()],
            "I wrote 1 PRD.\nNext: the ADRs.",
            "it used all its 150 turns before it finished",
            "iohr-preview",
        );
        for want in [
            "## What was asked\n\nbuild the site knowledge base",
            "- `atlas/prd/0002-a.md`",
            "> Next: the ADRs.",
            "it used all its 150 turns",
            "`iohr-preview ask --allow-edits \"continue the work\"`",
        ] {
            assert!(t.contains(want), "missing {want}: {t}");
        }
    }

    #[test]
    fn it_is_written_read_and_cleared() {
        let d = tempfile::tempdir().unwrap();
        assert!(read(d.path()).is_none());
        write(
            d.path(),
            "x",
            "",
            "it did not finish in 5 s and was stopped",
            "iohr",
        )
        .unwrap();
        let n = read(d.path()).unwrap();
        assert!(n.contains("it said nothing"), "{n}");
        assert!(preface(&n).contains("handoff note"));
        clear(d.path());
        assert!(read(d.path()).is_none());
    }

    #[test]
    fn long_text_is_cut() {
        let long = "x".repeat(5000);
        let t = render(&long, &[], &long, "why", "iohr");
        assert!(t.len() < 5000, "{}", t.len());
    }
}
