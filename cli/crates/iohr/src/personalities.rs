//! The built-in personalities `iohr project init` and `iohr atlas init` give a project
//! (RFC 0124.1, RFC 0205): `security` for every project, and one engineer for the language
//! chosen. The catalogue is data in `personalities/*.toml`, versioned with this binary; every
//! line carries the primary source it comes from.
//!
//! A project gets readable packs under `atlas/personalities/` and, when the person is signed
//! in, the same personalities in InOrbit through `POST /v1/coordinator/personalities/import`.
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
pub const DIR: &str = "atlas/personalities";
/// The one personality every project gets, whatever its language.
pub const BASE: &str = "security";
/// The root of the tree. Imported only when the account has no root of its own.
pub const ROOT: &str = "base-engineer";
/// The REST route that takes personalities.
pub const IMPORT_PATH: &str = "/v1/coordinator/personalities/import";
const LIST_PATH: &str = "/v1/coordinator/personalities";

const FILES: [&str; 6] = [
    include_str!("../personalities/base-engineer.toml"),
    include_str!("../personalities/security.toml"),
    include_str!("../personalities/rust-engineer.toml"),
    include_str!("../personalities/python-engineer.toml"),
    include_str!("../personalities/typescript-engineer.toml"),
    include_str!("../personalities/go-engineer.toml"),
];

const NOT_SIGNED_IN: &str = "Not signed in: the personalities stay in atlas/personalities/. \
     Run `iohr login`, then `iohr sync` (or `iohr atlas personalities import`) imports them.";

/// One line of a personality: its text and the sources it comes from.
#[derive(Debug, Clone, Deserialize)]
pub struct Line {
    /// `core` (always in the brief) or `recall` (found by topic).
    pub place: String,
    /// The heading it sits under.
    pub section: String,
    pub text: String,
    /// Primary sources, as URLs. Never empty.
    pub cites: Vec<String>,
}

/// A personality of the catalogue.
#[derive(Debug, Clone, Deserialize)]
pub struct Personality {
    pub name: String,
    /// Empty for the root.
    pub parent: String,
    pub title: String,
    /// Raised whenever the lines change.
    pub version: u32,
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
}

/// The whole catalogue, root first.
#[must_use]
pub fn catalogue() -> Vec<Personality> {
    FILES
        .iter()
        .filter_map(|t| toml::from_str::<Personality>(t).ok())
        .collect()
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

/// What a project gets: `security`, and the engineer of its language when there is one.
#[must_use]
pub fn selection(lang: Option<&str>) -> Vec<Personality> {
    let want = lang.and_then(canonical_language);
    catalogue()
        .into_iter()
        .filter(|p| p.name == BASE || (want.is_some() && p.language.as_deref() == want))
        .collect()
}

/// The language of the project at `root`: `language` in `inorbit.toml` (at the top or under
/// `[project]`), else the build file it has.
#[must_use]
pub fn detect_language(root: &Path) -> Option<&'static str> {
    let declared = std::fs::read_to_string(root.join("inorbit.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| {
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
        });
    declared.or_else(|| {
        let has = |f: &str| root.join(f).is_file();
        if has("Cargo.toml") {
            Some("rust")
        } else if has("go.mod") {
            Some("go")
        } else if has("tsconfig.json") || has("package.json") {
            Some("typescript")
        } else if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
            Some("python")
        } else {
            None
        }
    })
}

fn pack_rel(name: &str) -> String {
    format!("{DIR}/{name}.md")
}

fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// The size of a personality's core, in the service's measure (characters over four).
#[must_use]
pub fn core_tokens(p: &Personality) -> usize {
    p.lines
        .iter()
        .filter(|l| l.place == "core")
        .map(|l| tokens(&l.text))
        .sum()
}

