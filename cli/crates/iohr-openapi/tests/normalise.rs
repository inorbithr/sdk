//! The Rust normaliser against the sync tool: on the same raw document, the public
//! filter, N1 and the checks of what the platform states itself give the same bytes
//! `tools/spec-sync.py` wrote. The fixture pair is regenerated with
//! `mise run cli:normalise-golden`; its raw document is the live one `spec/` came from.

#![allow(
    clippy::unwrap_used,
    reason = "a fixture that does not load fails the test"
)]

use std::path::{Path, PathBuf};

use iohr_openapi::{cut_hash, normalise, public_only, render_json};
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/normalise")
}

fn read(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join(name)).unwrap()).unwrap()
}

#[test]
fn the_rust_normaliser_reproduces_the_sync_tool_byte_for_byte() {
    let raw = read("raw-public.json");
    let out = normalise(&public_only(&raw).unwrap()).unwrap();
    let expected_openapi =
        std::fs::read_to_string(fixtures().join("expected-openapi.json")).unwrap();
    let expected_problem =
        std::fs::read_to_string(fixtures().join("expected-problem.json")).unwrap();
    assert_eq!(
        render_json(&out.openapi),
        expected_openapi,
        "openapi.json differs from the sync tool's"
    );
    assert_eq!(
        render_json(&out.problem),
        expected_problem,
        "problem.json differs from the sync tool's"
    );
}

#[test]
fn the_committed_spec_matches_when_it_came_from_the_fixture() {
    // spec/SOURCE records the sha256 of the raw document the committed spec came from;
    // when it is the fixture's, the committed files must equal the fixture's expected
    // ones, so the two cannot drift apart silently.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let source = std::fs::read_to_string(repo.join("spec/SOURCE")).unwrap_or_default();
    let raw_bytes = std::fs::read(fixtures().join("raw-public.json")).unwrap();
    let digest = {
        use sha2::Digest as _;
        use std::fmt::Write as _;
        sha2::Sha256::digest(&raw_bytes)
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
    };
    if !source.contains(&format!("sha256: {digest}")) {
        eprintln!(
            "spec/SOURCE came from another document than the fixture; the equality check is skipped"
        );
        return;
    }
    let committed = std::fs::read_to_string(repo.join("spec/openapi.json")).unwrap();
    let expected = std::fs::read_to_string(fixtures().join("expected-openapi.json")).unwrap();
    assert_eq!(committed, expected);
}

#[test]
fn the_hash_vector_is_stable() {
    // The same document and hash are committed to the platform repository, so the
    // gateway's stamp and this crate's local hash agree.
    // The file is the platform's own vector (core: crates/protocol/tests/fixtures/
    // cut-hash.json): `{about, hash, document}`.
    let vector = read("cut-hash.json");
    let expected = vector["hash"].as_str().unwrap();
    assert_eq!(cut_hash(&vector["document"]), expected);
    assert_eq!(
        std::fs::read_to_string(fixtures().join("cut-hash.sha256"))
            .unwrap()
            .trim(),
        expected
    );
}
