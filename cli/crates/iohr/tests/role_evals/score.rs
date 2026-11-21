//! How a recorded run is scored, how the runs of an arm become its median, and the landing
//! rule that compares two arms. Deterministic: the same transcripts give the same scores.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::model::{
    Bar, Case, Defect, Diff, Event, Fallible, Finding, Kind, LOCKFILES, Landing, Run, SCORERS,
    Transcript, live_name,
};

// ---------------------------------------------------------------------------------------
// Trees and diffs

/// A directory's text files by relative path, `/`-separated. Build output and VCS data are
/// left out.
pub(crate) fn tree(root: &Path) -> Fallible<BTreeMap<String, String>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Fallible<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let ty = e.file_type()?;
            if ty.is_dir() {
                if matches!(
                    name.as_str(),
                    ".git" | "target" | "__pycache__" | "node_modules"
                ) {
                    continue;
                }
                walk(root, &e.path(), out)?;
            } else if ty.is_file() {
                let rel = e
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(
                    rel,
                    String::from_utf8_lossy(&std::fs::read(e.path())?).into_owned(),
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

/// Lines added and removed between two texts: a longest common subsequence, after the common
/// head and tail are taken off.
pub(crate) fn line_diff(before: &str, after: &str) -> (u32, u32, Vec<String>) {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let head = old.iter().zip(&new).take_while(|(x, y)| x == y).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (old, new) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    if old.len().saturating_mul(new.len()) > 4_000_000 {
        return (
            count(new.len()),
            count(old.len()),
            new.iter().map(|s| (*s).to_owned()).collect(),
        );
    }
    // lcs[i * width + j]: the longest common subsequence of old[i..] and new[j..].
    let width = new.len() + 1;
    let mut lcs = vec![0_u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i * width + j] = if old[i] == new[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut added = Vec::new();
    let mut removed = 0;
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * width + j] >= lcs[i * width + j + 1] {
            removed += 1;
            i += 1;
        } else {
            added.push(new[j].to_owned());
            j += 1;
        }
    }
    removed += old.len() - i;
    added.extend(new[j..].iter().map(|s| (*s).to_owned()));
    (count(added.len()), count(removed), added)
}

static TEST_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(func Test|def test_|async def test_|#\[(tokio::)?test|(test|it)\(|describe\()",
    )
    .unwrap_or_else(|e| panic!("{e}"))
});

/// What changed from one tree to the other. Lockfiles are listed but do not count.
pub(crate) fn diff_trees(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Diff {
    let mut d = Diff::default();
    let empty = String::new();
    let paths: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for p in paths {
        let (a, b) = (
            before.get(p).unwrap_or(&empty),
            after.get(p).unwrap_or(&empty),
        );
        if a == b && before.contains_key(p) == after.contains_key(p) {
            continue;
        }
        d.files.push(p.clone());
        let base = live_name(p.rsplit('/').next().unwrap_or(p));
        if LOCKFILES.contains(&base) {
            continue;
        }
        let (added, removed, lines) = line_diff(a, b);
        d.added += added;
        d.removed += removed;
        d.added_tests += u32::try_from(lines.iter().filter(|l| TEST_LINE.is_match(l)).count())
            .unwrap_or(u32::MAX);
    }
    d
}

/// The size of the reference fix: the fixture-to-clean diff, or the case's own figure when the
/// clean twin differs in more than the fix.
pub(crate) fn reference_lines(case: &Case) -> Fallible<u32> {
    if let Some(n) = case.reference_lines {
        return Ok(n);
    }
    let d = diff_trees(
        &tree(&case.dir.join("fixture"))?,
        &tree(&case.dir.join("clean"))?,
    );
    Ok((d.added + d.removed).max(1))
}

// ---------------------------------------------------------------------------------------
// One run

/// A path as an agent wrote it, against a path in the case: relative, absolute in the temp
/// copy, with `./` or with the stored `.eval`.
fn same_file(reported: &str, want: &str) -> bool {
    let r = live_name(reported.trim().trim_start_matches("./"));
    let w = live_name(want);
    r == w || r.ends_with(&format!("/{w}"))
}

fn names_word(f: &Finding, words: &[String]) -> bool {
    let hay = format!("{} {}", f.class, f.text).to_lowercase();
    words.iter().any(|w| hay.contains(&w.to_lowercase()))
}

/// The finding names the planted defect: its file, within three lines of its span, with its
/// class or one of its words.
pub(crate) fn names_defect(f: &Finding, d: &Defect) -> bool {
    same_file(&f.file, &d.file)
        && f.line + 3 >= d.lines[0]
        && f.line <= d.lines[1] + 3
        && (f.class.eq_ignore_ascii_case(&d.class) || names_word(f, &d.words))
}

fn names_id(f: &Finding, d: &Defect) -> bool {
    let hay = format!("{} {} {}", f.class, f.text, f.evidence);
    d.ids.iter().any(|id| hay.contains(id.as_str()))
}

/// How one finding is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Judged {
    True,
    False,
    /// Neither the planted defect nor a kept verdict: a person must judge it once.
    Unjudged,
}

