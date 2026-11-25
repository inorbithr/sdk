//! Managing a project's schemas tree: the manifest, and the three writes `iohr schemas`
//! makes (`new`, `bump`, `deprecate`). The same semantics as core's `devops/schemas/manage.py`
//! (RFC 0222), so a schema made here is one core accepts.
//!
//! Everything is planned in memory first ([`Plan`]), so `--dry-run` says what would change
//! and writes nothing; a plan is applied with the project writer
//! ([`crate::project::write_files`]) for new files, and the manifest is rewritten in place.
//! The layout check ([`crate::schema::check_tree`]) is the one that runs afterwards.
//!
//! `schemas/manifest.json` is optional. When it exists it lists every type, and the manifest
//! and the tree must agree: `{"base"?: url, "schemas": {"<area>/<name>": {visibility, status,
//! owner_account?, owner?, replaced_by?, note?}}}`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::schema::{MAX_FILE, Problem, families};

/// The manifest's file name, beside the `<area>` folders.
pub const FILE: &str = "manifest.json";

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";
const DRAFT_LINE: &str = " Draft line: it may still be corrected.";
/// The manifest rules, read from the embedded manifest schema found by its `$id`
/// ([`crate::schema::MANIFEST`]), by the paths core's `check.py` and `tbd-schemas-rules` use:
/// `/$defs/entry/properties/{visibility,status}/enum`, the keys under that `properties`, and the
/// `owner_account` pattern. Nothing about them is written here.
#[derive(Debug)]
pub struct Rules {
    pub visibility: Vec<String>,
    pub status: Vec<String>,
    pub keys: Vec<String>,
    account_id: regex::Regex,
}

impl Rules {
    fn read() -> Result<Self, String> {
        let schemas = crate::schema::Schemas::embedded()?;
        let doc = schemas
            .get(crate::schema::MANIFEST)
            .ok_or("the manifest schema is not embedded")?;
        let entry = doc
            .pointer("/$defs/entry/properties")
            .and_then(Value::as_object)
            .ok_or("the manifest schema has no entry properties")?;
        let list = |key: &str| -> Result<Vec<String>, String> {
            entry
                .get(key)
                .and_then(|p| p.get("enum"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .ok_or_else(|| format!("the manifest schema has no {key} enum"))
        };
        let pattern = entry
            .get("owner_account")
            .and_then(|p| p.get("pattern"))
            .and_then(Value::as_str)
            .ok_or("the manifest schema has no owner_account pattern")?;
        Ok(Self {
            visibility: list("visibility")?,
            status: list("status")?,
            keys: entry.keys().cloned().collect(),
            account_id: regex::Regex::new(pattern).map_err(|e| e.to_string())?,
        })
    }

    /// The rules of the embedded schema, read once.
    ///
    /// # Panics
    ///
    /// Never in a shipped build: a test reads the same file, so a schema that cannot give its
    /// rules fails there first.
    #[must_use]
    pub fn get() -> &'static Self {
        static RULES: OnceLock<Result<Rules, String>> = OnceLock::new();
        match RULES.get_or_init(Self::read) {
            Ok(r) => r,
            Err(e) => panic!("embedded manifest schema: {e}"),
        }
    }

    /// Whether `s` is an account id (the schema's pattern, which is anchored).
    #[must_use]
    pub fn is_account_id(&self, s: &str) -> bool {
        self.account_id.is_match(s)
    }
}

/// Who sees a type, from the manifest schema; the `--visibility` flag's values.
#[must_use]
pub fn visibility() -> Vec<&'static str> {
    Rules::get().visibility.iter().map(String::as_str).collect()
}
/// The `$id` base of a project that names none: its name goes after it.
pub const DEFAULT_BASE: &str = crate::schema::schemas_origin!();

/// One line of the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub visibility: String,
    pub owner_account: Option<String>,
    pub status: String,
    pub owner: Option<String>,
    pub replaced_by: Option<String>,
    pub note: Option<String>,
}

impl Entry {
    /// The keys in manage.py's order (a JSON map here is sorted, so the order is explicit).
    fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![("visibility", self.visibility.clone())];
        for (k, x) in [
            ("owner_account", &self.owner_account),
            ("status", &Some(self.status.clone())),
            ("owner", &self.owner),
            ("replaced_by", &self.replaced_by),
            ("note", &self.note),
        ] {
            if let Some(x) = x {
                v.push((k, x.clone()));
            }
        }
        v
    }
}

