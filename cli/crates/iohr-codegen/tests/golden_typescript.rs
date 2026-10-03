//! TypeScript golden tests: each case renders to exactly
//! `tests/golden/<case>/expected/typescript/`. `IOHR_UPDATE_GOLDEN=1` rewrites the
//! expected files; review the diff. `compile_typescript.rs` type-checks what they hold.

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
        check(case, Language::TypeScript, &Options::default());
    }
}

#[test]
fn a_profile_class_holds_only_its_operations() {
    let (_, files) = render("two-profiles", Language::TypeScript, &Options::default());
    let profiles = text(&files, "profiles.ts");
    let acme = profiles
        .split("export class AcmeCiAccounts")
        .nth(1)
        .unwrap()
        .split("export class")
        .next()
        .unwrap();
    assert!(acme.contains("getUsage("), "{acme}");
    let personal = profiles
        .split("export class PersonalAccounts")
        .nth(1)
        .map_or("", |s| s.split("export class").next().unwrap());
    assert!(!personal.contains("getUsage("), "{personal}");
    assert!(
        profiles.contains("Client.fromEnv(\"ACME_CI\")"),
        "{profiles}"
    );
}

#[test]
fn int64_fields_convert_and_writes_send_their_body() {
    let (_, files) = render("public", Language::TypeScript, &Options::default());
    let models = text(&files, "models.ts");
    assert!(
        models.contains("export const shapes: codegen.Shapes"),
        "{models}"
    );
    assert!(models.contains("\"i64\""), "{models}");
    let (_, files) = render("write-scopes", Language::TypeScript, &Options::default());
    let ops = text(&files, "operations.ts");
    assert!(ops.contains("body: CreateEndpointRequest"), "{ops}");
    assert!(ops.contains("method: \"POST\""), "{ops}");
}

#[test]
fn rendering_is_deterministic() {
    let (_, a) = render("two-profiles", Language::TypeScript, &Options::default());
    let (_, b) = render("two-profiles", Language::TypeScript, &Options::default());
    assert_eq!(a, b);
}
