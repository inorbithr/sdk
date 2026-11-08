//! The handoff note of a stopped agent run: `.inorbit/handoff.md` in the project.
//!
//! When an agent run stops at its turn or time cap (or fails) what it did is on disk but what it
//! meant to do next is in its head. This writes a short note: what was asked, which files were
//! written, and the agent's last words (its own summary of what is left). The next run in the
//! folder (`iohr ask --allow-edits`) reads it into its prompt and removes it when it ends well.
//!
//! The note is compacted: a second stop does not overwrite the first, it folds it to one line
//! under "Earlier runs" (the newest few kept, the oldest folded into a count), so the note stays
//! short however many runs it spans. `iohr handoff show|compact|clear` reads and tends it.
//!
//! The note is the person's own text and the agent's own words, kept in their project folder
//! and nowhere else: nothing here logs, sends or traces it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
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
/// Earlier runs kept one line each; the older ones are folded into a count.
const EARLIER_MAX: usize = 6;
const DIGEST_ASKED: usize = 160;
const DIGEST_SAID: usize = 240;
/// The first line of a note: how many runs it covers and how many older ones were folded.
const MARK: &str = "<!-- iohr-handoff";

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

fn one_line(s: &str, max: usize) -> String {
    clip(&s.split_whitespace().collect::<Vec<_>>().join(" "), max)
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

/// The parts of a note read back from disk, for compaction.
#[derive(Debug, Default)]
struct Parsed {
    runs: usize,
    folded: usize,
    earlier: Vec<String>,
    why: String,
    asked: String,
    files: Vec<String>,
    file_count: usize,
    said: String,
}

/// The body of `## head`, up to the next `## `.
fn section<'a>(text: &'a str, head: &str) -> &'a str {
    let key = format!("## {head}\n");
    let Some(i) = text.find(&key) else {
        return "";
    };
    let rest = &text[i + key.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end].trim()
}