/// The manifest as read.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    /// Where `$id`s start for this project (ends with `/`); none means the default.
    pub base: Option<String>,
    /// Keys beside `base` and `schemas`, kept as they were.
    head: Map<String, Value>,
    pub schemas: BTreeMap<String, Entry>,
    /// Lines that did not parse, by type, with what is wrong.
    bad: BTreeMap<String, String>,
}

fn text_of(e: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match e.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} is not a string")),
    }
}

fn parse_entry(v: &Value) -> Result<Entry, String> {
    let Some(e) = v.as_object() else {
        return Err("is not an object".into());
    };
    let rules = Rules::get();
    if let Some(k) = e.keys().find(|k| !rules.keys.contains(k)) {
        return Err(format!("has an unknown key {k:?}"));
    }
    let visibility = text_of(e, "visibility")?.unwrap_or_default();
    if !rules.visibility.contains(&visibility) {
        return Err(format!(
            "visibility is one of {}",
            rules.visibility.join(", ")
        ));
    }
    let status = text_of(e, "status")?.unwrap_or_default();
    if !rules.status.contains(&status) {
        return Err(format!("status is one of {}", rules.status.join(", ")));
    }
    let owner_account = text_of(e, "owner_account")?;
    if owner_account
        .as_deref()
        .is_some_and(|a| !rules.is_account_id(a))
    {
        return Err("owner_account is an account id, a lowercase UUID".into());
    }
    Ok(Entry {
        visibility,
        owner_account,
        status,
        owner: text_of(e, "owner")?,
        replaced_by: text_of(e, "replaced_by")?,
        note: text_of(e, "note")?,
    })
}

impl Manifest {
    /// A manifest with no types.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The manifest in `root`, or none when there is no file.
    ///
    /// # Errors
    ///
    /// A message when the file is too big, not JSON, or has no `schemas` object. A line that is
    /// wrong is not an error here: [`Manifest::problems`] reports it.
    pub fn read(root: &Path) -> Result<Option<Self>, String> {
        let path = root.join(FILE);
        let Ok(meta) = std::fs::metadata(&path) else {
            return Ok(None);
        };
        if meta.len() > MAX_FILE {
            return Err(format!("over {MAX_FILE} bytes"));
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("cannot read: {}", e.kind()))?;
        let doc: Value = serde_json::from_str(&text)
            .map_err(|e| format!("not JSON (line {}, column {})", e.line(), e.column()))?;
        let Some(top) = doc.as_object() else {
            return Err("is not a JSON object".into());
        };
        let Some(Value::Object(lines)) = top.get("schemas") else {
            return Err("has no schemas object".into());
        };
        let mut m = Self::default();
        for (k, v) in top {
            match k.as_str() {
                "schemas" => {}
                "base" => match v.as_str() {
                    Some(b) if b.starts_with("https://") => {
                        m.base = Some(if b.ends_with('/') {
                            b.to_owned()
                        } else {
                            format!("{b}/")
                        });
                    }
                    _ => return Err("base is an https URL".into()),
                },
                _ => {
                    m.head.insert(k.clone(), v.clone());
                }
            }
        }
        for (fam, v) in lines {
            match parse_entry(v) {
                Ok(e) => {
                    m.schemas.insert(fam.clone(), e);
                }
                Err(why) => {
                    m.bad.insert(fam.clone(), why);
                }
            }
        }
        Ok(Some(m))
    }

    /// Every type the manifest lists, even one whose line is wrong.
    fn listed(&self) -> BTreeSet<&str> {
        self.schemas
            .keys()
            .chain(self.bad.keys())
            .map(String::as_str)
            .collect()
    }