/// The readable pack of a personality: front matter (name, parent, domain, source version),
/// then its lines under their headings, each with its sources.
#[must_use]
pub fn pack(p: &Personality) -> String {
    let mut out = format!(
        "---\nname: {}\nparent: {}\ndomain: {}\nsource: {}\ncatalogue_version: {}\n---\n",
        p.name,
        p.parent,
        p.domain.join(", "),
        format_args!("iohr {} built-in catalogue", env!("CARGO_PKG_VERSION")),
        p.version
    );
    let _ = write!(
        out,
        "# {}\n\nA built-in personality of the `iohr` catalogue. Its lines come from primary \
         sources, named under each one. `iohr sync` imports it into InOrbit from the catalogue, \
         not from this file: an edit here is not read back.\n",
        p.title
    );
    for place in ["core", "recall"] {
        let heading = if place == "core" {
            "Core: always in the brief"
        } else {
            "Recall: found by topic"
        };
        let _ = write!(out, "\n## {heading}\n");
        let mut last = "";
        for l in p.lines.iter().filter(|l| l.place == place) {
            if l.section != last {
                let _ = write!(out, "\n### {}\n\n", l.section);
                last = &l.section;
            }
            let _ = write!(out, "- {}\n  Source: {}\n", l.text, l.cites.join(", "));
        }
    }
    out
}

/// The pack files a project of `lang` starts with, as relative path and text.
#[must_use]
pub fn pack_files(lang: Option<&str>) -> Vec<(String, String)> {
    selection(lang)
        .iter()
        .map(|p| (pack_rel(&p.name), pack(p)))
        .collect()
}

/// The `[personalities]` table of `inorbit.toml`: which built-ins the project uses.
#[must_use]
pub fn record(lang: Option<&str>) -> String {
    let names: Vec<String> = selection(lang)
        .iter()
        .map(|p| format!("\"{}\"", p.name))
        .collect();
    format!(
        "\n# --- Personalities ---\n\n\
         # The built-in InOrbit personalities this project uses. Their packs are in\n\
         # atlas/personalities/; `iohr sync` imports them into your InOrbit account.\n\
         [personalities]\nnames = [{}]\n",
        names.join(", ")
    )
}

/// Adds the record to `inorbit.toml` among a template's files, unless it has one already.
pub fn add_record(files: &mut [(String, String)], lang: Option<&str>) {
    if let Some((_, text)) = files.iter_mut().find(|(p, _)| p == "inorbit.toml")
        && !text.contains("[personalities]")
    {
        text.push_str(&record(lang));
    }
}

/// Adds the record to the `inorbit.toml` of an existing project, when it has one and no record.
///
/// # Errors
///
/// A message saying what could not be written.
pub fn ensure_record(root: &Path, lang: Option<&str>) -> Result<bool, String> {
    let path = root.join("inorbit.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if text.contains("[personalities]") {
        return Ok(false);
    }
    let mut next = text;
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&record(lang));
    std::fs::write(&path, next).map_err(|e| format!("cannot write inorbit.toml: {e}"))?;
    Ok(true)
}

// ---------------------------------------------------------------- the import body

fn sections(p: &Personality) -> Vec<Value> {
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

fn input(p: &Personality, parent: &str) -> Value {
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
pub fn import_body(send: &[&Personality], parent: &str, with_root: bool) -> Value {
    let mut list = Vec::new();
    if with_root && let Some(root) = catalogue().iter().find(|p| p.name == ROOT) {
        list.push(input(root, ""));
    }
    list.extend(send.iter().map(|p| input(p, parent)));
    json!({ "personalities": list })
}

// ---------------------------------------------------------------- what the project has

/// A pack in the project and what an import recorded in it.
#[derive(Debug, Clone)]
pub struct Local {
    pub personality: Personality,
    pub path: PathBuf,
    /// The version InOrbit gave it when we imported it.
    pub imported_version: Option<i64>,
    /// The catalogue version that import sent.
    pub imported_catalogue: Option<u32>,
}

/// The packs of `root` that name a personality of the catalogue.
#[must_use]
pub fn locals(root: &Path) -> Vec<Local> {
    let cat = catalogue();
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
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let name = atlas::get(&text, "name")?;
            if name == ROOT {
                return None;
            }
            let personality = cat.iter().find(|p| p.name == name)?.clone();
            Some(Local {
                personality,
                imported_version: atlas::get(&text, "imported_version")
                    .and_then(|v| v.trim().parse().ok()),
                imported_catalogue: atlas::get(&text, "imported_catalogue")
                    .and_then(|v| v.trim().parse().ok()),
                path,
            })
        })
        .collect()
}

// ---------------------------------------------------------------- the import

