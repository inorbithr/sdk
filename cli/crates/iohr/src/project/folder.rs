//! Where a new project goes, and what to call it (PRD 0088.1): proposals, not a hard-coded
//! folder. The default is `./NAME` in the folder you are in (the way `cargo new` and
//! `iohr ext init` work); a menu at a terminal adds the others with a reason each, and
//! anything without a terminal takes the first. All pure but the two `git` reads, so the
//! rules are tested with a temporary folder.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use super::check_name;

/// Which proposal this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `./NAME`: a new folder in the folder you are in.
    Here,
    /// The folder you are in, because it is empty (or holds only dotfiles).
    ThisFolder,
    /// `ROOT/NAME` from `[project] root`.
    Root,
    /// `../NAME`, beside the git repository you are in, so it does not nest.
    Sibling,
    /// Any path: asked for after the menu.
    Elsewhere,
    /// `--dir`.
    Given,
}

/// One place the project could go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Proposal {
    pub kind: Kind,
    /// The folder, absolute (empty for [`Kind::Elsewhere`], which is asked for).
    pub dir: PathBuf,
    /// The folder as a person reads it: relative to where they are (`./my-shop`).
    pub shown: String,
    /// Why this one, in a line.
    pub reason: String,
    /// The folder exists and holds something, so nothing may be written there.
    pub busy: bool,
}

/// What the proposals are made from.
#[derive(Debug, Clone)]
pub struct Where {
    pub cwd: PathBuf,
    /// The top of the git work tree the cwd is in, if it is in one.
    pub git_top: Option<PathBuf>,
    /// `[project] root`, as written.
    pub root: Option<String>,
    pub home: Option<PathBuf>,
}

impl Where {
    /// Reads the environment: the current folder, its git work tree, the home folder.
    #[must_use]
    pub fn here(root: Option<String>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let git_top = git_top(&cwd);
        Self {
            cwd,
            git_top,
            root,
            home: etcetera::home_dir().ok(),
        }
    }
}