    /// Where the manifest and the tree (`families`, every `<area>/<name>` with a version
    /// file) disagree, and every line that is not well formed.
    #[must_use]
    pub fn problems(&self, families: &BTreeSet<String>) -> Vec<Problem> {
        let say = |target: &str, message: String| Problem {
            target: target.to_owned(),
            message,
        };
        let mut out = Vec::new();
        for (fam, why) in &self.bad {
            out.push(say(fam, format!("{FILE}: {why}")));
        }
        let listed = self.listed();
        for fam in families.iter().filter(|f| !listed.contains(f.as_str())) {
            out.push(say(
                fam,
                format!("is not in {FILE}: list it (iohr schemas new does)"),
            ));
        }
        for fam in listed.iter().filter(|f| !families.contains(**f)) {
            out.push(say(
                fam,
                format!("is listed in {FILE} but has no v<major>.json"),
            ));
        }
        for (fam, e) in &self.schemas {
            if let Some(to) = &e.replaced_by
                && (e.status != "deprecated" || to == fam || !listed.contains(to.as_str()))
            {
                out.push(say(
                    fam,
                    format!(
                        "{FILE}: replaced_by names another listed schema and the schema is deprecated"
                    ),
                ));
            }
        }
        out
    }

    /// A manifest listing every type the tree already has, for a tree that had none. The
    /// status is inferred (a type with a v1 or later is stable, one with only v0 is a draft)
    /// and visibility is `account`, the safe one; both are the person's to correct.
    #[must_use]
    pub fn adopting(tree: &BTreeMap<String, Vec<u32>>) -> Self {
        let mut m = Self::default();
        for (fam, majors) in tree {
            let stable = majors.iter().any(|n| *n > 0);
            m.schemas.insert(
                fam.clone(),
                Entry {
                    visibility: "account".into(),
                    owner_account: None,
                    status: if stable { "stable" } else { "draft" }.into(),
                    owner: None,
                    replaced_by: None,
                    note: None,
                },
            );
        }
        m
    }

    /// The file's text: its own style, one line per type, so a change is one line.
    #[must_use]
    pub fn render(&self) -> String {
        let one = |v: &Value| serde_json::to_string(v).unwrap_or_else(|_| "null".into());
        let mut lines = vec!["{".to_owned()];
        if let Some(b) = &self.base {
            lines.push(format!("  \"base\": {},", one(&Value::from(b.clone()))));
        }
        for (k, v) in &self.head {
            lines.push(format!("  {}: {},", one(&Value::from(k.clone())), one(v)));
        }
        if self.schemas.is_empty() {
            lines.push("  \"schemas\": {}".into());
        } else {
            lines.push("  \"schemas\": {".into());
            let n = self.schemas.len();
            for (i, (fam, e)) in self.schemas.iter().enumerate() {
                let body: Vec<String> = e
                    .pairs()
                    .into_iter()
                    .map(|(k, v)| format!("\"{k}\": {}", one(&Value::from(v))))
                    .collect();
                lines.push(format!(
                    "    {}: {{ {} }}{}",
                    one(&Value::from(fam.clone())),
                    body.join(", "),
                    if i + 1 < n { "," } else { "" }
                ));
            }
            lines.push("  }".into());
        }
        lines.push("}".into());
        lines.join("\n") + "\n"
    }
}

/// The `$id` base for a project whose schemas folder is `root`: the manifest's `base`, else
/// `https://schemas.inorbit.hr/<project>/`, the project being named by the `inorbit.toml`
/// beside the folder (the one `iohr project init` writes), else by that folder's name.
#[must_use]
pub fn base_for(root: &Path, manifest: Option<&Manifest>) -> String {
    if let Some(b) = manifest.and_then(|m| m.base.as_deref()) {
        return b.to_owned();
    }
    let abs = std::fs::canonicalize(root)
        .or_else(|_| std::path::absolute(root))
        .unwrap_or_else(|_| root.to_path_buf());
    let project = abs.parent();
    let named = project
        .and_then(|p| std::fs::read_to_string(p.join("inorbit.toml")).ok())
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| {
            t.get("name")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            project
                .and_then(Path::file_name)
                .map(|f| f.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let slug: String = named
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    format!(
        "{DEFAULT_BASE}{}/",
        if slug.is_empty() { "project" } else { &slug }
    )
}

/// Whether `s` is `<area>/<name>`: two or more lowercase segments.
#[must_use]
pub fn is_path(s: &str) -> bool {
    let mut n = 0;
    for seg in s.split('/') {
        n += 1;
        let mut b = seg.bytes();
        let ok = b
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
        if !ok {
            return false;
        }
    }
    n >= 2
}

/// A type named on the command line: `<area>/<name>` or `<area>/<name>@v<major>`.
///
/// # Errors
///
/// A message when it is neither.
pub fn parse_spec(spec: &str) -> Result<(String, Option<u32>), String> {
    let (fam, major) = match spec.split_once('@') {
        None => (spec, None),
        Some((f, v)) => {
            let n = v
                .strip_prefix('v')
                .and_then(|n| n.parse::<u32>().ok())
                .ok_or_else(|| format!("{spec}: the version is @v<major>, such as @v1"))?;
            (f, Some(n))
        }
    };
    if !is_path(fam) {
        return Err(format!(
            "{fam}: a schema path is two or more lowercase segments, <area>/<name>"
        ));
    }
    Ok((fam.to_owned(), major))
}

/// What a plan does to a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Write,
    Update,
}

impl Action {
    /// The word a line starts with.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Update => "update",
        }
    }
}

