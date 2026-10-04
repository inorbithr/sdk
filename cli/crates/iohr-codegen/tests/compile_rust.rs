//! The generated surface builds against the runtime, and a call a profile may not
//! make does not compile. Runs with `IOHR_TEST_COMPILE=rust` (or `1`) (the Linux CI job sets it):
//! it writes a throwaway crate and runs `cargo check` twice, which takes a while.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

use std::path::Path;
use std::process::Command;

mod support;

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

fn cargo_check(dir: &Path) -> (bool, String) {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "check",
            "--all-targets",
            "--message-format",
            "short",
            "--quiet",
        ])
        .current_dir(dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status.success(), text)
}

#[test]
fn the_two_profile_surface_compiles_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::Rust) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=rust to run it (CI does); skipping");
        return;
    }
    let (_, files) = support::render("two-profiles", Language::Rust, &Options::default());

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let runtime = repo().join("rust").canonicalize().unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "surface-check"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
inorbithr = {{ path = {runtime:?} }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
        ),
    )
    .unwrap();
    files.write(&root.join("src/iohr"), true).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        r#"pub mod iohr;
use iohr::prelude::*;

/// What any program may write with this surface.
pub async fn fine() -> Result<(), inorbithr::Error> {
    let personal: Client<Personal> = Client::builder().token("t").build()?;
    let _ = personal.me().await?;
    let _ = personal.radar().list_digests(&iohr::RadarListDigestsParams { limit: Some(5), ..Default::default() }).await?;
    let ci: Client<AcmeCi> = Client::builder().token("t").build()?;
    let _ = ci.accounts().get_me().await?;
    let _ = ci.accounts().get_usage("acc_1", &Default::default()).await?;
    let mut events = ci.events().stream_events(&iohr::EventsStreamEventsParams { types: Some("key.created".into()), ..Default::default() }).await?;
    while let Some(event) = events.next().await {
        let _: iohr::StreamEventsResponse = event?;
    }
    Ok(())
}

/// A write, with the body the caller built.
pub async fn create(ci: &Client<AcmeCi>, body: &iohr::CreateEndpointRequest) -> Result<(), inorbithr::Error> {
    let _ = ci.events().create_endpoint(body).await?;
    Ok(())
}
"#,
    )
    .unwrap();
    let (ok, text) = cargo_check(root);
    assert!(ok, "the generated surface does not compile:\n{text}");

    // The personal cut has no usage:read: the call must be refused by the compiler.
    std::fs::write(
        root.join("src/wrong.rs"),
        r#"use crate::iohr::prelude::*;
pub async fn wrong(personal: Client<Personal>) -> Result<(), inorbithr::Error> {
    let _ = personal.accounts().get_usage("acc_1", &Default::default()).await?;
    Ok(())
}
"#,
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub mod iohr;\npub mod wrong;\n").unwrap();
    let (ok, text) = cargo_check(root);
    assert!(!ok, "a call the personal profile may not make compiled");
    assert!(
        text.contains("E0599") || text.contains("trait bound") || text.contains("no method named"),
        "unexpected failure: {text}"
    );

    // Nor may it open the event stream, which needs events:read (design.md section 7).
    std::fs::write(
        root.join("src/wrong.rs"),
        r"use crate::iohr::prelude::*;
pub async fn wrong(personal: Client<Personal>) -> Result<(), inorbithr::Error> {
    let _ = personal.events().stream_events(&Default::default()).await?;
    Ok(())
}
",
    )
    .unwrap();
    let (ok, text) = cargo_check(root);
    assert!(!ok, "a stream the personal profile may not open compiled");
    assert!(
        text.contains("E0599") || text.contains("trait bound") || text.contains("no method named"),
        "unexpected failure: {text}"
    );
}
