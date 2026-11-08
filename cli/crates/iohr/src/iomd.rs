//! `IO.md` context files: what a person tells Io about a folder, the way `CLAUDE.md` works.
//!
//! One loader, used by everything that builds a prompt from the folder (`?`, `??`, the
//! Role turns that reuse them, `iohr context`) and by `iohr project init`, which writes the
//! starter file with [`starter`]. There is no second implementation: PRD 0193's compacted
//! project brief plugs in here, as one more kind of file in [`Chain::docs`].
//!
//! The rules:
//!
//! * A folder's file is `IO.md` or `io.md`. When both are there, `IO.md` is read, `io.md` is
//!   skipped and the skip is said ([`Chain::skipped`]).
//! * The chain is the user-level file (in the config directory the rest of iohr uses), then
//!   the project root's file, down to the file in the current folder: farthest first,
//!   nearest last, so the nearest wins when they disagree.
//! * The project root is the nearest folder with `inorbit.toml`, else the git root. The walk
//!   never goes above the home folder. Outside a project only the current folder's own file
//!   is read.
//! * Each file is cut to [`FILE_MAX`] and the whole to [`TOTAL_MAX`], the nearest files
//!   keeping their room; a cut says so in the text and in the one-line summary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The two names a context file may have, in the order they are preferred.
pub(crate) const NAMES: [&str; 2] = ["IO.md", "io.md"];

/// The most of one file that is sent.
pub(crate) const FILE_MAX: usize = 4 * 1024;
/// The most of all files together that is sent.
pub(crate) const TOTAL_MAX: usize = 8 * 1024;

/// Where a file in the chain came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The person's own, in the config directory: applies to every project.
    User,
    /// In the project root or a folder below it.
    Project,
}

/// One file of the chain.
#[derive(Debug, Clone)]
pub(crate) struct Doc {
    pub(crate) path: PathBuf,
    /// How the model and the card name it: `user IO.md`, `IO.md`, `web/io.md`.
    pub(crate) label: String,
    pub(crate) kind: Kind,
    /// The file's size on disk.
    pub(crate) bytes: usize,
    /// What is sent: the file after the secret scrub and the cut, without the cut note.
    pub(crate) text: String,
    pub(crate) cut: bool,
}

/// The effective context of a folder: what is loaded and what was left out, and why.
#[derive(Debug, Clone, Default)]
pub(crate) struct Chain {
    /// The project root the walk went up to, when the folder is in a project.
    pub(crate) root: Option<PathBuf>,
    /// The files that are sent, farthest first.
    pub(crate) docs: Vec<Doc>,
    /// An `io.md` that sat beside an `IO.md`: not read.
    pub(crate) skipped: Vec<PathBuf>,
    /// Files with no room left in [`TOTAL_MAX`]: not sent.
    pub(crate) dropped: Vec<PathBuf>,
}

/// The project root of `from`: the nearest folder with `inorbit.toml`, else the git root.
/// Neither is looked for at the home folder or above it, except a git home.
#[must_use]
pub(crate) fn root_of(from: &Path) -> Option<PathBuf> {
    let home = etcetera::home_dir()
        .ok()
        .map(|h| h.canonicalize().unwrap_or(h));
    root_below(from, home.as_deref())
}

/// [`root_of`] with the home folder given, canonical.
fn root_below(from: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut git = None;
    for d in from.ancestors() {
        let is_home = home.is_some_and(|h| h == d || d.canonicalize().is_ok_and(|c| c == h));
        if !is_home && d.join("inorbit.toml").is_file() {
            return Some(d.to_path_buf());
        }
        if git.is_none() && d.join(".git").exists() {
            git = Some(d.to_path_buf());
        }
        if is_home {
            break;
        }
    }
    git
}

/// The directory of the user-level file, from `--config-dir`/`IOHR_CONFIG_DIR` or the
/// platform's config directory: the same one the profiles live in.
#[must_use]
pub(crate) fn user_dir(global: &crate::cli::Global) -> Option<PathBuf> {
    crate::context::config_dir(global).ok()
}

