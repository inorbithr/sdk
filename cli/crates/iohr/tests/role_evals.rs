//! Role evals (ADR 0021): does an agent that loads a built-in role catch the defects the role
//! is for, invent none, show its proof, keep the change small, and cost no more than it
//! should?
//!
//! The ordinary tests are offline and deterministic. They re-score the recorded transcripts
//! under `evals/roles/transcripts/` against the bar and the landing rule in
//! `evals/roles/thresholds.toml`, and fail when a pack changed but its transcripts were
//! recorded against the old pack hash: a roles change carries new recordings. The `#[ignore]`
//! tests record: `record_synthetic` with a scripted fake agent, `record_live` with the real
//! agent (the nightly workflow), and `fixture_checks` proves every fix check fails on its
//! fixture and passes on its clean twin.
//!
//! ```text
//! cargo nextest run -p iohr --test role_evals                       # replay
//! IOHR_UPDATE_EVALS=1 cargo nextest run -p iohr --test role_evals   # rewrite scores/
//! cargo nextest run -p iohr --test role_evals --run-ignored only -E 'test(record_live)'
//! ```

#![cfg(unix)]

#[path = "role_evals/model.rs"]
mod model;
#[path = "role_evals/record.rs"]
mod record;
#[path = "role_evals/score.rs"]
mod score;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use model::{Case, Event, Finding, Kind, Run, SHARED, Transcript};
use record::{Agent, Arm, Checker, Recorder};
use score::{ArmStats, Outcome, ScoreFile};

type Res = Result<(), Box<dyn Error>>;

fn env_list(key: &str) -> Option<Vec<String>> {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
}

fn wanted_roles() -> Result<Vec<String>, Box<dyn Error>> {
    let all = model::roles()?;
    Ok(match env_list("IOHR_EVAL_ROLES") {
        Some(only) => all.into_iter().filter(|r| only.contains(r)).collect(),
        None => all,
    })
}

// ---------------------------------------------------------------------------------------
// The replay

/// An arm's scores and the findings nobody judged, or why the arm cannot be scored.
type Scored = Result<(ArmStats, Vec<String>), String>;

/// Scores one arm of one role from its transcripts, or says what is missing.
fn score_arm(
    t: &model::Thresholds,
    cases: &[Case],
    pack: Option<&str>,
    model_name: &str,
) -> Result<Scored, Box<dyn Error>> {
    let runs = usize::try_from(t.arms.runs)?;
    let want_pack = pack.unwrap_or("none");
    let mut outcomes: Vec<Vec<Outcome>> = Vec::new();
    let mut unjudged = Vec::new();
    let mut synthetic = false;
    let mut model_id: Option<String> = None;
    for case in cases {
        let dir = model::arm_dir(pack, model_name, case);
        let recorded = model::runs(&dir)?;
        if recorded.len() < runs {
            return Ok(Err(format!(
                "{} has {} of {runs} runs in {}",
                case.id(),
                recorded.len(),
                dir.display()
            )));
        }
        let reference = score::reference_lines(case)?;
        let mut per_case = Vec::new();
        for run in &recorded[..runs] {
            for tr in [&run.fixture, &run.clean] {
                if tr.pack != want_pack || tr.model != model_name {
                    return Ok(Err(format!(
                        "{}: a transcript says pack {} model {}, want {want_pack} {model_name}",
                        dir.display(),
                        tr.pack,
                        tr.model
                    )));
                }
                synthetic |= tr.synthetic;
                match &model_id {
                    None => model_id = Some(tr.model_id.clone()),
                    Some(m) if *m != tr.model_id => {
                        return Ok(Err(format!(
                            "{}: mixed model ids ({m} and {}); record the arm again",
                            dir.display(),
                            tr.model_id
                        )));
                    }
                    Some(_) => {}
                }
            }
            let o = score::score_run(case, run, reference);
            unjudged.extend(o.unjudged.iter().cloned());
            per_case.push(o);
        }
        outcomes.push(per_case);
    }
    Ok(Ok((
        score::arm_stats(
            &outcomes,
            synthetic,
            model_id.as_deref().unwrap_or_default(),
        ),
        unjudged,
    )))
}

