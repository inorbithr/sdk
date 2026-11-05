//! Project templates as data (PRD 0088.1, ADR 0213): a template is a folder with a
//! `template.toml`; every other file in the folder is part of the project. Nothing about a
//! template is compiled in except the built-ins, which are the same folders embedded.
//!
//! The shape is borrowed, not invented. From copier's `copier.yml`
//! (<https://copier.readthedocs.io/en/stable/configuring/>): questions with a type, choices,
//! a default, a `when` condition on earlier answers and a validator, and `_extends`-style
//! layering. From cargo-generate's `cargo-generate.toml`
//! (<https://cargo-generate.github.io/cargo-generate/templates/builtin_placeholders.html>):
//! `[placeholders]` with a type, a prompt, choices and a regex, answered by `--define`.
//! Placeholders keep the existing `@@KEY@@` form, filled by [`crate::ext::init::fill`], the
//! one renderer of `ext init` and `project init`.
//!
//! Sources, in precedence order: a folder or git URL named by `--template`, then
//! `~/.inorbit/templates/<id>/`, then the built-ins.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::ext::init::fill;
use crate::ext::lint::lint;
use crate::ext::manifest::SOURCE_FILE;

/// The file that makes a folder a template.
pub const FILE: &str = "template.toml";
/// Placeholders every template has; a question may not take one of these keys.
pub const BUILTIN_KEYS: &[&str] = &["name", "id", "module", "created_at", "publisher"];
const MAX_FILES: usize = 500;
const MAX_BYTES: u64 = 1 << 20;

/// Live templates can be used; coming ones are listed and refuse.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Live,
    Coming,
}

/// The kind of an answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Free text (checked by `regex` when there is one).
    #[default]
    String,
    /// Yes or no.
    Bool,
    /// One of `choices`.
    Choice,
    /// Any of `choices`; a list.
    Multi,
}

/// An answer, or a default: text, a flag, or a list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Answer {
    Flag(bool),
    Text(String),
    List(Vec<String>),
}

impl Answer {
    /// What a placeholder is filled with.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Flag(b) => b.to_string(),
            Self::Text(t) => t.clone(),
            Self::List(l) => l.join(","),
        }
    }
}

/// The answers by key.
pub type Answers = BTreeMap<String, Answer>;

/// One question of a template.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Question {
    /// The key: lowercase letters, digits and underscores. `@@KEY@@` (upper case) is filled with
    /// the answer, and `--set key=value` gives it.
    pub key: String,
    /// What is asked.
    pub prompt: String,
    /// The kind of answer (default `string`).
    #[serde(rename = "type", default)]
    pub kind: Kind,
    /// The choices of a `choice` or `multi` question.
    #[serde(default)]
    pub choices: Vec<String>,
    /// The answer when none is given (`--yes`, no terminal, or Enter).
    #[serde(default)]
    pub default: Option<Answer>,
    /// Ask only when this holds over the earlier answers: `key`, `!key`, `key == value` or
    /// `key != value` (for a `multi` question, `==` means "contains").
    #[serde(default)]
    pub when: Option<String>,
    /// A regular expression a `string` answer must match in full.
    #[serde(default)]
    pub regex: Option<String>,
    /// One line of help shown with the question.
    #[serde(default)]
    pub help: Option<String>,
}

/// The built-in steps a template may list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StepName {
    /// A git repository with one commit.
    Git,
    /// The project's record and state under `~/.inorbit/projects/`.
    Register,
    /// Runs the project once and reads its heartbeat (the demo's service, with python3).
    Smoke,
    /// Starts your own agent (claude or codex) in the new folder with the template's
    /// `prompt`; the output is labelled AI.
    Agent,
}

/// One step: a built-in by name; `agent` carries the prompt.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepDef {
    pub name: StepName,
    /// For `agent`: the prompt template (placeholders filled).
    #[serde(default)]
    pub prompt: Option<String>,
}

/// `template.toml`: the contract editors and the Mac app read (`iohr project template schema`).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateToml {
    /// The template's id: lowercase letters, digits and dashes, 2 to 32. A folder in
    /// `~/.inorbit/templates/` is named for it.
    pub id: String,
    /// One line: what the project is.
    pub description: String,
    /// `live` (default) or `coming`.
    #[serde(default)]
    pub status: Status,
    /// The id of a template this one adds to: its files, questions and steps come first, and
    /// a file of the same path here replaces its file.
    #[serde(default)]
    pub extends: Option<String>,
    /// What the agent is told when the person describes the project (`--describe`).
    #[serde(default)]
    pub agent_prompt: Option<String>,
    /// Questions, asked in order.
    #[serde(default)]
    pub questions: Vec<Question>,
    /// Steps run after the files are written, in order. None listed: `git` and `register`.
    #[serde(default)]
    pub steps: Vec<StepDef>,
    /// Languages: the files of `variants/<id>/` over the common ones. With any, a `language`
    /// question is asked (`--lang <id>` is the same as `--set language=<id>`).
    #[serde(default)]
    pub variants: Vec<VariantDef>,
    /// The variant `--yes` and a terminal-less run take (default: the first live one).
    #[serde(default)]
    pub default_variant: Option<String>,
    /// Files written as programs (executable); paths, placeholders allowed.
    #[serde(default)]
    pub programs: Vec<String>,
}

/// What the smoke test of a language does: `heartbeat` (default) runs the project once and reads
/// the heartbeat it wrote; `test` runs the language's own `test` command, for a project that is
/// not a service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmokeBy {
    #[default]
    Heartbeat,
    Test,
}

/// One language of a template: the files of `variants/<id>/` are laid over the common files,
/// and the commands say how the smoke test and the self-test run it. In `build`, `run` and
/// `test`, `$DIR` is the project folder, `$STATE` and `$CACHE` its state folders; a build writes
/// under `$CACHE`, never into the project.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantDef {
    /// The id `--lang` takes: lowercase letters and digits, 2 to 16 (`python`, `go`).
    pub id: String,
    /// What the menu shows: the language and what it needs, in a few words.
    pub label: String,
    /// `live` (default) or `coming`: a coming one is listed and refuses.
    #[serde(default)]
    pub status: Status,
    /// Programs that must be on `PATH` to build and run it (`python3`, `node`, `go`, `cargo`);
    /// the menu marks the ones that are there.
    #[serde(default)]
    pub needs: Vec<String>,
    /// Run once before the smoke test, in your environment (a compiler needs its toolchain).
    #[serde(default)]
    pub build: Vec<String>,
    /// How the smoke test runs the project once, in a scrubbed environment.
    #[serde(default)]
    pub run: Vec<String>,
    /// How the self-test runs, as the README says.
    #[serde(default)]
    pub test: Vec<String>,
    /// What the smoke test does: `heartbeat` (default) or `test` (run `test`, in your
    /// environment, writing only under `$CACHE` when `env` says so).
    #[serde(default)]
    pub smoke: SmokeBy,
    /// Extra environment for `run`, `$DIR`, `$STATE` and `$CACHE` allowed.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Text per language: each `KEY` fills `@@KEY@@` (upper case letters, digits, underscores).
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

