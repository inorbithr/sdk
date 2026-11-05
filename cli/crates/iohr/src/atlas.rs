//! The project's local atlas (PRD 0189, RFC 0205, ADR 0216): the `atlas/` folder in the
//! project root, with its PRDs, RFCs, ADRs and studies as Markdown, a starter canvas and a
//! folder for 3D models.
//!
//! The formats are ones that already exist, so the folder imports without loss later:
//!
//! - documents are Markdown with the front matter the Decisions documents use (`title`,
//!   `status`, `date`, `public`, `summary`); the local-only keys ([`LOCAL_KEYS`]) are
//!   stripped before a push, so Decisions stores the text as the server shapes it;
//! - the canvas is the desktop app's `iohr.canvas` version 3;
//! - 3D is glTF 2.0, and nothing is settled in the desktop yet, so `models/` holds a README
//!   and no model.
//!
//! Everything here is a pure function over text, apart from the three that touch the disk
//! ([`ensure`], [`new_document`] and [`scan`]). Document text is never logged.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest as _, Sha256};

/// The folder, in the project root.
pub const DIR: &str = "atlas";

/// The front matter keys that stay on this machine: the file's own bookkeeping, never sent.
pub const LOCAL_KEYS: [&str; 6] = [
    "kind",
    "number",
    "remote_id",
    "remote_version",
    "remote_number",
    "remote_sha",
];

/// The four kinds of document, each with its own folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Prd,
    Rfc,
    Adr,
    Study,
}

impl Kind {
    /// Every kind, in the order the README lists them.
    pub const ALL: [Kind; 4] = [Kind::Prd, Kind::Rfc, Kind::Adr, Kind::Study];

    /// The folder under `atlas/`.
    #[must_use]
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Prd => "prd",
            Kind::Rfc => "rfc",
            Kind::Adr => "adr",
            Kind::Study => "studies",
        }
    }

    /// The kind as the Decisions API names it.
    #[must_use]
    pub fn api(self) -> &'static str {
        match self {
            Kind::Prd => "prd",
            Kind::Rfc => "rfc",
            Kind::Adr => "adr",
            Kind::Study => "study",
        }
    }

    /// The name a person reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Kind::Prd => "PRD",
            Kind::Rfc => "RFC",
            Kind::Adr => "ADR",
            Kind::Study => "study",
        }
    }

    /// The status a new document of this kind starts in.
    #[must_use]
    pub fn first_status(self) -> &'static str {
        match self {
            Kind::Prd | Kind::Rfc => "draft",
            Kind::Adr => "proposed",
            Kind::Study => "running",
        }
    }

    /// The kind a word names (`prd`, `rfc`, `adr`, `study` or `studies`).
    #[must_use]
    pub fn parse(word: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.api() == word || k.folder() == word)
    }

    /// `(heading, what goes under it)`, in order.
    fn sections(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Kind::Prd => &[
                (
                    "Problem",
                    "Say what is wrong today, and for whom, in plain words.",
                ),
                (
                    "Users",
                    "Who has the problem, and what they do about it now.",
                ),
                (
                    "Goals",
                    "What must be true when this is done. One line each.",
                ),
                ("Out of scope", "What this will not do."),
                (
                    "Success",
                    "How you will measure it. A number, not a feeling.",
                ),
                ("Status log", ""),
            ],
            Kind::Rfc => &[
                ("Problem", "What needs deciding, and why now."),
                ("Proposal", "What to build or change, concretely."),
                (
                    "Alternatives considered",
                    "What else was possible, and why not.",
                ),
                ("Trade-offs", "What this costs and what it buys."),
                ("Open questions", "What is still unknown."),
                ("Decision", "Filled in when it is decided."),
                ("Status log", ""),
            ],
            Kind::Adr => &[
                (
                    "Context",
                    "What forces the decision: constraints, facts, deadlines.",
                ),
                ("Decision", "What was decided, in one or two sentences."),
                ("Consequences", "What gets easier, what gets harder."),
                ("Status log", ""),
            ],
            Kind::Study => &[
                ("The question", "What you want to find out."),
                (
                    "What we believed",
                    "Your guess before measuring, so the result can surprise you.",
                ),
                ("Method", "What was run, on what, how many times."),
                ("Result", "Not measured yet."),
                ("What it means", "What changes because of the result."),
            ],
        }
    }
}