/// One personality's result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Outcome {
    pub name: String,
    /// `written`, `would write`, `unchanged`, `left alone`, `local` or `failed`.
    pub action: &'static str,
    pub version: Option<i64>,
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
    /// The lines to show: one per personality, then the message once.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .outcomes
            .iter()
            .map(|o| {
                let v = o.version.map_or_else(String::new, |v| format!(" v{v}"));
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
        Some(403) => "refused: only the account owner or an admin can import personalities, \
                      and an API key has no coordinator scope"
            .to_owned(),
        Some(s) => format!("InOrbit answered {s}: {e}"),
        None => format!("InOrbit could not be reached: {e}"),
    }
}

/// What InOrbit lists: name to (parent, head version).
fn remote_of(listed: Option<&Value>) -> BTreeMap<String, (String, i64)> {
    listed
        .and_then(|v| v.pointer("/personalities"))
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

/// Tells each local personality apart by what InOrbit has; the ones to send are returned,
/// the others are said in the report.
fn decide<'a>(
    found: &'a [Local],
    remote: &BTreeMap<String, (String, i64)>,
    report: &mut Report,
) -> Vec<&'a Local> {
    let mut send: Vec<&Local> = Vec::new();
    for f in found {
        let name = &f.personality.name;
        let (action, version, note) = match remote.get(name) {
            None => {
                send.push(f);
                continue;
            }
            Some((_, head)) => match f.imported_version {
                None => (
                    "left alone",
                    Some(*head),
                    "already in your account, not from this project",
                ),
                Some(v) if v != *head => (
                    "left alone",
                    Some(*head),
                    "changed in InOrbit since the import",
                ),
                Some(_)
                    if f.imported_catalogue
                        .is_some_and(|c| c < f.personality.version) =>
                {
                    send.push(f);
                    continue;
                }
                Some(_) => ("unchanged", Some(*head), ""),
            },
        };
        report.outcomes.push(Outcome {
            name: name.clone(),
            action,
            version,
            note: note.to_owned(),
        });
    }
    send
}

/// Sends the import; what InOrbit wrote as name to version, or the line saying why not.
async fn post(api: &Api, body: &Value) -> Result<BTreeMap<String, i64>, String> {
    let v = api
        .send(Method::Post, IMPORT_PATH, &[], Some(body))
        .await
        .and_then(|r| r.json::<Value>())
        .map_err(|e| why(&e))?;
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
            ("imported_catalogue", f.personality.version.to_string()),
        ],
    );
    match std::fs::write(&f.path, next) {
        Ok(()) => String::new(),
        Err(_) => "written, but the pack file could not record it".to_owned(),
    }
}

/// Every personality stays in the project; `why` is said once.
fn local_only(found: &[Local], report: &mut Report, why: &str) {
    for f in found {
        report.outcomes.push(Outcome {
            name: f.personality.name.clone(),
            action: "local",
            version: None,
            note: String::new(),
        });
    }
    report.message = Some(why.to_owned());
}