fn record_hint(role: &str) -> String {
    format!(
        "record them: IOHR_EVAL_ROLES={role} cargo nextest run -p iohr --test role_evals --run-ignored only -E 'test(record_live)' \
         (or test(record_synthetic) without a sign-in), then IOHR_UPDATE_EVALS=1 cargo nextest run -p iohr --test role_evals"
    )
}

/// Every role, every model with recordings: transcripts present for the pack the binary
/// builds, the bar cleared, the landing rule kept against the committed baseline, and the
/// committed scores equal to the recomputed ones. `IOHR_EVAL_ROLES` narrows it to some roles.
#[test]
fn replay() -> Res {
    let t = model::thresholds()?;
    let update = std::env::var("IOHR_UPDATE_EVALS").is_ok_and(|v| v == "1");
    let models = model::models()?;
    assert!(
        !models.is_empty(),
        "no transcripts under {}",
        model::root().join("transcripts").display()
    );
    let mut problems = Vec::new();
    for model_name in &models {
        for role in wanted_roles()? {
            replay_role(&t, model_name, &role, update, &mut problems)?;
        }
    }
    assert!(
        problems.is_empty(),
        "role evals:\n  {}",
        problems.join("\n  ")
    );
    Ok(())
}

const UNJUDGED: &str = "unjudged finding (add a [[verdict]] to the case)";

