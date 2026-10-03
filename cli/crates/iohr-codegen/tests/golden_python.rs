//! Python golden tests: each case renders to exactly
//! `tests/golden/<case>/expected/python/`. `IOHR_UPDATE_GOLDEN=1` rewrites the expected
//! files; review the diff. `compile_python.rs` type-checks what they hold.

#![allow(
    clippy::unwrap_used,
    reason = "a fixture that does not load fails the test"
)]

mod support;

use iohr_codegen::{Language, Options};

use support::{CASES, check, render, text};

#[test]
fn every_case_matches_its_golden_files() {
    for case in CASES {
        check(case, Language::Python, &Options::default());
    }
}

/// The body of `class <name>:` up to the next class.
fn class_body(source: &str, name: &str) -> String {
    source
        .split(&format!("\nclass {name}:"))
        .nth(1)
        .map_or_else(String::new, |s| {
            s.split("\nclass ").next().unwrap().to_owned()
        })
}

#[test]
fn a_profile_class_holds_only_its_operations_in_both_forms() {
    let (_, files) = render("two-profiles", Language::Python, &Options::default());
    let profiles = text(&files, "profiles.py");
    for prefix in ["", "Async"] {
        let acme = class_body(&profiles, &format!("{prefix}AcmeCiAccounts"));
        assert!(acme.contains("def get_usage("), "{acme}");
        let personal = class_body(&profiles, &format!("{prefix}PersonalAccounts"));
        assert!(!personal.contains("def get_usage("), "{personal}");
    }
    assert!(
        profiles.contains("async def get_usage("),
        "the async form awaits"
    );
    assert!(
        profiles.contains("Client.from_env(\"ACME_CI\")"),
        "{profiles}"
    );
}

#[test]
fn queries_are_keyword_only_and_writes_send_their_body() {
    let (_, files) = render("public", Language::Python, &Options::default());
    let ops = text(&files, "operations.py");
    assert!(
        ops.contains("def get_usage(org_id: str, *, from_: str | None = None"),
        "{ops}"
    );
    assert!(ops.contains("codegen.path_segment(org_id)"), "{ops}");
    let models = text(&files, "models.py");
    assert!(models.contains("Int64"), "{models}");
    let (_, files) = render("write-scopes", Language::Python, &Options::default());
    let ops = text(&files, "operations.py");
    assert!(ops.contains("body: CreateEndpointRequest"), "{ops}");
    assert!(ops.contains("method=\"POST\""), "{ops}");
}

#[test]
fn rendering_is_deterministic() {
    let (_, a) = render("two-profiles", Language::Python, &Options::default());
    let (_, b) = render("two-profiles", Language::Python, &Options::default());
    assert_eq!(a, b);
}
