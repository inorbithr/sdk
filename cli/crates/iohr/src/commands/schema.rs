//! `iohr schema check`: a project's JSON Schemas tree, checked against the layout of RFC 0147
//! (RFC 0234). Offline: it reads only the files in the folder.

use std::fmt::Write as _;

use serde::Serialize;

use crate::cli::SchemaCheck;
use crate::error::Error;
use crate::output::Out;
use crate::schema::{Problem, check_tree};
use crate::ui::{CheckRow, Mark, Paint};

/// What `--json` prints.
#[derive(Serialize)]
struct Answer<'a> {
    dir: String,
    types: usize,
    examples: usize,
    invalid: usize,
    problems: &'a [Problem],
}

/// Checks the tree in `args.dir`.
///
/// # Errors
///
/// [`Error::Usage`] when the folder is missing, [`Error::Findings`] (exit code 1) when a
/// schema breaks a rule, after printing every problem.
pub(crate) fn check(args: &SchemaCheck, out: Out) -> Result<(), Error> {
    let report = check_tree(&args.dir).map_err(Error::Usage)?;
    let n = report.problems.len();
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            one.to_owned()
        } else {
            many.to_owned()
        }
    };
    let checked = format!(
        "{} {}, {} {}, {} {} checked",
        report.types,
        plural(report.types, "schema", "schemas"),
        report.examples,
        plural(report.examples, "example", "examples"),
        report.invalid,
        plural(report.invalid, "invalid case", "invalid cases"),
    );
    let summary = format!("{checked}, {n} {}", plural(n, "problem", "problems"));
    let paint = Paint::stdout(out.json);
    if out.json {
        Out::print_json(&Answer {
            dir: args.dir.display().to_string(),
            types: report.types,
            examples: report.examples,
            invalid: report.invalid,
            problems: &report.problems,
        });
    } else if paint.color() {
        let rows: Vec<CheckRow> = report
            .problems
            .iter()
            .map(|p| CheckRow {
                mark: Mark::Fail,
                id: p.target.clone(),
                what: p.message.clone(),
                notes: Vec::new(),
            })
            .collect();
        Out::raw(paint.findings(&rows, &checked).as_bytes());
    } else {
        let mut text = String::new();
        for p in &report.problems {
            let _ = writeln!(text, "{}: {}", p.target, p.message);
        }
        Out::raw(text.as_bytes());
    }
    if n > 0 {
        return Err(Error::Findings(summary));
    }
    if !out.json && !paint.color() {
        Out::note(&summary);
    }
    Ok(())
}
