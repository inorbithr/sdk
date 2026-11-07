//! Ask, never "I can't" (PRD 0187, RFC 0203): when a `?` run would get no file tools, because
//! the folder is the home folder or the root, or because the question names a folder the run
//! was not started in, Io asks to read that folder. This module decides what to ask for and
//! words the note; the card, the terminal ask and the stored yes live where they already do
//! ([`crate::permissions::trust`], [`crate::actions`]).

use std::path::{Path, PathBuf};

use crate::headless::{Blocked, blocked};
use crate::permissions::secrets;

/// Why Io asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// The question names a folder outside the one it was asked in.
    Named,
    /// The folder is the home folder: too broad to read unasked.
    Home,
    /// The folder is the filesystem root: too broad to read unasked.
    Root,
    /// The folder cannot be listed: no yes would open it, so nothing is asked.
    Unreadable,
}

impl Why {
    fn name(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::Home => "home",
            Self::Root => "root",
            Self::Unreadable => "unreadable",
        }
    }
}

/// A request as `--json` shows it.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Shown {
    pub(crate) folder: String,
    pub(crate) why: &'static str,
    /// Whether a yes would help (false: the folder cannot be listed).
    pub(crate) askable: bool,
    /// How to give the yes for one run; null when it would not help.
    pub(crate) allow_with: Option<String>,
}

/// One folder Io needs a yes for, before it can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The folder as the system names it (canonical).
    pub(crate) folder: PathBuf,
    /// It is the folder the question was asked in, not one the question names.
    pub(crate) is_cwd: bool,
    pub(crate) why: Why,
}

impl Request {
    /// The request as data.
    #[must_use]
    pub(crate) fn shown(&self) -> Shown {
        let folder = self.folder.display().to_string();
        Shown {
            allow_with: self.askable().then(|| format!("--read {folder}")),
            why: self.why.name(),
            askable: self.askable(),
            folder,
        }
    }

    /// Whether a yes would help: false for a folder that cannot be listed.
    #[must_use]
    pub(crate) fn askable(&self) -> bool {
        self.why != Why::Unreadable
    }
}

/// A word of the question, as a path when it looks like one: quotes and trailing punctuation
/// off, no URL, no control character.
fn word_path(word: &str) -> Option<&str> {
    let w = word
        .trim_matches(['"', '\'', '`', '(', ')', '[', ']', '<', '>'])
        .trim_end_matches([',', ';', ':', '?', '!', '.']);
    let path_like = w == "~"
        || w.starts_with("~/")
        || w.starts_with('/')
        || w.starts_with("./")
        || w.starts_with("../")
        || w.contains('/');
    (path_like && w.len() <= 512 && !w.contains("://") && !w.chars().any(char::is_control))
        .then_some(w)
}

/// The folders the question names (`/etc/nginx`, `~/Downloads/x`, `../api`, `src/`), in order,
/// each one that exists, can be listed, is not the folder it was asked in and is not a secret
/// place. A bare name such as `Downloads` is the model's to offer, not guessed here.
pub(crate) fn named_folders<'a>(
    question: &'a str,
    cwd: &'a Path,
    home: Option<&'a Path>,
) -> impl Iterator<Item = PathBuf> + 'a {
    let here = cwd.canonicalize().ok();
    question.split_whitespace().filter_map(move |word| {
        let w = word_path(word)?;
        let at = if w == "~" {
            home?.to_path_buf()
        } else if let Some(rest) = w.strip_prefix("~/") {
            home?.join(rest)
        } else {
            cwd.join(w)
        };
        let real = allowable_at(&at, cwd, home)?;
        (Some(&real) != here.as_ref()).then_some(real)
    })
}

/// `path` with a leading `~` or `~/` taken from `home`; any other path as it is. Config and
/// environment values are written with it, as a question is.
#[must_use]
pub(crate) fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.to_str(), home) {
        (Some("~"), Some(h)) => h.to_path_buf(),
        (Some(p), Some(h)) => p
            .strip_prefix("~/")
            .map_or_else(|| path.to_path_buf(), |r| h.join(r)),
        _ => path.to_path_buf(),
    }
}

