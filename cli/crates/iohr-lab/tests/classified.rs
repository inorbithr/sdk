//! Every case in `tests/classified.json`: what the platform's TypeScript answers for
//! classified spans (the parse, `cut` at every clearance, and `check` findings), which
//! this crate must answer too, byte for byte for a cut, by rule id and line for a check.

use std::collections::BTreeMap;

use iohr_lab::{
    AccessLevel, Config, GENERIC_RULES, Kind, Redaction, SpanKind, check_document, cut, needles,
    parse_classified,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Cases {
    cases: BTreeMap<String, Case>,
    documents: BTreeMap<String, Document>,
}

#[derive(Deserialize)]
struct Case {
    text: String,
    problems: Vec<usize>,
    spans: Vec<Span>,
    cut: BTreeMap<String, Option<String>>,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct Span {
    kind: String,
    level: String,
    line: usize,
    end_line: usize,
}

#[derive(Deserialize)]
struct Document {
    text: String,
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    line: Option<usize>,
    rule: String,
}

#[derive(Deserialize)]
struct Bundle {
    config: Config,
}

const CASES: &str = include_str!("classified.json");

#[allow(clippy::unwrap_used)]
fn cases() -> Cases {
    serde_json::from_str(CASES).unwrap()
}

#[test]
fn every_case_parses_and_cuts_as_the_site_does() {
    let mut failures = Vec::new();
    let all = cases();
    for (name, case) in &all.cases {
        let parsed = parse_classified(&case.text);
        let problems: Vec<usize> = parsed.problems.iter().map(|p| p.line).collect();
        if problems != case.problems {
            failures.push(format!(
                "{name}: problems {problems:?}, want {:?}",
                case.problems
            ));
        }
        let spans: Vec<Span> = parsed
            .spans
            .iter()
            .map(|s| Span {
                kind: match s.kind {
                    SpanKind::Inline => "inline".into(),
                    SpanKind::Block => "block".into(),
                },
                level: s.level.as_str().into(),
                line: s.line,
                end_line: s.end_line,
            })
            .collect();
        if spans != case.spans {
            failures.push(format!("{name}: spans {spans:?}, want {:?}", case.spans));
        }
        assert_eq!(case.cut.len(), 5, "{name}: a cut for every rung");
        for (clearance, want) in &case.cut {
            let level = AccessLevel::from_name(clearance).unwrap();
            let got = cut(&case.text, level).ok();
            if &got != want {
                failures.push(format!("{name} at {clearance}: {got:?}, want {want:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "cases that differ:\n{}",
        failures.join("\n")
    );
    assert!(all.cases.len() >= 40, "the file has every case");
}

#[test]
fn every_document_finds_what_the_site_finds() {
    let bundle: Bundle =
        serde_json::from_str(include_str!("../../../../spec/lab/conformance.json")).unwrap();
    let redaction = Redaction::new(GENERIC_RULES, &bundle.config).unwrap();
    let mut failures = Vec::new();
    for (path, doc) in &cases().documents {
        let (folder, name) = path.split_once('/').unwrap();
        let mut got: Vec<(Option<usize>, String)> =
            check_document(name, &doc.text, Kind::of_folder(folder), &redaction)
                .into_iter()
                .map(|f| (f.line, f.rule))
                .collect();
        let mut want: Vec<(Option<usize>, String)> = doc
            .expected
            .iter()
            .map(|e| (e.line, e.rule.clone()))
            .collect();
        got.sort();
        want.sort();
        if got != want {
            failures.push(format!("{path}: got {got:?}, want {want:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "documents that differ:\n{}",
        failures.join("\n")
    );
}

/// The guarantee, over every case that parses: a cut below a span's level holds none
/// of its original, and every cut keeps the line count.
#[test]
fn cut_below_a_level_never_holds_the_original() {
    for (name, case) in &cases().cases {
        let parsed = parse_classified(&case.text);
        if !parsed.problems.is_empty() {
            for level in AccessLevel::ALL {
                assert!(cut(&case.text, level).is_err(), "{name}: cut on a guess");
            }
            continue;
        }
        let lines = case.text.split('\n').count();
        for clearance in AccessLevel::ALL {
            let out = cut(&case.text, clearance).unwrap();
            assert_eq!(out.split('\n').count(), lines, "{name} at {clearance}");
            // A needle a revealed span also holds (one canary at two levels) proves nothing.
            let revealed: Vec<&str> = parsed
                .spans
                .iter()
                .filter(|s| s.level <= clearance)
                .map(|s| s.original.as_str())
                .collect();
            for s in parsed.spans.iter().filter(|s| s.level > clearance) {
                for n in needles(s, 4)
                    .into_iter()
                    .filter(|n| !revealed.iter().any(|r| r.contains(n.as_str())))
                {
                    assert!(!out.contains(&n), "{name} at {clearance}: {n:?} leaked");
                }
            }
        }
        let public = cut(&case.text, AccessLevel::Public).unwrap();
        for s in &parsed.spans {
            assert!(
                !public.contains(s.original.trim()),
                "{name}: cut(public) holds an original"
            );
        }
    }
}
