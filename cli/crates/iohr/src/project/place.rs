//! Where a project keeps what iohr learns about it, and the one resolver every writer asks.
//!
//! Light PRD 0210: `iohr project init` asks once where project knowledge lives, in the
//! project folder (`<project>/.inorbit/`, travels with the repository) or in the person's home
//! (`~/.inorbit/projects/<name>/`, stays on this machine). The choice is `[project] data` in
//! `inorbit.toml`; `--keep-in` and `IOHR_INIT_KEEP_IN` override it for one init.
//!
//! The kinds that move with the choice: the project's notes (`memory/`), what a crawl keeps
//! (`crawl/`, its local index) and the machine state (`state/`, `cache/`, `project.toml`, the
//! queued space sync). With no choice recorded the layout is the one older versions made
//! (notes and crawl in the project, state in home), so no existing project changes.
//!
//! Stays in the project whatever the choice, because it is the repository's own text or it
//! binds a folder: `atlas/` (ADRs, PRDs, RFCs, roles), `inorbit.toml`, `.inorbit/space*`,
//! `.inorbit/permissions.toml`. Moving data is offered, never done silently, and the old copy
//! is never deleted ([`copy_over`]).

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, value};

/// The project's memory under its knowledge folder.
pub const MEMORY: &str = "memory";
/// What a crawl keeps under its knowledge folder.
pub const CRAWL: &str = "crawl";
/// The folder under the project that holds the project-place data.
pub const PROJECT_DIR: &str = ".inorbit";
/// The manifest the choice is written in.
pub const MANIFEST: &str = "inorbit.toml";

/// Where the project's knowledge lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// `<project>/.inorbit/`.
    Project,
    /// `~/.inorbit/projects/<name>/`.
    Home,
}

impl Place {
    /// The word `[project] data` and `--keep-in` use.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Place::Project => "project",
            Place::Home => "home",
        }
    }

    /// The place a word names.
    ///
    /// # Errors
    /// A word that is neither `project` nor `home`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "project" => Ok(Place::Project),
            "home" => Ok(Place::Home),
            other => Err(format!("{other:?} is not a place: use project or home")),
        }
    }
}

/// One kind of data the resolver places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Memory,
    Crawl,
    State,
}

/// The paths of one project: where each kind of data is, given the recorded choice.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
    name: String,
    place: Option<Place>,
    home: Option<PathBuf>,
}

impl Layout {
    /// The layout of the project at `root`: the choice in its `inorbit.toml`, its name from
    /// `name` there or the folder's own name.
    #[must_use]
    pub fn of(root: &Path) -> Self {
        let table = std::fs::read_to_string(root.join(MANIFEST))
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok());
        let place = table
            .as_ref()
            .and_then(|t| t.get("project")?.get("data")?.as_str())
            .and_then(|s| Place::parse(s).ok());
        let name = table
            .as_ref()
            .and_then(|t| t.get("name")?.as_str().map(str::to_owned))
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        Self {
            root: root.to_path_buf(),
            name,
            place,
            home: etcetera::home_dir().ok(),
        }
    }

    /// The layout of a project being made, named `name`, whose choice (if any) is `place`.
    #[must_use]
    pub fn new(root: &Path, name: &str, place: Option<Place>) -> Self {
        Self {
            root: root.to_path_buf(),
            name: name.to_owned(),
            place,
            home: etcetera::home_dir().ok(),
        }
    }

    /// The same layout with another home folder (tests, and a machine whose home is elsewhere).
    #[must_use]
    pub fn with_home(mut self, home: &Path) -> Self {
        self.home = Some(home.to_path_buf());
        self
    }

    /// The recorded choice, when there is one.
    #[must_use]
    pub fn place(&self) -> Option<Place> {
        self.place
    }

    fn in_project(&self) -> PathBuf {
        self.root.join(PROJECT_DIR)
    }

    fn in_home(&self) -> Option<PathBuf> {
        self.home
            .as_ref()
            .map(|h| h.join(".inorbit").join("projects").join(&self.name))
    }

    /// The folder `kind` lives in. With no recorded choice: notes and crawl in the project,
    /// state in home (what older versions did).
    fn base(&self, kind: Kind) -> Option<PathBuf> {
        let at = self.place.unwrap_or(match kind {
            Kind::Memory | Kind::Crawl => Place::Project,
            Kind::State => Place::Home,
        });
        match at {
            Place::Project => Some(self.in_project()),
            Place::Home => self.in_home(),
        }
    }

    /// The project's notes: `.inorbit/memory/` (the project place) or the home folder's.
    #[must_use]
    pub fn memory(&self) -> PathBuf {
        // No home folder: the project's own folder, so a writer never loses the notes.
        self.base(Kind::Memory)
            .unwrap_or_else(|| self.in_project())
            .join(MEMORY)
    }

    /// What a crawl keeps, per site, and the local search index.
    #[must_use]
    pub fn crawl(&self) -> PathBuf {
        self.base(Kind::Crawl)
            .unwrap_or_else(|| self.in_project())
            .join(CRAWL)
    }

    /// The folder holding `state/`, `cache/` and `project.toml`. `None` when the choice is
    /// home (or unset) and there is no home folder.
    #[must_use]
    pub fn state(&self) -> Option<PathBuf> {
        self.base(Kind::State)
    }

    /// The memory path as the project-relative form the scaffold uses, when it is inside the
    /// project (`.inorbit/memory`), else `None`.
    #[must_use]
    pub fn memory_rel(&self) -> Option<String> {
        self.memory()
            .strip_prefix(&self.root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// `p` as a person reads it: relative to the project when inside it, else in full.
    #[must_use]
    pub fn shown(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => p.to_string_lossy().into_owned(),
        }
    }

    /// Every place this layout writes, for a dry run and for the report: label and path.
    #[must_use]
    pub fn describe(&self) -> Vec<(&'static str, PathBuf)> {
        let mut v = vec![("notes", self.memory()), ("crawl", self.crawl())];
        if let Some(s) = self.state() {
            v.push(("state", s));
        }
        v
    }
}

