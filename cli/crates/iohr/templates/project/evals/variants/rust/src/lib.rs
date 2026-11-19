//! @@NAME@@: an offline eval suite with a pass bar. Standard library only, no network.
//!
//! Grade the solver on the dataset and check the pass bar. Exit 0 when the bar is met, 1 when it
//! is missed, 2 when the dataset or the bar is wrong. The report says which solver ran and
//! whether it is a stub. It holds case ids and scores only; the inputs and answers stay out of it
//! (they can be customer data), unless you ask for `--show`, which prints the failed cases to your
//! terminal and nowhere else.

pub mod bar;
pub mod dataset;
pub mod json;
pub mod runner;
pub mod scorers;
pub mod solver;

use dataset::{Case, DEFAULT_BAR, DEFAULT_DATASET};
use scorers::scorer_names;
use solver::{Solver, StubSolver};
use std::io::Write;
use std::path::Path;

fn round4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

/// A float as JSON, always with a point so 1 reads as 1.0.
fn num(x: f64) -> String {
    let s = round4(x).to_string();
    if s.contains('.') { s } else { format!("{s}.0") }
}

fn report_json(report: &runner::Report, missed: &[String]) -> String {
    let names = scorer_names();
    let scorers: Vec<String> = names
        .iter()
        .map(|n| format!("    {}: {}", json::quote(n), num(report.mean(n))))
        .collect();
    let results: Vec<String> = report
        .results
        .iter()
        .map(|r| {
            let scores: Vec<String> =
                r.scores.iter().map(|(k, v)| format!("{}: {v}", json::quote(k))).collect();
            let error = if r.error.is_empty() { "null".to_string() } else { json::quote(&r.error) };
            format!(
                "    {{\n      \"id\": {},\n      \"passed\": {},\n      \"scores\": {{{}}},\n      \"error\": {}\n    }}",
                json::quote(&r.id),
                r.passed(),
                scores.join(", "),
                error
            )
        })
        .collect();
    let miss: Vec<String> = missed
        .iter()
        .map(|m| format!("    {}", json::quote(m)))
        .collect();
    let list = |items: &[String]| {
        if items.is_empty() {
            "[]".to_string()
        } else {
            format!("[\n{}\n  ]", items.join(",\n"))
        }
    };
    format!(
        "{{\n  \"solver\": {},\n  \"stub\": {},\n  \"cases\": {},\n  \"pass_rate\": {},\n  \"errors\": {},\n  \"scorers\": {{\n{}\n  }},\n  \"bar_met\": {},\n  \"missed\": {},\n  \"results\": {}\n}}",
        json::quote(&report.solver),
        report.stub,
        report.results.len(),
        num(report.pass_rate()),
        report.errors(),
        scorers.join(",\n"),
        missed.is_empty(),
        list(&miss),
        list(&results)
    )
}

struct Options {
    dataset: String,
    bar: String,
    json: bool,
    show: bool,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        dataset: DEFAULT_DATASET.to_string(),
        bar: DEFAULT_BAR.to_string(),
        json: false,
        show: false,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dataset" | "--bar" => {
                let Some(value) = it.next() else {
                    return Err(format!("{arg} needs a value"));
                };
                if arg == "--dataset" {
                    o.dataset.clone_from(value);
                } else {
                    o.bar.clone_from(value);
                }
            }
            "--json" => o.json = true,
            "--show" => o.show = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(o)
}

/// Runs the whole thing and returns the exit code; `solver` is `None` for the stub.
pub fn suite(
    args: &[String],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    solver: Option<&dyn Solver>,
) -> u8 {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(why) => {
            let _ = writeln!(stderr, "usage error: {why}");
            return 2;
        }
    };
    let stub = StubSolver;
    let solver: &dyn Solver = solver.unwrap_or(&stub);
    let loaded = dataset::load_cases(Path::new(&opts.dataset))
        .and_then(|cases| bar::load_bar(Path::new(&opts.bar), &scorer_names()).map(|b| (cases, b)));
    let (cases, bar) = match loaded {
        Ok(pair) => pair,
        Err(why) => {
            let _ = writeln!(stderr, "suite error: {why}");
            return 2;
        }
    };
    let report = runner::evaluate(&cases, solver);
    let missed = bar::missed(&report, &bar);
    if opts.json {
        let _ = writeln!(stdout, "{}", report_json(&report, &missed));
    } else {
        let label = if report.stub {
            "STUB solver, not a model: "
        } else {
            ""
        };
        let _ = writeln!(
            stdout,
            "{label}{}: {}/{} cases pass",
            report.solver,
            report.passes(),
            report.results.len()
        );
        for name in scorer_names() {
            let _ = writeln!(
                stdout,
                "  {name}: mean {:.3} (bar {:.3})",
                report.mean(name),
                bar.scorers[name]
            );
        }
        let _ = writeln!(
            stdout,
            "  pass rate {:.3} (bar {:.3}), errors {} (at most {})",
            report.pass_rate(),
            bar.min_pass_rate,
            report.errors(),
            bar.max_errors
        );
        for r in report.results.iter().filter(|r| !r.passed()) {
            let why = if r.error.is_empty() {
                let zero: Vec<&str> = scorer_names()
                    .into_iter()
                    .filter(|n| r.scores[n] == 0)
                    .collect();
                zero.join(", ")
            } else {
                r.error.clone()
            };
            let _ = writeln!(stdout, "  FAILED {}: {why}", r.id);
        }
        if missed.is_empty() {
            let _ = writeln!(stdout, "BAR MET");
        } else {
            let _ = writeln!(stdout, "BAR MISSED: {}", missed.join("; "));
        }
    }
    if opts.show {
        let by_id: std::collections::HashMap<&str, &Case> =
            cases.iter().map(|c| (c.id.as_str(), c)).collect();
        for r in report.results.iter().filter(|r| !r.passed()) {
            let c = by_id[r.id.as_str()];
            let _ = writeln!(
                stderr,
                "  {}: input={:?} expected={:?} answer={:?}",
                r.id, c.input, c.expected, r.answer
            );
        }
    }
    u8::from(!missed.is_empty())
}
