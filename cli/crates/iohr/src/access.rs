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
    /// The folder holds or sits above secret places (`/Users`, `~/.config`, `~/Library`):
    /// no yes opens it whole, so nothing is asked.
    Broad,
    /// The folder cannot be listed: no yes would open it, so nothing is asked.
    Unreadable,
}

impl Why {
    fn name(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::Home => "home",
            Self::Root => "root",
            Self::Broad => "broad",
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
    /// How to give the yes for one run; null when it would not help (and for the filesystem
    /// root, which `--read` never opens).
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
            allow_with: (self.askable() && self.why != Why::Root)
                .then(|| format!("--read {folder}")),
            why: self.why.name(),
            askable: self.askable(),
            folder,
        }
    }

    /// Whether a yes would help: false for a folder that cannot be listed or is too broad.
    #[must_use]
    pub(crate) fn askable(&self) -> bool {
        !matches!(self.why, Why::Unreadable | Why::Broad)
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
        || w.contains('/')
        // Windows also writes `C:\Users\me`, `..\api` and `~\Downloads`.
        || (cfg!(windows) && w.contains('\\'));
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
        // Both slashes separate on Windows, but not after a `\\?\` prefix, which
        // `canonicalize` gives: a `/` there is part of the name.
        let w = if cfg!(windows) {
            std::borrow::Cow::Owned(w.replace('/', "\\"))
        } else {
            std::borrow::Cow::Borrowed(w)
        };
        let home_rest = w
            .strip_prefix("~/")
            .or_else(|| w.strip_prefix("~\\").filter(|_| cfg!(windows)));
        let at = if w == "~" {
            home?.to_path_buf()
        } else if let Some(rest) = home_rest {
            home?.join(rest)
        } else {
            cwd.join(&*w)
        };
        let real = allowable_at(&at, cwd, home, Source::Card).ok()?;
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

/// Where a yes for a folder came from; the audit line and `--json` name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// `--read` (alias `--add-dir`): named by the person for this run.
    Flag,
    /// `IOHR_ASK_READ`.
    Env,
    /// `[ask] read`.
    Config,
    /// A yes kept in `trusted.toml`: a folder from Add folder, or an Allow or Always for
    /// the question being asked.
    Trust,
    /// The card, asked at this question: a yes once or always, or a folder the question names.
    Card,
}

impl Source {
    /// The word `permissions.log` and `--json` use.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::Config => "config",
            Self::Trust => "trust",
            Self::Card => "card",
        }
    }

    /// Whether this way of asking may name the home folder itself. Only a person's own words
    /// for this run may: the flag the Home card offers, and the card asked at the question.
    /// The environment, the config and what is kept never open it.
    fn names_home(self) -> bool {
        matches!(self, Self::Flag | Self::Card)
    }

    /// Whether this way of asking may name the filesystem root: only the card, at the
    /// question.
    fn names_root(self) -> bool {
        self == Self::Card
    }
}

/// One folder a run was given, and where the yes came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    pub(crate) source: Source,
    /// The folder as the system names it (canonical).
    pub(crate) folder: PathBuf,
}

/// Why a folder may not be read, in words for the person who named it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// It is missing, is not a folder, or cannot be listed.
    Unreadable,
    /// It is a secret place, or inside one.
    Secret,
    /// It is the filesystem root, the home folder, or holds secret places below it.
    Broad,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "not a folder Io can read (it must exist and be listable)",
            Self::Secret => "a secret place: Io never reads it",
            Self::Broad => {
                "too broad: it is the root, your home folder or holds secret folders such as \
                 ~/.ssh or ~/.config; name a project folder instead"
            }
        })
    }
}