/// What the starters are filled from.
#[derive(Debug, Clone)]
pub struct Meta<'a> {
    /// The project's name.
    pub name: &'a str,
    /// The template it was made from.
    pub template: &'a str,
    /// The language, when the template has one.
    pub lang: Option<&'a str>,
    /// The person's own description, when they gave one. Goes into the PRD's problem only.
    pub describe: Option<&'a str>,
    /// When, as an RFC 3339 timestamp in UTC.
    pub created_at: &'a str,
}

impl Meta<'_> {
    fn date(&self) -> &str {
        self.created_at.get(..10).unwrap_or(self.created_at)
    }
}

// ---------------------------------------------------------------- front matter

/// Splits a document into its front matter (without the `---` lines) and the rest.
#[must_use]
pub fn split(text: &str) -> Option<(&str, &str)> {
    let after = text.strip_prefix("---\n")?;
    let end = after
        .find("\n---\n")
        .map(|i| (i, i + 5))
        .or_else(|| after.strip_suffix("\n---").map(|s| (s.len(), after.len())))?;
    Some((&after[..end.0], &after[end.1..]))
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    if let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    v.to_owned()
}

fn quote(v: &str) -> String {
    let plain = !v.is_empty()
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.starts_with(|c: char| "\"'#&*!|>%@`[]{},-?".contains(c))
        && !v.ends_with(':')
        && v.trim() == v;
    if plain {
        v.to_owned()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The value of a front matter key, unquoted.
#[must_use]
pub fn get(text: &str, key: &str) -> Option<String> {
    let (fm, _) = split(text)?;
    fm.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| unquote(v))
    })
}

/// The text with the given front matter keys set: an existing line is replaced, a missing
/// key is added at the end of the front matter. A text without front matter is returned as
/// it is.
#[must_use]
pub fn set_keys(text: &str, keys: &[(&str, String)]) -> String {
    let Some((fm, rest)) = split(text) else {
        return text.to_owned();
    };
    let mut lines: Vec<String> = fm.lines().map(str::to_owned).collect();
    for (k, v) in keys {
        let line = format!("{k}: {}", quote(v));
        match lines
            .iter()
            .position(|l| l.split_once(':').is_some_and(|(lk, _)| lk.trim() == *k))
        {
            Some(i) => lines[i] = line,
            None => lines.push(line),
        }
    }
    format!("---\n{}\n---\n{rest}", lines.join("\n"))
}

/// The text as Decisions should get it: the front matter without the [`LOCAL_KEYS`].
#[must_use]
pub fn strip_local(text: &str) -> String {
    let Some((fm, rest)) = split(text) else {
        return text.to_owned();
    };
    let kept: Vec<&str> = fm
        .lines()
        .filter(|l| {
            l.split_once(':')
                .is_none_or(|(k, _)| !LOCAL_KEYS.contains(&k.trim()))
        })
        .collect();
    format!("---\n{}\n---\n{rest}", kept.join("\n"))
}

/// The fingerprint of what a push sends: the SHA-256 of [`strip_local`], in hex. Changing
/// only the bookkeeping keys never changes it.
#[must_use]
pub fn fingerprint(text: &str) -> String {
    let digest = Sha256::digest(strip_local(text).as_bytes());
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ---------------------------------------------------------------- names and numbers

/// A file-name slug: lowercase letters and digits, single dashes, at most 60 characters.
#[must_use]
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.chars().take(60).collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "untitled".to_owned()
    } else {
        out.to_owned()
    }
}

fn is_md(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// The number a file name starts with: `0003-name.md` is 3.
#[must_use]
pub fn number_of_file(name: &str) -> Option<u32> {
    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || !is_md(name) {
        return None;
    }
    digits.parse().ok()
}

/// The folder of a kind, in the atlas of `root`.
#[must_use]
pub fn kind_dir(root: &Path, kind: Kind) -> PathBuf {
    root.join(DIR).join(kind.folder())
}

fn md_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| is_md(n) && n != "README.md")
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The next free number in a kind's folder: one above the highest file number.
#[must_use]
pub fn next_number(root: &Path, kind: Kind) -> u32 {
    md_files(&kind_dir(root, kind))
        .iter()
        .filter_map(|n| number_of_file(n))
        .max()
        .map_or(1, |n| n + 1)
}

// ---------------------------------------------------------------- documents