/// What the smoke test runs for one language, placeholders and `$DIR` filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runner {
    pub label: String,
    pub build: Vec<String>,
    pub run: Vec<String>,
    /// The language's own test command, run by the smoke test when `by_test`.
    pub test: Vec<String>,
    pub by_test: bool,
    pub env: BTreeMap<String, String>,
}

/// The key of the language question every template with variants has.
pub const LANGUAGE: &str = "language";

/// Where a template came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    User(PathBuf),
    Path(PathBuf),
    Git(String),
}

impl Origin {
    /// The origin column.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::BuiltIn => "built-in".into(),
            Self::User(p) => format!("user: {}", p.display()),
            Self::Path(p) => format!("path: {}", p.display()),
            Self::Git(u) => format!("git: {u}"),
        }
    }
}

/// A template with its `extends` applied: what init renders.
#[derive(Debug, Clone)]
pub struct Template {
    pub meta: TemplateToml,
    /// Relative path (placeholders allowed) and text.
    pub files: Vec<(String, String)>,
    /// The files of `variants/<id>/`: variant id, relative path and text.
    pub variant_files: Vec<(String, String, String)>,
    pub origin: Origin,
}

impl Template {
    /// The variant with this id.
    #[must_use]
    pub fn variant(&self, id: &str) -> Option<&VariantDef> {
        self.meta.variants.iter().find(|v| v.id == id)
    }

    /// The variants that can be used.
    #[must_use]
    pub fn live_variants(&self) -> Vec<&VariantDef> {
        self.meta
            .variants
            .iter()
            .filter(|v| v.status == Status::Live)
            .collect()
    }

    /// The variant taken without asking: `default_variant` when it is live, else the first live.
    #[must_use]
    pub fn default_variant(&self) -> Option<&VariantDef> {
        self.meta
            .default_variant
            .as_deref()
            .and_then(|d| self.variant(d))
            .filter(|v| v.status == Status::Live)
            .or_else(|| self.live_variants().into_iter().next())
    }

    /// Every question in order: the `language` question first when there are variants.
    #[must_use]
    pub fn all_questions(&self) -> Vec<Question> {
        let live = self.live_variants();
        if live.is_empty() {
            return self.meta.questions.clone();
        }
        let mut q = vec![Question {
            key: LANGUAGE.into(),
            prompt: "Which language?".into(),
            kind: Kind::Choice,
            choices: live.iter().map(|v| v.id.clone()).collect(),
            default: self.default_variant().map(|v| Answer::Text(v.id.clone())),
            when: None,
            regex: None,
            help: Some("The files of that language are laid over the common ones".into()),
        }];
        q.extend(self.meta.questions.clone());
        q
    }

    /// The template with one variant laid over the common files (a file of the same path
    /// replaces the common one); the variant files are gone from the result.
    ///
    /// # Errors
    ///
    /// An unknown variant, or one that is still coming.
    pub fn with_variant(&self, id: &str) -> Result<Self, String> {
        let v = self.variant(id).ok_or_else(|| {
            format!(
                "no language `{id}` in the `{}` template; it has {}",
                self.meta.id,
                self.language_list()
            )
        })?;
        if v.status != Status::Live {
            return Err(format!(
                "`{id}` is coming, not built yet; the `{}` template has {}",
                self.meta.id,
                self.language_list()
            ));
        }
        let own: Vec<&(String, String, String)> = self
            .variant_files
            .iter()
            .filter(|(i, _, _)| i == id)
            .collect();
        let mut files: Vec<(String, String)> = self
            .files
            .iter()
            .filter(|(p, _)| !own.iter().any(|(_, c, _)| c == p))
            .cloned()
            .collect();
        files.extend(own.into_iter().map(|(_, p, x)| (p.clone(), x.clone())));
        Ok(Self {
            files,
            variant_files: Vec::new(),
            ..self.clone()
        })
    }

    fn language_list(&self) -> String {
        let live: Vec<_> = self.live_variants().iter().map(|v| v.id.clone()).collect();
        if live.is_empty() {
            "no languages".into()
        } else {
            live.join(", ")
        }
    }

    /// The variant's build, run and env with the values filled.
    #[must_use]
    pub fn runner(&self, id: &str, vals: &[(String, String)]) -> Option<Runner> {
        let v = self.variant(id)?;
        let f = |s: &String| filled(s, vals);
        Some(Runner {
            label: v.label.clone(),
            build: v.build.iter().map(f).collect(),
            run: v.run.iter().map(f).collect(),
            test: v.test.iter().map(f).collect(),
            by_test: v.smoke == SmokeBy::Test,
            env: v.env.iter().map(|(k, x)| (k.clone(), f(x))).collect(),
        })
    }

    /// The steps init runs.
    #[must_use]
    pub fn steps(&self) -> Vec<StepDef> {
        if self.meta.steps.is_empty() {
            return [StepName::Git, StepName::Register]
                .into_iter()
                .map(|name| StepDef { name, prompt: None })
                .collect();
        }
        self.meta.steps.clone()
    }
}

// ---- sources -------------------------------------------------------------------------------

/// What `--template` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    /// A local folder.
    Path(PathBuf),
    /// A git repository: URL, folder inside it, branch or tag.
    Git {
        url: String,
        subdir: Option<String>,
        rev: Option<String>,
    },
    /// An id: `~/.inorbit/templates/<id>/`, then the built-ins.
    Id(String),
}

/// `gh:owner/repo[//subdir][@ref]`, `git:URL[//subdir][@ref]`, a path (has a separator, or
/// starts with `.` or `~`), or an id.
#[must_use]
pub fn parse_spec(s: &str) -> Spec {
    let s = s.trim();
    let git = |url: String, rest: &str| {
        let (rest, rev) = match rest.rsplit_once('@') {
            Some((r, v)) if !v.is_empty() => (r, Some(v.to_owned())),
            _ => (rest, None),
        };
        let (repo, subdir) = match rest.split_once("//") {
            Some((r, d)) if !d.is_empty() => (r, Some(d.trim_matches('/').to_owned())),
            _ => (rest, None),
        };
        (url, repo.to_owned(), subdir, rev)
    };
    if let Some(rest) = s.strip_prefix("gh:") {
        let (_, repo, subdir, rev) = git(String::new(), rest);
        return Spec::Git {
            url: format!("https://github.com/{repo}.git"),
            subdir,
            rev,
        };
    }
    if let Some(rest) = s.strip_prefix("git:") {
        // A URL holds `//` itself (https://), so the subdir marker is the last `//` after
        // the path; keep it simple: split off `@ref` then `//subdir` only after a `.git`.
        let (rest, rev) = match rest.rsplit_once('@') {
            Some((r, v)) if !v.is_empty() && !v.contains('/') && !r.ends_with(':') => {
                (r, Some(v.to_owned()))
            }
            _ => (rest, None),
        };
        let (url, subdir) = match rest.split_once(".git//") {
            Some((u, d)) => (format!("{u}.git"), Some(d.trim_matches('/').to_owned())),
            None => (rest.to_owned(), None),
        };
        return Spec::Git { url, subdir, rev };
    }
    if s.contains('/') || s.contains('\\') || s.starts_with('.') || s.starts_with('~') {
        return Spec::Path(expand_home(s));
    }
    Spec::Id(s.to_owned())
}