/// `path` as the system names it (symlinks resolved), or none when it does not exist. The one
/// place a folder is made canonical: the check below, the trust store and the card all use it.
#[must_use]
pub(crate) fn canonical(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

/// `p` lower-cased, for comparing places on a file system that ignores case (macOS, Windows):
/// `~/.SSH` is `~/.ssh` there. On a file system that keeps case this over-refuses at worst.
fn folded(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

/// Whether `real` (canonical) is too broad or too close to secrets to hand to a run whole:
/// the filesystem root, the home folder, every folder above the home folder (`/Users`,
/// `/home`), every folder above one of [`secrets::SECRET_DIRS`] (`~/.config`, `~/Library`),
/// and those folders themselves or anything inside them. A run handed `~/.config` could list
/// `gcloud` and `gh` beside it, so the secret rules alone, which look at one path, are not
/// enough. The one place this is decided: [`allowable_at`] and the working folder
/// ([`crate::headless::blocked`]) both ask it.
#[must_use]
pub(crate) fn broad_or_secret_bearing(real: &Path, home: Option<&Path>) -> bool {
    if real.parent().is_none() {
        return true;
    }
    let Some(home) = home else { return false };
    let home = canonical(home).unwrap_or_else(|| home.to_path_buf());
    let at = folded(real);
    // The home folder and everything above it.
    if folded(&home).starts_with(&at) {
        return true;
    }
    secrets::SECRET_DIRS.iter().any(|d| {
        let secret = folded(&home.join(d));
        secret.starts_with(&at) || at.starts_with(&secret)
    })
}

/// `folder` as the system names it, when Io may be allowed to read it, or why not: it exists,
/// is a folder that can be listed, is not a secret place and is not too broad
/// ([`broad_or_secret_bearing`]). A relative path is taken from `cwd`. The one checker every
/// way of granting a folder goes through (`--read`, `IOHR_ASK_READ`, `[ask] read`, Add folder,
/// the kept folders and the card); `source` says whether the home folder or the root itself may
/// be named ([`Source`]).
///
/// # Errors
///
/// The [`Refusal`] that says why not.
pub(crate) fn allowable_at(
    folder: &Path,
    cwd: &Path,
    home: Option<&Path>,
    source: Source,
) -> Result<PathBuf, Refusal> {
    let real = canonical(&cwd.join(folder)).ok_or(Refusal::Unreadable)?;
    if !real.is_dir() || std::fs::read_dir(&real).is_err() {
        return Err(Refusal::Unreadable);
    }
    if secrets::is_secret_path(&real.display().to_string(), cwd, home) {
        return Err(Refusal::Secret);
    }
    let home_real = home.and_then(canonical);
    let exactly_home = home_real.as_deref() == Some(real.as_path());
    let exactly_root = real.parent().is_none();
    if (exactly_home && source.names_home()) || (exactly_root && source.names_root()) {
        return Ok(real);
    }
    if broad_or_secret_bearing(&real, home) {
        return Err(Refusal::Broad);
    }
    Ok(real)
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
        Blocked::Broad => Why::Broad,
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
/// be listed, or is too broad, says so and offers nothing; the filesystem root has no flag.
#[must_use]
pub(crate) fn note(req: &Request, key: Option<&str>) -> String {
    let folder = clean(&req.folder.display().to_string());
    match req.why {
        Why::Unreadable => {
            return format!("Io cannot open {folder}: the system does not let it list that folder");
        }
        Why::Broad => {
            return format!(
                "Io will not read {folder}: it holds secret folders; ask from a project folder instead"
            );
        }
        Why::Root if key.is_none() => {
            return format!(
                "Io needs your OK to read {folder} \u{b7} ask from a project folder instead"
            );
        }
        _ => {}
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
        // A Windows path, written with its own separator, is named as well.
        #[cfg(windows)]
        {
            let q = format!("what is in {}?", home.join("Downloads").join("x").display());
            let r = needed(&q, &work, Some(&home), (true, &[])).unwrap();
            assert_eq!(r.folder, home.join("Downloads/x"));
            let up = needed("look at ..\\Downloads", &work, Some(&home), (true, &[])).unwrap();
            assert_eq!(up.folder, home.join("Downloads"));
        }
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

    #[test]
    fn broad_or_secret_bearing_refuses_root_home_and_every_folder_above_a_secret() {
        let (_d, home) = tree();
        let h = Some(home.as_path());
        // The root, the home folder and everything above it.
        assert!(broad_or_secret_bearing(Path::new("/"), h));
        assert!(broad_or_secret_bearing(&home, h));
        let above = home.parent().unwrap();
        assert!(broad_or_secret_bearing(above, h), "{}", above.display());
        // Folders that hold secret places below them, and the secret places themselves.
        for dir in [
            ".config",
            "Library",
            ".ssh",
            ".aws",
            ".gnupg",
            ".kube",
            "Library/Keychains",
            ".ssh/keys",
            ".config/gcloud",
        ] {
            assert!(broad_or_secret_bearing(&home.join(dir), h), "{dir}");
        }
        // A case variant is the same place on macOS and Windows.
        assert!(broad_or_secret_bearing(&home.join(".SSH"), h));
        assert!(broad_or_secret_bearing(&home.join("LIBRARY"), h));
        // A project folder passes, and so does one that only looks like a secret one.
        assert!(!broad_or_secret_bearing(&home.join("work"), h));
        assert!(!broad_or_secret_bearing(&home.join("work/api"), h));
        assert!(!broad_or_secret_bearing(&home.join(".sshfs-notes"), h));
        // Without a home folder only the root is known to be broad.
        assert!(broad_or_secret_bearing(Path::new("/"), None));
        assert!(!broad_or_secret_bearing(&home.join("work"), None));
    }

    #[test]
    fn every_way_of_granting_goes_through_the_same_check_and_only_a_person_names_home() {
        let (_d, home) = tree();
        let work = home.join("work");
        let h = Some(home.as_path());
        std::fs::create_dir_all(home.join(".config/gh")).unwrap();
        std::fs::create_dir_all(home.join("Library/Keychains")).unwrap();
        for source in [
            Source::Flag,
            Source::Env,
            Source::Config,
            Source::Trust,
            Source::Card,
        ] {
            for dir in [".config", "Library", ".config/gh", "Library/Keychains"] {
                assert!(
                    allowable_at(&home.join(dir), &work, h, source).is_err(),
                    "{source:?} {dir}"
                );
            }
            assert_eq!(
                allowable_at(&home.join("Downloads"), &work, h, source),
                Ok(home.join("Downloads")),
                "{source:?}"
            );
            assert_eq!(
                allowable_at(Path::new("/"), &work, h, source).is_ok(),
                source == Source::Card,
                "{source:?} root"
            );
            assert_eq!(
                allowable_at(&home, &work, h, source).is_ok(),
                matches!(source, Source::Flag | Source::Card),
                "{source:?} home"
            );
        }
        assert_eq!(
            allowable_at(&home.join("work/../.config"), &work, h, Source::Env),
            Err(Refusal::Broad)
        );
    }
}
