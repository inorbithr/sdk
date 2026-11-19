//! The built-in roles `iohr project init` and `iohr atlas init` give a project
//! (RFC 0124.1, RFC 0205): `base-engineer` and `security` for every project, one engineer for
//! each language it holds, and each role its files turn on (`markers`, ADR 0019). The
//! catalogue is data in `roles/*.toml`, versioned with this binary; every line carries the
//! primary source it comes from, and a role's `[[check]]` gates are the commands it runs
//! before it says the work is done.
//!
//! A project gets readable packs under `atlas/roles/` and, when the person is signed
//! in, the same roles in InOrbit through `POST /v1/coordinator/personalities/import` (the
//! wire keeps the old word: see [`WIRE_ROLES_PATH`]).
//! Nothing here blocks an init: an import that cannot happen is reported and left for
//! `iohr sync`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::atlas;
use crate::context::Api;
use inorbithr::Method;

/// The folder of the packs, under the project root.
pub const DIR: &str = "atlas/roles";
/// Where the packs were written before the word became Role.
const LEGACY_DIR: &str = "atlas/personalities";
/// The one role every project gets, whatever its language.
pub const BASE: &str = "security";
/// The root of the tree, written as a pack beside every role (ADR 0019) and never routed to:
/// its lines also reach every role by inheritance (in the pack, in `--role` and in InOrbit's
/// brief). Imported when the account has no root, and upgraded when the account's root is
/// this one, unchanged.
pub const ROOT: &str = "base-engineer";
/// The most tokens one pack's own core may hold (characters over four, as the service counts).
pub const PACK_CORE_MAX: usize = 700;
/// The most core tokens a run loads: the root's and one role's together (ADR 0019).
pub const LOADED_CORE_MAX: usize = 1_400;
/// The pack's heading over its gates, read by `--role` with the core.
pub const CHECKS_HEADING: &str = "Before you say done, run";

// The wire names. Core renamed the word Personality to Role (#1365) but kept every published
// name on the wire, and published names are never renamed. So these say `personalities`
// while everything a person reads says Role. Use these constants, never the literal.
/// `GET` lists the account's roles; `GET {WIRE_ROLES_PATH}/{name}` reads one.
pub const WIRE_ROLES_PATH: &str = "/v1/coordinator/personalities";
/// `POST` takes roles.
pub const WIRE_IMPORT_PATH: &str = "/v1/coordinator/personalities/import";
/// The key of the role list in the import body and in the list answer.
pub const WIRE_ROLES_KEY: &str = "personalities";
/// The key of one role in `GET {WIRE_ROLES_PATH}/{name}`.
pub const WIRE_ROLE_KEY: &str = "personality";
/// The hidden alias `iohr atlas personalities` keeps for scripts written before the rename.
pub const LEGACY_CLI_ALIAS: &str = "personalities";

/// The catalogue, each file with its name so a file that does not parse is named.
const FILES: [(&str, &str); 6] = [
    (
        "base-engineer.toml",
        include_str!("../roles/base-engineer.toml"),
    ),
    ("security.toml", include_str!("../roles/security.toml")),
    (
        "rust-engineer.toml",
        include_str!("../roles/rust-engineer.toml"),
    ),
    (
        "python-engineer.toml",
        include_str!("../roles/python-engineer.toml"),
    ),
    (
        "typescript-engineer.toml",
        include_str!("../roles/typescript-engineer.toml"),
    ),
    (
        "go-engineer.toml",
        include_str!("../roles/go-engineer.toml"),
    ),
];

/// The one line both inits (and an import with no sign-in) say when the roles stay in the
/// folder, with why.
#[must_use]
pub fn kept_local(why: &str) -> String {
    format!(
        "the built-in roles stay in {DIR}/, nothing was sent to your account ({why}); \
         `iohr sync` imports them"
    )
}

/// What both inits pick the roles by ([`Stack`]): the `hint` (a template's answer) when it
/// names a language the catalogue has, then every language the folder declares or builds
/// with, and the roles its files turn on.
#[must_use]
pub fn stack(root: &Path, hint: Option<&str>) -> Stack {
    stack_in(&catalogue(), root, hint)
}

/// One line of a role: its text and the sources it comes from.
#[derive(Debug, Clone, Deserialize)]
pub struct Line {
    /// `core` (always in the brief) or `recall` (found by topic).
    pub place: String,
    /// The heading it sits under.
    pub section: String,
    pub text: String,
    /// Primary sources, as URLs. Never empty.
    pub cites: Vec<String>,
    /// The day this line was last read against its sources, when it differs from its role's.
    #[serde(default)]
    pub checked: Option<String>,
}

impl Line {
    /// The day this line was last checked: its own date, else its role's.
    #[must_use]
    pub fn checked_on<'a>(&'a self, role: &'a Role) -> &'a str {
        self.checked.as_deref().unwrap_or(&role.checked)
    }
}

/// A role of the catalogue.
#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    pub name: String,
    /// Empty for the root.
    pub parent: String,
    pub title: String,
    /// Raised whenever the lines change.
    pub version: u32,
    /// The day its facts were last read against their primary sources (`YYYY-MM-DD`). Required:
    /// a pack and every memory seeded from it carry it as `checked:`. Move it only after the
    /// facts were read again: it is a claim, not a build date. A line may carry its own.
    pub checked: String,
    /// The language it is for, when it is one.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub domain: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub model: String,
    #[serde(default, rename = "line")]
    pub lines: Vec<Line>,
    /// The gates: what to run before saying the work is done (`[[check]]`).
    #[serde(default, rename = "check")]
    pub checks: Vec<Check>,
    /// The files that turn this role on in a project, beside its language engineer
    /// ([`marker_matches`]): `Dockerfile` (a file of that name), `*.tf` (a file name with a
    /// wildcard) or `migrations/` (a folder of that name).
    #[serde(default)]
    pub markers: Vec<String>,
}

/// A gate of a role (`[[check]]`): a command the agent runs before it says the work is done.
/// A command is not a line: the line lint ([`lint::findings`]) never reads it, so a flag such
/// as `--ignore-scripts` is kept as written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// The command, one line, as a person types it.
    pub command: String,
    /// What a pass shows.
    pub proves: String,
    /// Whether a failure stops the work from counting as done (`true` unless it says `false`):
    /// a check that does not block is run and its result reported.
    #[serde(default = "blocks_by_default")]
    pub blocks: bool,
}

const fn blocks_by_default() -> bool {
    true
}

/// The whole catalogue, root first.
///
/// # Panics
///
/// When a file of the catalogue does not parse. The files are built into the binary, so
/// that is a build that must not ship: a role is never dropped without a word.
#[must_use]
pub fn catalogue() -> Vec<Role> {
    FILES
        .iter()
        .map(|(file, text)| {
            toml::from_str::<Role>(text)
                .unwrap_or_else(|e| panic!("the built-in role roles/{file} does not parse: {e}"))
        })
        .collect()
}

/// The root of the catalogue, whose lines every other role inherits.
#[must_use]
pub fn root_role() -> Option<Role> {
    catalogue().into_iter().find(|r| r.name == ROOT)
}

/// A language as the templates and people name it, as the catalogue names it.
#[must_use]
pub fn canonical_language(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => Some("rust"),
        "python" | "python3" | "py" => Some("python"),
        "typescript" | "ts" | "javascript" | "js" | "node" | "nodejs" => Some("typescript"),
        "go" | "golang" => Some("go"),
        _ => None,
    }
}

/// What a project is built with, as the roles are picked by it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stack {
    /// Each language it holds, as the catalogue names them, in the order found: a template's
    /// answer first, then what `inorbit.toml` declares, else the build files, in
    /// [`LANGUAGE_FILES`] order.
    pub languages: Vec<&'static str>,
    /// The catalogue roles its files turn on (`markers`), by name, in catalogue order.
    pub marked: Vec<String>,
}

impl Stack {
    /// A project known only by one language (a template's answer, or none).
    #[must_use]
    pub fn of(lang: Option<&str>) -> Self {
        Self {
            languages: lang.and_then(canonical_language).into_iter().collect(),
            marked: Vec::new(),
        }
    }

