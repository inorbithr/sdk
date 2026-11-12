//! Where a project keeps what iohr learns about it, and the one resolver every writer asks.
//!
//! Light PRD 0210: `iohr project init` asks once where project knowledge lives, in the
//! project folder (`<project>/.inorbit/`, travels with the repository) or in the person's home
//! (`~/.inorbit/projects/<name>/`, stays on this machine). The choice is `[project] data` in
//! `inorbit.toml`; `--keep-in` and `IOHR_INIT_KEEP_IN` override it for one init. A third
//! choice, any folder the person names, is recorded as the path itself (`~` is the home
//! folder, a relative path is read from the project folder).
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

use std::path::{Component, Path, PathBuf};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `<project>/.inorbit/`.
    Project,
    /// `~/.inorbit/projects/<name>/`.
    Home,
    /// A folder the person named, as they wrote it (`~/notes/shop`, `../shared`, `/srv/x`):
    /// `~` is the home folder and a relative path starts at the project folder
    /// ([`custom_dir`]). Notes, crawl and state sit directly in it.
    Elsewhere(String),
}

impl Place {
    /// What `[project] data` and `--keep-in` hold: `project`, `home` or the path.
    #[must_use]
    pub fn word(&self) -> String {
        match self {
            Place::Project => "project".to_owned(),
            Place::Home => "home".to_owned(),
            Place::Elsewhere(path) => path.clone(),
        }
    }

    /// The place a word or a path names: `project` and `home` are the two named places
    /// (any case), anything else is a folder (write `./project` for a folder of that name).
    ///
    /// # Errors
    /// Nothing at all.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "" => Err("name a place: project, home or a folder".to_owned()),
            "project" => Ok(Place::Project),
            "home" => Ok(Place::Home),
            _ => Ok(Place::Elsewhere(s.to_owned())),
        }
    }
}

