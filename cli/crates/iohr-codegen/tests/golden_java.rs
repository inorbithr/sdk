//! Java golden tests: each case renders to exactly `tests/golden/<case>/expected/java/`.
//! `IOHR_UPDATE_GOLDEN=1` rewrites the expected files; review the diff.
//! `compile_java.rs` compiles what they hold against `java/`.

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
        check(case, Language::Java, &Options::default());
    }
}

#[test]
fn a_profile_class_holds_only_its_operations() {
    let (_, files) = render("two-profiles", Language::Java, &Options::default());
    let acme = text(&files, "AcmeCiAccounts.java");
    assert!(acme.contains(" getUsage("), "{acme}");
    let personal = files
        .iter()
        .find(|(p, _)| p.ends_with("PersonalAccounts.java"))
        .map_or_else(String::new, |(_, b)| String::from_utf8(b.to_vec()).unwrap());
    assert!(!personal.contains(" getUsage("), "{personal}");
    let profile = text(&files, "AcmeCi.java");
    assert!(profile.contains("Client.fromEnv(\"ACME_CI\")"), "{profile}");
}

#[test]
fn int64_fields_are_decimal_strings_and_writes_send_their_body() {
    let (_, files) = render("public", Language::Java, &Options::default());
    let digest = text(&files, "Digest.java");
    assert!(
        digest.contains("@JsonFormat(shape = JsonFormat.Shape.STRING) long id"),
        "{digest}"
    );
    let (_, files) = render("write-scopes", Language::Java, &Options::default());
    let ops = text(&files, "Operations.java");
    assert!(ops.contains("CreateEndpointRequest body"), "{ops}");
    assert!(ops.contains("Method.POST"), "{ops}");
    assert!(ops.contains(".body(body)"), "{ops}");
    let request = text(&files, "CreateEndpointRequest.java");
    assert!(
        request.contains("public static Builder builder()"),
        "{request}"
    );
}

#[test]
fn the_package_follows_the_options() {
    let (_, files) = render("public", Language::Java, &Options::default());
    assert!(text(&files, "Me.java").contains("package iohr;"));
    let options = Options {
        package: Some("com.example.api".into()),
        ..Options::default()
    };
    let (_, files) = render("public", Language::Java, &options);
    assert!(text(&files, "Me.java").contains("package com.example.api;"));
    let options = Options {
        in_package: true,
        ..Options::default()
    };
    let (_, files) = render("public", Language::Java, &options);
    assert!(text(&files, "Me.java").contains("package hr.inorbit.sdk.generated;"));
    let bad = Options {
        package: Some("com.class.api".into()),
        ..Options::default()
    };
    let api = support::load("public");
    assert!(iohr_codegen::render(Language::Java, &api, &bad).is_err());
}

#[test]
fn rendering_is_deterministic() {
    let (_, a) = render("two-profiles", Language::Java, &Options::default());
    let (_, b) = render("two-profiles", Language::Java, &Options::default());
    assert_eq!(a, b);
}