    /// The first language, the one a single-language reader (memory gaps, a starter's
    /// wording) takes.
    #[must_use]
    pub fn language(&self) -> Option<&'static str> {
        self.languages.first().copied()
    }
}

/// What a project gets: `base-engineer` and `security`, the engineer of each of its languages,
/// and each role its files turn on, in catalogue order. The root comes first: its lines are
/// loaded beside every role (ADR 0019).
#[must_use]
pub fn selection(s: &Stack) -> Vec<Role> {
    select(catalogue(), s)
}

fn select(cat: Vec<Role>, s: &Stack) -> Vec<Role> {
    cat.into_iter()
        .filter(|p| {
            p.name == ROOT
                || p.name == BASE
                || p.language
                    .as_deref()
                    .and_then(canonical_language)
                    .is_some_and(|l| s.languages.contains(&l))
                || s.marked.contains(&p.name)
        })
        .collect()
}

/// The build files that say a folder holds a language, as the catalogue names it. The one
/// table: `iohr sdk add` reads it too ([`languages_at`]).
pub(crate) const LANGUAGE_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", "rust"),
    ("go.mod", "go"),
    ("package.json", "typescript"),
    ("tsconfig.json", "typescript"),
    ("deno.json", "typescript"),
    ("deno.jsonc", "typescript"),
    ("pyproject.toml", "python"),
    ("requirements.txt", "python"),
    ("setup.py", "python"),
    ("uv.lock", "python"),
    ("poetry.lock", "python"),
    ("pdm.lock", "python"),
];

/// The languages whose build files are in `dir` itself, in [`LANGUAGE_FILES`] order.
#[must_use]
pub(crate) fn languages_at(dir: &Path) -> Vec<&'static str> {
    languages_of(
        LANGUAGE_FILES
            .iter()
            .map(|(f, _)| *f)
            .filter(|f| dir.join(f).is_file()),
    )
}

fn languages_of<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let names: Vec<&str> = names.into_iter().collect();
    let mut out: Vec<&'static str> = Vec::new();
    for (file, lang) in LANGUAGE_FILES {
        if names.contains(file) && !out.contains(lang) {
            out.push(lang);
        }
    }
    out
}

/// How deep the detection reads: the root, its folders and theirs (where workspace members
/// live: `ui/package.json`, `apps/web/package.json`, `db/migrations/`).
const SCAN_DEPTH: usize = 2;
/// The most folders the detection reads, so a huge tree stays quick.
const SCAN_DIRS: usize = 2_000;
/// Folders that hold what was built, fetched or kept as test data, never the project's own.
const SCAN_SKIP: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "out",
    "venv",
    "__pycache__",
    "testdata",
    "fixtures",
];

/// What the folder holds near its top: the names of the files and of the folders in it, its
/// folders and theirs ([`SCAN_DEPTH`]), hidden and built folders left out.
#[derive(Debug, Default)]
struct Seen {
    files: Vec<String>,
    dirs: Vec<String>,
}

fn scan(root: &Path) -> Seen {
    let mut seen = Seen::default();
    let mut todo: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut read = 0usize;
    while let Some((dir, depth)) = todo.pop() {
        if read == SCAN_DIRS {
            break;
        }
        read += 1;
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<std::fs::DirEntry> = rd.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            // A link is not followed: it could lead out of the project or round in a loop.
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                if name.starts_with('.') || SCAN_SKIP.contains(&name.as_str()) {
                    continue;
                }
                if depth < SCAN_DEPTH {
                    todo.push((e.path(), depth + 1));
                }
                seen.dirs.push(name);
            } else if kind.is_file() {
                seen.files.push(name);
            }
        }
    }
    seen
}

/// Whether `marker` names something the folder holds: `name/` a folder, a name with one `*` a
/// file whose name has that start and end (`*.tf`, `next.config.*`), else a file of that name.
#[must_use]
pub fn marker_matches(marker: &str, file_names: &[String], dir_names: &[String]) -> bool {
    if let Some(dir) = marker.strip_suffix('/') {
        return dir_names.iter().any(|d| d == dir);
    }
    match marker.split_once('*') {
        Some((head, tail)) => file_names.iter().any(|f| {
            f.len() >= head.len() + tail.len() && f.starts_with(head) && f.ends_with(tail)
        }),
        None => file_names.iter().any(|f| f == marker),
    }
}

fn stack_in(cat: &[Role], root: &Path, hint: Option<&str>) -> Stack {
    let seen = scan(root);
    let found = declared_language(root).map_or_else(
        || languages_of(seen.files.iter().map(String::as_str)),
        |l| vec![l],
    );
    let mut languages: Vec<&'static str> = hint.and_then(canonical_language).into_iter().collect();
    for l in found {
        if !languages.contains(&l) {
            languages.push(l);
        }
    }
    let marked = cat
        .iter()
        .filter(|p| {
            p.markers
                .iter()
                .any(|m| marker_matches(m, &seen.files, &seen.dirs))
        })
        .map(|p| p.name.clone())
        .collect();
    Stack { languages, marked }
}

/// `language` in `inorbit.toml` (at the top or under `[project]`): what the person declared
/// stands for the build files.
fn declared_language(root: &Path) -> Option<&'static str> {
    let t = std::fs::read_to_string(root.join("inorbit.toml"))
        .ok()?
        .parse::<toml::Table>()
        .ok()?;
    let at = |t: &toml::Table| {
        t.get("language")
            .and_then(toml::Value::as_str)
            .and_then(canonical_language)
    };
    at(&t).or_else(|| {
        t.get("project")
            .and_then(toml::Value::as_table)
            .and_then(at)
    })
}

fn pack_rel(name: &str) -> String {
    format!("{DIR}/{name}.md")
}

fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// The roles `p` inherits from, root first, as the catalogue has them. InOrbit's brief puts
/// the same lines first (its `core` and `recall` walk the same chain), so the pack, `--role`
/// and the brief read the same lines.
#[must_use]
pub fn ancestors(p: &Role) -> Vec<Role> {
    let cat = catalogue();
    let mut out: Vec<Role> = Vec::new();
    let mut parent = p.parent.as_str();
    while !parent.is_empty() && parent != p.name {
        let Some(r) = cat.iter().find(|r| r.name == parent) else {
            break;
        };
        if out.iter().any(|o| o.name == r.name) {
            break;
        }
        out.push(r.clone());
        parent = &r.parent;
    }
    out.reverse();
    out
}

/// The size of a role's own core, in the service's measure (characters over four).
#[must_use]
pub fn core_tokens(p: &Role) -> usize {
    p.lines
        .iter()
        .filter(|l| l.place == "core")
        .map(|l| tokens(&l.text))
        .sum()
}

/// The size of the core a run of `p` reads: its ancestors' and its own.
#[must_use]
pub fn brief_core_tokens(p: &Role) -> usize {
    ancestors(p).iter().map(core_tokens).sum::<usize>() + core_tokens(p)
}

/// What a pack says about itself: where its lines come from, what an edit to it does and what
/// it does not do.
pub const PACK_INTRO: &str = "A built-in role of the `iohr` catalogue. Its lines come from \
     primary sources, named under each one; the lines marked as inherited come from its parent \
     and reach it in InOrbit too. An edit here changes what `--role` gives in this project, \
     because the project's pack is read first. `iohr sync` does not send it: InOrbit gets the \
     catalogue's lines, so change a role for your whole account in InOrbit.";