/// The top of the git work tree `dir` is in. `None` when it is in none, or git did not answer.
#[must_use]
pub fn git_top(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

fn same(a: &Path, b: &Path) -> bool {
    let (x, y) = (a.canonicalize(), b.canonicalize());
    match (x, y) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Whether `inner` is `outer` or below it.
fn within(inner: &Path, outer: &Path) -> bool {
    let i = inner.canonicalize().unwrap_or_else(|_| inner.to_path_buf());
    let o = outer.canonicalize().unwrap_or_else(|_| outer.to_path_buf());
    i.starts_with(o)
}

/// `p` as a person reads it from `cwd`: `.`, `./x`, `../x` (up to three levels), else the
/// absolute path with the home folder as `~`.
#[must_use]
pub fn show(cwd: &Path, p: &Path, home: Option<&Path>) -> String {
    let (c, t): (Vec<Component>, Vec<Component>) =
        (cwd.components().collect(), p.components().collect());
    let common = c.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let ups = c.len() - common;
    // Nothing in common beyond the root is no relation worth a `../../..`.
    if common > 1 && ups <= 3 {
        let rest: PathBuf = t[common..].iter().collect();
        let rest = rest.to_string_lossy();
        return match (ups, rest.is_empty()) {
            (0, true) => ".".to_owned(),
            (0, false) => format!("./{rest}"),
            (n, true) => vec![".."; n].join("/"),
            (n, false) => format!("{}/{rest}", vec![".."; n].join("/")),
        };
    }
    match home.and_then(|h| p.strip_prefix(h).ok()) {
        Some(r) if r.as_os_str().is_empty() => "~".to_owned(),
        Some(r) => format!("~/{}", r.to_string_lossy()),
        None => p.to_string_lossy().into_owned(),
    }
}

/// The folder holds something: a folder with only dotfiles is not busy for [`Kind::ThisFolder`]
/// but is for the others; use [`only_dotfiles`] for that case.
fn has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

/// The folder exists and every entry in it starts with a dot (an empty one counts).
#[must_use]
pub fn only_dotfiles(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|d| {
        d.flatten()
            .all(|e| e.file_name().to_string_lossy().starts_with('.'))
    })
}

fn expand(root: &str, home: Option<&Path>) -> Option<PathBuf> {
    let r = root.trim();
    if r.is_empty() {
        return None;
    }
    match (r.strip_prefix("~/"), r == "~", home) {
        (Some(rest), _, Some(h)) => Some(h.join(rest)),
        (_, true, Some(h)) => Some(h.to_path_buf()),
        (Some(_), _, None) | (_, true, None) => None,
        _ => Some(PathBuf::from(r)),
    }
}

fn proposal(w: &Where, kind: Kind, dir: PathBuf, reason: &str, busy: bool) -> Proposal {
    Proposal {
        kind,
        shown: show(&w.cwd, &dir, w.home.as_deref()),
        dir,
        reason: reason.to_owned(),
        busy,
    }
}

/// The places for project `name`, the recommended one first. `Elsewhere` is always last and
/// has no folder yet.
///
/// Inside a git work tree, `./NAME` would nest a repository in a repository, so its reason
/// says so and the sibling of the work tree comes first.
#[must_use]
pub fn proposals(w: &Where, name: &str) -> Vec<Proposal> {
    let mut out: Vec<Proposal> = Vec::new();
    let here = w.cwd.join(name);
    let nests = |target: &Path| {
        w.git_top
            .as_ref()
            .filter(|top| within(target, top) && !same(target, top))
    };
    let cwd_name = w
        .cwd
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let can_use_cwd = only_dotfiles(&w.cwd);
    let this_first = can_use_cwd && cwd_name == name;

    if this_first {
        out.push(proposal(
            w,
            Kind::ThisFolder,
            w.cwd.clone(),
            "this folder is already called that, and it is empty",
            false,
        ));
    }
    let here_reason = match nests(&here) {
        Some(top) => format!(
            "in the folder you are in, but inside the git repository {}: it would nest",
            top.file_name()
                .map_or_else(String::new, |f| f.to_string_lossy().into_owned())
        ),
        None => "in the folder you are in".to_owned(),
    };
    let here_p = proposal(
        w,
        Kind::Here,
        here.clone(),
        &here_reason,
        has_entries(&here),
    );
    if let Some(top) = nests(&here)
        && let Some(parent) = top.parent()
    {
        let sib = parent.join(name);
        out.push(proposal(
            w,
            Kind::Sibling,
            sib.clone(),
            "beside the git repository you are in, so it does not nest",
            has_entries(&sib),
        ));
    }
    out.push(here_p);
    if can_use_cwd && !this_first {
        let reason = if nests(&w.cwd).is_some() {
            "this folder: it is empty, but inside a git repository, so it would nest"
        } else {
            "this folder: it is empty"
        };
        out.push(proposal(w, Kind::ThisFolder, w.cwd.clone(), reason, false));
    }
    if let Some(root) = w.root.as_deref().and_then(|r| expand(r, w.home.as_deref())) {
        let dir = root.join(name);
        if !out.iter().any(|p| p.dir == dir) {
            out.push(proposal(
                w,
                Kind::Root,
                dir.clone(),
                "your projects folder (`[project] root` in config.toml)",
                has_entries(&dir),
            ));
        }
    }
    out.push(Proposal {
        kind: Kind::Elsewhere,
        dir: PathBuf::new(),
        shown: "Somewhere else...".to_owned(),
        reason: "type a path".to_owned(),
        busy: false,
    });
    out
}

/// The proposal taken with no terminal or `--yes`: the first that is not the sibling (so the
/// answer is `./NAME` unless the folder is itself the project's), nest warning kept in its
/// reason.
#[must_use]
pub fn headless(list: &[Proposal]) -> Proposal {
    list.iter()
        .find(|p| matches!(p.kind, Kind::Here | Kind::ThisFolder))
        .cloned()
        .unwrap_or_else(|| list[0].clone())
}

/// A path given by hand (`--dir`, or typed after "Somewhere else..."), as a proposal.
#[must_use]
pub fn given(w: &Where, dir: &Path, reason: &str) -> Proposal {
    let abs = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        w.cwd.join(dir)
    };
    let abs = normalize(&abs);
    let busy = has_entries(&abs) && !only_dotfiles(&abs);
    proposal(w, Kind::Given, abs, reason, busy)
}