fn body(kind: Kind, intro: &str, problem: Option<&str>, date: &str, made: &str) -> String {
    let mut out = format!("{intro}\n");
    for (i, (heading, hint)) in kind.sections().iter().enumerate() {
        let _ = write!(out, "\n## {heading}\n\n");
        if *heading == "Status log" {
            let _ = writeln!(out, "- {date}: {made}");
        } else if i == 0
            && let Some(p) = problem
        {
            out.push_str(p);
            out.push('\n');
        } else {
            out.push_str(hint);
            out.push('\n');
        }
    }
    out
}

/// A whole document: front matter, then the body.
fn document(kind: Kind, number: u32, title: &str, summary: &str, date: &str, text: &str) -> String {
    format!(
        "---\nkind: {}\nnumber: {number}\ntitle: {}\nstatus: {}\ndate: {date}\npublic: false\nsummary: {}\n---\n\n{text}",
        kind.api(),
        quote(title),
        kind.first_status(),
        quote(summary),
    )
}

/// The file name of a document: `0002-the-title.md`.
#[must_use]
pub fn file_name(number: u32, title: &str) -> String {
    format!("{number:04}-{}.md", slug(title))
}

fn first_line_of(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    line.chars().take(max).collect()
}

fn starter(kind: Kind, m: &Meta<'_>) -> (String, String) {
    let lang = m.lang.map_or_else(String::new, |l| format!(", {l}"));
    let made = format!(
        "project made from the `{}` template{lang} by `iohr project init`.",
        m.template
    );
    let stub = "Stub: a starter written by `iohr project init`. Replace it with your own text.";
    let (title, summary, problem) = match kind {
        Kind::Prd => (
            format!("What {} is for", m.name),
            format!("The problem {} solves, for whom, and how to tell it works. Stub.", m.name),
            m.describe.map(|d| {
                let d = d.trim();
                d.chars().take(2000).collect::<String>()
            }),
        ),
        Kind::Rfc => (
            format!("How {} is built", m.name),
            format!("The first technical proposal for {}: what to build and what else was considered. Stub.", m.name),
            None,
        ),
        Kind::Adr => (
            "Keep project documents in atlas".to_owned(),
            "Where the project's PRDs, RFCs, ADRs and studies live: in the repo, under atlas/. Stub.".to_owned(),
            Some(
                "Stub: a starter. The project's documents are plain files in `atlas/`, so they \
                 are versioned with the code. They go to Decisions only when someone runs \
                 `iohr sync`."
                    .to_owned(),
            ),
        ),
        Kind::Study => (
            format!("The first measurement for {}", m.name),
            format!("What to measure first in {}, before any number exists. Stub: nothing is measured yet.", m.name),
            None,
        ),
    };
    let text = body(kind, stub, problem.as_deref(), m.date(), &made);
    let doc = document(
        kind,
        1,
        &title,
        &first_line_of(&summary, 300),
        m.date(),
        &text,
    );
    (
        format!("{DIR}/{}/{}", kind.folder(), file_name(1, &title)),
        doc,
    )
}

/// A new, empty document of a kind: front matter and the kind's headings with a line each
/// saying what goes there.
#[must_use]
pub fn blank(kind: Kind, number: u32, title: &str, date: &str) -> (String, String) {
    let intro = "Stub: a new document from `iohr atlas new`. Replace the lines under each heading.";
    let made = "opened with `iohr atlas new`.";
    let text = body(kind, intro, None, date, made);
    let summary = format!("{title}. Stub: write one sentence here.");
    (
        format!("{DIR}/{}/{}", kind.folder(), file_name(number, title)),
        document(kind, number, title, &summary, date, &text),
    )
}