/// One role on one model: scores both arms, holds the pack to the bar and to the committed
/// baseline, and checks (or with `update`, rewrites) its score file.
fn replay_role(
    t: &model::Thresholds,
    model_name: &str,
    role: &str,
    update: bool,
    problems: &mut Vec<String>,
) -> Res {
    let cases = model::cases(role)?;
    let hash = record::live_hash(role, model_name)?;
    let path = model::root()
        .join("scores")
        .join(model_name)
        .join(format!("{role}.toml"));
    let old_text = std::fs::read_to_string(&path).ok();
    let old = old_text.as_deref().map(ScoreFile::parse).transpose()?;
    let (prop, unjudged) = match score_arm(t, &cases, Some(&hash), model_name)? {
        Ok(v) => v,
        Err(why) => {
            problems.push(format!(
                "{role}@{hash} ({model_name}): no complete transcripts for this pack: {why}; {}",
                record_hint(role)
            ));
            return Ok(());
        }
    };
    problems.extend(unjudged.into_iter().map(|u| format!("{UNJUDGED}: {u}")));
    let none = match score_arm(t, &cases, None, model_name)? {
        Ok((s, more)) => {
            problems.extend(more.into_iter().map(|u| format!("{UNJUDGED}: {u}")));
            Some(s)
        }
        Err(why) => {
            problems.push(format!("{role} ({model_name}) arm without a role: {why}"));
            None
        }
    };

    // The baseline: the pack the score file was last written for, or, when that is this
    // pack, the one it landed against.
    let base_hash = old.as_ref().and_then(|o| {
        if o.current == hash {
            o.previous.clone()
        } else {
            Some(o.current.clone())
        }
    });
    let baseline = base_hash
        .as_ref()
        .and_then(|h| old.as_ref().and_then(|o| o.arms.get(h)).cloned());
    match baseline.as_ref().map(|b| (b, b.synthetic, prop.synthetic)) {
        Some((_, false, true)) => problems.push(format!(
            "{role} ({model_name}): synthetic transcripts may not replace a real baseline"
        )),
        Some((base, false, false)) => {
            problems.extend(
                score::landing(&t.landing, base, &prop)
                    .into_iter()
                    .map(|w| format!("{role} ({model_name}) does not land: {w}")),
            );
        }
        // No baseline, or a synthetic one that measures nothing: the bar is all that applies.
        _ => {}
    }
    problems.extend(
        score::below_bar(&t.bar, &prop)
            .into_iter()
            .map(|w| format!("{role}@{hash} ({model_name}) is under the bar: {w}")),
    );

    let mut arms = BTreeMap::new();
    if let Some(n) = none {
        arms.insert("none".to_owned(), n);
    }
    if let (Some(h), Some(b)) = (&base_hash, &baseline) {
        arms.insert(h.clone(), b.clone());
    }
    let synthetic = prop.synthetic || arms.values().any(|a| a.synthetic);
    arms.insert(hash.clone(), prop);
    let new = ScoreFile {
        role: role.to_owned(),
        model: model_name.to_owned(),
        current: hash,
        previous: baseline.as_ref().and(base_hash),
        synthetic,
        arms,
    };
    let text = new.render()?;
    if update {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, &text)?;
    } else if old_text.as_deref() != Some(text.as_str()) {
        problems.push(format!("{} is out of date: run IOHR_UPDATE_EVALS=1 cargo nextest run -p iohr --test role_evals", path.display()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// The bar and the landing rule

#[test]
fn thresholds_parse_and_every_scorer_has_a_bar() -> Res {
    let t = model::thresholds()?;
    for lang in ["go", "rust", "python", "typescript", "actions"] {
        assert!(
            t.languages.contains_key(lang),
            "thresholds.toml: no [languages.{lang}]"
        );
    }
    Ok(())
}

#[test]
fn a_bar_with_a_missing_or_unknown_scorer_is_refused() -> Res {
    let mut bar = model::thresholds()?.bar;
    bar.scorers.remove("proof");
    assert!(
        bar.validate()
            .unwrap_err()
            .to_string()
            .contains("`proof` has no bar")
    );
    bar.scorers.insert("proof".into(), 1.0);
    bar.validate()?;
    bar.scorers.insert("vibes".into(), 1.0);
    assert!(
        bar.validate()
            .unwrap_err()
            .to_string()
            .contains("`vibes` is not a scorer")
    );
    Ok(())
}

fn stats(recall: u32, precision: u32, proof: u32, cost: f64, spread: f64) -> ArmStats {
    ArmStats {
        synthetic: false,
        model_id: "m".into(),
        runs: 3,
        cases: 8,
        pass_rate: 0.75,
        recall: f64::from(recall) / 8.0,
        precision: f64::from(precision) / 8.0,
        proof: f64::from(proof) / 8.0,
        size: 1.0,
        recall_cases: recall,
        precision_cases: precision,
        proof_cases: proof,
        size_cases: 8,
        cost,
        spread,
        errors: 0,
    }
}

#[test]
fn the_landing_rule_takes_a_better_role_that_costs_a_little_more() -> Res {
    let rule = model::thresholds()?.landing;
    let cur = stats(6, 7, 7, 10_000.0, 0.02);
    assert_eq!(
        score::landing(&rule, &cur, &stats(7, 7, 7, 10_500.0, 0.02)),
        Vec::<String>::new()
    );
    // Within the measured noise: a 15% rise is allowed when the runs spread 10%.
    assert_eq!(
        score::landing(
            &rule,
            &stats(6, 7, 7, 10_000.0, 0.10),
            &stats(7, 7, 7, 11_500.0, 0.10)
        ),
        Vec::<String>::new()
    );
    Ok(())
}

#[test]
fn the_landing_rule_refuses_a_regression() -> Res {
    let rule = model::thresholds()?.landing;
    let cur = stats(6, 7, 7, 10_000.0, 0.02);
    // Recall up one, precision down two: more than one case lost.
    let why = score::landing(&rule, &cur, &stats(7, 5, 7, 10_000.0, 0.02));
    assert!(
        why.iter().any(|w| w.contains("precision fell by 2")),
        "{why:?}"
    );
    // Proof may not fall.
    let why = score::landing(&rule, &cur, &stats(7, 7, 6, 10_000.0, 0.02));
    assert!(why.iter().any(|w| w.contains("proof fell")), "{why:?}");
    // The same score is not an improvement.
    let why = score::landing(&rule, &cur, &stats(6, 7, 7, 9_000.0, 0.02));
    assert!(why.iter().any(|w| w.contains("neither recall")), "{why:?}");
    Ok(())
}

#[test]
fn the_landing_rule_refuses_a_cost_rise_above_ten_percent() -> Res {
    let rule = model::thresholds()?.landing;
    let why = score::landing(
        &rule,
        &stats(6, 7, 7, 10_000.0, 0.02),
        &stats(8, 8, 7, 11_100.0, 0.03),
    );
    assert_eq!(why.len(), 1, "{why:?}");
    assert!(
        why[0].contains("cost rose 11.0%") && why[0].contains("10.0% allowed"),
        "{why:?}"
    );
    Ok(())
}

#[test]
fn the_landing_rule_refuses_arms_of_different_models() -> Res {
    let rule = model::thresholds()?.landing;
    let mut prop = stats(8, 8, 8, 10_000.0, 0.0);
    prop.model_id = "other".into();
    let why = score::landing(&rule, &stats(6, 7, 7, 10_000.0, 0.0), &prop);
    assert!(why[0].contains("different models"), "{why:?}");
    Ok(())
}

#[test]
fn an_arm_under_the_bar_is_named() -> Res {
    let bar = model::thresholds()?.bar;
    let mut s = stats(8, 8, 8, 1.0, 0.0);
    s.pass_rate = 1.0;
    assert_eq!(score::below_bar(&bar, &s), Vec::<String>::new());
    s.recall = 0.5;
    s.errors = 1;
    let why = score::below_bar(&bar, &s);
    assert!(
        why.iter().any(|w| w.starts_with("recall 0.5 <"))
            && why.iter().any(|w| w.starts_with("1 errors")),
        "{why:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------
// The scorers, on hand-written runs

fn transcript(target: &str, events: Vec<Event>, findings: Vec<Finding>, added: u32) -> Transcript {
    Transcript {
        run: 1,
        target: target.into(),
        synthetic: true,
        recorded: "2026-11-19".into(),
        pack: "none".into(),
        model: "m".into(),
        model_id: "m".into(),
        events,
        findings,
        report: String::new(),
        usage: model::Usage {
            input: 100,
            output: 10,
        },
        diff: model::Diff {
            added,
            removed: added,
            files: vec!["pricing.go".into()],
            added_tests: 0,
        },
        error: None,
    }
}

fn go_case() -> Result<Case, Box<dyn Error>> {
    model::cases("go-engineer")?
        .into_iter()
        .find(|c| c.name == "http-no-timeout")
        .ok_or_else(|| "no case".into())
}

#[test]
fn a_run_that_names_and_fixes_the_defect_with_proof_scores_all_four() -> Res {
    let case = go_case()?;
    let found = Finding {
        file: "./pricing.go".into(),
        line: 18,
        class: "http-no-timeout".into(),
        text: "no timeout".into(),
        evidence: "go test ./... hangs".into(),
    };
    let run = Run {
        fixture: transcript(
            "fixture",
            vec![
                Event::Command {
                    cmd: "cd /tmp/x && go test ./...".into(),
                    exit: Some(1),
                },
                Event::Check {
                    cmd: "go test ./...".into(),
                    exit: Some(0),
                },
            ],
            vec![found.clone()],
            3,
        ),
        clean: transcript("clean", vec![], vec![], 0),
    };
    let o = score::score_run(&case, &run, 4);
    assert!(o.recall && o.precision && o.proof && o.size, "{o:?}");
    assert_eq!(o.tokens, 220);
    // The same finding on the clean twin is a false one; a denied command is no proof; a
    // diff over twice the reference is too big.
    let run = Run {
        fixture: transcript(
            "fixture",
            vec![Event::Command {
                cmd: "go test ./...".into(),
                exit: None,
            }],
            vec![found.clone()],
            5,
        ),
        clean: transcript("clean", vec![], vec![Finding { line: 30, ..found }], 0),
    };
    let o = score::score_run(&case, &run, 4);
    assert!(o.recall && !o.precision && !o.proof && !o.size, "{o:?}");
    assert_eq!(o.unjudged, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_finding_nobody_judged_is_reported() -> Res {
    let case = go_case()?;
    let other = Finding {
        file: "pricing.go".into(),
        line: 3,
        class: "naming".into(),
        text: "odd name".into(),
        evidence: String::new(),
    };
    let run = Run {
        fixture: transcript("fixture", vec![], vec![other], 0),
        clean: transcript("clean", vec![], vec![], 0),
    };
    let o = score::score_run(&case, &run, 4);
    assert_eq!(o.unjudged.len(), 1, "{o:?}");
    assert!(!o.recall && !o.precision);
    Ok(())
}

#[test]
fn the_method_case_needs_a_source_before_the_first_edit_and_the_right_version() -> Res {
    let case = model::cases("python-engineer")?
        .into_iter()
        .find(|c| c.kind == Kind::Method)
        .ok_or("no method case")?;
    let mut fx = transcript(
        "fixture",
        vec![
            Event::Fetch {
                url: "https://pypi.org/pypi/httpx/json".into(),
            },
            Event::Edit {
                path: "pyproject.toml".into(),
            },
            Event::Check {
                cmd: "sh check.sh".into(),
                exit: Some(0),
            },
        ],
        vec![],
        2,
    );
    fx.report = "Moved to httpx 0.28.1, checked 2026-11-19 at PyPI.".into();
    let mut run = Run {
        fixture: fx.clone(),
        clean: transcript("clean", vec![], vec![], 0),
    };
    assert!(score::score_run(&case, &run, 4).recall);
    run.fixture.report = "Moved to httpx 0.27.0, checked 2026-11-19.".into();
    assert!(!score::score_run(&case, &run, 4).recall, "a wrong version");
    run.fixture.report = fx.report.replace("2026-11-19", "2026-11-18");
    assert!(
        !score::score_run(&case, &run, 4).recall,
        "not the run's date"
    );
    run.fixture = fx;
    run.fixture.events.swap(0, 1);
    assert!(
        !score::score_run(&case, &run, 4).recall,
        "fetched after the first edit"
    );
    Ok(())
}

#[test]
fn medians_counts_and_spread_of_an_arm() {
    let o = |ok: bool, tokens: u64| Outcome {
        recall: ok,
        precision: true,
        proof: true,
        size: true,
        tokens,
        error: false,
        unjudged: vec![],
    };
    // Two cases, three runs: case one passes recall in two runs of three, case two in one.
    let outcomes = vec![
        vec![o(true, 100), o(true, 110), o(false, 90)],
        vec![o(false, 100), o(true, 100), o(false, 100)],
    ];
    let s = score::arm_stats(&outcomes, false, "m");
    assert_eq!((s.recall_cases, s.recall, s.pass_rate), (1, 0.5, 0.5));
    // Totals 200, 210, 190: median 200 per run, 100 per case, spread 20 / 200.
    assert_eq!((s.cost, s.spread), (100.0, 0.1));
}

#[test]
fn line_diff_counts_what_changed() {
    assert_eq!(
        score::line_diff("a\nb\nc\n", "a\nB\nc\nd\n"),
        (2, 1, vec!["B".to_owned(), "d".to_owned()])
    );
    assert_eq!(score::line_diff("same\n", "same\n").0, 0);
}

#[test]
fn the_agent_stream_becomes_events_a_report_and_usage() {
    let stream = r#"{"type":"system","subtype":"init","model":"claude-x"}
{"type":"stream_event","event":{}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Bash","input":{"command":"go test ./..."}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","is_error":true,"content":"Exit code 1\nFAIL"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":"curl x"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b","is_error":true,"content":"Claude requested permissions to use Bash, but you haven't granted it yet."}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c","name":"Edit","input":{"file_path":"/w/a.go"}}]}}
{"type":"result","subtype":"success","is_error":false,"result":"ok\n```json\n{\"findings\":[{\"file\":\"a.go\",\"line\":3,\"class\":\"x\",\"text\":\"t\",\"evidence\":\"go test\"}]}\n```","usage":{"input_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"output_tokens":7}}"#;
    let p = record::parse_stream(stream);
    assert_eq!(p.model_id, "claude-x");
    assert_eq!(
        p.events,
        vec![
            Event::Command {
                cmd: "go test ./...".into(),
                exit: Some(1)
            },
            Event::Command {
                cmd: "curl x".into(),
                exit: None
            },
            Event::Edit {
                path: "/w/a.go".into()
            },
        ]
    );
    assert_eq!(
        p.usage,
        model::Usage {
            input: 125,
            output: 7
        }
    );
    assert_eq!(p.error, None);
    let f = record::findings(&p.report);
    assert_eq!(
        (f.len(), f[0].line, f[0].evidence.as_str()),
        (1, 3, "go test")
    );
}

// ---------------------------------------------------------------------------------------
// The cases

const MANIFESTS: [&str; 11] = [
    "go.mod",
    "go.sum",
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "requirements.txt",
    "pyproject.toml",
    "uv.lock",
    "yarn.lock",
];

/// `*` within a name; enough for the dry run's `Read(**/<name>)` rules.
fn wild(pat: &str, name: &str) -> bool {
    match pat.split_once('*') {
        None => pat == name,
        Some((head, rest)) => {
            name.starts_with(head)
                && (0..=name.len() - head.len()).any(|i| {
                    name.is_char_boundary(head.len() + i) && wild(rest, &name[head.len() + i..])
                })
        }
    }
}

fn stored(dir: &Path, rel: &str) -> PathBuf {
    let plain = dir.join(rel);
    if plain.exists() {
        plain
    } else {
        dir.join(format!("{rel}.eval"))
    }
}

/// The agent may not read a file whose name the dry run denies, so no fixture may hold one.
fn denied_names() -> Result<Vec<String>, Box<dyn Error>> {
    let tmp = tempfile::tempdir()?;
    std::fs::create_dir_all(tmp.path().join("bin"))?;
    record::write_script(&tmp.path().join("bin/claude"), "#!/bin/sh\n")?;
    let (_, args) = record::dry_run(
        "base-engineer",
        "sonnet",
        tmp.path(),
        &format!("{}:/usr/bin:/bin", tmp.path().join("bin").display()),
        "x",
    )?;
    Ok(args
        .iter()
        .filter_map(|a| a.strip_prefix("--disallowedTools=Read(")?.strip_suffix(')'))
        .map(|p| {
            p.trim_start_matches("./")
                .trim_start_matches("**/")
                .to_owned()
        })
        .collect())
}

/// Manifests and workflows are stored with `.eval`, so nothing in this repository (Dependabot,
/// osv-scanner, `CodeQL`, Actions) takes them for its own, and no file name is one the
/// agent's Read rules deny.
fn stored_files_are_safe(c: &Case, deny: &[String]) -> Res {
    for target in ["fixture", "clean"] {
        for rel in score::tree(&c.dir.join(target))?.keys() {
            let base = rel.rsplit('/').next().unwrap_or(rel);
            assert!(
                !MANIFESTS.contains(&base),
                "{}: {target}/{rel} must be stored as {rel}.eval",
                c.id()
            );
            let eval = Path::new(rel)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("eval"));
            assert!(
                eval || !rel.starts_with(".github/workflows/"),
                "{}: {target}/{rel} must be stored with .eval",
                c.id()
            );
            for name in rel.split('/') {
                assert!(
                    !deny
                        .iter()
                        .filter(|d| !d.contains('/'))
                        .any(|d| wild(d, model::live_name(name))),
                    "{}: the agent may not read {rel}",
                    c.id()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn every_case_is_well_formed() -> Res {
    let roles = model::roles()?;
    let deny = denied_names()?;
    assert!(
        deny.iter().any(|d| d == "*secret*"),
        "the dry run's Read rules changed: {deny:?}"
    );
    let mut seen = 0;
    for role in &roles {
        let cases = model::cases(role)?;
        for name in SHARED {
            assert!(
                cases.iter().any(|c| c.name == name),
                "{role} has no shared case {name}"
            );
        }
        assert!(cases.len() > SHARED.len(), "{role} has no case of its own");
        for c in cases.iter().filter(|c| c.owner == c.id()) {
            seen += 1;
            let fixture = c.dir.join("fixture");
            let text = std::fs::read_to_string(stored(&fixture, &c.defect.file))?;
            let [lo, hi] = c.defect.lines;
            assert!(lo >= 1 && lo <= hi, "{}: bad span", c.id());
            let span: Vec<&str> = text
                .lines()
                .skip(usize::try_from(lo - 1)?)
                .take(usize::try_from(hi - lo + 1)?)
                .collect();
            assert!(
                span.join("\n").contains(&c.defect.text),
                "{}: `{}` is not on {}:{lo}-{hi}",
                c.id(),
                c.defect.text,
                c.defect.file
            );
            if c.kind == Kind::Stale {
                assert!(
                    !c.defect.ids.is_empty(),
                    "{}: a stale case names its advisory ids",
                    c.id()
                );
            }
            for u in c.scope.iter().flat_map(|s| &s.untouched) {
                assert!(
                    stored(&fixture, u).exists(),
                    "{}: untouched {u} is not in the fixture",
                    c.id()
                );
            }
            assert!(
                !c.proof.ran.is_empty() || !c.proof.ran_any.is_empty(),
                "{}: no proof commands",
                c.id()
            );
            for v in &c.verdicts {
                assert!(
                    !v.why.trim().is_empty(),
                    "{}: the verdict on {} says no why",
                    c.id(),
                    v.file
                );
            }
            if let Some(m) = &c.method {
                assert!(
                    chrono::NaiveDate::parse_from_str(&m.recorded, "%Y-%m-%d").is_ok(),
                    "{}: method.recorded is not a date",
                    c.id()
                );
                assert!(
                    !m.truth.is_empty() && !m.sources.is_empty(),
                    "{}: a method case names its sources and truth",
                    c.id()
                );
            }
            stored_files_are_safe(c, &deny)?;
        }
    }
    assert!(seen >= roles.len(), "too few cases");
    Ok(())
}

#[test]
fn every_built_in_role_has_cases() -> Res {
    let have = model::roles()?;
    let mut names: Vec<String> = iohr::roles::catalogue()
        .into_iter()
        .map(|r| r.name)
        .collect();
    names.sort();
    assert_eq!(
        names, have,
        "evals/roles/cases/ and the built-in roles differ"
    );
    Ok(())
}

/// The nightly workflow runs one job per role; a new role needs a line there.
#[test]
fn the_nightly_workflow_records_every_role() -> Res {
    let path = model::root().join("../../../../../.github/workflows/role-evals.yml");
    let text = std::fs::read_to_string(&path)?;
    let line = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("role: ["))
        .ok_or("no `role: [...]` matrix")?;
    let mut listed: Vec<String> = line
        .trim_end_matches(']')
        .split(',')
        .map(|r| r.trim().to_owned())
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        model::roles()?,
        "{}: the matrix and evals/roles/cases/ differ",
        path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Recording (ignored: run on purpose)

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn write_run(dir: &Path, n: u32, fixture: &Transcript, clean: &Transcript) -> Res {
    std::fs::create_dir_all(dir)?;
    let text = format!(
        "{}\n{}\n",
        serde_json::to_string(fixture)?,
        serde_json::to_string(clean)?
    );
    std::fs::write(dir.join(format!("run-{n}.jsonl")), text)?;
    Ok(())
}

fn record_all(agent: Agent) -> Res {
    let t = model::thresholds()?;
    let claude = match agent {
        Agent::Live => Some(which("claude").ok_or(
            "record_live needs `claude` on the PATH (npm install -g @anthropic-ai/claude-code)",
        )?),
        Agent::Synthetic => None,
    };
    let arms = env_list("IOHR_EVAL_ARMS").unwrap_or_else(|| vec!["none".into(), "current".into()]);
    let mut rec = Recorder {
        agent,
        thresholds: &t,
        model: t.live.model.clone(),
        today: today(),
        checker: Checker::default(),
        claude,
    };
    let runs = t.arms.runs;
    let transcripts = model::root().join("transcripts");
    for role in wanted_roles()? {
        let cases = model::cases(&role)?;
        let hash = record::live_hash(&role, &rec.model)?;
        let mut jobs: Vec<(Case, Arm, PathBuf)> = Vec::new();
        if arms.iter().any(|a| a == "current") {
            // Older packs' transcripts go: their scores stay in scores/ as the baseline.
            if transcripts.is_dir() {
                for e in std::fs::read_dir(&transcripts)? {
                    let p = e?.path();
                    let n = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if n.starts_with(&format!("{role}@"))
                        && n != format!("{role}@{hash}")
                        && p.join(&rec.model).is_dir()
                    {
                        std::fs::remove_dir_all(p.join(&rec.model))?;
                        let _ = std::fs::remove_dir(&p);
                    }
                }
            }
            jobs.extend(cases.iter().map(|c| {
                (
                    c.clone(),
                    Arm {
                        pack: Some(hash.clone()),
                    },
                    model::arm_dir(Some(&hash), &rec.model, c),
                )
            }));
        }
        if arms.iter().any(|a| a == "none") {
            jobs.extend(cases.iter().filter(|c| c.owner == c.id()).map(|c| {
                (
                    c.clone(),
                    Arm { pack: None },
                    model::arm_dir(None, &rec.model, c),
                )
            }));
        }
        // Runs in a shuffled order, so a slow hour does not land on one case.
        let mut order: Vec<(String, usize, u32)> = Vec::new();
        for (i, (c, a, _)) in jobs.iter().enumerate() {
            for n in 1..=runs {
                order.push((
                    record::hash12(&format!("{}{}{:?}{n}", rec.today, c.id(), a.pack)),
                    i,
                    n,
                ));
            }
        }
        order.sort();
        let mut cleared = std::collections::BTreeSet::new();
        for (_, i, n) in order {
            let (case, arm, dir) = &jobs[i];
            if cleared.insert(dir.clone()) && dir.is_dir() {
                std::fs::remove_dir_all(dir)?;
            }
            let fixture = rec.record(case, arm, "fixture", n)?;
            let clean = rec.record(case, arm, "clean", n)?;
            write_run(dir, n, &fixture, &clean)?;
        }
    }
    Ok(())
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(name))
            .find(|f| f.is_file())
    })
}

/// Records every role with the scripted fake agent: the reference fix and the expected
/// report, labelled `synthetic`. It proves the pipeline and the scorers, never a role.
#[test]
#[ignore = "records transcripts; run on purpose"]
fn record_synthetic() -> Res {
    record_all(Agent::Synthetic)
}

/// Records every role with the real agent (`claude` on the PATH, signed in with a
/// `CLAUDE_CODE_OAUTH_TOKEN` subscription token). The nightly workflow runs it.
#[test]
#[ignore = "records transcripts with the live agent; costs subscription usage"]
fn record_live() -> Res {
    record_all(Agent::Live)
}

/// Every fix check fails on its fixture and passes on its clean twin.
#[test]
#[ignore = "runs every language's toolchain; run on purpose and nightly"]
fn fixture_checks() -> Res {
    let mut checker = Checker::default();
    let mut wrong = Vec::new();
    for role in wanted_roles()? {
        for c in model::cases(&role)?
            .into_iter()
            .filter(|c| c.owner == c.id())
        {
            for (target, want_pass) in [("fixture", false), ("clean", true)] {
                let tmp = tempfile::tempdir()?;
                let work = tmp.path().join("repo");
                record::prepare(&c, target, &work)?;
                let code = checker.run(&c, &work)?;
                if (code == Some(0)) != want_pass {
                    wrong.push(format!("{} {target}: check exit {code:?}", c.id()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "fix checks:\n  {}", wrong.join("\n  "));
    Ok(())
}