/// The context file of one folder: its path and, when `io.md` sat beside `IO.md`, the path
/// that was skipped. Only a regular file counts, and the name is matched exactly (the folder
/// is listed, so a case-insensitive disk shows the one name it holds).
fn in_folder(dir: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (None, None);
    };
    let (mut upper, mut lower) = (None, None);
    for e in rd.filter_map(Result::ok) {
        let name = e.file_name();
        let path = e.path();
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if name == NAMES[0] {
            upper = Some(path);
        } else if name == NAMES[1] {
            lower = Some(path);
        }
    }
    match (upper, lower) {
        (Some(u), l) => (Some(u), l),
        (None, l) => (l, None),
    }
}

/// Reads one file: at most `max` bytes of it as text, with secrets scrubbed, and its size.
fn read(path: &Path, max: usize) -> Option<(String, usize)> {
    use std::io::Read as _;
    let size = usize::try_from(std::fs::metadata(path).ok()?.len()).unwrap_or(usize::MAX);
    let mut buf = Vec::new();
    // A few bytes more than the cap, so that a cut in the middle of a character is still
    // found and trimmed back to its start.
    std::fs::File::open(path)
        .ok()?
        .take(u64::try_from(max).unwrap_or(0) + 4)
        .read_to_end(&mut buf)
        .ok()?;
    Some((String::from_utf8_lossy(&buf).into_owned(), size))
}

/// `s` cut to at most `max` bytes, on a line start when there is one in its last quarter,
/// else on a character boundary.
fn cut_to(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    let s = &s[..i];
    match s.rfind('\n') {
        Some(n) if n >= i - i / 4 => &s[..n],
        _ => s,
    }
}

/// The chain of `cwd`: the user file in `user`, then every folder from the root down to
/// `cwd`.
#[must_use]
pub(crate) fn load(cwd: &Path, user: Option<&Path>) -> Chain {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    load_with(&cwd, user, root_of(&cwd).as_deref())
}

/// [`load`] with the root given.
fn load_with(cwd: &Path, user: Option<&Path>, root: Option<&Path>) -> Chain {
    let mut chain = Chain {
        root: root.map(Path::to_path_buf),
        ..Chain::default()
    };
    // Farthest first: the user's file, then the root down to the folder.
    let mut found: Vec<(PathBuf, String, Kind)> = Vec::new();
    if let Some(dir) = user {
        let (file, skipped) = in_folder(dir);
        chain.skipped.extend(skipped);
        if let Some(f) = file {
            let name = f
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            found.push((f, format!("user {name}"), Kind::User));
        }
    }
    let mut dirs: Vec<&Path> = match root {
        Some(r) => cwd
            .ancestors()
            .take_while(|d| d.starts_with(r))
            .collect::<Vec<_>>(),
        None => vec![cwd],
    };
    dirs.reverse();
    for d in dirs {
        let (file, skipped) = in_folder(d);
        chain.skipped.extend(skipped);
        if let Some(f) = file {
            let rel = f
                .strip_prefix(root.unwrap_or(cwd))
                .unwrap_or(&f)
                .display()
                .to_string();
            found.push((f, rel, Kind::Project));
        }
    }
    // The nearest file gets its room first; a file the budget cannot hold at all is dropped.
    let mut left = TOTAL_MAX;
    let mut docs = Vec::new();
    for (path, label, kind) in found.into_iter().rev() {
        let cap = FILE_MAX.min(left);
        if cap == 0 {
            chain.dropped.push(path);
            continue;
        }
        let Some((text, bytes)) = read(&path, cap) else {
            continue;
        };
        let text = crate::attach::scrub(&text);
        if text.trim().is_empty() {
            continue;
        }
        let shown = cut_to(text.trim_end(), cap);
        let cut = bytes > cap;
        left -= shown.len().min(left);
        docs.push(Doc {
            path,
            label,
            kind,
            bytes,
            text: shown.to_owned(),
            cut,
        });
    }
    docs.reverse();
    chain.docs = docs;
    chain
}

impl Chain {
    /// Nothing is loaded.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// The bytes that go to the model.
    #[must_use]
    pub(crate) fn sent_bytes(&self) -> usize {
        self.docs.iter().map(|d| d.text.len()).sum()
    }