/// `folder` as the system names it, when Io may be allowed to read it: it exists, is a folder
/// that can be listed and is not a secret place. A relative path is taken from `cwd`.
#[must_use]
pub(crate) fn allowable_at(folder: &Path, cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let real = cwd.join(folder).canonicalize().ok()?;
    let listable = real.is_dir() && std::fs::read_dir(&real).is_ok();
    let secret = secrets::is_secret_path(&real.display().to_string(), cwd, home);
    (listable && !secret).then_some(real)
}

/// [`allowable_at`] against the real home folder, as a yes/no.
#[must_use]
pub(crate) fn allowable(folder: &Path, cwd: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    allowable_at(folder, cwd, home.as_deref()).is_some()
}

/// What Io still needs a yes for: the folder the question names when the run cannot already
/// open it (`reads` says the run may open files in the working folder, `also` are the folders
/// already allowed besides it, the named one or one above it among them), else the working folder itself when it is too broad. None
/// when the run can read what the question is about.
#[must_use]
pub(crate) fn needed(
    question: &str,
    cwd: &Path,
    home: Option<&Path>,
    (reads, also): (bool, &[PathBuf]),
) -> Option<Request> {
    let here = cwd.canonicalize().ok();
    let mut named = named_folders(question, cwd, home).peekable();
    if named.peek().is_some() {
        // The first folder the run cannot open yet; none when it can open all of them.
        let folder = named.find(|f| {
            let inside = reads && here.as_ref().is_some_and(|c| f.starts_with(c));
            !(inside || also.iter().any(|a| f.starts_with(a)))
        })?;
        return Some(Request {
            folder,
            is_cwd: false,
            why: Why::Named,
        });
    }
    let why = match blocked(cwd, home)? {
        Blocked::Root => Why::Root,
        Blocked::Home => Why::Home,
        Blocked::Unreadable => Why::Unreadable,
    };
    if reads {
        return None;
    }
    Some(Request {
        folder: cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()),
        is_cwd: true,
        why,
    })
}