fn expand_home(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => etcetera::home_dir().map_or_else(|_| PathBuf::from(s), |h| h.join(rest)),
        None => PathBuf::from(s),
    }
}

/// `~/.inorbit/templates`.
///
/// # Errors
///
/// No home folder.
pub fn user_root() -> Result<PathBuf, Error> {
    etcetera::home_dir()
        .map(|h| h.join(".inorbit").join("templates"))
        .map_err(|e| Error::Failed(format!("no home folder: {e}")))
}

macro_rules! bf {
    ($path:expr, $src:expr) => {
        ($path, include_str!(concat!("../templates/project/", $src)))
    };
}

type Embedded = &'static [(&'static str, &'static str)];

const DEMO: Embedded = &[
    bf!("template.toml", "demo/template.toml"),
    bf!("extension.toml", "demo/extension.toml"),
    bf!("README.md", "demo/README.md"),
    bf!("inorbit.toml", "demo/inorbit.toml"),
    bf!(".gitignore", "demo/gitignore"),
    bf!("panel/index.html", "demo/panel/index.html"),
    bf!(
        "variants/python/src/@@MODULE@@/__init__.py",
        "demo/variants/python/src/module/__init__.py"
    ),
    bf!(
        "variants/python/src/@@MODULE@@/__main__.py",
        "demo/variants/python/src/module/__main__.py"
    ),
    bf!(
        "variants/python/src/@@MODULE@@/channel.py",
        "demo/variants/python/src/module/channel.py"
    ),
    bf!(
        "variants/python/tests/test_selftest.py",
        "demo/variants/python/tests/test_selftest.py"
    ),
    bf!(
        "variants/typescript/package.json",
        "demo/variants/typescript/package.json"
    ),
    bf!(
        "variants/typescript/src/channel.ts",
        "demo/variants/typescript/src/channel.ts"
    ),
    bf!(
        "variants/typescript/src/main.ts",
        "demo/variants/typescript/src/main.ts"
    ),
    bf!(
        "variants/typescript/tests/selftest.test.ts",
        "demo/variants/typescript/tests/selftest.test.ts"
    ),
    bf!("variants/go/go.mod", "demo/variants/go/go.mod"),
    bf!("variants/go/channel.go", "demo/variants/go/channel.go"),
    bf!("variants/go/main.go", "demo/variants/go/main.go"),
    bf!("variants/go/main_test.go", "demo/variants/go/main_test.go"),
    bf!("variants/rust/Cargo.toml", "demo/variants/rust/Cargo.toml"),
    bf!("variants/rust/Cargo.lock", "demo/variants/rust/Cargo.lock"),
    bf!(
        "variants/rust/src/channel.rs",
        "demo/variants/rust/src/channel.rs"
    ),
    bf!(
        "variants/rust/src/main.rs",
        "demo/variants/rust/src/main.rs"
    ),
    bf!(
        "variants/rust/tests/selftest.rs",
        "demo/variants/rust/tests/selftest.rs"
    ),
];
const BLANK: Embedded = &[
    bf!("template.toml", "blank/template.toml"),
    bf!("README.md", "blank/README.md"),
    bf!("inorbit.toml", "blank/inorbit.toml"),
    bf!(".gitignore", "blank/gitignore"),
    bf!(
        "variants/python/pyproject.toml",
        "blank/variants/python/pyproject.toml"
    ),
    bf!(
        "variants/python/src/@@MODULE@@/__init__.py",
        "blank/variants/python/src/module/__init__.py"
    ),
    bf!(
        "variants/python/tests/test_basic.py",
        "blank/variants/python/tests/test_basic.py"
    ),
    bf!(
        "variants/typescript/package.json",
        "blank/variants/typescript/package.json"
    ),
    bf!(
        "variants/typescript/src/index.ts",
        "blank/variants/typescript/src/index.ts"
    ),
    bf!(
        "variants/typescript/tests/index.test.ts",
        "blank/variants/typescript/tests/index.test.ts"
    ),
    bf!("variants/go/go.mod", "blank/variants/go/go.mod"),
    bf!("variants/go/main.go", "blank/variants/go/main.go"),
    bf!("variants/go/main_test.go", "blank/variants/go/main_test.go"),
    bf!("variants/rust/Cargo.toml", "blank/variants/rust/Cargo.toml"),
    bf!("variants/rust/Cargo.lock", "blank/variants/rust/Cargo.lock"),
    bf!(
        "variants/rust/src/main.rs",
        "blank/variants/rust/src/main.rs"
    ),
];
const COORDINATED: Embedded = &[
    bf!("template.toml", "coordinated/template.toml"),
    bf!("README.md", "coordinated/README.md"),
    bf!("inorbit.toml", "coordinated/inorbit.toml"),
    bf!("coordination.toml", "coordinated/coordination.toml"),
];
const MACOS_APP: Embedded = &[bf!("template.toml", "macos-app/template.toml")];
const UI_PANEL: Embedded = &[bf!("template.toml", "ui-panel/template.toml")];

/// The built-ins, in the order they are listed.
const BUILT_INS: &[(&str, Embedded)] = &[
    ("blank", BLANK),
    ("demo", DEMO),
    ("coordinated", COORDINATED),
    ("macos-app", MACOS_APP),
    ("ui-panel", UI_PANEL),
];

fn parse_meta(text: &str, from: &str) -> Result<TemplateToml, String> {
    toml::from_str(text).map_err(|e| format!("{from}/{FILE}: {}", e.message()))
}

/// The files of one variant: `(variant, path, text)`.
type VariantFiles = Vec<(String, String, String)>;

/// Splits `variants/<id>/path` files from the common ones.
fn split_variants(all: Vec<(String, String)>) -> (VariantFiles, Vec<(String, String)>) {
    let mut variant = Vec::new();
    let mut common = Vec::new();
    for (p, text) in all {
        match p.strip_prefix("variants/").and_then(|r| r.split_once('/')) {
            Some((id, rest)) => variant.push((id.to_owned(), rest.to_owned(), text)),
            None => common.push((p, text)),
        }
    }
    (variant, common)
}