/// One file of a plan, by its path under the schemas folder.
#[derive(Debug, Clone)]
pub struct Change {
    pub action: Action,
    pub rel: String,
    pub text: String,
}

/// What a management command would do: the files, and notes for the person.
#[derive(Debug, Default)]
pub struct Plan {
    pub changes: Vec<Change>,
    pub notes: Vec<String>,
}

impl Plan {
    fn write(&mut self, rel: String, text: String) {
        self.changes.push(Change {
            action: Action::Write,
            rel,
            text,
        });
    }

    /// The manifest, new or changed.
    fn manifest(&mut self, root: &Path, m: &Manifest) {
        let action = if root.join(FILE).exists() {
            Action::Update
        } else {
            Action::Write
        };
        self.changes.push(Change {
            action,
            rel: FILE.to_owned(),
            text: m.render(),
        });
    }

    /// Writes the plan under `root`. New files go through the project writer, which never
    /// overwrites, and are written first; the manifest is replaced last, through a temp file
    /// and a rename, so a failed write never leaves it listing a type with no files.
    ///
    /// # Errors
    ///
    /// A message when a write failed, or when a file a plan makes is there already; in
    /// both cases the manifest is unchanged.
    pub fn apply(&self, root: &Path) -> Result<(), String> {
        let files: Vec<(String, String)> = self
            .changes
            .iter()
            .filter(|c| matches!(c.action, Action::Write))
            .map(|c| (c.rel.clone(), c.text.clone()))
            .collect();
        // `symlink_metadata`, not `exists`: a dangling link counts as there.
        if let Some((rel, _)) = files
            .iter()
            .find(|(rel, _)| root.join(rel).symlink_metadata().is_ok())
        {
            return Err(format!("{rel} is there already; nothing is overwritten"));
        }
        let done = crate::project::write_files(root, &files, &[])?;
        if let Some(kept) = done.same.iter().chain(&done.kept).next() {
            return Err(format!("{kept} is there already; nothing is overwritten"));
        }
        for c in self
            .changes
            .iter()
            .filter(|c| matches!(c.action, Action::Update))
        {
            replace_file(&root.join(&c.rel), &c.text)
                .map_err(|e| format!("cannot write {}: {}", c.rel, e.kind()))?;
        }
        Ok(())
    }
}

/// Replaces `path` with `text` atomically: a temp file beside it, then a rename.
fn replace_file(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Replaces the value of the first `"key": old` member in a JSON text, keeping the rest of
/// the text as written. `None` when no such member is there.
fn replace_member(text: &str, key: &str, old: &str, new: &str) -> Option<String> {
    let one = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let (k, old_v) = (one(key), one(old));
    let mut from = 0;
    while let Some(i) = text[from..].find(&k) {
        let after = from + i + k.len();
        let rest = text[after..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let value = rest.trim_start();
            if value.starts_with(&old_v) {
                let at = text.len() - value.len();
                return Some(format!(
                    "{}{}{}",
                    &text[..at],
                    one(new),
                    &text[at + old_v.len()..]
                ));
            }
        }
        from = after;
    }
    None
}

fn dumps(v: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_default())
}

/// What `schemas new` takes besides the path.
#[derive(Debug, Default)]
pub struct NewArgs {
    pub title: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub visibility: String,
}

