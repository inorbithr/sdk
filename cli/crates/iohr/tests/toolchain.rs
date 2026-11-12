//! `iohr project init` and the latest stable of the language's toolchain: the binary with a
//! fake `rustc`, `cargo` and `rustup` first on PATH and a local server (port 0) standing in
//! for static.rust-lang.org. Behind is said in the plan and the report and the upgrade is
//! never run by `init` without a yes; offline, a hung server and `--json` never fail it.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use support::kept_env;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RUST: &str = include_str!("data/toolchain/rust.toml");

fn script(dir: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join(name);
    std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A rustc that says `1.90.0`, a cargo, and a rustup that leaves a marker when it is run.
fn fake_toolchain(home: &Path) -> std::path::PathBuf {
    let bin = home.join("bin");
    script(&bin, "rustc", "echo 'rustc 1.90.0 (abcdef012 2025-09-01)'");
    script(&bin, "cargo", "echo 'cargo 1.90.0'");
    script(
        &bin,
        "rustup",
        &format!("echo \"$*\" > '{}'", home.join("rustup-ran").display()),
    );
    bin
}

fn init(home: &Path, upstream: &str, extra: &[&str]) -> Output {
    init_named(home, "xx", upstream, extra)
}

/// `init` of project `name` (a project folder each, so one run never meets another's files).
fn init_named(home: &Path, name: &str, upstream: &str, extra: &[&str]) -> Output {
    let base = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![home.join("bin")];
    paths.extend(std::env::split_paths(&base));
    let d = home.join("w").join(name);
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args([
            "project",
            "init",
            name,
            "--template",
            "blank",
            "--lang",
            "rust",
            "--dir",
            d.to_str().unwrap(),
            "--no-smoke",
            "--yes",
        ])
        .args(extra)
        .env_clear()
        .envs(kept_env())
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .env("IOHR_TOOLCHAIN_UPSTREAM", upstream)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

async fn rust_says(server: &MockServer, body: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/rust"))
        .respond_with(body)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn behind_is_planned_in_a_dry_run_and_the_upgrade_is_never_run() {
    let h = tempfile::tempdir().unwrap();
    fake_toolchain(h.path());
    let server = MockServer::start().await;
    rust_says(&server, ResponseTemplate::new(200).set_body_string(RUST)).await;
    let uri = server.uri();
    let home = h.path().to_path_buf();
    let o = tokio::task::spawn_blocking(move || init(&home, &uri, &["--dry-run"]))
        .await
        .unwrap();
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        t.contains("would: report the toolchain check: Rust 1.90.0 is behind the latest stable, 1.99.1; upgrade with `rustup update stable`"),
        "{t}"
    );
    assert!(!h.path().join("rustup-ran").exists(), "{t}");
    assert!(!h.path().join("w/xx").exists(), "a dry run writes nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_terminal_the_command_is_printed_and_not_run() {
    let h = tempfile::tempdir().unwrap();
    fake_toolchain(h.path());
    let server = MockServer::start().await;
    rust_says(&server, ResponseTemplate::new(200).set_body_string(RUST)).await;
    let uri = server.uri();
    let home = h.path().to_path_buf();
    let o = tokio::task::spawn_blocking(move || init(&home, &uri, &[]))
        .await
        .unwrap();
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        t.contains("Rust 1.90.0 is behind the latest stable, 1.99.1; upgrade with `rustup update stable` (iohr did not run it)"),
        "{t}"
    );
    assert!(!h.path().join("rustup-ran").exists(), "{t}");
    assert!(
        h.path().join("w/xx").exists(),
        "init still wrote the project"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_is_gone_says_could_not_check_and_init_succeeds() {
    let h = tempfile::tempdir().unwrap();
    fake_toolchain(h.path());
    // A port that was open a moment ago and now refuses.
    let uri = {
        let s = MockServer::start().await;
        s.uri()
    };
    let home = h.path().to_path_buf();
    let o = tokio::task::spawn_blocking(move || init(&home, &uri, &[]))
        .await
        .unwrap();
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        t.contains("could not check whether Rust 1.90.0 is the latest stable"),
        "{t}"
    );
    assert!(!t.contains("1.99.1"), "no guessed version: {t}");
    assert!(!h.path().join("rustup-ran").exists(), "{t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_hangs_is_cut_off_at_the_budget_and_init_succeeds() {
    let h = tempfile::tempdir().unwrap();
    fake_toolchain(h.path());
    let server = MockServer::start().await;
    rust_says(
        &server,
        ResponseTemplate::new(200)
            .set_body_string(RUST)
            .set_delay(Duration::from_secs(30)),
    )
    .await;
    let uri = server.uri();
    let home = h.path().to_path_buf();
    let start = Instant::now();
    let o = tokio::task::spawn_blocking(move || init(&home, &uri, &[]))
        .await
        .unwrap();
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
    assert!(
        t.contains("could not check whether Rust 1.90.0 is the latest stable"),
        "{t}"
    );
    assert!(!t.contains("1.99.1"), "{t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn json_makes_no_request_and_says_nothing_of_the_toolchain() {
    let h = tempfile::tempdir().unwrap();
    fake_toolchain(h.path());
    let server = MockServer::start().await;
    rust_says(&server, ResponseTemplate::new(200).set_body_string(RUST)).await;
    let uri = server.uri();
    let home = h.path().to_path_buf();
    let o = tokio::task::spawn_blocking(move || init(&home, &uri, &["--json"]))
        .await
        .unwrap();
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(!t.contains("latest stable"), "{t}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_current_toolchain_is_said_so_and_off_asks_nobody() {
    let h = tempfile::tempdir().unwrap();
    fake_toolchain(h.path());
    script(
        &h.path().join("bin"),
        "rustc",
        "echo 'rustc 1.99.1 (58cf8f9c3 2026-10-13)'",
    );
    let server = MockServer::start().await;
    rust_says(&server, ResponseTemplate::new(200).set_body_string(RUST)).await;
    let uri = server.uri();
    let home = h.path().to_path_buf();
    let o = tokio::task::spawn_blocking(move || init(&home, &uri, &[]))
        .await
        .unwrap();
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(!t.contains("behind"), "{t}");
    assert!(
        t.contains("Rust 1.99.1 is the latest stable (1.99.1)"),
        "{t}"
    );
    // The same run with the lookup off asks nobody (the one request is the first run's). It is
    // a project of its own: the first run's folder may still be settling (git's background
    // maintenance after the commit), so it is never removed or reused here.
    let home = h.path().to_path_buf();
    let off = tokio::task::spawn_blocking(move || init_named(&home, "yy", "off", &[]))
        .await
        .unwrap();
    assert!(off.status.success(), "{}", text(&off));
    assert!(!text(&off).contains("latest stable"), "{}", text(&off));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