fn built_in(id: &str) -> Option<Template> {
    let (_, files) = BUILT_INS.iter().find(|(i, _)| *i == id)?;
    let meta = files
        .iter()
        .find(|(p, _)| *p == FILE)
        .and_then(|(_, t)| toml::from_str(t).ok())?;
    let (variant_files, files) = split_variants(
        files
            .iter()
            .filter(|(p, _)| *p != FILE)
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect(),
    );
    Some(Template {
        meta,
        files,
        variant_files,
        origin: Origin::BuiltIn,
    })
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {}", dir.display(), e.kind()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let p = e.path();
        let rel = crate::paths::slashed(p.strip_prefix(root).map_err(|e| e.to_string())?);
        let ft = e.file_type().map_err(|e| e.to_string())?;
        if ft.is_symlink() {
            return Err(format!("{rel}: symlinks are not followed in a template"));
        }
        if ft.is_dir() {
            if e.file_name() != ".git" {
                walk(root, &p, out)?;
            }
        } else if rel != FILE {
            if out.len() >= MAX_FILES {
                return Err(format!("more than {MAX_FILES} files"));
            }
            if p.metadata().map_err(|e| e.to_string())?.len() > MAX_BYTES {
                return Err(format!("{rel} is over 1 MiB; a template holds text"));
            }
            let text = std::fs::read_to_string(&p)
                .map_err(|_| format!("{rel} is not UTF-8 text; a template holds text"))?;
            out.push((rel, text));
        }
    }
    Ok(())
}

/// A template folder as it is, `extends` not applied.
fn from_dir(dir: &Path, origin: Origin) -> Result<Template, String> {
    let text = std::fs::read_to_string(dir.join(FILE))
        .map_err(|_| format!("{} has no {FILE}", dir.display()))?;
    let meta = parse_meta(&text, &dir.display().to_string())?;
    let mut all = Vec::new();
    walk(dir, dir, &mut all)?;
    let (variant_files, files) = split_variants(all);
    Ok(Template {
        meta,
        files,
        variant_files,
        origin,
    })
}

fn merge(parent: Template, child: Template) -> Template {
    let mut questions = parent.meta.questions;
    for q in child.meta.questions {
        match questions.iter().position(|p| p.key == q.key) {
            Some(i) => questions[i] = q,
            None => questions.push(q),
        }
    }
    let mut files: Vec<_> = parent
        .files
        .into_iter()
        .filter(|(p, _)| !child.files.iter().any(|(c, _)| c == p))
        .collect();
    files.extend(child.files);
    let mut variants = parent.meta.variants;
    for v in child.meta.variants {
        match variants.iter().position(|p| p.id == v.id) {
            Some(i) => variants[i] = v,
            None => variants.push(v),
        }
    }
    let mut variant_files: Vec<_> = parent
        .variant_files
        .into_iter()
        .filter(|(i, p, _)| {
            !child
                .variant_files
                .iter()
                .any(|(ci, cp, _)| ci == i && cp == p)
        })
        .collect();
    variant_files.extend(child.variant_files);
    let mut programs = parent.meta.programs;
    for p in child.meta.programs {
        if !programs.contains(&p) {
            programs.push(p);
        }
    }
    Template {
        variant_files,
        meta: TemplateToml {
            variants,
            programs,
            default_variant: child.meta.default_variant.or(parent.meta.default_variant),
            questions,
            steps: if child.meta.steps.is_empty() {
                parent.meta.steps
            } else {
                child.meta.steps
            },
            agent_prompt: child.meta.agent_prompt.or(parent.meta.agent_prompt),
            ..child.meta
        },
        files,
        origin: child.origin,
    }
}

fn user_template(id: &str, home: &Path) -> Option<PathBuf> {
    let d = home.join(id);
    d.join(FILE).is_file().then_some(d)
}

fn load_id(id: &str, root: &Path, depth: u8) -> Result<Template, String> {
    let t = if let Some(d) = user_template(id, root) {
        from_dir(&d, Origin::User(d.clone()))?
    } else {
        built_in(id).ok_or_else(|| format!("there is no `{id}` template"))?
    };
    extend(t, root, depth)
}

fn extend(t: Template, root: &Path, depth: u8) -> Result<Template, String> {
    let Some(parent) = t.meta.extends.clone() else {
        return Ok(t);
    };
    if depth >= 8 {
        return Err("`extends` goes more than 8 deep; is it a cycle?".into());
    }
    let p = load_id(&parent, root, depth + 1)
        .map_err(|e| format!("`{}` extends `{parent}`: {e}", t.meta.id))?;
    Ok(merge(p, t))
}

/// A shallow clone into a temp folder, read into memory, and removed. Network and disk:
/// a dry run never calls it.
fn from_git(
    url: &str,
    subdir: Option<&str>,
    rev: Option<&str>,
    root: &Path,
) -> Result<Template, String> {
    if url.starts_with('-') || url.starts_with("ext::") {
        return Err("that is not a git URL I will clone".into());
    }
    if subdir.is_some_and(|d| {
        Path::new(d)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    }) {
        return Err("the subfolder must stay inside the repository".into());
    }
    let tmp = std::env::temp_dir().join(format!(
        "iohr-template-{}-{}",
        std::process::id(),
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "protocol.ext.allow=never",
        "clone",
        "-q",
        "--depth",
        "1",
    ]);
    if let Some(r) = rev {
        cmd.args(["--branch", r]);
    }
    cmd.arg("--").arg(url).arg(&tmp);
    let out = cmd
        .output()
        .map_err(|e| format!("git could not start: {}", e.kind()))?;
    let loaded = if out.status.success() {
        let dir = subdir.map_or_else(|| tmp.clone(), |d| tmp.join(d));
        from_dir(&dir, Origin::Git(url.to_owned())).and_then(|t| extend(t, root, 0))
    } else {
        Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("")
        ))
    };
    let _ = std::fs::remove_dir_all(&tmp);
    loaded
}

/// Loads the template a spec names, `extends` applied.
///
/// # Errors
///
/// One line, in plain words.
pub fn load(spec: &Spec, root: &Path) -> Result<Template, String> {
    match spec {
        Spec::Id(id) => load_id(id, root, 0).map_err(|e| {
            if e.starts_with("there is no") {
                format!("{e}; `iohr project template list` shows them")
            } else {
                e
            }
        }),
        Spec::Path(p) => {
            let t = from_dir(p, Origin::Path(p.clone()))?;
            extend(t, root, 0)
        }
        Spec::Git { url, subdir, rev } => from_git(url, subdir.as_deref(), rev.as_deref(), root),
    }
}

/// One line of `iohr project template list`.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub name: String,
    pub description: String,
    pub status: String,
    pub default: bool,
    pub origin: String,
}

