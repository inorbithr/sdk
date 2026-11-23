//! One JSON Schema validator for the CLI (RFC 0234, slice 1).
//!
//! Schemas are looked up by `$id`, never by a relative path. The ones the CLI ships
//! (`scrum/card`, `scrum/board`, `monitors/monitor`) are embedded; a project's own tree is read
//! from a folder with the layout of RFC 0147: `<area>/<name>/v<major>.json`, an `examples/`
//! folder whose files must validate and an `invalid/` folder whose files must not.
//!
//! A `$ref` is served from memory, from the embedded set and the tree being checked. Nothing is
//! ever fetched, so validating touches neither the network nor any other file.
//!
//! A violation names the field (a JSON pointer) and the rule it broke, never the value, so it
//! is safe to show and to log.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jsonschema::{Draft, Retrieve, Uri, Validator};
use serde::Serialize;
use serde_json::Value;

/// Where the CLI's own schemas are published.
pub const BASE: &str = "https://schemas.inorbit.hr/iohr/";
pub const SCRUM_CARD: &str = "https://schemas.inorbit.hr/iohr/scrum/card/v0.json";
pub const SCRUM_BOARD: &str = "https://schemas.inorbit.hr/iohr/scrum/board/v0.json";
pub const MONITOR: &str = "https://schemas.inorbit.hr/iohr/monitors/monitor/v0.json";

/// The schemas inside the binary, by `$id`.
const EMBEDDED: [(&str, &str); 3] = [
    (SCRUM_CARD, include_str!("../schemas/scrum/card/v0.json")),
    (SCRUM_BOARD, include_str!("../schemas/scrum/board/v0.json")),
    (MONITOR, include_str!("../schemas/monitors/monitor/v0.json")),
];

/// The largest schema, example or invalid case that is read.
const MAX_FILE: u64 = 1024 * 1024;
/// How deep a tree is walked; a deeper one is not a layout this check knows.
const MAX_DEPTH: usize = 6;
/// How many violations one validation reports.
const MAX_VIOLATIONS: usize = 8;

/// One broken rule: where in the document, and which rule of the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// The JSON pointer of the field (`/checklist/0/text`); `/` for the whole document.
    pub path: String,
    /// The schema's own pointer to the rule that was broken (`/properties/due/format`).
    pub rule: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "field {} breaks {}", self.path, self.rule)
    }
}

/// Why a validation did not give a yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidateError {
    /// No loaded schema has this `$id`.
    Unknown(String),
    /// The schema does not compile.
    Broken(String),
    /// The document breaks the schema; at most [`MAX_VIOLATIONS`] are listed.
    Invalid(Vec<Violation>),
}

impl fmt::Display for ValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "no schema with $id {id}"),
            Self::Broken(why) => f.write_str(why),
            Self::Invalid(all) => {
                let lines: Vec<String> = all.iter().map(ToString::to_string).collect();
                f.write_str(&lines.join("; "))
            }
        }
    }
}

impl std::error::Error for ValidateError {}

/// The loaded schemas, by `$id`.
#[derive(Debug)]
pub struct Schemas {
    by_id: Arc<BTreeMap<String, Value>>,
    compiled: Mutex<BTreeMap<String, Arc<Validator>>>,
}

/// Serves `$ref`s from the loaded set and refuses every other address.
struct Memory(Arc<BTreeMap<String, Value>>);

impl Retrieve for Memory {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let whole = uri.as_str();
        let doc = whole.split('#').next().unwrap_or(whole);
        self.0
            .get(doc)
            .cloned()
            .ok_or_else(|| "iohr resolves schemas only from the ones it has loaded".into())
    }
}