/// The readable pack of a role: front matter (name, parent, domain, source version, the day
/// it was checked), then the lines it inherits and its own under their headings, each with
/// its sources, and after the core the gates (inherited first) under [`CHECKS_HEADING`].
#[must_use]
pub fn pack(p: &Role) -> String {
    let mut out = format!(
        "---\nname: {}\nparent: {}\ndomain: {}\nsource: {}\ncatalogue_version: {}\nchecked: {}\n---\n",
        p.name,
        p.parent,
        p.domain.join(", "),
        format_args!("iohr {} built-in catalogue", env!("CARGO_PKG_VERSION")),
        p.version,
        p.checked,
    );
    let _ = write!(out, "# {}\n\n{PACK_INTRO}\n", p.title);
    let chain = ancestors(p);
    for place in ["core", "recall"] {
        let heading = if place == "core" {
            "Core: always in the brief"
        } else {
            "Recall: found by topic"
        };
        let _ = write!(out, "\n## {heading}\n");
        let mut last = String::new();
        let from = chain.iter().map(|r| (r, true)).chain([(p, false)]);
        for (role, inherited) in from {
            for l in role.lines.iter().filter(|l| l.place == place) {
                let section = if inherited {
                    format!("{} (inherited from {})", l.section, role.name)
                } else {
                    l.section.clone()
                };
                if section != last {
                    let _ = write!(out, "\n### {section}\n\n");
                    last = section;
                }
                let _ = write!(out, "- {}\n  Source: {}\n", l.text, l.cites.join(", "));
                if let Some(day) = &l.checked {
                    let _ = writeln!(out, "  Checked: {day}");
                }
            }
        }
        if place == "core" {
            write_checks(&mut out, &chain, p);
        }
    }
    out
}

/// The gates of `p` and of the roles it inherits from, root first, as a section of the pack:
/// nothing when there are none.
fn write_checks(out: &mut String, chain: &[Role], p: &Role) {
    let from: Vec<(&Role, bool)> = chain
        .iter()
        .map(|r| (r, true))
        .chain([(p, false)])
        .collect();
    if from.iter().all(|(r, _)| r.checks.is_empty()) {
        return;
    }
    let _ = write!(
        out,
        "\n## {CHECKS_HEADING}\n\nEach must pass before the work counts as done; one marked \
         (reports only) informs and does not block.\n\n"
    );
    for (role, inherited) in from {
        for c in &role.checks {
            let tick = if c.command.contains('`') { "``" } else { "`" };
            let _ = write!(out, "- {tick}{}{tick}", c.command);
            if inherited {
                let _ = write!(out, " (inherited from {})", role.name);
            }
            if !c.blocks {
                out.push_str(" (reports only)");
            }
            let _ = writeln!(out, "\n  Proves: {}", c.proves);
        }
    }
}

/// The pack files a project of `s` starts with, as relative path and text.
#[must_use]
pub fn pack_files(s: &Stack) -> Vec<(String, String)> {
    selection(s)
        .iter()
        .map(|p| (pack_rel(&p.name), pack(p)))
        .collect()
}

/// The `[roles]` table of `inorbit.toml`: which built-ins the project uses.
#[must_use]
pub fn record(s: &Stack) -> String {
    let names: Vec<String> = selection(s)
        .iter()
        .map(|p| format!("\"{}\"", p.name))
        .collect();
    format!(
        "\n# --- Roles ---\n\n\
         # The built-in InOrbit roles this project uses. Their packs are in\n\
         # atlas/roles/; `iohr sync` imports them into your InOrbit account.\n\
         [roles]\nnames = [{}]\n",
        names.join(", ")
    )
}

/// Adds the record to `inorbit.toml` among a template's files, unless it has one already.
pub fn add_record(files: &mut [(String, String)], s: &Stack) {
    if let Some((_, text)) = files.iter_mut().find(|(p, _)| p == "inorbit.toml")
        && !text.contains("[roles]")
    {
        text.push_str(&record(s));
    }
}

/// Adds the record to the `inorbit.toml` of an existing project, when it has one and no record.
///
/// # Errors
///
/// A message saying what could not be written.
pub fn ensure_record(root: &Path, s: &Stack) -> Result<bool, String> {
    let path = root.join("inorbit.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if text.contains("[roles]") {
        return Ok(false);
    }
    let mut next = text;
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&record(s));
    std::fs::write(&path, next).map_err(|e| format!("cannot write inorbit.toml: {e}"))?;
    Ok(true)
}

/// Moves the packs of an older `atlas/personalities/` into `atlas/roles/`, once. A file that
/// `atlas/roles/` already has stays where it is: nothing in `atlas/roles/` is overwritten.
/// Returns the one line to say, or `None` when there was nothing to move.
///
/// # Errors
///
/// A message saying what could not be moved.
pub fn migrate_legacy(root: &Path) -> Result<Option<String>, String> {
    let old = root.join(LEGACY_DIR);
    if !old.is_dir() {
        return Ok(None);
    }
    let new = root.join(DIR);
    std::fs::create_dir_all(&new).map_err(|e| format!("cannot make {DIR}/: {e}"))?;
    let (mut moved, mut left) = (0usize, 0usize);
    let rd = std::fs::read_dir(&old).map_err(|e| format!("cannot read {LEGACY_DIR}/: {e}"))?;
    for entry in rd.filter_map(Result::ok) {
        let from = entry.path();
        if !from.is_file() {
            left += 1;
            continue;
        }
        let to = new.join(entry.file_name());
        if to.exists() {
            left += 1;
        } else {
            std::fs::rename(&from, &to)
                .map_err(|e| format!("cannot move {}: {e}", from.display()))?;
            moved += 1;
        }
    }
    // Gone when empty; an old folder that still holds something is left for the person.
    let _ = std::fs::remove_dir(&old);
    // Said once: a run that moves nothing (the leftovers were named the time before) is quiet.
    if moved == 0 {
        return Ok(None);
    }
    let kept = if left == 0 {
        String::new()
    } else {
        format!(", {left} left in {LEGACY_DIR}/ because {DIR}/ has it or it is not a file")
    };
    Ok(Some(format!(
        "moved {LEGACY_DIR}/ to {DIR}/ ({moved} file{}{kept})",
        if moved == 1 { "" } else { "s" }
    )))
}

// ---------------------------------------------------------------- the import body

fn sections(p: &Role) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut order: Vec<(&str, &str)> = Vec::new();
    for l in &p.lines {
        if !order.contains(&(l.place.as_str(), l.section.as_str())) {
            order.push((l.place.as_str(), l.section.as_str()));
        }
    }
    for (place, title) in order {
        let lines: Vec<Value> = p
            .lines
            .iter()
            .filter(|l| l.place == place && l.section == title)
            .map(|l| {
                json!({
                    "text": l.text,
                    "cites": l.cites,
                    "origin": "doctrine",
                    "names_categories": false,
                })
            })
            .collect();
        out.push(json!({ "place": place, "title": title, "lines": lines }));
    }
    out
}

fn input(p: &Role, parent: &str) -> Value {
    json!({
        "name": p.name,
        "parent": parent,
        "domain": p.domain,
        "aliases": p.aliases,
        "settings": { "model": p.model },
        "sections": sections(p),
    })
}

/// The body of an import: `send` hang under `parent`, and the root joins them when
/// `with_root` (the account has none).
#[must_use]
pub fn import_body(send: &[&Role], parent: &str, with_root: bool) -> Value {
    let mut list = Vec::new();
    if with_root && let Some(root) = catalogue().iter().find(|p| p.name == ROOT) {
        list.push(input(root, ""));
    }
    list.extend(send.iter().map(|p| input(p, parent)));
    json!({ WIRE_ROLES_KEY: list })
}

// ---------------------------------------------------------------- what the project has

/// A pack in the project and what an import recorded in it.
#[derive(Debug, Clone)]
pub struct Local {
    pub role: Role,
    pub path: PathBuf,
    /// The version InOrbit gave it when we imported it.
    pub imported_version: Option<i64>,
    /// The catalogue version that import sent.
    pub imported_catalogue: Option<u32>,
}

/// The pack files of `root` (`atlas/roles/*.md`), by name: the one list `locals` and the
/// runtime's role lookup both read.
pub(crate) fn pack_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root.join(DIR)) else {
        return Vec::new();
    };
    let mut names: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    names.sort();
    names
}

/// The packs of `root` that name a role of the catalogue.
#[must_use]
pub fn locals(root: &Path) -> Vec<Local> {
    let cat = catalogue();
    pack_paths(root)
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let name = atlas::get(&text, "name")?;
            if name == ROOT {
                return None;
            }
            let role = cat.iter().find(|p| p.name == name)?.clone();
            Some(Local {
                role,
                imported_version: atlas::get(&text, "imported_version")
                    .and_then(|v| v.trim().parse().ok()),
                imported_catalogue: atlas::get(&text, "imported_catalogue")
                    .and_then(|v| v.trim().parse().ok()),
                path,
            })
        })
        .collect()
}