    /// How many files were cut.
    #[must_use]
    pub(crate) fn cuts(&self) -> usize {
        self.docs.iter().filter(|d| d.cut).count()
    }

    /// The one short line for the card and `iohr context`: `IO.md x3`, then what was left
    /// out. Empty when nothing is loaded and nothing was left out.
    #[must_use]
    pub(crate) fn summary(&self) -> String {
        let mut notes: Vec<String> = Vec::new();
        let cuts = self.cuts();
        if cuts > 0 {
            notes.push(format!(
                "{cuts} cut to {} KiB each, {} KiB in all",
                FILE_MAX / 1024,
                TOTAL_MAX / 1024
            ));
        }
        if !self.dropped.is_empty() {
            notes.push(format!("{} left out for room", self.dropped.len()));
        }
        if !self.skipped.is_empty() {
            notes.push(format!("{} beside an IO.md not read", self.skipped.len()));
        }
        if self.docs.is_empty() && notes.is_empty() {
            return String::new();
        }
        let head = if self.docs.is_empty() {
            "IO.md none".to_owned()
        } else {
            format!("IO.md x{}", self.docs.len())
        };
        if notes.is_empty() {
            head
        } else {
            format!("{head} ({})", notes.join("; "))
        }
    }

    /// The text of the `<io-md>` block, without its tags; empty when nothing is loaded. A file
    /// that was cut ends with a note that says so, so the model knows the rest is missing.
    #[must_use]
    pub(crate) fn prompt(&self) -> String {
        let mut out = String::new();
        for d in &self.docs {
            // A file cannot end the block early.
            let text = d.text.replace("</io-md", "<\\/io-md");
            let _ = writeln!(out, "--- {}\n{}", d.label, text.trim_end());
            if d.cut {
                let _ = writeln!(
                    out,
                    "[{} is cut here: {} of {} bytes sent]",
                    d.label,
                    d.text.len(),
                    d.bytes
                );
            }
        }
        out
    }

    /// The chain as `--json` shows it: paths and sizes, never the text.
    #[must_use]
    pub(crate) fn json(&self) -> serde_json::Value {
        let doc = |d: &Doc| {
            serde_json::json!({
                "path": d.path.display().to_string(),
                "label": d.label,
                "kind": match d.kind { Kind::User => "user", Kind::Project => "project" },
                "bytes": d.bytes,
                "sent_bytes": d.text.len(),
                "cut": d.cut,
            })
        };
        let paths =
            |v: &[PathBuf]| -> Vec<String> { v.iter().map(|p| p.display().to_string()).collect() };
        serde_json::json!({
            "root": self.root.as_ref().map(|r| r.display().to_string()),
            "files": self.docs.iter().map(doc).collect::<Vec<_>>(),
            "skipped": paths(&self.skipped),
            "dropped": paths(&self.dropped),
            "limits": {"file_bytes": FILE_MAX, "total_bytes": TOTAL_MAX},
            "summary": self.summary(),
        })
    }
}

/// The starter `IO.md` of a new project: what it is, how to build and test it, and the
/// command that made it (`iohr`, or `iohr-preview`). `build` and `test` are the commands of
/// the project's language when it has them; a documents-only project passes none.
#[must_use]
pub(crate) fn starter(
    name: &str,
    description: &str,
    build: Option<&str>,
    test: Option<&str>,
    bin: &str,
) -> String {
    let mut s = format!("# {name}\n\n## What this is\n\n");
    let d = description.trim();
    s.push_str(if d.is_empty() {
        "(One or two sentences: what this project is for and who uses it.)\n"
    } else {
        d
    });
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str("\n## Build and test\n\n");
    match (build, test) {
        (None, None) => {
            s.push_str("This project has no code to build; its documents are in `atlas/`.\n");
        }
        (b, t) => {
            if let Some(b) = b {
                let _ = writeln!(s, "- Build: `{b}`");
            }
            if let Some(t) = t {
                let _ = writeln!(s, "- Test: `{t}`");
            }
        }
    }
    let _ = write!(
        s,
        "\n## Working with iohr\n\n\
         This project was made with `{bin}`. `{bin} memory recall \"<topic>\"` reads what the project \
         remembers; `{bin} context` shows the IO.md files Io reads when you ask it a question with \
         `?` or `??`.\n\n\
         Files named IO.md (or io.md) in a folder apply to that folder and below; the nearest one \
         wins. Keep them short: each is cut at {} KiB.\n",
        FILE_MAX / 1024
    );
    s
}