/// `text` (an `inorbit.toml`, or nothing) with `[project] data` set, comments kept.
///
/// # Errors
/// `text` is not valid TOML.
pub fn with_place(text: &str, place: Place) -> Result<String, String> {
    set_project_key(text, "data", place.word())
}

/// `text` with `[project] <key>` set to `val`, comments and the rest as they were.
///
/// # Errors
/// `text` is not valid TOML, or `project` is not a table.
pub fn set_project_key(text: &str, key: &str, val: &str) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{MANIFEST} is not valid TOML: {e}"))?;
    if !doc.contains_key("project") {
        doc["project"] = toml_edit::table();
    }
    let project = doc["project"]
        .as_table_mut()
        .ok_or_else(|| format!("[project] in {MANIFEST} is not a table"))?;
    project.insert(key, value(val));
    Ok(doc.to_string())
}

/// The `[project] <key>` string in `inorbit.toml` under `root`.
#[must_use]
pub fn project_key(root: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(root.join(MANIFEST))
        .ok()?
        .parse::<toml::Table>()
        .ok()?
        .get("project")?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// Writes `[project] <key>` into `inorbit.toml` under `root` (made when missing).
///
/// # Errors
/// The file cannot be read as TOML or written.
pub fn write_project_key(root: &Path, key: &str, val: &str) -> Result<(), String> {
    let file = root.join(MANIFEST);
    let old = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let text = set_project_key(&old, key, val)?;
    if text != old {
        std::fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    }
    Ok(())
}

/// Records the choice in `inorbit.toml` under `root`.
///
/// # Errors
/// As [`write_project_key`].
pub fn record(root: &Path, place: Place) -> Result<(), String> {
    write_project_key(root, "data", place.word())
}

/// What the project folder's `.inorbit/.gitignore` has to hold when the machine state lives
/// there (`state/`, `cache/`, the record with this machine's path, the queued sync): the lines it adds, never a rewrite of a line already in it.
///
/// # Errors
/// The file cannot be written.
pub fn ignore_state(root: &Path) -> Result<(), String> {
    let file = root.join(PROJECT_DIR).join(".gitignore");
    // A folder with no ignore file yet starts from the space link's own, so neither is lost.
    let old =
        std::fs::read_to_string(&file).unwrap_or_else(|_| super::space::GITIGNORE_TEXT.to_owned());
    let had = file.is_file();
    let mut text = old.clone();
    for line in ["state/", "cache/", "project.toml", "space-sync.*"] {
        if !old.lines().any(|l| l.trim() == line) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(line);
            text.push('\n');
        }
    }
    if text != old || !had {
        std::fs::create_dir_all(root.join(PROJECT_DIR))
            .and_then(|()| std::fs::write(&file, text))
            .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    }
    Ok(())
}