pub mod lint;
pub(crate) mod notes;
pub(crate) mod runtime;

// ---------------------------------------------------------------- the import

/// One role's result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Outcome {
    pub name: String,
    /// `written`, `upgraded`, `would write`, `would upgrade`, `unchanged`, `left alone`, `local` or
    /// `failed`.
    pub action: &'static str,
    pub version: Option<i64>,
    /// For `upgraded`: the version the account had before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    pub note: String,
}

/// What an import did, for the person and for `--json`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
    /// Whether a session answered.
    pub signed_in: bool,
    /// The one line about what could not happen, when something could not.
    pub message: Option<String>,
    /// The body that is, or in a dry run would be, sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Report {
    /// The lines to show: one per role, then the message once.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .outcomes
            .iter()
            .map(|o| {
                let v = match (o.from, o.version) {
                    (Some(from), Some(to)) => format!(" v{from} \u{2192} v{to}"),
                    (Some(from), None) => format!(" v{from} \u{2192} next"),
                    (None, v) => v.map_or_else(String::new, |v| format!(" v{v}")),
                };
                let n = if o.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", o.note)
                };
                format!("{}: {}{v}{n}", o.name, o.action)
            })
            .collect();
        if let Some(m) = &self.message {
            out.push(m.clone());
        }
        out
    }
}

fn number(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// What InOrbit says about a failed call, in one line.
fn why(e: &inorbithr::Error) -> String {
    match e.status() {
        Some(401) => "not signed in: run `iohr login`".to_owned(),
        Some(403) => "refused: only the account owner or an admin can import roles, \
                      and an API key has no coordinator scope"
            .to_owned(),
        Some(s) => format!("InOrbit answered {s}: {e}"),
        None => format!("InOrbit could not be reached: {e}"),
    }
}

/// What InOrbit lists: name to (parent, head version).
fn remote_of(listed: Option<&Value>) -> BTreeMap<String, (String, i64)> {
    listed
        .and_then(|v| v.get(WIRE_ROLES_KEY))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let name = p.get("name")?.as_str()?.to_owned();
                    let parent = p.get("parent")?.as_str().unwrap_or_default().to_owned();
                    Some((name, (parent, number(p.get("head_version")?).unwrap_or(0))))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What InOrbit's own history says about an account's copy of a role the project has no
/// record of (a fresh project, the copy came from an earlier one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Its lines are the catalogue's lines as this build carries them.
    Current,
    /// Every version of it was an import and its lines are not this build's: an older
    /// built-in nobody changed since.
    OlderBuiltIn,
    /// Something other than an import wrote a version (a learned line, a revert), or the
    /// lines are the person's own.
    Edited,
}

/// Reads the account's copy of `role`: its head's lines against this catalogue's, its parent
/// against the one it should hang under (`same_parent`), and whether every version in its
/// history was an import (`ListPersonalityVersions` names the cause of each: `import`,
/// `auto_applied`, `applied`, `reverted`). A different parent is a change like a different
/// line: the copy is current only when both are the catalogue's. `None` when InOrbit does not
/// answer, and the role is then left alone as before.
async fn account_copy(api: &Api, role: &Role, same_parent: bool) -> Option<Provenance> {
    let path = format!("{WIRE_ROLES_PATH}/{}", role.name);
    let head = api
        .send(Method::Get, &path, &[], None)
        .await
        .ok()?
        .json::<Value>()
        .ok()?;
    let versions = api
        .send(Method::Get, &format!("{path}/versions"), &[], None)
        .await
        .ok()?
        .json::<Value>()
        .ok()?;
    let mut theirs: Vec<(String, String, String)> = head
        .pointer("/lines")
        .and_then(Value::as_array)?
        .iter()
        .filter(|l| l.get("origin").and_then(Value::as_str) != Some("learned"))
        .map(|l| {
            let g = |k: &str| {
                l.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            };
            (g("place"), g("section"), g("text"))
        })
        .collect();
    let mut mine: Vec<(String, String, String)> = role
        .lines
        .iter()
        .map(|l| (l.place.clone(), l.section.clone(), l.text.trim().to_owned()))
        .collect();
    mine.sort();
    theirs.sort();
    let all_imports = versions
        .pointer("/versions")
        .and_then(Value::as_array)
        .is_some_and(|a| {
            !a.is_empty()
                && a.iter()
                    .all(|v| v.get("cause").and_then(Value::as_str) == Some("import"))
        });
    Some(if mine == theirs && same_parent {
        Provenance::Current
    } else if all_imports {
        Provenance::OlderBuiltIn
    } else {
        Provenance::Edited
    })
}

/// What [`account_copy`] finds for each role the account has and this project never imported;
/// each should hang under `parent`.
async fn copies_of(
    api: &Api,
    found: &[Local],
    remote: &BTreeMap<String, (String, i64)>,
    parent: &str,
) -> BTreeMap<String, Provenance> {
    let mut copies = BTreeMap::new();
    for f in found {
        if f.imported_version.is_none()
            && let Some((theirs, _)) = remote.get(&f.role.name)
            && let Some(c) = account_copy(api, &f.role, theirs == parent).await
        {
            copies.insert(f.role.name.clone(), c);
        }
    }
    copies
}

/// The command that takes the catalogue's version over an edited copy.
const UPGRADE_HINT: &str = "`iohr atlas roles import --upgrade`";

/// Why an account's copy is upgraded with no `--upgrade`: InOrbit's history says nobody
/// changed it there, so the catalogue's newer lines replace only what an import wrote.
const NOT_EDITED: &str = "not edited in InOrbit";

/// Why an edited copy is replaced: the person asked.
const ON_ASK: &str = "--upgrade: the catalogue's version replaces your copy";

/// A role to send and why, said in its outcome; empty for a role new to the account.
type Sending<'a> = (&'a Local, String);

/// Tells each local role apart by what InOrbit has; the ones to send are returned with why,
/// the others are said in the report. `copies` is what [`account_copy`] found for the roles
/// this project has no import record of; `parent` is the role each should hang under. A parent
/// that differs is a change under the same rules as a line that differs. With `upgrade`, an
/// edited copy is sent too.
fn decide<'a>(
    found: &'a [Local],
    remote: &BTreeMap<String, (String, i64)>,
    copies: &BTreeMap<String, Provenance>,
    parent: &str,
    upgrade: bool,
    report: &mut Report,
) -> Vec<Sending<'a>> {
    let mut send: Vec<Sending<'a>> = Vec::new();
    for f in found {
        let name = &f.role.name;
        let newer = format!(
            "catalogue v{} differs from your copy: {UPGRADE_HINT} takes it",
            f.role.version
        );
        let Some((theirs, head)) = remote.get(name) else {
            send.push((f, String::new()));
            continue;
        };
        let moved = (theirs != parent).then(|| format!("parent {theirs} \u{2192} {parent}"));
        let (action, note) = match f.imported_version {
            None => match copies.get(name) {
                Some(Provenance::OlderBuiltIn) => {
                    let why = moved.map_or_else(
                        || format!("an older built-in, {NOT_EDITED}"),
                        |m| format!("an older built-in, {NOT_EDITED}; {m}"),
                    );
                    send.push((f, why));
                    continue;
                }
                Some(Provenance::Current) => ("unchanged", String::new()),
                Some(Provenance::Edited) if upgrade => {
                    send.push((f, ON_ASK.to_owned()));
                    continue;
                }
                Some(Provenance::Edited) => ("left alone", format!("you changed it; {newer}")),
                None => (
                    "left alone",
                    "already in your account, not from this project".to_owned(),
                ),
            },
            Some(v) if v != *head => {
                if upgrade {
                    send.push((f, ON_ASK.to_owned()));
                    continue;
                }
                let differs =
                    f.imported_catalogue.is_some_and(|c| c < f.role.version) || moved.is_some();
                let note = if differs {
                    format!("changed in InOrbit since the import; {newer}")
                } else {
                    "changed in InOrbit since the import".to_owned()
                };
                ("left alone", note)
            }
            Some(_) => {
                let mut why: Vec<String> = Vec::new();
                if let Some(c) = f.imported_catalogue.filter(|c| *c < f.role.version) {
                    why.push(format!("catalogue v{c} \u{2192} v{}", f.role.version));
                }
                why.extend(moved);
                if why.is_empty() {
                    ("unchanged", String::new())
                } else {
                    why.push(format!("{NOT_EDITED} since the import"));
                    send.push((f, why.join("; ")));
                    continue;
                }
            }
        };
        report.outcomes.push(Outcome {
            name: name.clone(),
            action,
            version: Some(*head),
            from: None,
            note,
        });
    }
    send
}

