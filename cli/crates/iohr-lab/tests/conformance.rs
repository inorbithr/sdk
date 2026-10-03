//! Every case in `spec/lab/conformance.json`, synced from the platform: the findings
//! the InOrbit site's build reports for each fixture, which this crate must report too,
//! by rule id and line, in any order.

use std::collections::BTreeMap;

use iohr_lab::{Config, GENERIC_RULES, Kind, Redaction, check_document, check_folder};
use serde::Deserialize;

#[derive(Deserialize)]
struct Bundle {
    config: Config,
    files: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
struct Case {
    text: String,
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    line: Option<usize>,
    rule: String,
}

fn key(line: Option<usize>, rule: &str) -> String {
    format!(
        "{} {rule}",
        line.map_or_else(|| "-".to_owned(), |l| l.to_string())
    )
}

#[test]
fn every_conformance_case_finds_what_the_site_finds() {
    let bundle: Bundle =
        serde_json::from_str(include_str!("../../../../spec/lab/conformance.json")).unwrap();
    let redaction = Redaction::new(GENERIC_RULES, &bundle.config).unwrap();
    let mut folders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for path in bundle.files.keys() {
        let (folder, name) = path.split_once('/').unwrap();
        folders.entry(folder).or_default().push(name);
    }
    let mut failures = Vec::new();
    for (folder, names) in &folders {
        let whole = check_folder(names);
        for name in names {
            let case = &bundle.files[&format!("{folder}/{name}")];
            let mut got: Vec<String> =
                check_document(name, &case.text, Kind::of_folder(folder), &redaction)
                    .iter()
                    .chain(whole.get(name).into_iter().flatten())
                    .map(|f| key(f.line, &f.rule))
                    .collect();
            let mut want: Vec<String> =
                case.expected.iter().map(|e| key(e.line, &e.rule)).collect();
            got.sort();
            want.sort();
            if got != want {
                failures.push(format!("{folder}/{name}: got {got:?}, want {want:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "cases that differ:\n{}",
        failures.join("\n")
    );
    assert!(bundle.files.len() >= 10, "the bundle has every case");
}