/// Every template of the user's folder and the built-ins: a user template of a built-in's id
/// shadows it (and says so). A broken one is listed `invalid` with the reason.
#[must_use]
pub fn list(root: &Path, default: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let mut user: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join(FILE).is_file())
                .collect()
        })
        .unwrap_or_default();
    user.sort();
    let user_id = |p: &Path| p.file_name().map(|f| f.to_string_lossy().into_owned());
    let row = |id: String, d: String, s: &str, o: String| Row {
        default: id == default,
        name: id,
        description: d,
        status: s.to_owned(),
        origin: o,
    };
    for (id, _) in BUILT_INS {
        let shadow = user.iter().find(|p| user_id(p).as_deref() == Some(id));
        let t = shadow
            .and_then(|p| from_dir(p, Origin::User(p.clone())).ok())
            .or_else(|| built_in(id));
        if let Some(t) = t {
            let origin = if shadow.is_some() {
                format!("{} (shadows built-in)", t.origin.label())
            } else {
                t.origin.label()
            };
            rows.push(row(
                (*id).into(),
                t.meta.description,
                if t.meta.status == Status::Live {
                    "live"
                } else {
                    "coming"
                },
                origin,
            ));
        }
    }
    for p in &user {
        let Some(id) = user_id(p) else { continue };
        if BUILT_INS.iter().any(|(b, _)| *b == id) {
            continue;
        }
        rows.push(match from_dir(p, Origin::User(p.clone())) {
            Ok(t) => row(
                id,
                t.meta.description,
                if t.meta.status == Status::Live {
                    "live"
                } else {
                    "coming"
                },
                Origin::User(p.clone()).label(),
            ),
            Err(e) => row(id, e, "invalid", Origin::User(p.clone()).label()),
        });
    }
    rows
}

// ---- questions and answers ------------------------------------------------------------------

fn truthy(a: Option<&Answer>) -> bool {
    match a {
        Some(Answer::Flag(b)) => *b,
        Some(Answer::Text(t)) => !t.is_empty() && t != "false",
        Some(Answer::List(l)) => !l.is_empty(),
        None => false,
    }
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(v)
}

/// The `when` grammar: `key`, `!key`, `key == value`, `key != value`.
/// A question asked at the terminal.
pub type Asker<'a> = dyn FnMut(&Question) -> Result<Answer, Error> + 'a;

/// Key, optional (equals, value) comparison, negated.
type When = (String, Option<(bool, String)>, bool);

fn parse_when(expr: &str) -> Result<When, String> {
    let e = expr.trim();
    let key_ok = |k: &str| {
        !k.is_empty()
            && k.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    for (op, eq) in [("==", true), ("!=", false)] {
        if let Some((k, v)) = e.split_once(op) {
            let k = k.trim();
            return if key_ok(k) {
                Ok((k.to_owned(), Some((eq, unquote(v).to_owned())), false))
            } else {
                Err(format!("`when = {expr:?}`: `{k}` is not a question key"))
            };
        }
    }
    let (neg, k) = e.strip_prefix('!').map_or((false, e), |r| (true, r.trim()));
    if key_ok(k) {
        Ok((k.to_owned(), None, neg))
    } else {
        Err(format!(
            "`when = {expr:?}` is not `key`, `!key`, `key == value` or `key != value`"
        ))
    }
}

/// Whether a `when` holds over the answers so far. A key that was not answered is false and
/// empty.
///
/// # Errors
///
/// An expression that is not in the grammar.
pub fn when_holds(expr: &str, answers: &Answers) -> Result<bool, String> {
    let (key, cmp, neg) = parse_when(expr)?;
    let a = answers.get(&key);
    Ok(match cmp {
        None => truthy(a) != neg,
        Some((eq, v)) => {
            let hit = match a {
                Some(Answer::List(l)) => l.contains(&v),
                Some(other) => other.text() == v,
                None => v.is_empty(),
            };
            hit == eq
        }
    })
}

fn parse_flag(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "y" | "1" | "on" => Some(true),
        "false" | "no" | "n" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// An answer checked against its question: the type, the choices and the regex.
///
/// # Errors
///
/// One line saying what is wrong.
pub fn coerce(q: &Question, raw: &Answer) -> Result<Answer, String> {
    let key = &q.key;
    let one_of = |v: &str| {
        if q.choices.iter().any(|c| c == v) {
            Ok(())
        } else {
            Err(format!(
                "{key}: `{v}` is not one of {}",
                q.choices.join(", ")
            ))
        }
    };
    match q.kind {
        Kind::Bool => match raw {
            Answer::Flag(b) => Ok(Answer::Flag(*b)),
            Answer::Text(t) => parse_flag(t)
                .map(Answer::Flag)
                .ok_or_else(|| format!("{key}: `{t}` is not yes or no")),
            Answer::List(_) => Err(format!("{key}: a yes or no, not a list")),
        },
        Kind::Choice => {
            let v = raw.text();
            one_of(&v)?;
            Ok(Answer::Text(v))
        }
        Kind::Multi => {
            let items: Vec<String> = match raw {
                Answer::List(l) => l.clone(),
                other => other
                    .text()
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect(),
            };
            for i in &items {
                one_of(i)?;
            }
            Ok(Answer::List(items))
        }
        Kind::String => {
            let v = raw.text();
            if let Some(re) = &q.regex {
                let re = Regex::new(&format!("^(?:{re})$"))
                    .map_err(|e| format!("{key}: the regex is bad: {e}"))?;
                if !re.is_match(&v) {
                    return Err(format!(
                        "{key}: `{v}` does not match {}",
                        q.regex.as_deref().unwrap_or_default()
                    ));
                }
            }
            Ok(Answer::Text(v))
        }
    }
}

/// `--set key=value` items.
///
/// # Errors
///
/// An item without `=`.
pub fn parse_set(items: &[String]) -> Result<Answers, String> {
    let mut out = Answers::new();
    for i in items {
        let (k, v) = i
            .split_once('=')
            .ok_or_else(|| format!("--set {i}: write it as key=value"))?;
        out.insert(k.trim().to_owned(), Answer::Text(v.to_owned()));
    }
    Ok(out)
}

/// `--answers file.json|toml`: a table of key to value.
///
/// # Errors
///
/// An unreadable file, an unknown extension, or a bad table.
pub fn read_answers(path: &Path) -> Result<Answers, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e.kind()))?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Some("toml") => {
            toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e.message()))
        }
        _ => Err(format!(
            "{}: answers are a .json or .toml file",
            path.display()
        )),
    }
}

/// The values `@@KEY@@` take: the built-ins, then one per answered question.
#[must_use]
pub fn values(
    name: &str,
    publisher: &str,
    created_at: &str,
    answers: &Answers,
) -> Vec<(String, String)> {
    let mut v = vec![
        ("@@ID@@".to_owned(), format!("{publisher}/{name}")),
        ("@@NAME@@".to_owned(), name.to_owned()),
        ("@@MODULE@@".to_owned(), name.replace('-', "_")),
        ("@@CREATED_AT@@".to_owned(), created_at.to_owned()),
    ];
    v.extend(
        answers
            .iter()
            .map(|(k, a)| (format!("@@{}@@", k.to_ascii_uppercase()), a.text())),
    );
    v
}

fn filled(text: &str, vals: &[(String, String)]) -> String {
    let refs: Vec<(&str, String)> = vals.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    fill(text, &refs)
}