/// What becomes of the account's root, decided before anything is sent: written when the
/// account has none, upgraded when it is this catalogue's root and InOrbit's history says
/// nobody edited it (or on `--upgrade`), else kept. Another root is always kept: the built-ins
/// hang under it and inherit its lines. `Some((from, why))` when the root goes.
async fn plan_root(
    api: &Api,
    remote: &BTreeMap<String, (String, i64)>,
    root_name: Option<&str>,
    upgrade: bool,
    report: &mut Report,
) -> Option<(Option<i64>, String)> {
    let Some(name) = root_name else {
        return Some((None, "the account had no root role".to_owned()));
    };
    if name != ROOT {
        return None;
    }
    let root = root_role()?;
    let head = remote.get(ROOT).map(|(_, v)| *v);
    match account_copy(api, &root, true).await? {
        Provenance::Current => None,
        Provenance::OlderBuiltIn => Some((head, format!("an older built-in, {NOT_EDITED}"))),
        Provenance::Edited if upgrade => Some((head, ON_ASK.to_owned())),
        Provenance::Edited => {
            report.outcomes.push(Outcome {
                name: ROOT.to_owned(),
                action: "left alone",
                version: head,
                from: None,
                note: format!(
                    "you changed it; catalogue v{} differs from your copy: {UPGRADE_HINT} takes it",
                    root.version
                ),
            });
            None
        }
    }
}

/// Takes out of `send` (and the root out of `root`) what InOrbit's import would refuse
/// ([`lint::findings`]), each as `failed` with the first reason, so nothing is sent that would
/// fail the batch.
fn refuse_locally(
    send: &mut Vec<Sending<'_>>,
    root: &mut Option<(Option<i64>, String)>,
    report: &mut Report,
) {
    let mut refused: Vec<String> = Vec::new();
    let mut fail = |name: &str, f: &[String], report: &mut Report| {
        let more = if f.len() > 1 {
            format!(" (and {} more)", f.len() - 1)
        } else {
            String::new()
        };
        report.outcomes.push(Outcome {
            name: name.to_owned(),
            action: "failed",
            version: None,
            from: None,
            note: format!("not sent: {}{more}", f[0]),
        });
        refused.push(name.to_owned());
    };
    send.retain(|(l, _)| {
        let f = lint::findings(&l.role);
        if f.is_empty() {
            return true;
        }
        fail(&l.role.name, &f, report);
        false
    });
    if root.is_some()
        && let Some(r) = root_role()
    {
        let f = lint::findings(&r);
        if !f.is_empty() {
            fail(ROOT, &f, report);
            *root = None;
        }
    }
    if !refused.is_empty() {
        report.message = Some(format!(
            "Not sent, because InOrbit would refuse it: {}. A built-in line fails the service's \
             lint; this build of iohr should not have shipped it.",
            refused.join(", ")
        ));
    }
}