/// `text` with no control character, for the terminal.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The one quiet line under the card for a folder Io did not read: what it needs and how to
/// give it (the key in the menu when the hook is there, else the flag). A folder that cannot
/// be listed says so, and offers nothing.
#[must_use]
pub(crate) fn note(req: &Request, key: Option<&str>) -> String {
    let folder = clean(&req.folder.display().to_string());
    if !req.askable() {
        return format!("Io cannot open {folder}: the system does not let it list that folder");
    }
    match key {
        Some(k) => format!("Io needs your OK to read {folder} \u{b7} {k} then l to allow"),
        None => {
            format!("Io needs your OK to read {folder} \u{b7} ask again with --read {folder}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join("work/api")).unwrap();
        std::fs::create_dir_all(home.join("Downloads/x")).unwrap();
        (d, home)
    }

    #[test]
    fn a_broad_folder_is_asked_for_and_never_answered_blind() {
        let (_d, home) = tree();
        let r = needed("explain this directory", &home, Some(&home), (false, &[])).unwrap();
        assert_eq!(r.why, Why::Home);
        assert!(r.is_cwd && r.askable());
        assert_eq!(r.folder, home);
        let root = needed("what is here", Path::new("/"), None, (false, &[])).unwrap();
        assert_eq!(root.why, Why::Root);
        assert!(root.askable());
    }

    #[test]
    fn a_readable_folder_asks_nothing() {
        let (_d, home) = tree();
        let work = home.join("work");
        assert_eq!(
            needed("explain this directory", &work, Some(&home), (true, &[])),
            None
        );
        // Granted already: home with its yes in hand.
        assert_eq!(needed("explain", &home, Some(&home), (true, &[])), None);
    }

    #[test]
    fn an_unreadable_folder_is_said_not_asked() {
        let (_d, home) = tree();
        let gone = home.join("gone");
        let r = needed("explain", &gone, Some(&home), (false, &[])).unwrap();
        assert_eq!(r.why, Why::Unreadable);
        assert!(!r.askable());
        assert!(note(&r, Some("\u{2193}")).contains("cannot open"));
    }

    #[test]
    fn a_folder_the_question_names_outside_the_working_one_is_asked_for() {
        let (_d, home) = tree();
        let work = home.join("work");
        let q = format!("what is in {}/Downloads/x?", home.display());
        let r = needed(&q, &work, Some(&home), (true, &[])).unwrap();
        assert_eq!(r.why, Why::Named);
        assert!(!r.is_cwd);
        assert_eq!(r.folder, home.join("Downloads/x"));
        // `~/` and `../` name folders too.
        let tilde = needed("list ~/Downloads", &work, Some(&home), (true, &[])).unwrap();
        assert_eq!(tilde.folder, home.join("Downloads"));
        let up = needed(
            "look at ../Downloads please",
            &work,
            Some(&home),
            (true, &[]),
        )
        .unwrap();
        assert_eq!(up.folder, home.join("Downloads"));
    }

    #[test]
    fn a_named_folder_inside_a_readable_working_folder_or_already_allowed_is_not_asked() {
        let (_d, home) = tree();
        let work = home.join("work");
        assert_eq!(
            needed("explain api/", &work, Some(&home), (true, &[])),
            None
        );
        let down = home.join("Downloads");
        assert_eq!(
            needed(
                "list ~/Downloads",
                &work,
                Some(&home),
                (true, std::slice::from_ref(&down))
            ),
            None
        );
        // Home is broad: a folder inside it is still asked for until home itself is allowed.
        assert!(needed("explain work/api", &home, Some(&home), (false, &[])).is_some());
    }

    #[test]
    fn a_folder_is_allowed_by_any_of_several_or_by_one_it_sits_in() {
        let (_d, home) = tree();
        let work = home.join("work");
        let (down, api) = (home.join("Downloads"), work.join("api"));
        let also = [api, down];
        assert_eq!(
            needed("list ~/Downloads/x", &work, Some(&home), (true, &also)),
            None
        );
        assert_eq!(
            needed("list ~/Downloads", &work, Some(&home), (true, &also)),
            None
        );
        std::fs::create_dir_all(home.join("Other")).unwrap();
        let r = needed("list ~/Other", &work, Some(&home), (true, &also)).unwrap();
        assert_eq!(r.folder, home.join("Other"));
    }

    #[test]
    fn a_leading_tilde_is_the_home_folder_and_nothing_else_is_changed() {
        let home = Path::new("/home/me");
        assert_eq!(expand_home(Path::new("~"), Some(home)), home);
        assert_eq!(
            expand_home(Path::new("~/a/b"), Some(home)),
            home.join("a/b")
        );
        assert_eq!(
            expand_home(Path::new("/x/~/y"), Some(home)),
            Path::new("/x/~/y")
        );
        assert_eq!(
            expand_home(Path::new("~me/x"), Some(home)),
            Path::new("~me/x")
        );
        assert_eq!(expand_home(Path::new("~/a"), None), Path::new("~/a"));
    }

    #[test]
    fn words_that_are_not_folders_name_nothing() {
        let (_d, home) = tree();
        let work = home.join("work");
        for q in [
            "and/or",
            "see https://example.com/a/b",
            "what is Downloads",
            "explain ../nothing-here",
        ] {
            assert_eq!(named_folders(q, &work, Some(&home)).next(), None, "{q}");
        }
        // A file is not a folder.
        std::fs::write(work.join("a.txt"), "x").unwrap();
        assert_eq!(
            named_folders("read ./a.txt", &work, Some(&home)).next(),
            None
        );
    }

    #[test]
    fn a_secret_place_is_never_offered() {
        let (_d, home) = tree();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let work = home.join("work");
        assert_eq!(
            named_folders("list ~/.ssh", &work, Some(&home)).next(),
            None
        );
    }

    #[test]
    fn the_note_names_the_key_or_the_flag() {
        let r = Request {
            folder: PathBuf::from("/data/x"),
            is_cwd: false,
            why: Why::Named,
        };
        assert_eq!(
            note(&r, Some("\u{2193}")),
            "Io needs your OK to read /data/x \u{b7} \u{2193} then l to allow"
        );
        assert_eq!(
            note(&r, None),
            "Io needs your OK to read /data/x \u{b7} ask again with --read /data/x"
        );
    }
}