/// The folder a typed path names for the project at `root`: `~` and `~/..` from `home`, a
/// relative path from `root`, an absolute one as it is; `.` and `..` folded away.
#[must_use]
pub fn custom_dir(root: &Path, raw: &str, home: Option<&Path>) -> PathBuf {
    let typed = crate::access::expand_home(Path::new(raw.trim()), home);
    let full = if typed.is_absolute() {
        typed
    } else {
        root.join(typed)
    };
    let mut out = PathBuf::new();
    for c in full.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The folder `raw` names, when notes can live there: not a file, and either the folder is
/// there and writable or the nearest folder above it is (the place is created when the run
/// is confirmed).
///
/// # Errors
/// A reason a person can act on.
pub fn check_custom(root: &Path, raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("type a folder, for example ~/notes/shop".to_owned());
    }
    if raw.starts_with('~') && home.is_none() && (raw == "~" || raw.starts_with("~/")) {
        return Err("there is no home folder on this machine to read ~ from".to_owned());
    }
    let dir = custom_dir(root, raw, home);
    let mut at = dir.as_path();
    loop {
        if at.exists() {
            if !at.is_dir() {
                return Err(format!("{} is a file, not a folder", at.display()));
            }
            let readonly = std::fs::metadata(at).is_ok_and(|m| m.permissions().readonly());
            if readonly {
                return Err(format!("cannot write in {}", at.display()));
            }
            return Ok(dir);
        }
        match at.parent() {
            Some(up) => at = up,
            None => return Ok(dir),
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
        self.place.clone()
    }

    /// The folder of an [`Place::Elsewhere`] choice, when that is the choice.
    #[must_use]
    pub fn custom_dir(&self) -> Option<PathBuf> {
        match &self.place {
            Some(Place::Elsewhere(raw)) => Some(custom_dir(&self.root, raw, self.home.as_deref())),
            _ => None,
        }
    }

    /// Makes the folder of an [`Place::Elsewhere`] choice (nothing for the other places, which
    /// are made as they are written to).
    ///
    /// # Errors
    /// The folder cannot be made or written to, with the path and the reason.
    pub fn ensure_custom(&self) -> Result<(), String> {
        let Some(dir) = self.custom_dir() else {
            return Ok(());
        };
        std::fs::create_dir_all(&dir).map_err(|e| {
            format!(
                "cannot use {} to keep the project's data: {e}",
                dir.display()
            )
        })?;
        if std::fs::metadata(&dir).is_ok_and(|m| m.permissions().readonly()) {
            return Err(format!("cannot write in {}", dir.display()));
        }
        Ok(())
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
        match &self.place {
            Some(Place::Project) => Some(self.in_project()),
            Some(Place::Home) => self.in_home(),
            Some(Place::Elsewhere(raw)) => Some(custom_dir(&self.root, raw, self.home.as_deref())),
            None => match kind {
                Kind::Memory | Kind::Crawl => Some(self.in_project()),
                Kind::State => self.in_home(),
            },
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
pub fn with_place(text: &str, place: &Place) -> Result<String, String> {
    set_project_key(text, "data", &place.word())
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
pub fn record(root: &Path, place: &Place) -> Result<(), String> {
    write_project_key(root, "data", &place.word())
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
        record(r.path(), &Place::Home).unwrap();
        assert_eq!(Layout::of(r.path()).place(), Some(Place::Home));
        let t = std::fs::read_to_string(r.path().join(MANIFEST)).unwrap();
        assert!(t.contains("# keep me") && t.contains("description = \"x\""));
        assert!(with_place("x = [", &Place::Home).is_err());
        assert!(Place::parse(" ").is_err());
        // A missing manifest is made.
        let n = tempfile::tempdir().unwrap();
        record(n.path(), &Place::Project).unwrap();
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

    #[test]
    fn a_folder_the_person_names_holds_everything_and_round_trips() {
        let (r, h) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let home = Some(h.path());
        // `~` is the home folder, a relative path starts at the project, `..` is folded.
        assert_eq!(
            custom_dir(r.path(), "~/notes/shop", home),
            h.path().join("notes/shop")
        );
        assert_eq!(
            custom_dir(r.path(), "./data/../kept", home),
            r.path().join("kept")
        );
        assert_eq!(
            custom_dir(r.path(), "/srv/x", home),
            PathBuf::from("/srv/x")
        );
        let elsewhere = Place::parse("~/notes/shop").unwrap();
        assert_eq!(elsewhere, Place::Elsewhere("~/notes/shop".into()));
        assert_eq!(Place::parse("HOME").unwrap(), Place::Home);
        let l = layout(r.path(), h.path(), Some(elsewhere.clone()));
        let base = h.path().join("notes/shop");
        assert_eq!(l.memory(), base.join("memory"));
        assert_eq!(l.crawl(), base.join("crawl"));
        assert_eq!(l.state().unwrap(), base);
        assert_eq!(l.memory_rel(), None);
        // Recorded as typed in `[project] data`, read back by the same resolver.
        record(r.path(), &elsewhere).unwrap();
        assert_eq!(Layout::of(r.path()).place(), Some(elsewhere));
        // A path inside the project is a relative memory path like the project place.
        let inside = layout(r.path(), h.path(), Some(Place::parse("./kept").unwrap()));
        assert_eq!(inside.memory_rel().as_deref(), Some("kept/memory"));
    }

    #[test]
    fn a_folder_that_cannot_hold_notes_is_refused_with_the_reason() {
        let (r, h) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let home = Some(h.path());
        std::fs::write(r.path().join("afile"), "x").unwrap();
        let e = check_custom(r.path(), "afile", home).unwrap_err();
        assert!(e.contains("is a file, not a folder"), "{e}");
        let e = check_custom(r.path(), "afile/inside", home).unwrap_err();
        assert!(e.contains("is a file, not a folder"), "{e}");
        assert!(check_custom(r.path(), "  ", home).is_err());
        assert!(
            check_custom(r.path(), "~/x", None)
                .unwrap_err()
                .contains("no home folder")
        );
        // A folder that is not there yet is fine: it is made on confirm.
        let ok = check_custom(r.path(), "~/new/deep", home).unwrap();
        assert_eq!(ok, h.path().join("new/deep"));
        let l = layout(
            r.path(),
            h.path(),
            Some(Place::Elsewhere("~/new/deep".into())),
        );
        l.ensure_custom().unwrap();
        assert!(ok.is_dir());
        // A place that cannot be made says which path and why.
        let bad = layout(
            r.path(),
            h.path(),
            Some(Place::Elsewhere("afile/inside".into())),
        );
        let e = bad.ensure_custom().unwrap_err();
        assert!(e.contains("cannot use") && e.contains("afile"), "{e}");
        // The two named places have nothing to make.
        layout(r.path(), h.path(), Some(Place::Home))
            .ensure_custom()
            .unwrap();
    }
}