impl Schemas {
    /// The schemas embedded in the binary.
    ///
    /// # Errors
    ///
    /// A message when an embedded file is not JSON or its `$id` is not the one it is
    /// listed under; a test keeps that from shipping.
    pub fn embedded() -> Result<Self, String> {
        let mut by_id = BTreeMap::new();
        for (id, text) in EMBEDDED {
            let value: Value =
                serde_json::from_str(text).map_err(|e| format!("embedded {id}: {e}"))?;
            if value.get("$id").and_then(Value::as_str) != Some(id) {
                return Err(format!("embedded {id}: its $id is not that address"));
            }
            by_id.insert(id.to_owned(), value);
        }
        Ok(Self::of(by_id))
    }

    fn of(by_id: BTreeMap<String, Value>) -> Self {
        Self {
            by_id: Arc::new(by_id),
            compiled: Mutex::default(),
        }
    }

    /// These schemas plus more, each by its `$id`; a schema with the same `$id` replaces.
    #[must_use]
    pub fn with(&self, more: impl IntoIterator<Item = (String, Value)>) -> Self {
        let mut by_id = (*self.by_id).clone();
        by_id.extend(more);
        Self::of(by_id)
    }

    /// The `$id`s loaded, sorted.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.by_id.keys().map(String::as_str)
    }

    /// The schema with this `$id`.
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.by_id.get(id)
    }

    fn build(&self, schema: &Value) -> Result<Validator, ValidateError> {
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .with_retriever(Memory(self.by_id.clone()))
            .build(schema)
            .map_err(|e| {
                let at = e.instance_path().to_string();
                let at = if at.is_empty() { "/".to_owned() } else { at };
                ValidateError::Broken(format!(
                    "the schema does not compile at {at}, rule {}",
                    e.schema_path()
                ))
            })
    }

    /// The compiled validator for the schema with this `$id`, compiled once and kept.
    fn validator(&self, id: &str) -> Result<Arc<Validator>, ValidateError> {
        if let Some(v) = self.compiled.lock().ok().and_then(|c| c.get(id).cloned()) {
            return Ok(v);
        }
        let schema = self
            .by_id
            .get(id)
            .ok_or_else(|| ValidateError::Unknown(id.to_owned()))?;
        let v = Arc::new(self.build(schema)?);
        if let Ok(mut c) = self.compiled.lock() {
            c.insert(id.to_owned(), v.clone());
        }
        Ok(v)
    }

    /// Checks `value` against the schema with this `$id`.
    ///
    /// # Errors
    ///
    /// [`ValidateError::Unknown`] for an `$id` nobody loaded, [`ValidateError::Broken`] for a
    /// schema that does not compile, [`ValidateError::Invalid`] with the broken fields.
    pub fn validate(&self, id: &str, value: &Value) -> Result<(), ValidateError> {
        violations(self.validator(id)?.as_ref(), value).map_err(ValidateError::Invalid)
    }
}

fn violations(v: &Validator, value: &Value) -> Result<(), Vec<Violation>> {
    let found: Vec<Violation> = v
        .iter_errors(value)
        .take(MAX_VIOLATIONS)
        .map(|e| {
            let path = e.instance_path().to_string();
            Violation {
                path: if path.is_empty() {
                    "/".to_owned()
                } else {
                    path
                },
                rule: e.schema_path().to_string(),
            }
        })
        .collect();
    if found.is_empty() { Ok(()) } else { Err(found) }
}

/// One thing the layout check found wrong: what it is about, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// A type (`scrum/card/v0`) or a file under it (`scrum/card/invalid/bad.json`).
    pub target: String,
    pub message: String,
}

/// What [`check_tree`] found.
#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// Schemas found (`<area>/<name>/v<major>.json`).
    pub types: usize,
    pub examples: usize,
    pub invalid: usize,
    pub problems: Vec<Problem>,
}

impl Report {
    fn fail(&mut self, target: impl Into<String>, message: impl Into<String>) {
        self.problems.push(Problem {
            target: target.into(),
            message: message.into(),
        });
    }
}