/// Manifest to work from: the one in the folder, or one adopting the types the tree has.
fn working_manifest(root: &Path, plan: &mut Plan) -> Result<Manifest, String> {
    if let Some(m) = Manifest::read(root).map_err(|e| format!("{FILE}: {e}"))? {
        return Ok(m);
    }
    let tree = families(root);
    let m = Manifest::adopting(&tree);
    if !tree.is_empty() {
        plan.notes.push(format!(
            "{FILE} did not exist: it now lists the {} types already here, status inferred (v0 only is a draft, a later major is stable) and visibility account; correct them if they differ",
            tree.len()
        ));
    }
    Ok(m)
}

/// A new schema as a v0 draft: the schema, one valid and one refused example, and a manifest
/// line.
///
/// # Errors
///
/// A message when the path is not `<area>/<name>`, the type exists, or the manifest cannot be read.
pub fn plan_new(root: &Path, fam: &str, a: &NewArgs) -> Result<Plan, String> {
    if !is_path(fam) {
        return Err(format!(
            "{fam}: a schema path is two or more lowercase segments, <area>/<name>"
        ));
    }
    let rules = Rules::get();
    if !rules.visibility.contains(&a.visibility) {
        return Err(format!(
            "visibility is one of {}",
            rules.visibility.join(", ")
        ));
    }
    let mut plan = Plan::default();
    let mut m = working_manifest(root, &mut plan)?;
    if m.schemas.contains_key(fam) || m.bad.contains_key(fam) || root.join(fam).exists() {
        return Err(format!(
            "{fam} exists already; bump it, or pick another name"
        ));
    }
    let title = a.title.clone().unwrap_or_else(|| {
        let last = fam.rsplit('/').next().unwrap_or(fam).replace('-', " ");
        let mut c = last.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    });
    let description = a.description.clone().unwrap_or_else(|| {
        format!("{title}: what the fields mean, their types and limits.{DRAFT_LINE}")
    });
    let base = base_for(root, Some(&m));
    let one = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let schema = format!(
        "{{\n  \"$schema\": {},\n  \"$id\": {},\n  \"title\": {},\n  \"description\": {},\n  \"type\": \"object\",\n  \"additionalProperties\": false,\n  \"required\": [],\n  \"properties\": {{}}\n}}\n",
        one(DRAFT),
        one(&format!("{base}{fam}/v0.json")),
        one(&title),
        one(&description),
    );
    plan.write(format!("{fam}/v0.json"), schema);
    plan.write(format!("{fam}/examples/minimal.json"), "{}\n".to_owned());
    plan.write(
        format!("{fam}/invalid/unknown-field.json"),
        dumps(&serde_json::json!({ "unknown": true })),
    );
    m.schemas.insert(
        fam.to_owned(),
        Entry {
            visibility: a.visibility.clone(),
            owner_account: None,
            status: "draft".into(),
            owner: a.owner.clone().filter(|o| !o.is_empty()),
            replaced_by: None,
            note: None,
        },
    );
    plan.manifest(root, &m);
    plan.notes.push(format!(
        "next: add fields to {fam}/v0.json, an example under {fam}/examples/ and a refused case under {fam}/invalid/"
    ));
    Ok(plan)
}