/// Writes a new document of a kind into the atlas of `root` and returns its path relative to
/// `root` and its number. The number is the next free one in the folder, never an existing
/// file's. Refuses a file that already exists.
///
/// # Errors
///
/// A message saying what could not be written.
pub fn new_document(
    root: &Path,
    kind: Kind,
    title: &str,
    date: &str,
) -> Result<(String, u32), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("the title is empty".to_owned());
    }
    let number = next_number(root, kind);
    let (rel, text) = blank(kind, number, title, date);
    let path = root.join(&rel);
    if path.exists() {
        return Err(format!("{rel} already exists"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("cannot write {rel}: {e}"))?;
    Ok((rel, number))
}

// ---------------------------------------------------------------- canvas, 3D, README

/// The starter canvas: `iohr.canvas` version 3, one frame and one note, so it opens in the
/// desktop app as it is. It has no `document` items: those need Decisions ids, which a
/// project that has not synced does not have.
#[must_use]
pub fn canvas_starter(m: &Meta<'_>) -> (String, String) {
    let id = format!("cnv_{}-start", slug(m.name));
    let value = json!({
        "schema": "iohr.canvas",
        "version": 3,
        "canvas_id": id,
        "title": format!("{}: start", m.name),
        "description": "Stub: a starter board from `iohr project init`. Replace the note, add cards.",
        "created_at": m.created_at,
        "updated_at": m.created_at,
        "revision": 0,
        "items": [
            {
                "item_id": "itm_start-frame",
                "type": "frame",
                "title": "Start here",
                "x": 0, "y": 0, "w": 560, "h": 320,
                "tone": "blue",
            },
            {
                "item_id": "itm_start-note",
                "type": "note",
                "title": "Stub",
                "text": "A starter note. Say what this board is for, then add cards for the documents in atlas/.",
                "x": 32, "y": 64, "w": 320, "h": 160,
                "frame_id": "itm_start-frame",
                "tone": "amber",
            },
        ],
        "links": [],
        "threads": [],
    });
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
    text.push('\n');
    (format!("{DIR}/canvas/start.canvas.json"), text)
}

fn models_readme() -> (String, String) {
    let text = "\
# 3D models

This folder is for 3D models of the project. It holds none yet, on purpose: a placeholder
file would look like a model.

The format is glTF 2.0. Use `.gltf` (JSON, with its textures beside it) or `.glb` (the same
in one file). Nothing else is settled yet, so this folder is not read by anything today and
`iohr sync` does not send it.
";
    (format!("{DIR}/models/README.md"), text.to_owned())
}

fn readme(m: &Meta<'_>) -> (String, String) {
    let text = format!(
        "\
# Atlas: {name}

The documents about {name}, kept in the repo beside the code. Everything here is a plain
file, so it is versioned, reviewed and merged like code.

| Folder | What is in it |
| --- | --- |
| `prd/` | Product requirements: the problem and how to tell it is solved. |
| `rfc/` | Proposals for how to build something, before it is built. |
| `adr/` | Decisions that were made, and why. |
| `studies/` | Measurements, with the question asked before the result. |
| `canvas/` | Boards, in the InOrbit canvas format (`iohr.canvas`, version 3). |
| `models/` | 3D models, in glTF 2.0. Empty for now. |
| `personalities/` | The built-in InOrbit personalities for this project: `security` and the engineer of its language. Each line names its source. |
| `memory/` | How this project's agents remember and recall through InOrbit memory. |

The files named `0001-...` are starters. Each says so in its first line. Replace their text.

Add a document with `iohr atlas new prd \"Title\"` (or `rfc`, `adr`, `study`). It takes the
next free number.

## Sync

Nothing here leaves your machine unless you run `iohr sync`. It sends the documents in
`prd/`, `rfc/`, `adr/` and `studies/` to the project's Decisions space (the one named in
`.inorbit/space`). Canvas and 3D files stay local. A document that was changed in Decisions
since the last sync is reported and never overwritten. `iohr sync --dry-run` shows what it
would do and sends nothing.

`iohr sync` also imports the files in `personalities/` into your InOrbit account (signed in
only; `--dry-run` shows what would be sent). The packs are copies of the catalogue: an edit
here is not read back, so change a personality in InOrbit.

The keys `kind`, `number`, `remote_id`, `remote_version`, `remote_number` and `remote_sha`
at the top of a document are bookkeeping for sync. They are not sent.
",
        name = m.name
    );
    (format!("{DIR}/README.md"), text)
}

// ---------------------------------------------------------------- the starter set

/// Every file `iohr project init` writes under `atlas/`, as relative path and text.
#[must_use]
pub fn files(m: &Meta<'_>) -> Vec<(String, String)> {
    let mut out = vec![readme(m)];
    out.extend(Kind::ALL.into_iter().map(|k| starter(k, m)));
    out.push(canvas_starter(m));
    out.push(models_readme());
    out.extend(crate::personalities::pack_files(m.lang));
    out.push(crate::personalities::memory_readme(&slug(m.name)));
    out
}

/// Adds the atlas files to a template's rendered files, skipping a path the template already
/// ships. The one place every template gets them, as [`crate::project::add_space_link`] is
/// for the space link.
pub fn add_atlas(files_out: &mut Vec<(String, String)>, m: &Meta<'_>) {
    for (path, text) in files(m) {
        if !files_out.iter().any(|(p, _)| *p == path) {
            files_out.push((path, text));
        }
    }
    // The personalities are recorded in the manifest, and the agents are told about memory.
    crate::personalities::add_record(files_out, m.lang);
    crate::personalities::add_agents(files_out, &slug(m.name));
}