/// A schema file found in a tree.
struct Found {
    /// `scrum/card/v0`.
    name: String,
    id: String,
    dir: PathBuf,
}

fn read_json(path: &Path) -> Result<Value, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read: {}", e.kind()))?;
    if meta.len() > MAX_FILE {
        return Err(format!("over {MAX_FILE} bytes"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read: {}", e.kind()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("not JSON (line {}, column {})", e.line(), e.column()))
}

/// The `.json` files directly in `dir`, sorted; none when it is missing.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `v0.json`, `v12.json`: a version file.
fn is_version_file(name: &str) -> bool {
    name.strip_suffix(".json")
        .and_then(|s| s.strip_prefix('v'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Every `v<n>.json` under `dir`, skipping `examples/`, `invalid/`, hidden folders and
/// symbolic links (a link could point out of the tree being checked).
fn version_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(PathBuf, std::fs::FileType)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| Some((e.path(), e.file_type().ok()?)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (p, kind) in entries {
        let name = file_name(&p);
        if kind.is_dir() {
            if depth < MAX_DEPTH
                && !matches!(name.as_str(), "examples" | "invalid")
                && !name.starts_with('.')
            {
                version_files(&p, depth + 1, out);
            }
        } else if kind.is_file() && is_version_file(&name) {
            out.push(p);
        }
    }
}

/// Checks a project's schemas tree.
///
/// Rules, one problem per breach: a schema sits at `<area>/<name>/v<major>.json`; it parses; it
/// has a string `$id` that ends with that path and that no other schema in the tree has; it
/// compiles; it has examples and invalid cases; every example validates; every invalid case is
/// rejected. A `$ref` to the CLI's own schemas resolves; any other address is refused.
///
/// # Errors
///
/// A message when `root` is not a folder.
pub fn check_tree(root: &Path) -> Result<Report, String> {
    if !root.is_dir() {
        return Err(format!("{}: no such folder", root.display()));
    }
    let mut paths = Vec::new();
    version_files(root, 0, &mut paths);
    let mut report = Report::default();
    let mut found: Vec<Found> = Vec::new();
    let mut loaded: BTreeMap<String, Value> = BTreeMap::new();
    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    for p in paths {
        let rel = p.strip_prefix(root).unwrap_or(&p).with_extension("");
        let name = rel.to_string_lossy().replace('\\', "/");
        report.types += 1;
        let depth = rel.components().count();
        if depth < 3 {
            report.fail(&name, "a schema sits at <area>/<name>/v<major>.json");
            continue;
        }
        let value = match read_json(&p) {
            Ok(v) => v,
            Err(why) => {
                report.fail(&name, why);
                continue;
            }
        };
        let Some(id) = value.get("$id").and_then(Value::as_str).map(str::to_owned) else {
            report.fail(&name, "has no $id");
            continue;
        };
        if !id.ends_with(&format!("/{name}.json")) {
            report.fail(&name, format!("$id {id} does not end with /{name}.json"));
            continue;
        }
        if let Some(other) = owner.insert(id.clone(), name.clone()) {
            report.fail(&name, format!("$id is also the $id of {other}"));
            continue;
        }
        loaded.insert(id.clone(), value);
        found.push(Found {
            name,
            id,
            dir: p.parent().unwrap_or(root).to_owned(),
        });
    }
    let embedded = Schemas::embedded()?;
    let schemas = embedded.with(loaded.clone());
    for f in found {
        check_type(&schemas, &f, &mut report);
    }
    if report.types == 0 {
        // A mistyped folder must not look green.
        report.fail(".", "no schema found (<area>/<name>/v<major>.json)");
    }
    report.problems.sort_by(|a, b| a.target.cmp(&b.target));
    Ok(report)
}

fn check_type(schemas: &Schemas, f: &Found, r: &mut Report) {
    let v = match schemas.validator(&f.id) {
        Ok(v) => v,
        Err(why) => {
            r.fail(&f.name, why.to_string());
            return;
        }
    };
    let examples = json_files(&f.dir.join("examples"));
    let invalid = json_files(&f.dir.join("invalid"));
    if examples.is_empty() {
        r.fail(&f.name, "has no examples");
    }
    if invalid.is_empty() {
        r.fail(&f.name, "has no invalid cases");
    }
    let base = f.name.rsplit_once('/').map_or("", |(dir, _)| dir);
    for p in &examples {
        let target = format!("{base}/examples/{}", file_name(p));
        match read_json(p) {
            Ok(x) => {
                if let Err(found) = violations(&v, &x) {
                    let all: Vec<String> = found.iter().map(ToString::to_string).collect();
                    r.fail(target, format!("example is rejected: {}", all.join("; ")));
                }
            }
            Err(why) => r.fail(target, why),
        }
    }
    for p in &invalid {
        let target = format!("{base}/invalid/{}", file_name(p));
        match read_json(p) {
            Ok(x) => {
                if v.is_valid(&x) {
                    r.fail(target, "invalid case is accepted by the schema");
                }
            }
            Err(why) => r.fail(target, why),
        }
    }
    r.examples += examples.len();
    r.invalid += invalid.len();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A tree with one type, `acme/thing`, that passes.
    fn good_tree() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        put(
            t.path(),
            "acme/thing/v0.json",
            r#"{ "$schema": "https://json-schema.org/draft/2020-12/schema",
                 "$id": "https://schemas.example.test/acme/thing/v0.json",
                 "type": "object", "required": ["n"], "additionalProperties": false,
                 "properties": { "n": { "type": "integer" } } }"#,
        );
        put(t.path(), "acme/thing/examples/one.json", r#"{ "n": 1 }"#);
        put(
            t.path(),
            "acme/thing/invalid/text.json",
            r#"{ "n": "one" }"#,
        );
        t
    }

    fn problems(root: &Path) -> Vec<String> {
        check_tree(root)
            .unwrap()
            .problems
            .iter()
            .map(|p| format!("{}: {}", p.target, p.message))
            .collect()
    }

    #[test]
    fn a_schema_is_found_by_its_id() {
        let s = Schemas::embedded().unwrap();
        assert_eq!(s.ids().count(), 3);
        for id in [SCRUM_CARD, SCRUM_BOARD, MONITOR] {
            let doc = s.get(id).unwrap();
            assert_eq!(doc["$id"], id);
        }
        assert!(s.get("scrum/card/v0.json").is_none(), "a path is not an id");
        assert_eq!(
            s.validate("https://schemas.inorbit.hr/iohr/nope/v0.json", &json!({})),
            Err(ValidateError::Unknown(
                "https://schemas.inorbit.hr/iohr/nope/v0.json".into()
            ))
        );
    }

    #[test]
    fn an_error_names_the_field_and_never_the_value() {
        let s = Schemas::embedded().unwrap();
        let mut card: Value =
            serde_json::from_str(include_str!("../schemas/scrum/card/examples/smallest.json"))
                .unwrap();
        card["id"] = json!("a-secret-value-id");
        card["list"] = json!("nowhere");
        let Err(ValidateError::Invalid(found)) = s.validate(SCRUM_CARD, &card) else {
            panic!("the card should be rejected");
        };
        assert!(found.iter().any(|v| v.path == "/list"), "{found:?}");
        let shown = ValidateError::Invalid(found).to_string();
        assert!(
            !shown.contains("nowhere") && !shown.contains("a-secret-value"),
            "{shown}"
        );
    }

    #[test]
    fn a_ref_outside_the_loaded_set_is_refused() {
        let s = Schemas::embedded().unwrap().with([(
            "https://x.test/a/b/v0.json".to_owned(),
            json!({ "$id": "https://x.test/a/b/v0.json", "$ref": "https://example.com/other.json" }),
        )]);
        assert!(matches!(
            s.validate("https://x.test/a/b/v0.json", &json!(1)),
            Err(ValidateError::Broken(_))
        ));
    }

    #[test]
    fn a_ref_to_an_embedded_schema_resolves() {
        let s = Schemas::embedded().unwrap().with([(
            "https://x.test/a/b/v0.json".to_owned(),
            json!({ "$id": "https://x.test/a/b/v0.json", "$ref": SCRUM_CARD }),
        )]);
        assert!(matches!(
            s.validate("https://x.test/a/b/v0.json", &json!({})),
            Err(ValidateError::Invalid(_))
        ));
    }

    #[test]
    fn the_embedded_schemas_pass_their_own_layout_check() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas");
        let r = check_tree(&root).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(r.types, 3);
        assert!(r.examples >= 7 && r.invalid >= 16, "{r:?}");
    }

    #[test]
    fn a_good_tree_passes() {
        let t = good_tree();
        let r = check_tree(t.path()).unwrap();
        assert_eq!((r.types, r.examples, r.invalid), (1, 1, 1));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
    }

    #[test]
    fn a_folder_with_no_schema_is_a_problem() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "notes/readme.json", "{}");
        assert_eq!(
            problems(t.path()),
            [".: no schema found (<area>/<name>/v<major>.json)"]
        );
    }

    #[test]
    fn a_missing_id_is_caught() {
        let t = good_tree();
        put(t.path(), "acme/other/v0.json", r#"{ "type": "object" }"#);
        assert_eq!(problems(t.path()), ["acme/other/v0: has no $id"]);
    }

    #[test]
    fn an_id_that_is_not_the_path_or_is_repeated_is_caught() {
        let t = good_tree();
        put(
            t.path(),
            "acme/copy/v0.json",
            r#"{ "$id": "https://schemas.example.test/acme/thing/v0.json" }"#,
        );
        let p = problems(t.path());
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].starts_with("acme/copy/v0: $id https://schemas.example.test/acme/thing/v0.json does not end with"), "{p:?}");
    }

    #[test]
    fn an_example_that_fails_is_caught_with_its_field() {
        let t = good_tree();
        put(
            t.path(),
            "acme/thing/examples/bad.json",
            r#"{ "n": "text" }"#,
        );
        let p = problems(t.path());
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].starts_with("acme/thing/examples/bad.json: example is rejected: field /n breaks"),
            "{p:?}"
        );
        assert!(!p[0].contains("text\""), "no value in the line: {p:?}");
    }

    #[test]
    fn an_invalid_case_that_passes_is_caught() {
        let t = good_tree();
        put(t.path(), "acme/thing/invalid/fine.json", r#"{ "n": 2 }"#);
        assert_eq!(
            problems(t.path()),
            ["acme/thing/invalid/fine.json: invalid case is accepted by the schema"]
        );
    }

    #[test]
    fn a_type_without_examples_or_invalid_cases_is_caught() {
        let t = tempfile::tempdir().unwrap();
        put(
            t.path(),
            "acme/bare/v0.json",
            r#"{ "$id": "https://x.test/acme/bare/v0.json", "type": "object" }"#,
        );
        assert_eq!(
            problems(t.path()),
            [
                "acme/bare/v0: has no examples",
                "acme/bare/v0: has no invalid cases"
            ]
        );
    }

    #[test]
    fn a_schema_in_the_wrong_place_or_not_json_is_caught() {
        let t = good_tree();
        put(t.path(), "v0.json", "{}");
        put(t.path(), "acme/broken/v1.json", "{ not json");
        let p = problems(t.path());
        assert!(
            p.contains(&"v0: a schema sits at <area>/<name>/v<major>.json".to_owned()),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|l| l.starts_with("acme/broken/v1: not JSON (line 1")),
            "{p:?}"
        );
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_error_not_a_pass() {
        assert!(check_tree(Path::new("/definitely/not/here")).is_err());
    }
}