/// The next major of a type: its newest major copied with the `$id` corrected. A draft
/// becomes stable (a stable major is never edited again; the next change is the next major).
///
/// # Errors
///
/// A message when the type is not there, is deprecated, or its newest file cannot be read.
pub fn plan_bump(root: &Path, fam: &str) -> Result<Plan, String> {
    if !is_path(fam) {
        return Err(format!(
            "{fam}: a schema path is two or more lowercase segments, <area>/<name>"
        ));
    }
    let tree = families(root);
    let Some(majors) = tree.get(fam) else {
        return Err(format!("{fam} has no v<major>.json"));
    };
    let mut plan = Plan::default();
    let mut m = working_manifest(root, &mut plan)?;
    let Some(entry) = m.schemas.get_mut(fam) else {
        return Err(format!(
            "{fam} is not in {FILE}, or its line there is wrong"
        ));
    };
    if entry.status == "deprecated" {
        return Err(format!(
            "{fam} is deprecated; a deprecated schema gets no new version"
        ));
    }
    let cur = majors.last().copied().unwrap_or(0);
    let next = cur + 1;
    let from = format!("{fam}/v{cur}.json");
    let text = std::fs::read_to_string(root.join(&from))
        .map_err(|e| format!("{from}: cannot read: {}", e.kind()))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "{from}: not JSON (line {}, column {})",
            e.line(),
            e.column()
        )
    })?;
    let old_id = doc.get("$id").and_then(Value::as_str).map(str::to_owned);
    let new_id = match &old_id {
        Some(id) => match id.strip_suffix(&format!("/v{cur}.json")) {
            Some(head) => format!("{head}/v{next}.json"),
            None => return Err(format!("{from}: its $id does not end with /v{cur}.json")),
        },
        None => {
            return Err(format!("{from} has no $id; run `iohr schemas check` first"));
        }
    };
    // Rewritten in the text, so the file keeps the author's key order and layout.
    // Only the `$id` member is rewritten, never the same string elsewhere in the file.
    let mut copy = text.clone();
    if let Some(old) = &old_id {
        copy = replace_member(&copy, "$id", old, &new_id)
            .ok_or_else(|| format!("{from}: its $id is written in a way iohr cannot rewrite"))?;
    }
    if cur == 0
        && let Some(d) = doc.get("description").and_then(Value::as_str)
    {
        let cleaned = d.replace(DRAFT_LINE, "").trim().to_owned();
        if cleaned != d
            && let Some(c) = replace_member(&copy, "description", d, &cleaned)
        {
            copy = c;
        }
    }
    plan.write(format!("{fam}/v{next}.json"), copy);
    if entry.status == "draft" {
        "stable".clone_into(&mut entry.status);
        plan.notes.push(format!(
            "{fam} is stable in {FILE} now: v{next} never changes once published"
        ));
    }
    plan.manifest(root, &m);
    plan.notes.push(format!(
        "next: edit {fam}/v{next}.json, add an example under {fam}/examples/ and run iohr schemas check; v{cur} stays as it is"
    ));
    Ok(plan)
}