/// Fills a prompt or any text with the values.
#[must_use]
pub fn fill_text(text: &str, vals: &[(String, String)]) -> String {
    filled(text, vals)
}

/// [`values`] plus what the chosen language adds: `@@LANGUAGE_LABEL@@` and the variant's own
/// `vars`, first, so a text that holds other placeholders is filled by the later ones.
#[must_use]
pub fn all_values(
    t: &Template,
    name: &str,
    publisher: &str,
    created_at: &str,
    answers: &Answers,
) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(var) = answers.get(LANGUAGE).and_then(|a| t.variant(&a.text())) {
        v.push(("@@LANGUAGE_LABEL@@".to_owned(), var.label.clone()));
        v.extend(
            var.vars
                .iter()
                .map(|(k, x)| (format!("@@{k}@@"), x.clone())),
        );
    }
    v.extend(values(name, publisher, created_at, answers));
    v
}

/// The paths of the template's `programs`, placeholders filled.
#[must_use]
pub fn programs(t: &Template, vals: &[(String, String)]) -> Vec<String> {
    t.meta.programs.iter().map(|p| filled(p, vals)).collect()
}

/// The template's files with placeholders filled in text and paths. A question that was
/// skipped by its `when` fills as empty.
///
/// # Errors
///
/// A filled path that leaves the folder.
pub fn render(t: &Template, vals: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut vals = vals.to_vec();
    if !vals.iter().any(|(x, _)| x == "@@DESCRIPTION@@") {
        vals.push(("@@DESCRIPTION@@".to_owned(), String::new()));
    }
    for q in &t.all_questions() {
        let k = format!("@@{}@@", q.key.to_ascii_uppercase());
        if !vals.iter().any(|(x, _)| *x == k) {
            vals.push((k, String::new()));
        }
    }
    t.files
        .iter()
        .map(|(p, text)| {
            let path = filled(p, &vals);
            let ok = !path.is_empty()
                && Path::new(&path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)));
            if ok {
                Ok((path, filled(text, &vals)))
            } else {
                Err(format!(
                    "`{p}` fills to `{path}`, which leaves the project folder"
                ))
            }
        })
        .collect()
}

/// Settles every answer: `given` (`--set`, then `--answers`) first, then `ask` at a terminal,
/// then the default. A question whose `when` is false is skipped. A question with nothing to
/// answer it is an error naming the key, never a guess.
///
/// # Errors
///
/// A bad answer, a key no question has, or a question nothing answers.
pub fn answer(
    t: &Template,
    given: &Answers,
    mut ask: Option<&mut Asker<'_>>,
) -> Result<Answers, Error> {
    let questions = t.all_questions();
    for k in given.keys() {
        if !questions.iter().any(|q| &q.key == k) {
            return Err(Error::Usage(format!(
                "no question named `{k}` in the `{}` template",
                t.meta.id
            )));
        }
    }
    if let Some(l) = given.get(LANGUAGE).map(Answer::text) {
        t.with_variant(&l).map_err(Error::Usage)?;
    }
    let mut out = Answers::new();
    for q in &questions {
        if let Some(w) = &q.when
            && !when_holds(w, &out).map_err(Error::Usage)?
        {
            continue;
        }
        let a = if let Some(g) = given.get(&q.key) {
            g.clone()
        } else if let Some(f) = ask.as_mut() {
            f(q)?
        } else if let Some(d) = &q.default {
            d.clone()
        } else {
            return Err(Error::Usage(format!(
                "`{}` needs an answer ({}): pass --set {}=VALUE or --answers FILE",
                q.key, q.prompt, q.key
            )));
        };
        out.insert(q.key.clone(), coerce(q, &a).map_err(Error::Usage)?);
    }
    Ok(out)
}

// ---- lint, new, schema ----------------------------------------------------------------------

/// What the lint found.
#[derive(Debug, Clone, Serialize)]
pub struct LintReport {
    pub ok: bool,
    pub findings: Vec<String>,
}

fn id_ok(id: &str) -> bool {
    (2..=32).contains(&id.len())
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("@@") {
        rest = &rest[i + 2..];
        if let Some(j) = rest.find("@@") {
            let k = &rest[..j];
            if !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                out.push(k.to_owned());
                rest = &rest[j + 2..];
            }
        }
    }
    out
}

fn lint_variants(t: &Template, f: &mut Vec<String>) {
    let m = &t.meta;
    if m.variants.is_empty() {
        if let Some((i, p, _)) = t.variant_files.first() {
            f.push(format!(
                "variants/{i}/{p}: no [[variants]] entry, so no language has files"
            ));
        }
        if m.default_variant.is_some() {
            f.push("default_variant names no variant: there are no [[variants]]".into());
        }
        return;
    }
    if m.questions.iter().any(|q| q.key == LANGUAGE) {
        f.push(format!(
            "question `{LANGUAGE}`: that key is the language question of the variants"
        ));
    }
    let mut ids: Vec<&str> = Vec::new();
    for v in &m.variants {
        let id = v.id.as_str();
        let ok = (2..=16).contains(&id.len())
            && id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !ok {
            f.push(format!(
                "variant `{id}`: lowercase letters and digits, 2 to 16, starting with a letter"
            ));
        }
        if ids.contains(&id) {
            f.push(format!("variant `{id}` is listed twice"));
        }
        ids.push(id);
        if v.label.trim().is_empty() {
            f.push(format!("variant `{id}` has no label"));
        }
        if v.status == Status::Live && m.steps.iter().any(|s| s.name == StepName::Smoke) {
            if v.smoke == SmokeBy::Test && v.test.is_empty() {
                f.push(format!(
                    "variant `{id}` smokes by `test` and is live, but it has no `test`"
                ));
            } else if v.smoke == SmokeBy::Heartbeat && v.run.is_empty() {
                f.push(format!(
                    "variant `{id}` is live and the template has a smoke step, but it has no `run`"
                ));
            }
        }
        if v.needs
            .iter()
            .any(|n| n.is_empty() || n.contains(['/', ' ', '\\']))
        {
            f.push(format!(
                "variant `{id}`: `needs` lists program names, not paths"
            ));
        }
        for k in v.vars.keys() {
            let key_ok = !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !key_ok {
                f.push(format!(
                    "variant `{id}`: var `{k}` is upper case letters, digits and underscores"
                ));
            } else if [
                "NAME",
                "ID",
                "MODULE",
                "CREATED_AT",
                "PUBLISHER",
                "LANGUAGE",
                "LANGUAGE_LABEL",
            ]
            .contains(&k.as_str())
            {
                f.push(format!(
                    "variant `{id}`: var `{k}` is a built-in placeholder"
                ));
            }
        }
    }
    if let Some(d) = &m.default_variant
        && !m
            .variants
            .iter()
            .any(|v| &v.id == d && v.status == Status::Live)
    {
        f.push(format!("default_variant `{d}` is not a live variant"));
    }
    for (i, p, _) in &t.variant_files {
        if !ids.contains(&i.as_str()) {
            f.push(format!("variants/{i}/{p}: no [[variants]] entry `{i}`"));
        }
    }
    for p in &m.programs {
        if Path::new(p).is_absolute() || p.contains("..") {
            f.push(format!("programs: `{p}` must be a path inside the project"));
        }
    }
}

