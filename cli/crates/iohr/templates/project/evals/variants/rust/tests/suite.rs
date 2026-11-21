//! The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
//! is wrong, and the harness refuses a broken dataset or bar instead of grading less.

use @@MODULE@@::bar::{Bar, load_bar, missed};
use @@MODULE@@::dataset::{Case, DEFAULT_BAR, DEFAULT_DATASET, load_cases};
use @@MODULE@@::json::{self, Json};
use @@MODULE@@::runner::evaluate;
use @@MODULE@@::scorers::{exact, format, scorer_names};
use @@MODULE@@::solver::{LABELS, Solver, StubSolver};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn run(args: &[&str]) -> (u8, String, String) {
    run_with(args, None)
}

fn run_with(args: &[&str], solver: Option<&dyn Solver>) -> (u8, String, String) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = @@MODULE@@::suite(&args, &mut out, &mut err, solver);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A file of its own in a folder of its own, so tests running side by side never share one.
fn tmp(name: &str, text: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("evals-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    file
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn cases() -> Vec<Case> {
    load_cases(Path::new(DEFAULT_DATASET)).unwrap()
}

#[test]
fn the_fixture_meets_its_bar_with_no_network() {
    // Nothing here can reach a network: the crate has no dependency and the stub is local rules.
    let (code, out, _) = run(&[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("BAR MET"), "{out}");
}

#[test]
fn the_dataset_is_a_dozen_labelled_cases_covering_every_label() {
    let cases = cases();
    assert_eq!(cases.len(), 12);
    for label in LABELS {
        assert!(
            cases.iter().any(|c| c.expected == label),
            "no case expects {label}"
        );
    }
}

#[test]
fn the_report_says_the_solver_is_a_stub_and_holds_ids_and_scores_never_inputs() {
    let (code, out, _) = run(&["--json"]);
    assert_eq!(code, 0);
    let Json::Object(report) = json::parse(&out).unwrap() else {
        panic!("not an object")
    };
    assert_eq!(report["stub"], Json::Bool(true));
    assert_eq!(report["bar_met"], Json::Bool(true));
    assert_eq!(report["pass_rate"], Json::Number(1.0));
    let Json::Str(name) = &report["solver"] else {
        panic!("no solver")
    };
    assert!(name.starts_with("stub-"));
    for case in cases() {
        assert!(!out.contains(&case.input), "{}", case.input);
    }
}

#[test]
fn two_runs_give_the_same_report() {
    assert_eq!(run(&["--json"]), run(&["--json"]));
}

#[test]
fn lowering_one_expected_answer_fails_the_suite_and_names_the_case() {
    let text = std::fs::read_to_string(DEFAULT_DATASET).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let first = lines[0].replace("\"expected\": \"billing\"", "\"expected\": \"other\"");
    assert_ne!(first, lines[0]);
    lines[0] = first;
    let file = tmp("cases.jsonl", &(lines.join("\n") + "\n"));
    let (code, out, _) = run(&["--dataset", s(&file)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("BAR MISSED"), "{out}");
    assert!(out.contains("FAILED billing-double-charge"), "{out}");
}

#[test]
fn a_chatty_answer_fails_the_format_scorer_even_when_it_is_right() {
    struct Chatty;
    impl Solver for Chatty {
        fn name(&self) -> String {
            "stub-chatty".to_string()
        }
        fn stub(&self) -> bool {
            true
        }
        fn solve(&self, case: &Case) -> Result<String, String> {
            Ok(format!("The label is {}.", StubSolver.solve(case)?))
        }
    }
    let report = evaluate(&cases(), &Chatty);
    assert_eq!(report.mean("format"), 0.0);
    let bar = load_bar(Path::new(DEFAULT_BAR), &scorer_names()).unwrap();
    assert!(!missed(&report, &bar).is_empty());
}

#[test]
fn a_solver_that_fails_or_panics_is_an_error_and_never_a_pass() {
    struct Broken(bool);
    impl Solver for Broken {
        fn name(&self) -> String {
            "stub-broken".to_string()
        }
        fn stub(&self) -> bool {
            true
        }
        fn solve(&self, _: &Case) -> Result<String, String> {
            if self.0 {
                panic!("provider down")
            } else {
                Err("provider down".to_string())
            }
        }
    }
    let bar = load_bar(Path::new(DEFAULT_BAR), &scorer_names()).unwrap();
    for panics in [false, true] {
        let report = evaluate(&cases(), &Broken(panics));
        assert_eq!(report.errors(), 12);
        assert_eq!(report.pass_rate(), 0.0);
        assert!(missed(&report, &bar).iter().any(|m| m.contains("error")));
    }
    let (code, _, _) = run_with(&[], Some(&Broken(false)));
    assert_eq!(code, 1);
}

#[test]
fn a_broken_dataset_stops_the_run() {
    let row = "{\"id\": \"a\", \"input\": \"i\", \"expected\": \"bug\"}\n";
    let broken = [
        (format!("{row}{row}"), "twice"),
        (
            "{\"id\": \"a\", \"input\": \"\", \"expected\": \"bug\"}\n".to_string(),
            "input",
        ),
        ("\n".to_string(), "no cases"),
        ("not json\n".to_string(), "not JSON"),
    ];
    for (text, why) in broken {
        let file = tmp("c.jsonl", &text);
        let (code, _, err) = run(&["--dataset", s(&file)]);
        assert_eq!(code, 2, "{why}");
        assert!(err.contains(why), "{why}: {err}");
    }
}

#[test]
fn a_scorer_with_no_bar_stops_the_run() {
    let file = tmp(
        "t.toml",
        "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n",
    );
    let (code, _, err) = run(&["--bar", s(&file)]);
    assert_eq!(code, 2);
    assert!(err.contains("no bar for the scorer(s) format"), "{err}");
}

#[test]
fn a_bar_that_is_not_understood_stops_the_run() {
    for text in [
        "[bar]\nmin_pass_rate = high\n",
        "min_pass_rate = 1\n",
        "[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n",
    ] {
        let file = tmp("t.toml", text);
        assert_eq!(run(&["--bar", s(&file)]).0, 2, "{text}");
    }
}

#[test]
fn a_missing_bar_or_dataset_stops_the_run() {
    assert_eq!(run(&["--bar", "/nonexistent/t.toml"]).0, 2);
    assert_eq!(run(&["--dataset", "/nonexistent/c.jsonl"]).0, 2);
    let _: Result<Bar, _> = load_bar(Path::new("/nonexistent/t.toml"), &[]);
}

#[test]
fn the_scorers() {
    let billing = &cases()[0];
    assert_eq!(exact(billing, " Billing\n"), 1);
    assert_eq!(exact(billing, "bug"), 0);
    assert_eq!(format(billing, "billing"), 1);
    for bad in ["Billing", "billing.", "billing, bug", ""] {
        assert_eq!(format(billing, bad), 0, "{bad}");
    }
}

#[test]
fn every_scorer_has_a_bar() {
    let bar = load_bar(Path::new(DEFAULT_BAR), &scorer_names()).unwrap();
    let mut have: Vec<&str> = bar.scorers.keys().map(String::as_str).collect();
    have.sort_unstable();
    let mut want = scorer_names();
    want.sort_unstable();
    assert_eq!(have, want);
}