/// Marks a type deprecated, with the type that replaces it when one is named. The files stay.
///
/// # Errors
///
/// A message when the type, or the one named as its replacement, is not listed.
pub fn plan_deprecate(root: &Path, fam: &str, replaced_by: Option<&str>) -> Result<Plan, String> {
    if !is_path(fam) {
        return Err(format!(
            "{fam}: a schema path is two or more lowercase segments, <area>/<name>"
        ));
    }
    let mut plan = Plan::default();
    let mut m = working_manifest(root, &mut plan)?;
    if let Some(to) = replaced_by {
        if to == fam {
            return Err("a schema cannot replace itself".into());
        }
        if !m.schemas.contains_key(to) {
            return Err(format!("{to} is not in {FILE}; add it first"));
        }
    }
    let Some(entry) = m.schemas.get_mut(fam) else {
        return Err(format!(
            "{fam} is not in {FILE}, or its line there is wrong"
        ));
    };
    if let Some(to) = replaced_by {
        entry.replaced_by = Some(to.to_owned());
    }
    "deprecated".clone_into(&mut entry.status);
    plan.manifest(root, &m);
    plan.notes.push(format!(
        "{fam} is deprecated{}; its files stay at their addresses so existing documents keep validating",
        replaced_by.map_or_else(String::new, |t| format!(", replaced by {t}"))
    ));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schemas;

    #[test]
    fn the_rules_come_from_the_manifest_schema_by_its_id() {
        let r = Rules::get();
        let sorted = |v: &[String]| {
            let mut v = v.to_vec();
            v.sort();
            v
        };
        assert_eq!(sorted(&r.visibility), ["account", "public", "shared"]);
        assert_eq!(sorted(&r.status), ["deprecated", "draft", "stable"]);
        assert_eq!(
            sorted(&r.keys),
            [
                "note",
                "owner",
                "owner_account",
                "replaced_by",
                "status",
                "visibility"
            ]
        );
        assert!(r.is_account_id("0a1b2c3d-0000-4000-8000-0123456789ab"));
        for bad in [
            "",
            "0A1B2C3D-0000-4000-8000-0123456789ab",
            "not-an-id",
            "0a1b2c3d-0000-4000-8000-0123456789abx",
        ] {
            assert!(!r.is_account_id(bad), "{bad}");
        }
        // The words this file writes for a new or bumped type are words the schema allows.
        for w in ["draft", "stable"] {
            assert!(r.status.iter().any(|s| s == w), "{w}");
        }
        assert!(r.visibility.iter().any(|v| v == "account"));
    }

    #[test]
    fn the_vendored_manifest_schema_is_its_published_id() {
        // Offline: the copy is core's file; `schemas:check` in core proves the published bytes
        // equal that source. Here the copy must name the `$id` it is looked up by.
        let doc = Schemas::embedded().unwrap();
        let doc = doc.get(crate::schema::MANIFEST).unwrap();
        assert_eq!(
            doc["$id"],
            "https://schemas.inorbit.hr/schemas/manifest/v0.json"
        );
    }

    fn fams(l: &[&str]) -> BTreeSet<String> {
        l.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_path_is_two_or_more_lowercase_segments() {
        assert!(is_path("scrum/card"));
        assert!(is_path("a/b-c/d1"));
        for bad in ["card", "Scrum/card", "a//b", "a/-b", "a/b/", "a b/c"] {
            assert!(!is_path(bad), "{bad}");
        }
        assert_eq!(parse_spec("a/b@v2").unwrap(), ("a/b".to_owned(), Some(2)));
        assert!(parse_spec("a/b@2").is_err());
    }

    #[test]
    fn the_manifest_round_trips_in_its_own_style() {
        let text = r#"{ "base": "https://s.test/p/", "schemas": {
          "a/b": { "visibility": "account", "status": "draft", "owner": "team" },
          "a/c": { "visibility": "public", "status": "deprecated", "replaced_by": "a/b" } } }"#;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), text).unwrap();
        let m = Manifest::read(dir.path()).unwrap().unwrap();
        assert_eq!(m.base.as_deref(), Some("https://s.test/p/"));
        assert_eq!(m.problems(&fams(&["a/b", "a/c"])), Vec::new());
        let out = m.render();
        std::fs::write(dir.path().join(FILE), &out).unwrap();
        let again = Manifest::read(dir.path()).unwrap().unwrap();
        assert_eq!(again.render(), out);
        assert!(out.contains("    \"a/b\": { \"visibility\": \"account\", \"status\": \"draft\", \"owner\": \"team\" },\n"), "{out}");
    }

    #[test]
    fn the_manifest_and_the_tree_must_agree() {
        let text = r#"{ "schemas": { "a/b": { "visibility": "x", "status": "draft" },
          "a/gone": { "visibility": "account", "status": "draft" },
          "a/d": { "visibility": "account", "status": "draft", "replaced_by": "a/d" } } }"#;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), text).unwrap();
        let m = Manifest::read(dir.path()).unwrap().unwrap();
        let p: Vec<String> = m
            .problems(&fams(&["a/b", "a/d", "a/new"]))
            .iter()
            .map(|p| format!("{}: {}", p.target, p.message))
            .collect();
        assert!(
            p.iter()
                .any(|l| l.starts_with("a/b: manifest.json: visibility is one of")),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|l| l.starts_with("a/gone: is listed in manifest.json but has no")),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|l| l.starts_with("a/new: is not in manifest.json")),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|l| l.starts_with("a/d: manifest.json: replaced_by")),
            "{p:?}"
        );
    }

    #[test]
    fn the_base_is_the_manifests_else_the_project_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("My Project").join("schemas");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            base_for(&root, None),
            "https://schemas.inorbit.hr/my-project/"
        );
        std::fs::write(
            dir.path().join("My Project").join("inorbit.toml"),
            "name = \"eye\"\n",
        )
        .unwrap();
        assert_eq!(base_for(&root, None), "https://schemas.inorbit.hr/eye/");
        let m = Manifest {
            base: Some("https://x.test/y/".into()),
            ..Manifest::default()
        };
        assert_eq!(base_for(&root, Some(&m)), "https://x.test/y/");
    }

    #[test]
    fn adopting_infers_the_status_from_the_majors() {
        let tree = BTreeMap::from([("a/b".to_owned(), vec![0]), ("a/c".to_owned(), vec![0, 1])]);
        let m = Manifest::adopting(&tree);
        assert_eq!(m.schemas["a/b"].status, "draft");
        assert_eq!(m.schemas["a/c"].status, "stable");
    }
}