pub(crate) fn judge(case: &Case, target: &str, f: &Finding) -> Judged {
    let d = &case.defect;
    if target == "fixture" {
        if names_defect(f, d)
            || (case.kind == Kind::Stale && same_file(&f.file, &d.file) && names_id(f, d))
        {
            return Judged::True;
        }
    } else if same_file(&f.file, &d.file)
        && (f.class.eq_ignore_ascii_case(&d.class) || names_word(f, &d.words))
    {
        // The defect is fixed in the clean twin: naming it there is a false finding.
        return Judged::False;
    }
    match case
        .verdicts
        .iter()
        .find(|v| same_file(&f.file, &v.file) && v.class.eq_ignore_ascii_case(&f.class))
    {
        Some(v) if v.true_finding => Judged::True,
        Some(_) => Judged::False,
        None => Judged::Unjudged,
    }
}

/// The commands that ran with a status.
fn commands(t: &Transcript) -> impl Iterator<Item = &str> {
    t.events.iter().filter_map(|e| match e {
        Event::Command { cmd, exit: Some(_) } => Some(cmd.as_str()),
        _ => None,
    })
}

fn ran(t: &Transcript, entry: &str) -> bool {
    entry
        .split('|')
        .any(|alt| commands(t).any(|c| c.contains(alt.trim())))
}

/// The programs a command line runs: the first word of each `&&`, `;` or `|` segment, past
/// any `VAR=value`.
fn programs(cmd: &str) -> Vec<String> {
    cmd.split(['&', ';', '|'])
        .filter_map(|seg| seg.split_whitespace().find(|w| !w.contains('=')))
        .map(|w| w.rsplit('/').next().unwrap_or(w).to_owned())
        .filter(|w| w != "cd")
        .collect()
}

static VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\s@=:^~>v\x22'(]{0,4}(\d+\.\d+(?:\.\d+)?)").unwrap_or_else(|e| panic!("{e}"))
});

/// Every version the report cites right after a package name is that package's truth, and
/// there is at least one.
fn versions_hold(report: &str, truth: &BTreeMap<String, String>) -> bool {
    let mut cited = 0;
    for (pkg, want) in truth {
        for (at, _) in report.match_indices(pkg.as_str()) {
            if let Some(c) = VERSION.captures(&report[at + pkg.len()..]) {
                cited += 1;
                if c[1] != *want && !want.starts_with(&format!("{}.", &c[1])) {
                    return false;
                }
            }
        }
    }
    cited > 0
}

fn method_holds(case: &Case, t: &Transcript) -> bool {
    let Some(m) = &case.method else { return false };
    let first_edit = t
        .events
        .iter()
        .position(|e| matches!(e, Event::Edit { .. }))
        .unwrap_or(t.events.len());
    let fetched = t.events[..first_edit]
        .iter()
        .any(|e| matches!(e, Event::Fetch { url } if m.sources.iter().any(|s| url.starts_with(s.as_str()))));
    fetched
        && t.report.contains(&format!("checked {}", t.recorded))
        && versions_hold(&t.report, &m.truth)
}

/// The four scores of one run of one case, and what it cost.
#[derive(Debug, Clone, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each scorer is pass or fail per run"
)]
pub(crate) struct Outcome {
    pub(crate) recall: bool,
    pub(crate) precision: bool,
    pub(crate) proof: bool,
    pub(crate) size: bool,
    pub(crate) tokens: u64,
    pub(crate) error: bool,
    pub(crate) unjudged: Vec<String>,
}

impl Outcome {
    pub(crate) fn get(&self, scorer: &str) -> bool {
        match scorer {
            "recall" => self.recall,
            "precision" => self.precision,
            "proof" => self.proof,
            _ => self.size,
        }
    }
}