/// Sends the import; what InOrbit wrote as name to version, or the status and the line
/// saying why not.
async fn post(api: &Api, body: &Value) -> Result<BTreeMap<String, i64>, (Option<u16>, String)> {
    let v = api
        .send(Method::Post, WIRE_IMPORT_PATH, &[], Some(body))
        .await
        .and_then(|r| r.json::<Value>())
        .map_err(|e| (e.status(), why(&e)))?;
    Ok(v.pointer("/written")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|w| {
                    Some((
                        w.get("name")?.as_str()?.to_owned(),
                        number(w.get("version")?)?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Records the import in the pack file; the note is what went wrong, when something did.
fn write_back(f: &Local, v: i64) -> String {
    let Ok(text) = std::fs::read_to_string(&f.path) else {
        return "written, but the pack file could not be read".to_owned();
    };
    let next = atlas::set_keys(
        &text,
        &[
            ("imported_version", v.to_string()),
            ("imported_catalogue", f.role.version.to_string()),
        ],
    );
    match std::fs::write(&f.path, next) {
        Ok(()) => String::new(),
        Err(_) => "written, but the pack file could not record it".to_owned(),
    }
}

/// Every role stays in the project; `why` is said once.
fn local_only(found: &[Local], report: &mut Report, why: &str) {
    for f in found {
        report.outcomes.push(Outcome {
            name: f.role.name.clone(),
            action: "local",
            version: None,
            from: None,
            note: String::new(),
        });
    }
    report.message = Some(why.to_owned());
}

/// One outcome for each role that was sent: `written` or, over an account's copy, `upgraded`
/// with the version it had and why; `failed` when InOrbit did not say it was written.
fn record_written(
    send: Vec<Sending<'_>>,
    written: &BTreeMap<String, i64>,
    remote: &BTreeMap<String, (String, i64)>,
    report: &mut Report,
) {
    for (f, why) in send {
        let name = f.role.name.clone();
        let Some(&v) = written.get(&name) else {
            report.outcomes.push(Outcome {
                name,
                action: "failed",
                version: None,
                from: None,
                note: "InOrbit did not say it was written".to_owned(),
            });
            continue;
        };
        let note = [why, write_back(f, v)]
            .into_iter()
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let from = remote.get(&name).map(|(_, v)| *v);
        report.outcomes.push(Outcome {
            name,
            action: if from.is_some() {
                "upgraded"
            } else {
                "written"
            },
            version: Some(v),
            from,
            note,
        });
    }
}

/// Imports the packs of `root` into the account of `api`, one role at a time told
/// apart by what is already there:
/// - absent in InOrbit: imported;
/// - present, never imported from this project: read its history ([`account_copy`]); an
///   unchanged older built-in is `upgraded` (`v1 → v2`, "not edited in InOrbit"), one the
///   person changed is `left alone` and names the command (`--upgrade`) that takes the
///   catalogue's version;
/// - present and changed in InOrbit since our import: `left alone`, the same hint;
/// - present at the version we imported and the catalogue moved on, or its parent is not the
///   one it should hang under: `upgraded`, saying which;
/// - otherwise `unchanged`.
///
/// The root goes as [`plan_root`] says. A role the service's lint would refuse is not sent
/// ([`refuse_locally`]). With `dry`, nothing is sent and nothing written, and the report holds
/// the body. With no `api`, every role stays local, with one message saying so. Never fails.
pub(crate) async fn import(api: Option<&Api>, root: &Path, dry: bool, upgrade: bool) -> Report {
    let found = locals(root);
    let mut report = Report::default();
    if found.is_empty() {
        return report;
    }
    let Some(api) = api else {
        let refs: Vec<&Role> = found.iter().map(|f| &f.role).collect();
        if dry {
            report.body = Some(import_body(&refs, ROOT, true));
        }
        local_only(
            &found,
            &mut report,
            &format!("Not signed in: {}", kept_local("run `iohr login` first")),
        );
        return report;
    };
    let listed = match api.send(Method::Get, WIRE_ROLES_PATH, &[], None).await {
        Ok(raw) => raw.json::<Value>().ok(),
        Err(e) => {
            local_only(
                &found,
                &mut report,
                &format!("The roles stay local: {}.", why(&e)),
            );
            return report;
        }
    };
    report.signed_in = true;
    let remote = remote_of(listed.as_ref());
    let root_name = remote
        .iter()
        .find(|(_, (parent, _))| parent.is_empty())
        .map(|(n, _)| n.clone());
    let parent = root_name.clone().unwrap_or_else(|| ROOT.to_owned());
    // A role the account has and this project never imported: its history says whether it
    // is an unchanged built-in (upgraded) or the person's own (left alone, with the command).
    let copies = copies_of(api, &found, &remote, &parent).await;
    let mut send = decide(&found, &remote, &copies, &parent, upgrade, &mut report);
    let mut root_goes = plan_root(api, &remote, root_name.as_deref(), upgrade, &mut report).await;
    // A missing root goes with the roles that hang under it; an older one goes on its own.
    if send.is_empty() && root_name.is_none() {
        root_goes = None;
    }
    refuse_locally(&mut send, &mut root_goes, &mut report);
    if send.is_empty() && root_goes.is_none() {
        return report;
    }
    let with_root = root_goes.is_some();
    let refs: Vec<&Role> = send.iter().map(|(f, _)| &f.role).collect();
    let body = import_body(&refs, &parent, with_root);
    if dry {
        dry_rows(root_goes.as_ref(), &send, &remote, &mut report);
        report.body = Some(body);
        return report;
    }
    let written = match post(api, &body).await {
        Ok(w) => w,
        // InOrbit lints the batch as a whole, so one refused role (400) would keep every
        // other one local: send each alone and keep only the refused ones back.
        Err((Some(400), _)) if send.len() > 1 => {
            let locals: Vec<&Local> = send.iter().map(|(f, _)| *f).collect();
            let (written, refused) = one_by_one(api, &locals, &parent, with_root).await;
            let ok: Vec<Sending<'_>> = send
                .into_iter()
                .filter(|(f, _)| !refused.iter().any(|(n, _)| *n == f.role.name))
                .collect();
            record_written(ok, &written, &remote, &mut report);
            record_refused(&refused, &mut report);
            root_outcome(root_goes, &written, &mut report);
            return report;
        }
        Err((_, line)) => {
            for (f, _) in &send {
                report.outcomes.push(Outcome {
                    name: f.role.name.clone(),
                    action: "failed",
                    version: None,
                    from: None,
                    note: String::new(),
                });
            }
            report.message = Some(format!(
                "The roles stay local ({line}). `iohr sync` tries again."
            ));
            return report;
        }
    };
    record_written(send, &written, &remote, &mut report);
    root_outcome(root_goes, &written, &mut report);
    report
}

/// A `failed` outcome with InOrbit's line for each refused role, and one message naming them.
fn record_refused(refused: &[(String, String)], report: &mut Report) {
    for (name, line) in refused {
        report.outcomes.push(Outcome {
            name: name.clone(),
            action: "failed",
            version: None,
            from: None,
            note: line.clone(),
        });
    }
    let names: Vec<&str> = refused.iter().map(|(n, _)| n.as_str()).collect();
    report.message = Some(format!(
        "InOrbit refused {}; the other roles were imported. Edit the refused line and \
         run `iohr sync`.",
        names.join(", ")
    ));
}

/// What a dry run would send, one `would write` or `would upgrade` row each, the root first.
fn dry_rows(
    root_goes: Option<&(Option<i64>, String)>,
    send: &[Sending<'_>],
    remote: &BTreeMap<String, (String, i64)>,
    report: &mut Report,
) {
    let root_row = root_goes.map(|(from, why)| (ROOT.to_owned(), *from, why.clone()));
    let rows = root_row.into_iter().chain(send.iter().map(|(f, why)| {
        (
            f.role.name.clone(),
            remote.get(&f.role.name).map(|(_, v)| *v),
            why.clone(),
        )
    }));
    for (name, from, note) in rows {
        report.outcomes.push(Outcome {
            name,
            action: if from.is_some() {
                "would upgrade"
            } else {
                "would write"
            },
            version: None,
            from,
            note,
        });
    }
}

/// The root role's outcome, when this import sent it (`root_goes`, from [`plan_root`]) and
/// InOrbit wrote it: `written` into an account that had none, or `upgraded` with why.
fn root_outcome(
    root_goes: Option<(Option<i64>, String)>,
    written: &BTreeMap<String, i64>,
    report: &mut Report,
) {
    let Some((from, note)) = root_goes else {
        return;
    };
    if let Some(&v) = written.get(ROOT) {
        report.outcomes.push(Outcome {
            name: ROOT.to_owned(),
            action: if from.is_some() {
                "upgraded"
            } else {
                "written"
            },
            version: Some(v),
            from,
            note,
        });
    }
}

/// Sends each role alone, the root with the first one InOrbit takes: what was written, and
/// each refused role with InOrbit's line.
async fn one_by_one(
    api: &Api,
    send: &[&Local],
    parent: &str,
    mut with_root: bool,
) -> (BTreeMap<String, i64>, Vec<(String, String)>) {
    let mut written = BTreeMap::new();
    let mut refused = Vec::new();
    for f in send {
        match post(api, &import_body(&[&f.role], parent, with_root)).await {
            Ok(w) => {
                with_root = with_root && !w.contains_key(ROOT);
                written.extend(w);
            }
            Err((_, line)) => refused.push((f.role.name.clone(), line)),
        }
    }
    (written, refused)
}

// ---------------------------------------------------------------- agent files

/// The marker that says the section is in an agent-instructions file.
pub const AGENTS_MARK: &str = "<!-- iohr:memory -->";

/// How an agent works with this project's documents, memory and sync, with `bin` as the
/// command that made the project (`iohr`, or `iohr-preview`). One text for the agent files
/// and for the prompt of an agent `init` starts, so they never differ.
#[must_use]
pub fn usage_lines(bin: &str) -> String {
    format!(
        "- Documents: PRDs, RFCs, ADRs and studies are Markdown files in `atlas/prd/`, \
         `atlas/rfc/`, `atlas/adr/` and `atlas/studies/`, each starting with front matter \
         (`title`, `status`, `date`, `public`, `summary`). Make one with `{bin} atlas new \
         <prd|rfc|adr|study> \"<title>\"`: it takes the next free number and writes the front \
         matter and the headings; then edit the file and write a one-sentence `summary`. A file \
         that still says \"Stub\" is a starter: rewrite it rather than add another.\n\
         - Memory: before work, run `{bin} memory recall \"<topic>\"`. After a lesson, run \
         `{bin} memory remember \"<fact>\" --why \"<reason>\"`. Memories are files in \
         `.inorbit/memory/`, one per memory, committed with the code; no sign-in is needed. \
         `{bin} memory check` finds the wrong and the stale ones. See \
         `.inorbit/memory/README.md`.\n\
         - Sync: the person sends `atlas/` to the project's Decisions space with `{bin} sync`. \
         `{bin} atlas sync --dry-run` lists what would be sent and sends nothing.\n\
         - Context: `IO.md` (or `io.md`) files hold what the person wants Io to know about a folder \
         and its project; `?` and `??` load them from the project root down to the current \
         folder, the nearest last. `{bin} context` shows what would be loaded; keep them short.\n\
         - The built-in roles for this project are in `atlas/roles/`.\n"
    )
}

/// The short section of CLAUDE.md or AGENTS.md: how the project's agents use its documents,
/// memory and sync. Memory is local first, so nothing here needs a sign-in.
#[must_use]
pub fn agents_section(bin: &str) -> String {
    format!(
        "\n{AGENTS_MARK}\n## Project documents, memory and roles\n\n{}",
        usage_lines(bin)
    )
}

fn instruction_files(files: &[(String, String)]) -> Vec<usize> {
    files
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| p == "AGENTS.md" || p == "CLAUDE.md")
        .map(|(i, _)| i)
        .collect()
}

/// Adds the memory section to the agent-instructions file among a template's files, or an
/// `AGENTS.md` when the template ships none.
pub fn add_agents(files: &mut Vec<(String, String)>, bin: &str) {
    let found = instruction_files(files);
    if found.is_empty() {
        files.push((
            "AGENTS.md".to_owned(),
            format!("# Agent instructions\n{}", agents_section(bin)),
        ));
        return;
    }
    for i in found {
        if !files[i].1.contains(AGENTS_MARK) {
            files[i].1.push_str(&agents_section(bin));
        }
    }
}

/// The same for an existing project: the section goes into each of `AGENTS.md` and
/// `CLAUDE.md` that exists and lacks it, and an `AGENTS.md` is made when neither exists.
/// Returns the files it wrote.
///
/// # Errors
///
/// A message saying what could not be written.
pub fn ensure_agents(root: &Path, bin: &str) -> Result<Vec<String>, String> {
    let mut wrote = Vec::new();
    let mut any = false;
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = root.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        any = true;
        if text.contains(AGENTS_MARK) {
            continue;
        }
        let mut next = text;
        if !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&agents_section(bin));
        std::fs::write(&path, next).map_err(|e| format!("cannot write {name}: {e}"))?;
        wrote.push(name.to_owned());
    }
    if !any {
        let path = root.join("AGENTS.md");
        std::fs::write(
            &path,
            format!("# Agent instructions\n{}", agents_section(bin)),
        )
        .map_err(|e| format!("cannot write AGENTS.md: {e}"))?;
        wrote.push("AGENTS.md".to_owned());
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_text(p: &Role) -> String {
        p.lines
            .iter()
            .filter(|l| l.place == "core")
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_catalogue_parses_and_every_line_has_a_source() {
        let cat = catalogue();
        assert_eq!(cat.len(), FILES.len(), "every file parses");
        for p in &cat {
            assert!(!p.lines.is_empty(), "{} has lines", p.name);
            for l in &p.lines {
                assert!(
                    !l.cites.is_empty(),
                    "{}: a line without a source: {}",
                    p.name,
                    l.text
                );
                for c in &l.cites {
                    assert!(c.starts_with("https://"), "{}: {c}", p.name);
                }
                assert!(
                    l.place == "core" || l.place == "recall",
                    "{}: {}",
                    p.name,
                    l.place
                );
            }
            assert!(
                p.lines.iter().any(|l| l.place == "core"),
                "{} has a core",
                p.name
            );
            assert!(p.lines.iter().any(|l| l.place == "recall") || p.name == ROOT);
        }
    }

    #[test]
    fn the_catalogue_has_one_root_and_children_of_it() {
        let cat = catalogue();
        let roots: Vec<_> = cat.iter().filter(|p| p.parent.is_empty()).collect();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, ROOT);
        for p in cat.iter().filter(|p| p.name != ROOT) {
            assert_eq!(p.parent, ROOT, "{}", p.name);
        }
        let langs: Vec<_> = cat.iter().filter_map(|p| p.language.as_deref()).collect();
        for want in ["rust", "python", "typescript", "go"] {
            assert!(langs.contains(&want), "{want}");
        }
        assert!(cat.iter().any(|p| p.name == BASE));
    }

    /// What the service's import would refuse ([`lint::findings`]), the budget a run's core
    /// keeps to (the role's own core and the core it inherits), and the catalogue's own bar: no
    /// line that would widen what an agent may do (core `text.rs` `WIDEN`, 00c977e9d), which the
    /// service applies to a change of a role.
    #[test]
    fn the_catalogue_passes_the_services_lint_in_what_a_client_can_check() {
        let widen = regex::Regex::new(concat!(
            r"(?i)(\b(allow(ed|s)? to|grant(ed|s)?|permission|may now|can now|without (asking|approval|a go|the owner|review)|",
            r"no need to (ask|check|wait)|not needed|optional|unnecessary|skip (the )?(check|gate|review|tests?|ci)|",
            r"bypass|auto-?approve|force-?push|you can (merge|roll|deploy|publish|release)|merge (it )?yourself|",
            r"don'?t (need to )?(ask|wait|check))\b|(^|\s)--admin\b)"
        ))
        .unwrap();
        let override_words = regex::Regex::new(lint::OVERRIDE).unwrap();
        for p in catalogue() {
            assert_eq!(lint::findings(&p), Vec::<String>::new(), "{}", p.name);
            assert!(
                core_tokens(&p) <= PACK_CORE_MAX,
                "{} own core is {} tokens",
                p.name,
                core_tokens(&p)
            );
            // The base loads with every role: the two together, gates included, stay in budget.
            let mut chain = ancestors(&p);
            chain.push(p.clone());
            let gates: usize = chain
                .iter()
                .flat_map(|r| &r.checks)
                .map(|c| tokens(&c.command) + tokens(&c.proves))
                .sum();
            assert!(
                brief_core_tokens(&p) + gates <= LOADED_CORE_MAX,
                "{} core with the base it loads beside is {} tokens",
                p.name,
                brief_core_tokens(&p) + gates
            );
            for c in &p.checks {
                assert!(crate::privacy::refuse(&c.command).is_none(), "{}", p.name);
            }
            assert!(crate::privacy::refuse(&p.title).is_none(), "{}", p.name);
            if p.parent.is_empty() {
                continue;
            }
            assert!(!override_words.is_match(&core_text(&p)), "{} core", p.name);
            for l in &p.lines {
                assert!(!widen.is_match(&l.text), "{}: {}", p.name, l.text);
            }
        }
        assert_eq!(selection(&Stack::default()).len(), 2);
    }

    #[test]
    fn every_role_says_the_day_it_was_checked_and_it_has_come() {
        let today = chrono::Local::now().date_naive();
        for p in catalogue() {
            for (who, day) in std::iter::once((p.name.clone(), p.checked.as_str())).chain(
                p.lines.iter().filter_map(|l| {
                    l.checked
                        .as_deref()
                        .map(|d| (format!("{}: {}", p.name, l.text), d))
                }),
            ) {
                let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                    .unwrap_or_else(|e| panic!("{who}: checked {day:?} is not YYYY-MM-DD: {e}"));
                assert!(d <= today, "{who}: checked {day} is in the future");
            }
        }
    }

    #[test]
    fn a_line_every_engineer_needs_is_kept_once_in_the_root() {
        let cat = catalogue();
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for p in &cat {
            for l in &p.lines {
                if let Some(first) = seen.insert(&l.text, &p.name) {
                    panic!("{first} and {} both hold: {}", p.name, l.text);
                }
            }
        }
        let root = root_role().unwrap();
        assert!(
            root.lines
                .iter()
                .any(|l| l.text.starts_with("Put a timeout on every remote call")),
            "the root holds the timeout line"
        );
        for p in cat.iter().filter(|p| p.language.is_some()) {
            assert!(
                !p.lines
                    .iter()
                    .any(|l| l.text.contains("Put a timeout on every remote call")),
                "{} repeats the root",
                p.name
            );
            assert_eq!(
                ancestors(p)
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>(),
                [ROOT]
            );
        }
    }

    #[test]
    fn a_project_gets_the_base_security_and_its_language() {
        let names = |l: Option<&str>| {
            selection(&Stack::of(l))
                .into_iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(None), [ROOT, "security"]);
        assert_eq!(names(Some("rust")), [ROOT, "security", "rust-engineer"]);
        assert_eq!(names(Some("golang")), [ROOT, "security", "go-engineer"]);
        assert_eq!(names(Some("TS")), [ROOT, "security", "typescript-engineer"]);
        assert_eq!(names(Some("cobol")), [ROOT, "security"]);
    }

    #[test]
    fn a_repo_of_two_languages_gets_both_engineers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module x\n").unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        std::fs::write(dir.path().join("web/package.json"), "{}").unwrap();
        let s = stack(dir.path(), None);
        assert_eq!(s.languages, ["go", "typescript"]);
        let names: Vec<String> = selection(&s).into_iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [ROOT, "security", "typescript-engineer", "go-engineer"]
        );
        // A template's answer comes first and is not repeated.
        let s = stack(dir.path(), Some("ts"));
        assert_eq!(s.languages, ["typescript", "go"]);
        // A dependency folder is not the project's.
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(dir.path().join("node_modules/x/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join(".hidden/pyproject.toml"), "").unwrap();
        assert_eq!(stack(dir.path(), None).languages, ["go", "typescript"]);
    }

    fn fixture(name: &str, markers: &[&str]) -> Role {
        let mut r = catalogue().remove(1);
        r.name = name.to_owned();
        r.language = None;
        r.markers = markers.iter().map(|m| (*m).to_owned()).collect();
        r
    }

    #[test]
    fn files_turn_on_the_roles_that_name_them() {
        let cat = || {
            let mut c = catalogue();
            c.push(fixture("infra", &["Dockerfile", "*.tf"]));
            c.push(fixture("sql", &["migrations/", "*.sql"]));
            c
        };
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            stack_in(&cat(), dir.path(), None).marked,
            Vec::<String>::new()
        );
        std::fs::create_dir(dir.path().join("deploy")).unwrap();
        std::fs::write(dir.path().join("deploy/main.tf"), "").unwrap();
        let s = stack_in(&cat(), dir.path(), None);
        assert_eq!(s.marked, ["infra"]);
        std::fs::create_dir(dir.path().join("migrations")).unwrap();
        let s = stack_in(&cat(), dir.path(), Some("rust"));
        assert_eq!(s.marked, ["infra", "sql"]);
        let names: Vec<String> = select(cat(), &s).into_iter().map(|p| p.name).collect();
        assert_eq!(names, [ROOT, "security", "rust-engineer", "infra", "sql"]);
        // Each kind of marker, alone.
        let files = |f: &[&str]| f.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert!(marker_matches("Dockerfile", &files(&["Dockerfile"]), &[]));
        assert!(!marker_matches(
            "Dockerfile",
            &files(&["Dockerfile.dev"]),
            &[]
        ));
        assert!(marker_matches("*.sql", &files(&["001_init.sql"]), &[]));
        assert!(!marker_matches("*.sql", &files(&["sql"]), &[]));
        assert!(marker_matches("migrations/", &[], &files(&["migrations"])));
        assert!(!marker_matches("migrations/", &files(&["migrations"]), &[]));
    }

    #[test]
    fn a_check_parses_and_blocks_unless_it_says_not() {
        let gates: toml::Table = r#"
[[check]]
command = "cargo nextest run"
proves = "the tests pass"

[[check]]
command = "cargo deny check"
proves = "no banned licence"
blocks = false
"#
        .parse()
        .unwrap();
        let mut t: toml::Table = FILES[1].1.parse().unwrap();
        t.extend(gates);
        let r: Role = t.clone().try_into().unwrap();
        assert_eq!(r.checks.len(), 2);
        assert!(r.checks[0].blocks);
        assert!(!r.checks[1].blocks);
        let text = pack(&r);
        let gates = text.find(CHECKS_HEADING).unwrap();
        assert!(gates > text.find("## Core").unwrap());
        assert!(text.contains("- `cargo nextest run`\n  Proves: the tests pass"));
        assert!(text.contains("- `cargo deny check` (reports only)"));
        // A misspelt field is refused, not dropped.
        let mut bad = t;
        bad.insert(
            "check".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(
                "command = \"x\"\nproves = \"y\"\nblock = false\n"
                    .parse()
                    .unwrap(),
            )]),
        );
        assert!(Role::deserialize(bad).is_err());
    }

    #[test]
    fn a_role_without_checks_has_no_gate_section() {
        for p in catalogue() {
            assert_eq!(
                pack(&p).contains(CHECKS_HEADING),
                !p.checks.is_empty(),
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn the_pack_has_front_matter_and_every_source() {
        let p = selection(&Stack::of(Some("rust"))).remove(2);
        let text = pack(&p);
        assert_eq!(atlas::get(&text, "name").as_deref(), Some("rust-engineer"));
        assert_eq!(
            atlas::get(&text, "parent").as_deref(),
            Some("base-engineer")
        );
        assert!(atlas::get(&text, "domain").is_some_and(|d| d.contains("tokio")));
        assert_eq!(
            atlas::get(&text, "catalogue_version").as_deref(),
            Some(p.version.to_string().as_str())
        );
        assert_eq!(
            atlas::get(&text, "checked").as_deref(),
            Some(p.checked.as_str())
        );
        assert!(atlas::get(&text, "source").is_some_and(|s| s.starts_with("iohr ")));
        let root = root_role().unwrap();
        for l in p.lines.iter().chain(&root.lines) {
            assert!(text.contains(l.text.as_str()), "{}", l.text);
            for c in &l.cites {
                assert!(text.contains(c.as_str()), "{c}");
            }
        }
        let inherited = text.find("(inherited from base-engineer)").unwrap();
        let own = text.find(p.lines[0].text.as_str()).unwrap();
        assert!(inherited < own, "the inherited core comes first");
        assert!(text.contains("changes what `--role` gives in this project"));
        assert!(text.contains("`iohr sync` does not send it"));
    }

    #[test]
    fn the_import_body_is_what_the_service_takes() {
        // The root goes once, as the root: what a project sends is its roles without it.
        let sel: Vec<Role> = selection(&Stack::of(Some("python")))
            .into_iter()
            .filter(|p| p.name != ROOT)
            .collect();
        let refs: Vec<&Role> = sel.iter().collect();
        let body = import_body(&refs, "base-engineer", true);
        let list = body[WIRE_ROLES_KEY].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["name"], "base-engineer");
        assert_eq!(list[0]["parent"], "");
        assert_eq!(list[1]["name"], "security");
        assert_eq!(list[1]["parent"], "base-engineer");
        assert_eq!(list[2]["settings"]["model"], "sonnet");
        let first = &list[1]["sections"][0];
        assert_eq!(first["place"], "core");
        assert_eq!(first["lines"][0]["origin"], "doctrine");
        assert_eq!(first["lines"][0]["names_categories"], false);
        assert!(
            first["lines"][0]["cites"][0]
                .as_str()
                .unwrap()
                .starts_with("https://")
        );
        let without = import_body(&refs, "company-root", false);
        assert_eq!(without[WIRE_ROLES_KEY].as_array().unwrap().len(), 2);
        assert_eq!(without[WIRE_ROLES_KEY][0]["parent"], "company-root");
        assert_eq!(
            WIRE_ROLES_KEY, "personalities",
            "a published name, never renamed"
        );
    }

    #[test]
    fn language_is_read_from_the_record_then_the_build_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(stack(dir.path(), None).language(), None);
        std::fs::write(dir.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(stack(dir.path(), None).language(), Some("go"));
        std::fs::write(dir.path().join("inorbit.toml"), "language = \"python\"\n").unwrap();
        assert_eq!(stack(dir.path(), None).languages, ["python"]);
    }

    #[test]
    fn the_record_and_agent_sections_are_added_once() {
        let mut files = vec![
            ("inorbit.toml".to_owned(), "name = \"x\"\n".to_owned()),
            ("CLAUDE.md".to_owned(), "# Mine\n".to_owned()),
        ];
        add_record(&mut files, &Stack::of(Some("rust")));
        add_record(&mut files, &Stack::of(Some("rust")));
        add_agents(&mut files, "iohr");
        add_agents(&mut files, "iohr");
        assert_eq!(files[0].1.matches("[roles]").count(), 1);
        assert!(files[0].1.contains("\"rust-engineer\""));
        assert_eq!(files[1].1.matches(AGENTS_MARK).count(), 1);
        assert!(files[1].1.contains("iohr memory recall"));
        assert_eq!(files.len(), 2, "a shipped file is extended, none is added");
        let mut none: Vec<(String, String)> = Vec::new();
        add_agents(&mut none, "iohr");
        assert_eq!(none[0].0, "AGENTS.md");
        assert!(none[0].1.contains(".inorbit/memory/README.md"));
    }

    #[test]
    fn the_agent_file_names_the_binary_that_made_the_project() {
        let mut files: Vec<(String, String)> = Vec::new();
        add_agents(&mut files, "iohr-preview");
        let text = &files[0].1;
        for want in [
            "`iohr-preview atlas new <prd|rfc|adr|study>",
            "`iohr-preview memory remember",
            "`iohr-preview memory recall",
            "`iohr-preview sync`",
            "`iohr-preview atlas sync --dry-run`",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
        assert!(!text.contains("`iohr "), "no bare iohr left: {text}");
    }
}