/// Checks a template folder: the file parses, ids and keys are well formed, defaults fit
/// their questions, every `when` names an earlier key, every placeholder in a file or path is a
/// built-in or a question, `extends` resolves, and a rendered `extension.toml` passes the
/// extension lint.
#[must_use]
#[allow(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "one flat list of checks, each a short finding"
)]
pub fn lint_dir(path: &Path, root: &Path) -> LintReport {
    let mut f: Vec<String> = Vec::new();
    let t = match from_dir(path, Origin::Path(path.to_path_buf())) {
        Ok(t) => t,
        Err(e) => {
            return LintReport {
                ok: false,
                findings: vec![e],
            };
        }
    };
    let m = &t.meta;
    if !id_ok(&m.id) {
        f.push(format!(
            "id `{}`: lowercase letters, digits and dashes, 2 to 32, starting with a letter",
            m.id
        ));
    }
    if m.description.trim().is_empty() {
        f.push("description is empty".into());
    }
    let mut seen: Vec<&str> = Vec::new();
    for q in &m.questions {
        let k = q.key.as_str();
        let key_ok = k.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && k.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !key_ok {
            f.push(format!(
                "question `{k}`: a key is lowercase letters, digits and underscores"
            ));
        }
        if BUILTIN_KEYS.contains(&k) {
            f.push(format!(
                "question `{k}`: that key is a built-in placeholder"
            ));
        }
        if seen.contains(&k) {
            f.push(format!("question `{k}` is asked twice"));
        }
        if q.prompt.trim().is_empty() {
            f.push(format!("question `{k}` has no prompt"));
        }
        let needs_choices = matches!(q.kind, Kind::Choice | Kind::Multi);
        if needs_choices && q.choices.is_empty() {
            f.push(format!("question `{k}` is a {:?} with no choices", q.kind).to_lowercase());
        }
        if !needs_choices && !q.choices.is_empty() {
            f.push(format!(
                "question `{k}` has choices but is not a choice or multi"
            ));
        }
        if q.regex.as_ref().is_some_and(|r| Regex::new(r).is_err()) {
            f.push(format!("question `{k}`: the regex does not compile"));
        }
        if let Some(d) = &q.default
            && let Err(e) = coerce(q, d)
        {
            f.push(format!("the default of {e}"));
        }
        if let Some(w) = &q.when {
            match parse_when(w) {
                Ok((wk, _, _)) if !seen.contains(&wk.as_str()) => f.push(format!(
                    "question `{k}`: `when` names `{wk}`, which is not an earlier question"
                )),
                Ok(_) => {}
                Err(e) => f.push(format!("question `{k}`: {e}")),
            }
        }
        seen.push(k);
    }
    lint_variants(&t, &mut f);
    if m.steps.iter().filter(|s| s.name == StepName::Agent).count() > 1 {
        f.push("more than one agent step".into());
    }
    if m.steps
        .iter()
        .any(|s| s.name != StepName::Agent && s.prompt.is_some())
    {
        f.push("only an agent step has a prompt".into());
    }
    if t.files.is_empty() && m.extends.is_none() && m.status == Status::Live {
        f.push("a live template with no files".into());
    }
    match extend(t.clone(), root, 0) {
        Err(e) => f.push(e),
        Ok(full) => {
            let known: Vec<String> = ["NAME", "ID", "MODULE", "CREATED_AT", "DESCRIPTION"]
                .iter()
                .map(|s| (*s).to_owned())
                .chain(
                    full.all_questions()
                        .iter()
                        .map(|q| q.key.to_ascii_uppercase()),
                )
                .chain(
                    full.meta
                        .variants
                        .iter()
                        .flat_map(|v| v.vars.keys().cloned())
                        .chain(
                            (!full.meta.variants.is_empty()).then(|| "LANGUAGE_LABEL".to_owned()),
                        ),
                )
                .collect();
            let prompts = full
                .meta
                .steps
                .iter()
                .filter_map(|s| s.prompt.as_deref())
                .chain(full.meta.agent_prompt.as_deref());
            let own = t
                .files
                .iter()
                .flat_map(|(p, x)| [p.as_str(), x.as_str()])
                .chain(
                    t.variant_files
                        .iter()
                        .flat_map(|(_, p, x)| [p.as_str(), x.as_str()]),
                )
                .chain(m.variants.iter().flat_map(|v| {
                    v.vars
                        .values()
                        .chain(&v.build)
                        .chain(&v.run)
                        .chain(&v.test)
                        .chain(v.env.values())
                        .map(String::as_str)
                }))
                .chain(prompts);
            for p in own.flat_map(placeholders) {
                if !known.contains(&p) {
                    let msg = format!("@@{p}@@ is not a built-in or a question key");
                    if !f.contains(&msg) {
                        f.push(msg);
                    }
                }
            }
            // A sample render of every language (or of the one set of files): defaults where
            // there are some, `sample` where not.
            let mut sample = Answers::new();
            for q in &full.all_questions() {
                let a = q.default.clone().unwrap_or_else(|| match q.kind {
                    Kind::Bool => Answer::Flag(false),
                    Kind::Choice => Answer::Text(q.choices.first().cloned().unwrap_or_default()),
                    Kind::Multi => Answer::List(vec![]),
                    Kind::String => Answer::Text("sample".into()),
                });
                sample.insert(q.key.clone(), a);
            }
            let languages: Vec<Option<String>> = if full.meta.variants.is_empty() {
                vec![None]
            } else {
                full.live_variants()
                    .iter()
                    .map(|v| Some(v.id.clone()))
                    .collect()
            };
            for lang in languages {
                let mut answers = sample.clone();
                let mut tpl = full.clone();
                let tag = lang.as_ref().map_or(String::new(), |l| format!("{l}: "));
                if let Some(l) = &lang {
                    answers.insert(LANGUAGE.into(), Answer::Text(l.clone()));
                    match full.with_variant(l) {
                        Ok(x) => tpl = x,
                        Err(e) => {
                            f.push(e);
                            continue;
                        }
                    }
                }
                let vals = all_values(&tpl, "sample", "local", "2026-01-01T00:00:00Z", &answers);
                match render(&tpl, &vals) {
                    Err(e) => f.push(format!("{tag}{e}")),
                    Ok(files) => {
                        for (p, x) in &files {
                            for left in placeholders(p).into_iter().chain(placeholders(x)) {
                                let msg = format!("{tag}@@{left}@@ is left unfilled in {p}");
                                if !tpl.meta.variants.is_empty() && !f.contains(&msg) {
                                    f.push(msg);
                                }
                            }
                        }
                        if let Some((_, manifest)) = files.iter().find(|(p, _)| p == SOURCE_FILE) {
                            let r = lint(manifest);
                            f.extend(
                                r.findings
                                    .iter()
                                    .map(|x| format!("{tag}{SOURCE_FILE}: {}", x.message)),
                            );
                        }
                    }
                }
            }
        }
    }
    LintReport {
        ok: f.is_empty(),
        findings: f,
    }
}

