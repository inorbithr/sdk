//! What the role evals read: the bar (`thresholds.toml`), the cases (`cases/<role>/<case>/`)
//! and the recorded transcripts (`transcripts/`). Nothing here runs a program.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) type Fallible<T> = Result<T, Box<dyn Error>>;

/// The scorers, in the order they are reported. Every one needs a bar and no bar may name
/// another (the template's rule: a run refuses to start when a scorer has no bar).
pub(crate) const SCORERS: [&str; 4] = ["recall", "precision", "proof", "size"];

/// The shared cases every role carries, beside its own.
pub(crate) const SHARED: [&str; 5] = [
    "stale-dependency",
    "dead-code",
    "no-test",
    "scope-trap",
    "method",
];

/// Lockfiles: written by the tools, so they do not count towards the size of a change.
pub(crate) const LOCKFILES: [&str; 6] = [
    "go.sum",
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "uv.lock",
    "yarn.lock",
];

/// `cli/crates/iohr/evals/roles`.
pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evals")
        .join("roles")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Thresholds {
    pub(crate) bar: Bar,
    pub(crate) landing: Landing,
    pub(crate) arms: Arms,
    pub(crate) task: TaskText,
    pub(crate) live: Live,
    pub(crate) languages: BTreeMap<String, Language>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bar {
    pub(crate) min_pass_rate: f64,
    pub(crate) max_errors: u32,
    pub(crate) scorers: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Landing {
    pub(crate) max_cost_rise: f64,
    pub(crate) spread_factor: f64,
    pub(crate) max_case_drop: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Arms {
    pub(crate) runs: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskText {
    pub(crate) suffix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Live {
    pub(crate) model: String,
    pub(crate) timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Language {
    pub(crate) allow: Vec<String>,
}

impl Bar {
    /// Refuses a bar that misses a scorer or names one that does not exist.
    pub(crate) fn validate(&self) -> Fallible<()> {
        for s in SCORERS {
            if !self.scorers.contains_key(s) {
                return Err(format!("thresholds.toml: scorer `{s}` has no bar").into());
            }
        }
        if let Some(k) = self.scorers.keys().find(|k| !SCORERS.contains(&k.as_str())) {
            return Err(format!("thresholds.toml: `{k}` is not a scorer ({SCORERS:?})").into());
        }
        Ok(())
    }
}

pub(crate) fn thresholds() -> Fallible<Thresholds> {
    let t: Thresholds = toml::from_str(&std::fs::read_to_string(root().join("thresholds.toml"))?)?;
    t.bar.validate()?;
    if t.arms.runs < 3 {
        return Err("thresholds.toml: [arms] runs must be 3 or more".into());
    }
    Ok(t)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    /// A planted defect: found when a finding names it or the fix check passes.
    Planted,
    /// A pinned version with a published advisory.
    Stale,
    /// An unused function next to the change.
    Dead,
    /// A bug fix with no test for the path.
    NoTest,
    /// A messy file the task never touches.
    Scope,
    /// Add or upgrade a library, checking the official source first.
    Method,
    /// Something that looks removable but is in use: it must become a finding.
    Keep,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Defect {
    pub(crate) file: String,
    pub(crate) lines: [u32; 2],
    pub(crate) text: String,
    pub(crate) class: String,
    pub(crate) words: Vec<String>,
    #[serde(default)]
    pub(crate) ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    pub(crate) cmd: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    /// Each entry must have run, with its exit status; `a|b` takes either.
    #[serde(default)]
    pub(crate) ran: Vec<String>,
    /// At least one of these must have run.
    #[serde(default)]
    pub(crate) ran_any: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    pub(crate) untouched: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Method {
    /// The day the truth below was read at its source.
    pub(crate) recorded: String,
    /// Official sources: a fetch starting with one of these counts.
    pub(crate) sources: Vec<String>,
    pub(crate) truth_url: String,
    pub(crate) truth_pointer: String,
    /// Package to its latest version on `recorded`.
    pub(crate) truth: BTreeMap<String, String>,
}

/// A person's verdict on a finding that is not the planted defect, kept so replays stay the
/// same.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Verdict {
    pub(crate) file: String,
    pub(crate) class: String,
    pub(crate) true_finding: bool,
    pub(crate) why: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    same_as: Option<String>,
    kind: Option<Kind>,
    language: Option<String>,
    task: Option<String>,
    reference_lines: Option<u32>,
    defect: Option<Defect>,
    check: Option<Check>,
    #[serde(default)]
    proof: Proof,
    scope: Option<Scope>,
    method: Option<Method>,
    #[serde(default)]
    verdict: Vec<Verdict>,
}

/// One case as a role runs it: a shared case carries the role and name it is listed under,
/// and everything else from the case it points to.
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) role: String,
    pub(crate) name: String,
    /// `<role>/<case>` of the directory holding `fixture/`, `clean/` and `check/`.
    pub(crate) owner: String,
    pub(crate) dir: PathBuf,
    pub(crate) kind: Kind,
    pub(crate) language: String,
    pub(crate) task: String,
    pub(crate) reference_lines: Option<u32>,
    pub(crate) defect: Defect,
    pub(crate) check: Check,
    pub(crate) proof: Proof,
    pub(crate) scope: Option<Scope>,
    pub(crate) method: Option<Method>,
    pub(crate) verdicts: Vec<Verdict>,
}

impl Case {
    pub(crate) fn id(&self) -> String {
        format!("{}/{}", self.role, self.name)
    }
}

fn read_case(dir: &Path) -> Fallible<CaseFile> {
    let p = dir.join("case.toml");
    toml::from_str(&std::fs::read_to_string(&p)?)
        .map_err(|e| format!("{}: {e}", p.display()).into())
}

fn owned(role: &str, name: &str, owner: &str, dir: &Path, f: CaseFile) -> Fallible<Case> {
    let need = |what: &str| format!("{owner}: case.toml has no {what}");
    let kind = f.kind.ok_or_else(|| need("kind"))?;
    if kind == Kind::Method && f.method.is_none() {
        return Err(need("[method]").into());
    }
    if kind == Kind::Scope && f.scope.is_none() {
        return Err(need("[scope]").into());
    }
    Ok(Case {
        role: role.to_owned(),
        name: name.to_owned(),
        owner: owner.to_owned(),
        dir: dir.to_path_buf(),
        kind,
        language: f.language.ok_or_else(|| need("language"))?,
        task: f.task.ok_or_else(|| need("task"))?,
        reference_lines: f.reference_lines,
        defect: f.defect.ok_or_else(|| need("[defect]"))?,
        check: f.check.ok_or_else(|| need("[check]"))?,
        proof: f.proof,
        scope: f.scope,
        method: f.method,
        verdicts: f.verdict,
    })
}

fn dirs(p: &Path) -> Fallible<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(p)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            out.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// Every role that has a `cases/` directory.
pub(crate) fn roles() -> Fallible<Vec<String>> {
    dirs(&root().join("cases"))
}

/// The cases of one role, shared ones resolved, by name.
pub(crate) fn cases(role: &str) -> Fallible<Vec<Case>> {
    let base = root().join("cases");
    let mut out = Vec::new();
    for name in dirs(&base.join(role))? {
        let dir = base.join(role).join(&name);
        let f = read_case(&dir)?;
        let case = match f.same_as {
            Some(ref target) => {
                if f.kind.is_some() || f.defect.is_some() || !f.verdict.is_empty() {
                    return Err(format!("{role}/{name}: a same_as case holds nothing else").into());
                }
                let tdir = base.join(target);
                let t = read_case(&tdir).map_err(|e| format!("{role}/{name} -> {target}: {e}"))?;
                if t.same_as.is_some() {
                    return Err(
                        format!("{role}/{name} -> {target}: that is a same_as case too").into(),
                    );
                }
                owned(role, &name, target, &tdir, t)?
            }
            None => owned(role, &name, &format!("{role}/{name}"), &dir, f)?,
        };
        out.push(case);
    }
    Ok(out)
}

/// A file name in a fixture: manifests and lockfiles carry `.eval` so no scanner of this
/// repository (Dependabot, osv-scanner, `CodeQL`) reads them as this repository's own.
pub(crate) fn live_name(stored: &str) -> &str {
    stored.strip_suffix(".eval").unwrap_or(stored)
}

/// One finding as an agent reported it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Finding {
    #[serde(default)]
    pub(crate) file: String,
    #[serde(default)]
    pub(crate) line: u32,
    #[serde(default)]
    pub(crate) class: String,
    #[serde(default)]
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) evidence: String,
}

/// What the agent did, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Event {
    /// A shell command; `exit` is None when it never ran (denied) or no status came back.
    Command {
        cmd: String,
        exit: Option<i32>,
    },
    Fetch {
        url: String,
    },
    Edit {
        path: String,
    },
    /// The case's fix check, run by the recorder on the tree the agent left.
    Check {
        cmd: String,
        exit: Option<i32>,
    },
    /// The live truth for the method case, read by the recorder at its source.
    Truth {
        versions: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Usage {
    pub(crate) input: u64,
    pub(crate) output: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Diff {
    pub(crate) added: u32,
    pub(crate) removed: u32,
    pub(crate) files: Vec<String>,
    pub(crate) added_tests: u32,
}

/// One line of a transcript: one run of one case on one target (`fixture` or `clean`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transcript {
    pub(crate) run: u32,
    pub(crate) target: String,
    /// True when a scripted fake agent produced it, not a model.
    pub(crate) synthetic: bool,
    /// The day it was recorded (YYYY-MM-DD).
    pub(crate) recorded: String,
    /// The pack hash, or `none` for the arm without a role.
    pub(crate) pack: String,
    pub(crate) model: String,
    pub(crate) model_id: String,
    pub(crate) events: Vec<Event>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) report: String,
    pub(crate) usage: Usage,
    pub(crate) diff: Diff,
    pub(crate) error: Option<String>,
}

impl Transcript {
    pub(crate) fn check_exit(&self) -> Option<i32> {
        self.events.iter().rev().find_map(|e| match e {
            Event::Check { exit, .. } => Some(*exit),
            _ => None,
        })?
    }
}

/// One run of one case: its fixture and clean lines.
#[derive(Debug, Clone)]
pub(crate) struct Run {
    pub(crate) fixture: Transcript,
    pub(crate) clean: Transcript,
}

/// `transcripts/<role>@<hash>/<model>/<case>` or, for the arm without a role,
/// `transcripts/none/<model>/<owner role>/<owner case>`.
pub(crate) fn arm_dir(pack: Option<&str>, model: &str, case: &Case) -> PathBuf {
    let t = root().join("transcripts");
    match pack {
        Some(h) => t
            .join(format!("{}@{h}", case.role))
            .join(model)
            .join(&case.name),
        None => t.join("none").join(model).join(&case.owner),
    }
}

/// The runs recorded for one case and arm, by run number.
pub(crate) fn runs(dir: &Path) -> Fallible<Vec<Run>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f)?;
        let mut fixture = None;
        let mut clean = None;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let t: Transcript =
                serde_json::from_str(line).map_err(|e| format!("{}: {e}", f.display()))?;
            match t.target.as_str() {
                "fixture" => fixture = Some(t),
                "clean" => clean = Some(t),
                other => return Err(format!("{}: unknown target `{other}`", f.display()).into()),
            }
        }
        match (fixture, clean) {
            (Some(fixture), Some(clean)) => out.push(Run { fixture, clean }),
            _ => return Err(format!("{}: needs a fixture and a clean line", f.display()).into()),
        }
    }
    Ok(out)
}

/// The models that have scores or transcripts.
pub(crate) fn models() -> Fallible<Vec<String>> {
    let mut out = std::collections::BTreeSet::new();
    let scores = root().join("scores");
    if scores.is_dir() {
        out.extend(dirs(&scores)?);
    }
    let t = root().join("transcripts");
    if t.is_dir() {
        for arm in dirs(&t)? {
            out.extend(dirs(&t.join(arm))?);
        }
    }
    Ok(out.into_iter().collect())
}