/// Copies what `from` holds into `to`, one file at a time, never over a file that is there
/// and never deleting anything: the old copy stays. Returns the files copied.
///
/// # Errors
/// A folder or file that cannot be read or written.
pub fn copy_over(from: &Path, to: &Path) -> Result<usize, String> {
    if !from.is_dir() || from == to {
        return Ok(0);
    }
    let mut n = 0;
    let rd = std::fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    for e in rd.filter_map(Result::ok) {
        let path = e.path();
        let target = to.join(e.file_name());
        if path.is_dir() {
            n += copy_over(&path, &target)?;
        } else if !target.exists() {
            std::fs::create_dir_all(to)
                .and_then(|()| std::fs::copy(&path, &target).map(|_| ()))
                .map_err(|e| format!("cannot copy to {}: {e}", target.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// True when some kind sits somewhere else under `to` than under `from` and the old place
/// holds a file: there is something to offer to copy.
#[must_use]
pub fn movable(from: &Layout, to: &Layout) -> bool {
    from.describe()
        .into_iter()
        .zip(to.describe())
        .any(|((_, a), (_, b))| a != b && has_files(&a))
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.filter_map(Result::ok)
            .any(|e| e.path().is_file() || has_files(&e.path()))
    })
}

/// Copies every kind that sits somewhere else under `to` than under `from`; the count and
/// each kind's label. Nothing is removed.
///
/// # Errors
/// As [`copy_over`].
pub fn copy_layout(from: &Layout, to: &Layout) -> Result<Vec<(&'static str, usize)>, String> {
    let mut out = Vec::new();
    for ((label, a), (_, b)) in from.describe().into_iter().zip(to.describe()) {
        if a != b {
            out.push((label, copy_over(&a, &b)?));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(root: &Path, home: &Path, place: Option<Place>) -> Layout {
        Layout::new(root, "shop", place).with_home(home)
    }

    #[test]
    fn no_choice_keeps_the_layout_older_versions_made() {
        let (r, h) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let l = layout(r.path(), h.path(), None);
        assert_eq!(l.memory(), r.path().join(".inorbit/memory"));
        assert_eq!(l.crawl(), r.path().join(".inorbit/crawl"));
        assert_eq!(l.state().unwrap(), h.path().join(".inorbit/projects/shop"));
    }

    #[test]
    fn each_choice_moves_every_kind_together() {
        let (r, h) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let p = layout(r.path(), h.path(), Some(Place::Project));
        assert_eq!(p.memory(), r.path().join(".inorbit/memory"));
        assert_eq!(p.crawl(), r.path().join(".inorbit/crawl"));
        assert_eq!(p.state().unwrap(), r.path().join(".inorbit"));
        let home = layout(r.path(), h.path(), Some(Place::Home));
        let base = h.path().join(".inorbit/projects/shop");
        assert_eq!(home.memory(), base.join("memory"));
        assert_eq!(home.crawl(), base.join("crawl"));
        assert_eq!(home.state().unwrap(), base);
        assert_eq!(home.memory_rel(), None);
        assert_eq!(p.memory_rel().as_deref(), Some(".inorbit/memory"));
    }

    #[test]
    fn the_choice_is_read_from_inorbit_toml_and_written_with_comments_kept() {
        let r = tempfile::tempdir().unwrap();
        std::fs::write(
            r.path().join(MANIFEST),
            "# keep me\nname = \"shop\"\n\n[project]\ndescription = \"x\"\n",
        )
        .unwrap();
        assert_eq!(Layout::of(r.path()).place(), None);
        record(r.path(), Place::Home).unwrap();
        assert_eq!(Layout::of(r.path()).place(), Some(Place::Home));
        let t = std::fs::read_to_string(r.path().join(MANIFEST)).unwrap();
        assert!(t.contains("# keep me") && t.contains("description = \"x\""));
        assert!(with_place("x = [", Place::Home).is_err());
        assert!(Place::parse("cloud").is_err());
        // A missing manifest is made.
        let n = tempfile::tempdir().unwrap();
        record(n.path(), Place::Project).unwrap();
        assert_eq!(Layout::of(n.path()).place(), Some(Place::Project));
    }

    #[test]
    fn copying_never_overwrites_and_never_deletes() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir_all(a.path().join("sub")).unwrap();
        std::fs::write(a.path().join("one.md"), "old one").unwrap();
        std::fs::write(a.path().join("two.md"), "old two").unwrap();
        std::fs::write(a.path().join("sub/three.md"), "old three").unwrap();
        std::fs::write(b.path().join("two.md"), "mine").unwrap();
        assert_eq!(copy_over(a.path(), b.path()).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(b.path().join("two.md")).unwrap(),
            "mine"
        );
        assert_eq!(
            std::fs::read_to_string(b.path().join("sub/three.md")).unwrap(),
            "old three"
        );
        assert!(a.path().join("one.md").is_file() && a.path().join("sub/three.md").is_file());
        // Run again: nothing left to copy.
        assert_eq!(copy_over(a.path(), b.path()).unwrap(), 0);
    }

    #[test]
    fn the_state_folders_are_ignored_in_the_project_place() {
        let r = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(r.path().join(".inorbit")).unwrap();
        std::fs::write(r.path().join(".inorbit/.gitignore"), "space.*").unwrap();
        ignore_state(r.path()).unwrap();
        ignore_state(r.path()).unwrap();
        let t = std::fs::read_to_string(r.path().join(".inorbit/.gitignore")).unwrap();
        assert_eq!(t, "space.*\nstate/\ncache/\nproject.toml\nspace-sync.*\n");
    }

    #[test]
    fn there_is_something_to_move_only_when_the_old_place_holds_files_and_differs() {
        let (r, h) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let old = layout(r.path(), h.path(), None);
        let to_home = layout(r.path(), h.path(), Some(Place::Home));
        // Nothing written yet: nothing to offer.
        assert!(!movable(&old, &to_home));
        std::fs::create_dir_all(old.memory()).unwrap();
        std::fs::write(old.memory().join("a.md"), "x").unwrap();
        assert!(movable(&old, &to_home));
        // The same place is never a move.
        assert!(!movable(&old, &old));
    }
}