/// A new template of your own in `root/<id>/`: a working one, with a question and a `when`.
///
/// # Errors
///
/// A bad id, a folder that exists, or a write that failed.
pub fn scaffold(root: &Path, id: &str, extends: Option<&str>) -> Result<PathBuf, String> {
    if !id_ok(id) {
        return Err(format!(
            "`{id}`: lowercase letters, digits and dashes, 2 to 32, starting with a letter"
        ));
    }
    let dir = root.join(id);
    if dir.exists() {
        return Err(format!(
            "{} already exists; nothing was changed",
            dir.display()
        ));
    }
    let ext = extends.map_or(String::new(), |e| format!("extends = {e:?}\n"));
    let toml = format!(
        "# A template of your own. `iohr project template lint {path}` checks it,\n# `iohr project template schema` prints the contract, `iohr project init NAME --template {id} --dry-run` tries it.\nid = {id:?}\ndescription = \"What a project from this template is\"\nstatus = \"live\"\n{ext}\n# Questions are asked in a terminal, and answered with --set key=value, --answers FILE or --yes.\n[[questions]]\nkey = \"flavour\"\nprompt = \"Which flavour?\"\ntype = \"choice\"\nchoices = [\"plain\", \"fancy\"]\ndefault = \"plain\"\nhelp = \"Becomes @@FLAVOUR@@ in the files\"\n\n[[questions]]\nkey = \"motto\"\nprompt = \"A motto for the fancy flavour\"\ndefault = \"hello\"\nwhen = \"flavour == fancy\"\nregex = \"[a-z ]{{1,40}}\"\n\n# Steps after the files are written: git, register, smoke, agent. The agent step starts your\n# own claude or codex in the new folder (its output is labelled AI).\n[[steps]]\nname = \"git\"\n\n[[steps]]\nname = \"register\"\n",
        path = dir.display()
    );
    let readme = "# @@NAME@@\n\nMade by `iohr project init` from the `@@ID@@` template.\n\nFlavour: @@FLAVOUR@@ (@@MOTTO@@)\n";
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("cannot create {}: {}", dir.display(), e.kind()))?;
    std::fs::write(dir.join(FILE), toml).map_err(|e| e.kind().to_string())?;
    std::fs::write(dir.join("README.md"), readme).map_err(|e| e.kind().to_string())?;
    Ok(dir)
}

/// The JSON Schema of `template.toml`.
#[must_use]
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(TemplateToml)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs() {
        assert_eq!(parse_spec("demo"), Spec::Id("demo".into()));
        assert_eq!(parse_spec("./x"), Spec::Path("./x".into()));
        assert_eq!(
            parse_spec("gh:acme/tpl//sub/dir@v1"),
            Spec::Git {
                url: "https://github.com/acme/tpl.git".into(),
                subdir: Some("sub/dir".into()),
                rev: Some("v1".into())
            }
        );
        assert_eq!(
            parse_spec("gh:acme/tpl"),
            Spec::Git {
                url: "https://github.com/acme/tpl.git".into(),
                subdir: None,
                rev: None
            }
        );
    }

    #[test]
    fn when_grammar() {
        let mut a = Answers::new();
        a.insert("kind".into(), Answer::Text("web".into()));
        a.insert("on".into(), Answer::Flag(true));
        a.insert("tags".into(), Answer::List(vec!["a".into(), "b".into()]));
        assert!(when_holds("kind == web", &a).unwrap());
        assert!(when_holds("kind == \"web\"", &a).unwrap());
        assert!(!when_holds("kind != web", &a).unwrap());
        assert!(when_holds("on", &a).unwrap());
        assert!(!when_holds("!on", &a).unwrap());
        assert!(!when_holds("missing", &a).unwrap());
        assert!(when_holds("tags == b", &a).unwrap());
        assert!(when_holds("Bad Key", &a).is_err());
    }

    #[test]
    fn built_ins_load_and_coordinated_extends_demo() {
        let root = Path::new("/nonexistent-iohr-test");
        let d = load(&Spec::Id("demo".into()), root).unwrap();
        let c = load(&Spec::Id("coordinated".into()), root).unwrap();
        assert_eq!(c.files.len(), d.files.len() + 1);
        assert!(c.meta.agent_prompt.is_some());
        assert_eq!(c.steps().len(), 3);
    }

    fn demo() -> Template {
        load(
            &Spec::Id("demo".into()),
            Path::new("/nonexistent-iohr-test"),
        )
        .unwrap()
    }

    #[test]
    fn demo_has_four_live_languages_and_asks_for_one_first() {
        let d = demo();
        let ids: Vec<&str> = d.live_variants().iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["python", "typescript", "go", "rust"]);
        assert_eq!(d.default_variant().unwrap().id, "python");
        let q = d.all_questions();
        assert_eq!(q[0].key, LANGUAGE);
        assert_eq!(q[0].choices, ids);
    }

    #[test]
    fn a_variant_replaces_the_common_files_it_names_and_leaves_the_rest() {
        let d = demo();
        let go = d.with_variant("go").unwrap();
        let paths: Vec<&str> = go.files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"main.go"), "{paths:?}");
        assert!(paths.contains(&"extension.toml"), "{paths:?}");
        assert!(
            !paths
                .iter()
                .any(|p| Path::new(p).extension().is_some_and(|e| e == "py")),
            "{paths:?}"
        );
        assert_eq!(go.variant_files.len(), 0);
    }

    #[test]
    fn an_unknown_or_coming_language_is_refused_and_the_live_ones_are_named() {
        let d = demo();
        let e = d.with_variant("cobol").unwrap_err();
        assert!(e.contains("python, typescript, go, rust"), "{e}");
        let mut c = d.clone();
        c.meta.variants[2].status = Status::Coming;
        let e = c.with_variant("go").unwrap_err();
        assert!(e.contains("coming"), "{e}");
        assert!(!e.contains("go,"), "{e}");
        assert!(!c.all_questions()[0].choices.contains(&"go".to_owned()));
    }

    #[test]
    fn the_runner_has_its_values_filled_in() {
        let d = demo();
        let vals = all_values(
            &d,
            "my-shop",
            "acme",
            "2026-01-01",
            &Answers::from([(LANGUAGE.to_owned(), Answer::Text("go".into()))]),
        );
        let r = d.runner("go", &vals).unwrap();
        assert!(r.build.iter().any(|w| w.contains("my-shop")), "{r:?}");
        assert!(d.runner("cobol", &vals).is_none());
    }
}