pub(crate) fn score_run(case: &Case, run: &Run, reference: u32) -> Outcome {
    let fx = &run.fixture;
    let passes = fx.check_exit() == Some(0);
    let changed = fx.diff.added + fx.diff.removed > 0;
    let d = &case.defect;
    let recall = match case.kind {
        Kind::Planted => fx.findings.iter().any(|f| names_defect(f, d)) || (passes && changed),
        Kind::Stale => passes && fx.findings.iter().any(|f| names_id(f, d)),
        Kind::Dead => passes && case.proof.ran_any.iter().any(|e| ran(fx, e)),
        Kind::NoTest => passes && fx.diff.added_tests > 0,
        Kind::Scope => {
            let untouched = case
                .scope
                .as_ref()
                .map(|s| s.untouched.as_slice())
                .unwrap_or_default();
            passes
                && !fx
                    .diff
                    .files
                    .iter()
                    .any(|p| untouched.iter().any(|u| same_file(p, u)))
        }
        Kind::Method => passes && method_holds(case, fx),
        Kind::Keep => passes && fx.findings.iter().any(|f| names_defect(f, d)),
    };
    let mut unjudged = Vec::new();
    let mut precision = true;
    for t in [fx, &run.clean] {
        for f in &t.findings {
            match judge(case, &t.target, f) {
                Judged::True => {}
                Judged::False => precision = false,
                Judged::Unjudged => {
                    precision = false;
                    unjudged.push(format!(
                        "{} run {} {}: {}:{} [{}] {}",
                        case.id(),
                        t.run,
                        t.target,
                        f.file,
                        f.line,
                        f.class,
                        f.text
                    ));
                }
            }
        }
    }
    let progs: Vec<String> = commands(fx).flat_map(programs).collect();
    let proof = case.proof.ran.iter().all(|e| ran(fx, e))
        && (case.proof.ran_any.is_empty() || case.proof.ran_any.iter().any(|e| ran(fx, e)))
        && fx.findings.iter().all(|f| {
            progs
                .iter()
                .any(|p| !p.is_empty() && f.evidence.contains(p.as_str()))
        });
    let size = fx.diff.added + fx.diff.removed <= reference.saturating_mul(2);
    let tokens = [fx, &run.clean]
        .iter()
        .map(|t| t.usage.input + t.usage.output)
        .sum();
    let error = fx.error.is_some() || run.clean.error.is_some();
    Outcome {
        recall,
        precision,
        proof,
        size,
        tokens,
        error,
        unjudged,
    }
}

// ---------------------------------------------------------------------------------------
// An arm

/// An arm's scores: the median over its runs, and its spread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArmStats {
    pub(crate) synthetic: bool,
    pub(crate) model_id: String,
    pub(crate) runs: u32,
    pub(crate) cases: u32,
    pub(crate) pass_rate: f64,
    pub(crate) recall: f64,
    pub(crate) precision: f64,
    pub(crate) proof: f64,
    pub(crate) size: f64,
    /// Cases that pass in most runs, per scorer: what the landing rule counts.
    pub(crate) recall_cases: u32,
    pub(crate) precision_cases: u32,
    pub(crate) proof_cases: u32,
    pub(crate) size_cases: u32,
    /// Median input plus output tokens per case, both targets.
    pub(crate) cost: f64,
    /// (max - min) / median of the runs' total tokens.
    pub(crate) spread: f64,
    pub(crate) errors: u32,
}

impl ArmStats {
    pub(crate) fn scorer(&self, s: &str) -> f64 {
        match s {
            "recall" => self.recall,
            "precision" => self.precision,
            "proof" => self.proof,
            _ => self.size,
        }
    }
}

pub(crate) fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => f64::midpoint(v[n / 2 - 1], v[n / 2]),
    }
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[allow(clippy::cast_precision_loss)] // counts and token sums far below 2^52
fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

/// Scores an arm from `outcomes[case][run]`.
#[allow(clippy::cast_precision_loss)] // token sums far below 2^52
pub(crate) fn arm_stats(outcomes: &[Vec<Outcome>], synthetic: bool, model_id: &str) -> ArmStats {
    let cases = outcomes.len();
    let runs = outcomes.iter().map(Vec::len).min().unwrap_or(0);
    let per_run = |s: &str| -> f64 {
        median(
            (0..runs)
                .map(|r| ratio(outcomes.iter().filter(|c| c[r].get(s)).count(), cases))
                .collect(),
        )
    };
    let majority =
        |c: &Vec<Outcome>, s: &str| c[..runs].iter().filter(|o| o.get(s)).count() * 2 > runs;
    let count = |s: &str| {
        u32::try_from(outcomes.iter().filter(|c| majority(c, s)).count()).unwrap_or(u32::MAX)
    };
    let all = outcomes
        .iter()
        .filter(|c| SCORERS.iter().all(|s| majority(c, s)))
        .count();
    let totals: Vec<f64> = (0..runs)
        .map(|r| outcomes.iter().map(|c| c[r].tokens).sum::<u64>() as f64)
        .collect();
    let mid = median(totals.clone());
    let spread = if mid > 0.0 {
        (totals.iter().copied().fold(f64::MIN, f64::max)
            - totals.iter().copied().fold(f64::MAX, f64::min))
            / mid
    } else {
        0.0
    };
    ArmStats {
        synthetic,
        model_id: model_id.to_owned(),
        runs: u32::try_from(runs).unwrap_or(u32::MAX),
        cases: u32::try_from(cases).unwrap_or(u32::MAX),
        pass_rate: round4(ratio(all, cases)),
        recall: round4(per_run("recall")),
        precision: round4(per_run("precision")),
        proof: round4(per_run("proof")),
        size: round4(per_run("size")),
        recall_cases: count("recall"),
        precision_cases: count("precision"),
        proof_cases: count("proof"),
        size_cases: count("size"),
        cost: round4(if cases == 0 { 0.0 } else { mid / cases as f64 }),
        spread: round4(spread),
        errors: u32::try_from(outcomes.iter().flatten().filter(|o| o.error).count())
            .unwrap_or(u32::MAX),
    }
}

