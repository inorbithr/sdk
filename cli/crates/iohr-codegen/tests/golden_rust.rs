//! Rust golden tests: each case under `tests/golden/<case>/input/<profile>.json` renders
//! to exactly `tests/golden/<case>/expected/rust/`. `IOHR_UPDATE_GOLDEN=1` rewrites the
//! expected files; review the diff. The compile test builds what they hold.

#![allow(
    clippy::unwrap_used,
    reason = "a fixture that does not load fails the test"
)]

mod support;

use iohr_codegen::{Language, Options};

use support::{check, render, text};

fn rust(in_package: bool) -> Options {
    Options {
        in_package,
        ..Options::default()
    }
}

#[test]
fn the_public_surface_matches_its_golden_files() {
    check("public", Language::Rust, &rust(false));
}

#[test]
fn two_profiles_share_models_and_keep_their_own_operations() {
    check("two-profiles", Language::Rust, &rust(false));
    let (api, files) = render("two-profiles", Language::Rust, &rust(false));
    let ops = text(&files, "ops.rs");
    assert!(ops.contains("impl ListDigests for super::profiles::AcmeCi {}"));
    assert!(ops.contains("impl ListDigests for super::profiles::Personal {}"));
    assert!(ops.contains("impl GetUsage for super::profiles::AcmeCi {}"));
    assert!(!ops.contains("impl GetUsage for super::profiles::Personal {}"));
    assert_eq!(
        api.profiles["personal"].hash,
        format!("sha256:{}", "ab".repeat(32))
    );
    // The stream is generated, and gated like every other operation (design.md §7).
    assert!(ops.contains("impl StreamEvents for super::profiles::AcmeCi {}"));
    assert!(!ops.contains("impl StreamEvents for super::profiles::Personal {}"));
    let surface = text(&files, "surface.rs");
    assert!(
        surface.contains("EventStream<StreamEventsResponse>"),
        "{surface}"
    );
    assert!(surface.contains(".rpc(\"iohr.events.v1.EventsService/StreamEvents\")"));
    assert!(files.notes().is_empty(), "{:?}", files.notes());
}

#[test]
fn write_scopes_render_bodies_and_mark_nothing_idempotent() {
    check("write-scopes", Language::Rust, &rust(false));
    let (api, files) = render("write-scopes", Language::Rust, &rust(false));
    assert!(api.profiles["hooks"].local);
    let surface = text(&files, "surface.rs");
    assert!(
        surface.contains("body: &CreateEndpointRequest"),
        "{surface}"
    );
    assert!(surface.contains(".json(body)?"), "{surface}");
    assert!(
        !surface.contains("list_endpoints"),
        "a read is not in a write-only cut"
    );
}

#[test]
fn rendering_is_deterministic() {
    let (_, a) = render("two-profiles", Language::Rust, &rust(false));
    let (_, b) = render("two-profiles", Language::Rust, &rust(false));
    assert_eq!(a, b);
}