/// Whether the atlas of `root` already has this starter: the file itself, or any file a person
/// made in its place (a renamed starter is theirs).
fn present(root: &Path, rel: &str) -> bool {
    let path = root.join(rel);
    if path.exists() {
        return true;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    let folder = parent
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if is_md(rel) && Kind::parse(folder).is_some() {
        return !md_files(parent).is_empty();
    }
    if folder == "canvas" {
        return std::fs::read_dir(parent).is_ok_and(|rd| {
            rd.filter_map(Result::ok)
                .any(|e| e.path().extension().is_some_and(|x| x == "json"))
        });
    }
    false
}

/// Writes the starters that are missing under `root` and returns their paths. A file that
/// exists is never touched, and a folder that holds a document (or a canvas) of the person's
/// gets no starter.
///
/// # Errors
///
/// A message saying what could not be written.
pub fn ensure(root: &Path, m: &Meta<'_>) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (rel, text) in files(m) {
        if present(root, &rel) {
            continue;
        }
        let path = root.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("cannot write {rel}: {e}"))?;
        written.push(rel);
    }
    Ok(written)
}

/// The atlas of the project that holds `from`: the nearest folder up the tree that has an
/// `atlas/` directory. Returns the project root.
#[must_use]
pub fn find_root(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|d| d.join(DIR).is_dir())
        .map(Path::to_path_buf)
}

/// One document file found by [`scan`].
#[derive(Debug, Clone)]
pub struct Found {
    pub kind: Kind,
    /// The path relative to the project root, with `/`.
    pub rel: String,
    pub path: PathBuf,
    pub text: String,
}

/// The documents under `atlas/` of `root`, kind by kind, in file-name order. Files that are
/// not readable as text are skipped.
#[must_use]
pub fn scan(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let dir = kind_dir(root, kind);
        for name in md_files(&dir) {
            let path = dir.join(&name);
            if let Ok(text) = std::fs::read_to_string(&path) {
                out.push(Found {
                    kind,
                    rel: format!("{DIR}/{}/{name}", kind.folder()),
                    path,
                    text,
                });
            }
        }
    }
    out
}