/// What keeps an arm under the bar; empty when it clears it.
pub(crate) fn below_bar(bar: &Bar, s: &ArmStats) -> Vec<String> {
    let mut out = Vec::new();
    if s.pass_rate < bar.min_pass_rate {
        out.push(format!("pass rate {} < {}", s.pass_rate, bar.min_pass_rate));
    }
    if s.errors > bar.max_errors {
        out.push(format!("{} errors > {}", s.errors, bar.max_errors));
    }
    for (name, min) in &bar.scorers {
        if s.scorer(name) < *min {
            out.push(format!("{name} {} < {min}", s.scorer(name)));
        }
    }
    out
}

/// The landing rule (thresholds.toml `[landing]`): why `proposed` may not replace `current`;
/// empty when it may.
pub(crate) fn landing(rule: &Landing, current: &ArmStats, proposed: &ArmStats) -> Vec<String> {
    let mut out = Vec::new();
    if current.cases != proposed.cases {
        out.push(format!(
            "the arms ran different cases ({} and {})",
            current.cases, proposed.cases
        ));
        return out;
    }
    if current.model_id != proposed.model_id {
        out.push(format!(
            "the arms ran different models ({} and {}): record the current pack again with the same model",
            current.model_id, proposed.model_id
        ));
        return out;
    }
    if proposed.recall_cases <= current.recall_cases
        && proposed.precision_cases <= current.precision_cases
    {
        out.push(format!(
            "neither recall ({} -> {} cases) nor precision ({} -> {} cases) went up",
            current.recall_cases,
            proposed.recall_cases,
            current.precision_cases,
            proposed.precision_cases
        ));
    }
    for (name, cur, prop) in [
        ("recall", current.recall_cases, proposed.recall_cases),
        (
            "precision",
            current.precision_cases,
            proposed.precision_cases,
        ),
    ] {
        if cur > prop + rule.max_case_drop {
            out.push(format!(
                "{name} fell by {} cases (at most {})",
                cur - prop,
                rule.max_case_drop
            ));
        }
    }
    if proposed.proof_cases < current.proof_cases {
        out.push(format!(
            "proof fell ({} -> {} cases)",
            current.proof_cases, proposed.proof_cases
        ));
    }
    let allowed = rule
        .max_cost_rise
        .max(rule.spread_factor * current.spread.max(proposed.spread));
    if proposed.cost > current.cost * (1.0 + allowed) {
        out.push(format!(
            "cost rose {:.1}% ({} -> {} tokens per case), more than the {:.1}% allowed",
            (proposed.cost / current.cost - 1.0) * 100.0,
            current.cost,
            proposed.cost,
            allowed * 100.0
        ));
    }
    out
}

// ---------------------------------------------------------------------------------------
// Score files

/// `scores/<model>/<role>.toml`: the committed scores of a role's arms, the baseline the next
/// pack change is held to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreFile {
    pub(crate) role: String,
    pub(crate) model: String,
    /// The pack hash these scores were recorded for.
    pub(crate) current: String,
    /// The pack hash before it, kept as the baseline the current one landed against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) previous: Option<String>,
    pub(crate) synthetic: bool,
    /// `none` (no role) and one entry per pack hash.
    pub(crate) arms: BTreeMap<String, ArmStats>,
}

pub(crate) const SCORE_HEADER: &str = "# Written by tests/role_evals.rs with IOHR_UPDATE_EVALS=1 from the transcripts; do not edit.\n# synthetic = true: recorded with a scripted fake agent, not a model (see ../../README.md).\n\n";

impl ScoreFile {
    pub(crate) fn render(&self) -> Fallible<String> {
        Ok(format!("{SCORE_HEADER}{}", toml::to_string(self)?))
    }

    pub(crate) fn parse(text: &str) -> Fallible<Self> {
        Ok(toml::from_str(text)?)
    }
}
