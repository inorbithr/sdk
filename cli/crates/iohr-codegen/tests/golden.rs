//! Golden tests: each case under `tests/golden/<case>/input/<profile>.json` renders to
//! exactly `tests/golden/<case>/expected/`. `IOHR_UPDATE_GOLDEN=1` rewrites the
//! expected files; review the diff. The compile test builds what they hold.

#![allow(
    clippy::unwrap_used,
    reason = "a fixture that does not load fails the test"
)]

use std::path::{Path, PathBuf};

use iohr_codegen::{Files, RustOptions, RustTarget, Target as _};
use iohr_openapi::Api;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Renders a case with the Rust target.
fn render_case(case: &str, in_crate: bool) -> (Api, Files) {
    let dir = golden().join(case).join("input");
    let mut docs = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let profile = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let doc = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        docs.push((profile, doc));
    }
    let api = Api::from_documents(docs).unwrap();
    let options = RustOptions {
        runtime: "inorbithr".into(),
        in_crate,
    };
    let files = RustTarget.render(&api, &options).unwrap();
    (api, files)
}

fn check(case: &str) {
    let (_, files) = render_case(case, false);
    let expected = golden().join(case).join("expected");
    if std::env::var_os("IOHR_UPDATE_GOLDEN").is_some() {
        files.write(&expected, true).unwrap();
        return;
    }
    let diff = files.diff(&expected).unwrap();
    assert!(
        diff.is_empty(),
        "{case}: the rendered surface differs from tests/golden/{case}/expected ({diff:?}); run with IOHR_UPDATE_GOLDEN=1 and review the diff"
    );
}

#[test]
fn the_public_surface_matches_its_golden_files() {
    check("public");
}

#[test]
fn two_profiles_share_models_and_keep_their_own_operations() {
    check("two-profiles");
    let (api, files) = render_case("two-profiles", false);
    let ops = String::from_utf8(
        files
            .iter()
            .find(|(p, _)| p.ends_with("ops.rs"))
            .unwrap()
            .1
            .to_vec(),
    )
    .unwrap();
    assert!(ops.contains("impl ListDigests for super::profiles::AcmeCi {}"));
    assert!(ops.contains("impl ListDigests for super::profiles::Personal {}"));
    assert!(ops.contains("impl GetUsage for super::profiles::AcmeCi {}"));
    assert!(!ops.contains("impl GetUsage for super::profiles::Personal {}"));
    assert_eq!(
        api.profiles["personal"].hash,
        format!("sha256:{}", "ab".repeat(32))
    );
    assert!(
        files
            .notes()
            .iter()
            .any(|n| n.contains("text/event-stream")),
        "{:?}",
        files.notes()
    );
}

#[test]
fn write_scopes_render_bodies_and_mark_nothing_idempotent() {
    check("write-scopes");
    let (api, files) = render_case("write-scopes", false);
    assert!(api.profiles["hooks"].local);
    let surface = String::from_utf8(
        files
            .iter()
            .find(|(p, _)| p.ends_with("surface.rs"))
            .unwrap()
            .1
            .to_vec(),
    )
    .unwrap();
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
    let (_, a) = render_case("two-profiles", false);
    let (_, b) = render_case("two-profiles", false);
    assert_eq!(a, b);
}