/// Imports the packs of `root` into the account of `api`, one personality at a time told
/// apart by what is already there:
/// - absent in InOrbit: imported;
/// - present, never imported from this project: `left alone` (it is the account's own);
/// - present and changed in InOrbit since our import: `left alone`;
/// - present at the version we imported and the catalogue moved on: imported again;
/// - otherwise `unchanged`.
///
/// With `dry`, nothing is sent and nothing written, and the report holds the body. With no
/// `api`, every personality stays local, with one message saying so. Never fails.
pub(crate) async fn import(api: Option<&Api>, root: &Path, dry: bool) -> Report {
    let found = locals(root);
    let mut report = Report::default();
    if found.is_empty() {
        return report;
    }
    let Some(api) = api else {
        let refs: Vec<&Personality> = found.iter().map(|f| &f.personality).collect();
        if dry {
            report.body = Some(import_body(&refs, ROOT, true));
        }
        local_only(&found, &mut report, NOT_SIGNED_IN);
        return report;
    };
    let listed = match api.send(Method::Get, LIST_PATH, &[], None).await {
        Ok(raw) => raw.json::<Value>().ok(),
        Err(e) => {
            local_only(
                &found,
                &mut report,
                &format!("The personalities stay local: {}.", why(&e)),
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
    let send = decide(&found, &remote, &mut report);
    if send.is_empty() {
        return report;
    }
    let parent = root_name.clone().unwrap_or_else(|| ROOT.to_owned());
    let refs: Vec<&Personality> = send.iter().map(|f| &f.personality).collect();
    let body = import_body(&refs, &parent, root_name.is_none());
    if dry {
        for f in &send {
            report.outcomes.push(Outcome {
                name: f.personality.name.clone(),
                action: "would write",
                version: None,
                note: String::new(),
            });
        }
        report.body = Some(body);
        return report;
    }
    let written = match post(api, &body).await {
        Ok(w) => w,
        Err(line) => {
            for f in &send {
                report.outcomes.push(Outcome {
                    name: f.personality.name.clone(),
                    action: "failed",
                    version: None,
                    note: String::new(),
                });
            }
            report.message = Some(format!(
                "The personalities stay local ({line}). `iohr sync` tries again."
            ));
            return report;
        }
    };
    for f in send {
        let name = f.personality.name.clone();
        let Some(&v) = written.get(&name) else {
            report.outcomes.push(Outcome {
                name,
                action: "failed",
                version: None,
                note: "InOrbit did not say it was written".to_owned(),
            });
            continue;
        };
        let note = write_back(f, v);
        report.outcomes.push(Outcome {
            name,
            action: "written",
            version: Some(v),
            note,
        });
    }
    if root_name.is_none() && written.contains_key(ROOT) {
        report.outcomes.push(Outcome {
            name: ROOT.to_owned(),
            action: "written",
            version: written.get(ROOT).copied(),
            note: "the account had no root personality".to_owned(),
        });
    }
    report
}

// ---------------------------------------------------------------- agent files

/// The marker that says the section is in an agent-instructions file.
pub const AGENTS_MARK: &str = "<!-- iohr:memory -->";

/// The short section of CLAUDE.md or AGENTS.md: how the project's agents use its memory.
/// Memory is local first, so nothing here needs a sign-in.
#[must_use]
pub fn agents_section() -> String {
    format!(
        "\n{AGENTS_MARK}\n## Project memory and personalities\n\n\
         - Before work, run `iohr memory recall \"<topic>\"`. After a lesson, run \
         `iohr memory remember \"<fact>\" --why \"<reason>\"`. Memories are files in \
         `.inorbit/memory/`, committed with the code; no sign-in is needed. \
         `iohr memory check` finds the wrong and the stale ones. See \
         `.inorbit/memory/README.md`.\n\
         - The built-in personalities for this project are in `atlas/personalities/`.\n"
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
pub fn add_agents(files: &mut Vec<(String, String)>) {
    let found = instruction_files(files);
    if found.is_empty() {
        files.push((
            "AGENTS.md".to_owned(),
            format!("# Agent instructions\n{}", agents_section()),
        ));
        return;
    }
    for i in found {
        if !files[i].1.contains(AGENTS_MARK) {
            files[i].1.push_str(&agents_section());
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
pub fn ensure_agents(root: &Path) -> Result<Vec<String>, String> {
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
        next.push_str(&agents_section());
        std::fs::write(&path, next).map_err(|e| format!("cannot write {name}: {e}"))?;
        wrote.push(name.to_owned());
    }
    if !any {
        let path = root.join("AGENTS.md");
        std::fs::write(&path, format!("# Agent instructions\n{}", agents_section()))
            .map_err(|e| format!("cannot write AGENTS.md: {e}"))?;
        wrote.push("AGENTS.md".to_owned());
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_text(p: &Personality) -> String {
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

    /// The service's lint, in what it can tell from the text: the budget, the override words
    /// a child's core must not hold, the privacy words and the secret shapes.
    #[test]
    fn the_catalogue_passes_the_services_lint_in_what_a_client_can_check() {
        let override_words =
            regex::Regex::new(r"(?i)\b(ignore|disregard|override|bypass|unless|except when)\b")
                .unwrap();
        let widen = regex::Regex::new(
            r"(?i)\b(allow(ed|s)? to|grant(ed|s)?|permission|may now|can now|optional|unnecessary|not needed|skip (the )?(check|gate|review|tests?|ci)|auto-?approve|force-?push)\b",
        )
        .unwrap();
        let private = regex::Regex::new(
            r"(?i)\b(tenders?|investors?|investments?|banks?|loan|invoices?|revenue|profit|salary|salaries|taxes|finances?|equity|contracts?|classified|confidential|procurement|valuation|funding)\b|[€$£]\s*\d|\d{11}|(password|passwd|secret|api[_-]?key|token)\s*[:=]\s*\S{6,}",
        )
        .unwrap();
        for p in catalogue() {
            assert!(
                core_tokens(&p) <= 700,
                "{} core is {} tokens",
                p.name,
                core_tokens(&p)
            );
            if p.parent.is_empty() {
                continue;
            }
            assert!(!override_words.is_match(&core_text(&p)), "{} core", p.name);
            for l in &p.lines {
                assert!(!widen.is_match(&l.text), "{}: {}", p.name, l.text);
                assert!(!private.is_match(&l.text), "{}: {}", p.name, l.text);
                for c in &l.cites {
                    assert!(!private.is_match(c), "{}: {c}", p.name);
                }
            }
            assert!(!private.is_match(&p.title), "{}", p.name);
        }
        assert_eq!(selection(None).len(), 1);
    }

    #[test]
    fn a_project_gets_security_and_its_language() {
        let names = |l: Option<&str>| selection(l).into_iter().map(|p| p.name).collect::<Vec<_>>();
        assert_eq!(names(None), ["security"]);
        assert_eq!(names(Some("rust")), ["security", "rust-engineer"]);
        assert_eq!(names(Some("golang")), ["security", "go-engineer"]);
        assert_eq!(names(Some("TS")), ["security", "typescript-engineer"]);
        assert_eq!(names(Some("cobol")), ["security"]);
    }

    #[test]
    fn the_pack_has_front_matter_and_every_source() {
        let p = selection(Some("rust")).remove(1);
        let text = pack(&p);
        assert_eq!(atlas::get(&text, "name").as_deref(), Some("rust-engineer"));
        assert_eq!(
            atlas::get(&text, "parent").as_deref(),
            Some("base-engineer")
        );
        assert!(atlas::get(&text, "domain").is_some_and(|d| d.contains("tokio")));
        assert_eq!(atlas::get(&text, "catalogue_version").as_deref(), Some("1"));
        assert!(atlas::get(&text, "source").is_some_and(|s| s.starts_with("iohr ")));
        for l in &p.lines {
            for c in &l.cites {
                assert!(text.contains(c.as_str()), "{c}");
            }
        }
    }

    #[test]
    fn the_import_body_is_what_the_service_takes() {
        let sel = selection(Some("python"));
        let refs: Vec<&Personality> = sel.iter().collect();
        let body = import_body(&refs, "base-engineer", true);
        let list = body["personalities"].as_array().unwrap();
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
        assert_eq!(without["personalities"].as_array().unwrap().len(), 2);
        assert_eq!(without["personalities"][0]["parent"], "company-root");
    }

    #[test]
    fn language_is_read_from_the_record_then_the_build_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_language(dir.path()), None);
        std::fs::write(dir.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(detect_language(dir.path()), Some("go"));
        std::fs::write(dir.path().join("inorbit.toml"), "language = \"python\"\n").unwrap();
        assert_eq!(detect_language(dir.path()), Some("python"));
    }

    #[test]
    fn the_record_and_agent_sections_are_added_once() {
        let mut files = vec![
            ("inorbit.toml".to_owned(), "name = \"x\"\n".to_owned()),
            ("CLAUDE.md".to_owned(), "# Mine\n".to_owned()),
        ];
        add_record(&mut files, Some("rust"));
        add_record(&mut files, Some("rust"));
        add_agents(&mut files);
        add_agents(&mut files);
        assert_eq!(files[0].1.matches("[personalities]").count(), 1);
        assert!(files[0].1.contains("\"rust-engineer\""));
        assert_eq!(files[1].1.matches(AGENTS_MARK).count(), 1);
        assert!(files[1].1.contains("iohr memory recall"));
        assert_eq!(files.len(), 2, "a shipped file is extended, none is added");
        let mut none: Vec<(String, String)> = Vec::new();
        add_agents(&mut none);
        assert_eq!(none[0].0, "AGENTS.md");
        assert!(none[0].1.contains(".inorbit/memory/README.md"));
    }
}