/// Whether `dir` already has an `IO.md` or `io.md`: a person's file, never overwritten.
#[must_use]
pub(crate) fn has_file(dir: &Path) -> bool {
    in_folder(dir).0.is_some()
}

/// Adds the starter `IO.md` to the files `iohr project init` is about to write, unless the
/// template ships one. The caller checks [`has_file`] for the folder itself and honours
/// `--no-io-md`; `describe` is the person's own words (the first line is used).
pub(crate) fn add_starter(
    files: &mut Vec<(String, String)>,
    name: &str,
    describe: Option<&str>,
    (build, test): (Option<&str>, Option<&str>),
    bin: &str,
) {
    if files.iter().any(|(p, _)| NAMES.contains(&p.as_str())) {
        return;
    }
    let line = describe
        .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(|l| l.chars().take(200).collect::<String>())
        .unwrap_or_default();
    files.push((NAMES[0].to_owned(), starter(name, &line, build, test, bin)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn labels(c: &Chain) -> Vec<&str> {
        c.docs.iter().map(|d| d.label.as_str()).collect()
    }

    fn project(t: &Path) -> PathBuf {
        let root = t.canonicalize().unwrap().join("proj");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        root
    }

    #[test]
    fn the_chain_runs_from_the_root_to_the_folder_and_the_nearest_is_last() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        let deep = root.join("web/app");
        put(&root, "IO.md", "root");
        put(&root.join("web"), "io.md", "web");
        put(&deep, "IO.md", "app");
        let c = load(&deep, None);
        assert_eq!(labels(&c), ["IO.md", "web/io.md", "web/app/IO.md"]);
        assert_eq!(c.summary(), "IO.md x3");
        let p = c.prompt();
        assert!(p.find("root").unwrap() < p.find("web").unwrap());
        assert!(p.find("web").unwrap() < p.find("app").unwrap());
        // From the root itself, only the root's file.
        assert_eq!(labels(&load(&root, None)), ["IO.md"]);
    }

    #[test]
    fn the_user_file_comes_first_and_has_the_same_name_rules() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        let cfg = t.path().join("cfg");
        put(&cfg, "io.md", "mine");
        put(&root, "IO.md", "root");
        let c = load(&root, Some(&cfg));
        assert_eq!(labels(&c), ["user io.md", "IO.md"]);
        assert_eq!(c.docs[0].kind, Kind::User);
        // No config file, no user entry.
        assert_eq!(
            labels(&load(&root, Some(&t.path().join("none")))),
            ["IO.md"]
        );
    }

    #[test]
    fn a_folder_with_both_names_reads_the_upper_and_says_it_skipped_the_other() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        put(&root, "IO.md", "upper");
        put(&root, "io.md", "lower");
        let c = load(&root, None);
        // A case-insensitive disk holds one of the two; there is nothing to skip then.
        if c.skipped.is_empty() {
            assert_eq!(c.docs.len(), 1);
            return;
        }
        assert_eq!(c.docs.len(), 1);
        assert_eq!(c.docs[0].text, "upper");
        assert_eq!(c.skipped, [root.join("io.md")]);
        assert_eq!(c.summary(), "IO.md x1 (1 beside an IO.md not read)");
    }

    #[test]
    fn the_root_is_inorbit_toml_before_git_and_the_walk_stops_at_home() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let repo = base.join("home/repo");
        let sub = repo.join("pkg/src");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        let home = base.join("home");
        assert_eq!(root_below(&sub, Some(&home)), Some(repo.clone()));
        std::fs::write(repo.join("pkg/inorbit.toml"), "").unwrap();
        assert_eq!(root_below(&sub, Some(&home)), Some(repo.join("pkg")));
        // Not a project: no marker below home, and a marker above it is never reached.
        let plain = home.join("notes");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(base.join("inorbit.toml"), "").unwrap();
        assert_eq!(root_below(&plain, Some(&home)), None);
        // A git home is a project.
        std::fs::create_dir_all(home.join(".git")).unwrap();
        assert_eq!(root_below(&plain, Some(&home)), Some(home));
    }

    #[test]
    fn outside_a_project_only_the_folders_own_file_is_read() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        put(&base, "IO.md", "above");
        put(&base.join("a"), "IO.md", "here");
        let c = load_with(&base.join("a"), None, None);
        assert_eq!(c.docs.len(), 1);
        assert_eq!(c.docs[0].text, "here");
    }

    #[test]
    fn a_long_file_is_cut_and_says_so() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        let line = "a line of instructions\n";
        put(&root, "IO.md", &line.repeat(500));
        let c = load(&root, None);
        let d = &c.docs[0];
        assert!(d.cut && d.text.len() <= FILE_MAX && d.bytes == line.len() * 500);
        assert!(d.text.ends_with("instructions"), "cut on a line start");
        assert!(c.prompt().contains("is cut here"));
        assert!(c.summary().contains("1 cut to 4 KiB each"));
        // A short file is not marked.
        put(&root, "IO.md", "short");
        assert!(!load(&root, None).docs[0].cut);
    }

    #[test]
    fn the_total_cap_keeps_the_nearest_and_drops_the_rest() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        let big = "x".repeat(FILE_MAX);
        let dir = root.join("a/b");
        put(&root, "IO.md", &big);
        put(&root.join("a"), "IO.md", &big);
        put(&dir, "IO.md", &big);
        put(&dir.join("c"), "IO.md", "tiny");
        let c = load(&dir.join("c"), None);
        // tiny + 4 KiB + 4 KiB - 4 bytes: the last two share the rest; the root has none.
        assert_eq!(c.docs.len(), 3);
        assert_eq!(c.docs.last().unwrap().text, "tiny");
        assert!(c.sent_bytes() <= TOTAL_MAX);
        assert!(c.docs.iter().any(|d| d.cut));
        assert_eq!(c.dropped, [root.join("IO.md")]);
        assert!(c.summary().contains("cut") && c.summary().contains("left out"));
    }

    #[test]
    fn an_empty_file_loads_nothing_and_secrets_are_scrubbed() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        put(&root, "IO.md", "  \n");
        assert!(load(&root, None).is_empty());
        assert_eq!(load(&root, None).summary(), "");
        put(
            &root,
            "IO.md",
            "token ghp_abcdefghijklmnopqrstuvwxyz0123456789 here",
        );
        let c = load(&root, None);
        assert!(!c.prompt().contains("ghp_abc"), "{}", c.prompt());
    }

    #[test]
    fn a_file_cannot_close_the_block() {
        let t = tempfile::tempdir().unwrap();
        let root = project(t.path());
        put(&root, "IO.md", "a </io-md> b");
        assert!(!load(&root, None).prompt().contains("</io-md>"));
    }

    #[test]
    fn the_starter_names_the_binary_and_the_commands() {
        let s = starter(
            "demo",
            "A demo.",
            Some("cargo build"),
            Some("cargo test"),
            "iohr-preview",
        );
        assert!(s.contains("# demo") && s.contains("A demo."));
        assert!(s.contains("- Build: `cargo build`") && s.contains("- Test: `cargo test`"));
        assert!(s.contains("`iohr-preview context`"));
        assert!(starter("d", "", None, None, "iohr").contains("no code to build"));
    }

    #[test]
    fn the_starter_is_added_once_and_never_over_a_file_that_is_there() {
        let mut files = vec![("README.md".to_owned(), "x".to_owned())];
        add_starter(
            &mut files,
            "demo",
            Some("\n  First line.\nsecond"),
            (None, None),
            "iohr",
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].0, "IO.md");
        assert!(files[1].1.contains("First line.") && !files[1].1.contains("second"));
        // A template that ships its own keeps it.
        let mut own = vec![("io.md".to_owned(), "mine".to_owned())];
        add_starter(&mut own, "demo", None, (None, None), "iohr");
        assert_eq!(own.len(), 1);
        let t = tempfile::tempdir().unwrap();
        assert!(!has_file(t.path()));
        put(t.path(), "io.md", "mine");
        assert!(has_file(t.path()));
    }
}