/// Resolves `.` and `..` by name, without touching the disk (the folder may not exist yet).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The warning when writing `target` would put a repository inside the git work tree the
/// person is in, with the folder that would not.
#[must_use]
pub fn nest_warning(w: &Where, target: &Path) -> Option<String> {
    let top = w.git_top.as_ref()?;
    if !within(target, top) || same(target, top) {
        return None;
    }
    let name = target.file_name()?.to_string_lossy().into_owned();
    let sib = top.parent()?.join(&name);
    Some(format!(
        "{} is inside the git repository {}: the new project would nest in it. {} is beside it",
        show(&w.cwd, target, w.home.as_deref()),
        top.file_name()
            .map_or_else(String::new, |f| f.to_string_lossy().into_owned()),
        show(&w.cwd, &sib, w.home.as_deref()),
    ))
}

/// A name from a folder or repository name: lowercase, `_` and spaces as dashes, and only if
/// the result is a valid project name.
fn tidy(raw: &str) -> Option<String> {
    let s: String = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c == '_' || c == ' ' || c == '.' {
                '-'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim_matches('-').to_owned();
    check_name(&s).ok().map(|()| s)
}

/// The repository name in a remote URL: `git@github.com:acme/my-shop.git` is `my-shop`.
fn repo_of(url: &str) -> Option<String> {
    let last = url.trim().trim_end_matches('/').rsplit(['/', ':']).next()?;
    tidy(last.strip_suffix(".git").unwrap_or(last))
}

/// The name proposals when none was given: the folder you are in, then the repository name
/// of its `origin` remote, each with its reason, no duplicates.
#[must_use]
pub fn name_proposals(cwd: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |n: Option<String>, why: &str| {
        if let Some(n) = n
            && !out.iter().any(|(x, _)| *x == n)
        {
            out.push((n, why.to_owned()));
        }
    };
    add(
        cwd.file_name().and_then(|f| tidy(&f.to_string_lossy())),
        "the name of the folder you are in",
    );
    let remote = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["remote", "get-url", "origin"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    add(
        remote.as_deref().and_then(repo_of),
        "the repository name of this folder's git remote",
    );
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

    use super::*;

    fn at(cwd: &Path) -> Where {
        Where {
            cwd: cwd.canonicalize().unwrap(),
            git_top: None,
            root: None,
            home: Some(PathBuf::from("/nonexistent-home")),
        }
    }

    fn git_init(dir: &Path) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }

    #[test]
    fn the_default_is_a_new_folder_in_the_cwd_not_under_home() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("notes.txt"), "x").unwrap();
        let w = at(d.path());
        let list = proposals(&w, "my-shop");
        assert_eq!(list[0].kind, Kind::Here);
        assert_eq!(list[0].dir, w.cwd.join("my-shop"));
        assert_eq!(list[0].shown, "./my-shop");
        assert_eq!(list[0].reason, "in the folder you are in");
        assert_eq!(headless(&list).dir, w.cwd.join("my-shop"));
        assert_eq!(list.last().unwrap().kind, Kind::Elsewhere);
        assert!(
            !list.iter().any(|p| p.kind == Kind::ThisFolder),
            "a folder with files in it is not offered"
        );
    }

    #[test]
    fn an_empty_folder_or_one_with_only_dotfiles_offers_this_folder() {
        let d = tempfile::tempdir().unwrap();
        let w = at(d.path());
        assert!(
            proposals(&w, "my-shop")
                .iter()
                .any(|p| p.kind == Kind::ThisFolder)
        );
        std::fs::write(d.path().join(".envrc"), "").unwrap();
        let list = proposals(&w, "my-shop");
        let this = list.iter().find(|p| p.kind == Kind::ThisFolder).unwrap();
        assert_eq!(this.shown, ".");
        assert_eq!(
            list[0].kind,
            Kind::Here,
            "the new folder is still the recommended one"
        );
        std::fs::write(d.path().join("a.txt"), "").unwrap();
        assert!(
            !proposals(&w, "my-shop")
                .iter()
                .any(|p| p.kind == Kind::ThisFolder)
        );
    }

    #[test]
    fn an_empty_folder_called_like_the_project_is_the_project() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("my-shop");
        std::fs::create_dir(&sub).unwrap();
        let list = proposals(&at(&sub), "my-shop");
        assert_eq!(list[0].kind, Kind::ThisFolder);
        assert_eq!(headless(&list).kind, Kind::ThisFolder);
    }

    #[test]
    fn the_configured_root_is_offered_and_a_busy_folder_is_marked() {
        let d = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("my-shop")).unwrap();
        std::fs::write(root.path().join("my-shop/x"), "").unwrap();
        let mut w = at(d.path());
        w.root = Some(root.path().to_string_lossy().into_owned());
        let list = proposals(&w, "my-shop");
        let r = list.iter().find(|p| p.kind == Kind::Root).unwrap();
        assert!(r.busy);
        assert!(r.reason.contains("[project] root"));
        w.root = None;
        assert!(
            !proposals(&w, "my-shop")
                .iter()
                .any(|p| p.kind == Kind::Root)
        );
        w.root = Some("~/code".into());
        w.home = Some(PathBuf::from("/h"));
        let list = proposals(&w, "my-shop");
        assert_eq!(
            list.iter().find(|p| p.kind == Kind::Root).unwrap().dir,
            PathBuf::from("/h/code/my-shop")
        );
    }

    #[test]
    fn inside_a_git_repository_it_warns_about_nesting_and_proposes_the_sibling() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("app");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);
        std::fs::write(repo.join("README.md"), "").unwrap();
        let mut w = at(&repo);
        w.git_top = git_top(&w.cwd);
        assert_eq!(
            w.git_top.as_deref().map(|p| p.canonicalize().unwrap()),
            Some(w.cwd.clone())
        );
        let list = proposals(&w, "my-shop");
        assert_eq!(list[0].kind, Kind::Sibling);
        assert_eq!(list[0].shown, "../my-shop");
        let here = list.iter().find(|p| p.kind == Kind::Here).unwrap();
        assert!(here.reason.contains("it would nest"), "{}", here.reason);
        assert_eq!(
            headless(&list).kind,
            Kind::Here,
            "no terminal: ./NAME, warned"
        );
        let warn = nest_warning(&w, &w.cwd.join("my-shop")).unwrap();
        assert!(
            warn.contains("would nest") && warn.contains("../my-shop"),
            "{warn}"
        );
        assert!(
            nest_warning(&w, &w.cwd).is_none(),
            "the repository itself does not nest"
        );
        assert!(nest_warning(&w, &w.cwd.parent().unwrap().join("my-shop")).is_none());
    }

    #[test]
    fn outside_a_repository_there_is_no_warning() {
        let d = tempfile::tempdir().unwrap();
        let w = at(d.path());
        assert!(nest_warning(&w, &w.cwd.join("x")).is_none());
    }

    #[test]
    fn paths_read_from_the_cwd() {
        let cwd = Path::new("/a/b/c");
        let h = Some(Path::new("/Users/me"));
        assert_eq!(show(cwd, Path::new("/a/b/c/x"), h), "./x");
        assert_eq!(show(cwd, Path::new("/a/b/c"), h), ".");
        assert_eq!(show(cwd, Path::new("/a/b/x"), h), "../x");
        assert_eq!(show(cwd, Path::new("/Users/me/code/x"), h), "~/code/x");
        assert_eq!(show(cwd, Path::new("/z/x"), None), "/z/x");
        assert_eq!(
            show(Path::new("/a/b/c/d/e/f"), Path::new("/a/x"), None),
            "/a/x"
        );
    }

    #[test]
    fn a_given_path_is_made_absolute_from_the_cwd() {
        let d = tempfile::tempdir().unwrap();
        let w = at(d.path());
        let p = given(&w, Path::new("../other/x"), "you passed --dir");
        assert_eq!(p.dir, w.cwd.parent().unwrap().join("other/x"));
        assert_eq!(p.shown, "../other/x");
    }

    #[test]
    fn names_come_from_the_folder_and_the_remote() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("My_Shop");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);
        assert_eq!(
            name_proposals(&repo),
            vec![(
                "my-shop".to_owned(),
                "the name of the folder you are in".to_owned()
            )]
        );
        let ok = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "remote",
                "add",
                "origin",
                "git@github.com:acme/web-store.git",
            ])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let names: Vec<String> = name_proposals(&repo).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["my-shop", "web-store"]);
        assert_eq!(
            repo_of("https://github.com/acme/x-y.git/").as_deref(),
            Some("x-y")
        );
        assert_eq!(repo_of("https://github.com/acme/Bad%20Name"), None);
        assert_eq!(name_proposals(Path::new("/")), []);
    }
}