/// The canvas and 3D files under `atlas/`, which stay local: relative paths.
#[must_use]
pub fn local_only(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for sub in ["canvas", "models"] {
        let dir = root.join(DIR).join(sub);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.path().is_file())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| n != "README.md")
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        out.extend(names.into_iter().map(|n| format!("{DIR}/{sub}/{n}")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta<'static> {
        Meta {
            name: "my-shop",
            template: "demo",
            lang: Some("rust"),
            describe: Some("A shop for hand-made chairs.\nSecond line."),
            created_at: "2026-11-05T10:00:00Z",
        }
    }

    #[test]
    fn init_files_cover_every_folder() {
        let files = files(&meta());
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            [
                "atlas/README.md",
                "atlas/prd/0001-what-my-shop-is-for.md",
                "atlas/rfc/0001-how-my-shop-is-built.md",
                "atlas/adr/0001-keep-project-documents-in-atlas.md",
                "atlas/studies/0001-the-first-measurement-for-my-shop.md",
                "atlas/canvas/start.canvas.json",
                "atlas/models/README.md",
                "atlas/personalities/security.md",
                "atlas/personalities/rust-engineer.md",
                "atlas/memory/README.md",
            ]
        );
    }

    #[test]
    fn starters_say_they_are_stubs_and_use_the_description() {
        let files = files(&meta());
        for (p, t) in &files {
            if is_md(p) && !p.ends_with("README.md") && !p.starts_with("atlas/personalities/") {
                assert!(t.contains("Stub"), "{p} is not labelled a stub");
                assert_eq!(get(t, "number").as_deref(), Some("1"));
            }
        }
        let prd = &files[1].1;
        assert!(prd.contains("A shop for hand-made chairs."));
        assert_eq!(get(prd, "status").as_deref(), Some("draft"));
        assert_eq!(get(&files[3].1, "status").as_deref(), Some("proposed"));
        assert_eq!(get(&files[4].1, "status").as_deref(), Some("running"));
        assert!(prd.contains("`demo` template, rust"));
    }

    #[test]
    fn the_canvas_starter_is_canvas_v3() {
        let (_, text) = canvas_starter(&meta());
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema"], "iohr.canvas");
        assert_eq!(v["version"], 3);
        let id = v["canvas_id"].as_str().unwrap();
        let re = regex::Regex::new(r"^[a-z]{3}_[0-9a-z_-]{1,80}$").unwrap();
        assert!(re.is_match(id), "{id}");
        for item in v["items"].as_array().unwrap() {
            assert!(re.is_match(item["item_id"].as_str().unwrap()));
        }
    }

    #[test]
    fn front_matter_reads_writes_and_strips() {
        let t = "---\nkind: rfc\nnumber: 3\ntitle: \"A: b\"\nstatus: draft\n---\n\nBody\n";
        assert_eq!(get(t, "title").as_deref(), Some("A: b"));
        assert_eq!(get(t, "missing"), None);
        let set = set_keys(
            t,
            &[
                ("remote_id", "ldoc_1".to_owned()),
                ("remote_version", "2".to_owned()),
                ("status", "accepted".to_owned()),
            ],
        );
        assert_eq!(get(&set, "remote_id").as_deref(), Some("ldoc_1"));
        assert_eq!(get(&set, "status").as_deref(), Some("accepted"));
        assert!(set.ends_with("---\n\nBody\n"));
        let stripped = strip_local(&set);
        assert!(!stripped.contains("remote_id") && !stripped.contains("kind:"));
        assert!(stripped.contains("status: accepted") && stripped.contains("Body"));
    }

    #[test]
    fn the_fingerprint_ignores_bookkeeping_only() {
        let t = starter(Kind::Rfc, &meta()).1;
        let synced = set_keys(
            &t,
            &[
                ("remote_id", "ldoc_9".to_owned()),
                ("remote_sha", "x".to_owned()),
            ],
        );
        assert_eq!(fingerprint(&t), fingerprint(&synced));
        assert_ne!(fingerprint(&t), fingerprint(&format!("{t}more")));
    }

    #[test]
    fn slugs_and_numbers() {
        assert_eq!(
            slug("  Use Postgres, not SQLite! "),
            "use-postgres-not-sqlite"
        );
        assert_eq!(slug("!!!"), "untitled");
        assert_eq!(file_name(2, "Cache it"), "0002-cache-it.md");
        assert_eq!(number_of_file("0007-x.md"), Some(7));
        assert_eq!(number_of_file("x.md"), None);
        assert_eq!(number_of_file("0007-x.txt"), None);
    }

    #[test]
    fn new_documents_take_the_next_number_and_ensure_keeps_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let written = ensure(root, &meta()).unwrap();
        assert_eq!(written.len(), 10);
        // A person edits the RFC starter and renames it.
        let rfc = root.join("atlas/rfc/0001-how-my-shop-is-built.md");
        let renamed = root.join("atlas/rfc/0001-my-own-name.md");
        std::fs::write(&rfc, "mine").unwrap();
        std::fs::rename(&rfc, &renamed).unwrap();
        let prd = root.join("atlas/prd/0001-what-my-shop-is-for.md");
        std::fs::write(&prd, "edited").unwrap();
        // Running it again writes nothing and changes nothing.
        assert_eq!(ensure(root, &meta()).unwrap(), Vec::<String>::new());
        assert_eq!(std::fs::read_to_string(&prd).unwrap(), "edited");
        assert!(!rfc.exists());
        let (rel, n) = new_document(root, Kind::Rfc, "Cache the lists", "2026-11-05").unwrap();
        assert_eq!((rel.as_str(), n), ("atlas/rfc/0002-cache-the-lists.md", 2));
        let text = std::fs::read_to_string(root.join(&rel)).unwrap();
        assert_eq!(get(&text, "kind").as_deref(), Some("rfc"));
        assert_eq!(get(&text, "title").as_deref(), Some("Cache the lists"));
        assert!(new_document(root, Kind::Rfc, "  ", "2026-11-05").is_err());
    }

    #[test]
    fn scan_finds_documents_and_local_only_files() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path(), &meta()).unwrap();
        let found = scan(dir.path());
        assert_eq!(found.len(), 4);
        assert_eq!(found[3].kind, Kind::Study);
        assert_eq!(local_only(dir.path()), ["atlas/canvas/start.canvas.json"]);
        assert_eq!(
            find_root(&dir.path().join("atlas/rfc")).as_deref(),
            Some(dir.path())
        );
    }
}