fn marker_num(text: &str, key: &str) -> usize {
    text.lines()
        .find(|l| l.starts_with(MARK))
        .and_then(|l| l.split(key).nth(1))
        .and_then(|r| {
            r.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

fn parse(text: &str) -> Parsed {
    let done = section(text, "What was done");
    Parsed {
        // A note of the first kind (no marker) covers one run.
        runs: marker_num(text, "runs=").max(1),
        folded: marker_num(text, "folded="),
        earlier: section(text, "Earlier runs")
            .lines()
            .filter(|l| l.starts_with("- run "))
            .map(str::to_owned)
            .collect(),
        why: section(text, "Why it stopped")
            .trim_end_matches('.')
            .to_owned(),
        asked: section(text, "What was asked").to_owned(),
        files: done
            .lines()
            .filter_map(|l| l.strip_prefix("- `")?.strip_suffix('`'))
            .map(str::to_owned)
            .collect(),
        file_count: done
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0),
        said: section(text, "What is left")
            .lines()
            .filter_map(|l| l.strip_prefix("> "))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

impl Parsed {
    /// This run folded to one line, numbered `n`.
    fn digest(&self, n: usize) -> String {
        let left = if self.said.is_empty() {
            "(nothing said)".to_owned()
        } else {
            one_line(&self.said, DIGEST_SAID)
        };
        format!(
            "- run {n}: {}; asked: {}; {} files; left: {left}",
            one_line(&self.why, 80),
            one_line(&self.asked, DIGEST_ASKED),
            self.file_count,
        )
    }
}

/// Keeps the newest `EARLIER_MAX` lines; each older one is folded into the count.
fn compact_earlier(earlier: &mut Vec<String>, folded: &mut usize) {
    while earlier.len() > EARLIER_MAX {
        earlier.remove(0);
        *folded += 1;
    }
}

/// What a note says of the runs before it.
struct History<'a> {
    /// How many runs the note covers, this one included.
    runs: usize,
    /// One line per earlier run, oldest first.
    earlier: &'a [String],
    /// How many older runs were folded away.
    folded: usize,
}

fn render_with(
    asked: &str,
    files: &[String],
    said: &str,
    why: &str,
    bin: &str,
    h: &History<'_>,
) -> String {
    let mut t = format!(
        "{MARK} runs={} folded={} -->\n# Handoff: the agent run stopped before it finished\n\n\
         Written by iohr when the run stopped. Read it before you go on, and delete the file \
         when the work is done (`{bin} handoff clear`). Earlier runs are kept as one line each \
         and the oldest are folded away, so the note stays short.\n\n",
        h.runs, h.folded
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
    if !h.earlier.is_empty() || h.folded > 0 {
        let _ = write!(t, "\n## Earlier runs\n\n");
        if h.folded > 0 {
            let _ = writeln!(t, "{} older runs were folded away.\n", h.folded);
        }
        for e in h.earlier {
            let _ = writeln!(t, "{e}");
        }
    }
    let _ = write!(
        t,
        "\n## Continue\n\nRun `{bin} ask --allow-edits \"continue the work\"` in this folder.\n"
    );
    t
}

/// The text of a first note (no earlier runs).
pub(crate) fn render(asked: &str, files: &[String], said: &str, why: &str, bin: &str) -> String {
    let h = History {
        runs: 1,
        earlier: &[],
        folded: 0,
    };
    render_with(asked, files, said, why, bin, &h)
}

fn save(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("cannot write {FILE}: {}", e.kind()))
}

/// Writes the note under `dir`. A note already there is not overwritten but compacted: its run
/// becomes one line under "Earlier runs", and only the newest lines are kept.
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
    let (mut runs, mut earlier, mut folded) = (1, Vec::new(), 0);
    if let Some(old) = std::fs::read_to_string(&path)
        .ok()
        .filter(|t| !t.trim().is_empty())
    {
        let p = parse(&old);
        earlier = p.earlier.clone();
        earlier.push(p.digest(p.runs));
        folded = p.folded;
        runs = p.runs + 1;
        compact_earlier(&mut earlier, &mut folded);
    }
    let h = History {
        runs,
        earlier: &earlier,
        folded,
    };
    save(&path, &render_with(asked, &files, said, why, bin, &h))?;
    Ok(files.len())
}

/// Re-compacts the note on disk (one edited by hand, or written by an older iohr): returns how
/// many earlier runs this call folded away.
///
/// # Errors
/// A message when there is no note or it cannot be written.
pub(crate) fn compact(root: &Path, bin: &str) -> Result<usize, String> {
    let path = root.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no {FILE} here"))?;
    let mut p = parse(&text);
    let before = p.folded;
    compact_earlier(&mut p.earlier, &mut p.folded);
    let h = History {
        runs: p.runs,
        earlier: &p.earlier,
        folded: p.folded,
    };
    save(
        &path,
        &render_with(&p.asked, &p.files, &p.said, &p.why, bin, &h),
    )?;
    Ok(p.folded - before)
}

/// How many runs the note at `root` covers, when it has one.
pub(crate) fn runs(root: &Path) -> Option<usize> {
    let t = std::fs::read_to_string(root.join(FILE)).ok()?;
    (!t.trim().is_empty()).then(|| parse(&t).runs)
}

/// The project root of the current folder.
pub(crate) fn root_here() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .and_then(|d| crate::memory::find_root(&d))
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
pub(crate) fn continue_from(edits: bool, question: String) -> (Option<PathBuf>, String) {
    let root = edits
        .then(root_here)
        .flatten()
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
    fn a_second_stop_folds_the_first_and_old_runs_become_a_count() {
        let d = tempfile::tempdir().unwrap();
        for i in 1..=9 {
            write(
                d.path(),
                &format!("task {i}"),
                &format!("left {i}"),
                "why",
                "iohr",
            )
            .unwrap();
        }
        let n = std::fs::read_to_string(d.path().join(FILE)).unwrap();
        assert_eq!(runs(d.path()), Some(9));
        assert!(n.contains("## What was asked\n\ntask 9"), "{n}");
        assert!(n.contains("- run 8: why; asked: task 8"), "{n}");
        assert!(!n.contains("- run 2:"), "{n}");
        assert!(n.contains("2 older runs were folded away"), "{n}");
        assert_eq!(n.matches("\n- run ").count(), EARLIER_MAX);
    }

    #[test]
    fn compact_keeps_the_note_and_folds_a_long_history() {
        let d = tempfile::tempdir().unwrap();
        assert!(compact(d.path(), "iohr").is_err());
        write(d.path(), "a", "b", "w", "iohr").unwrap();
        write(d.path(), "c", "d", "w", "iohr").unwrap();
        let before = std::fs::read_to_string(d.path().join(FILE)).unwrap();
        assert_eq!(compact(d.path(), "iohr").unwrap(), 0);
        assert_eq!(before, std::fs::read_to_string(d.path().join(FILE)).unwrap());
        // A hand-grown history is cut to the newest lines.
        let mut long = before.replace("\n## Continue", "");
        for i in 10..20 {
            let _ = writeln!(long, "- run {i}: w; asked: t; 0 files; left: x");
        }
        std::fs::write(d.path().join(FILE), long).unwrap();
        assert!(compact(d.path(), "iohr").unwrap() >= 5);
        assert_eq!(runs(d.path()), Some(2));
    }

    #[test]
    fn long_text_is_cut() {
        let long = "x".repeat(5000);
        let t = render(&long, &[], &long, "why", "iohr");
        assert!(t.len() < 5000, "{}", t.len());
    }
}
