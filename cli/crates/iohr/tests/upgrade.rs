//! `iohr upgrade` end to end against an in-process GitHub: `--check` for a newer and
//! the current release, the install method it reports, and the refusals that leave the
//! installed binary untouched (a checksum that does not match, no attestation).

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(not(windows))]
use iohr::ext::layer::tar_gz;
#[cfg(not(windows))]
use iohr::ext::oci::Digest;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const THIS: &str = env!("CARGO_PKG_VERSION");

/// A copy of the binary in its own directory, as the install script would leave it.
fn installed() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("iohr");
    std::fs::copy(env!("CARGO_BIN_EXE_iohr"), &exe).unwrap();
    (dir, exe)
}

/// What the child needs to run at all: `SYSTEMROOT` is required for sockets on Windows,
/// the temp variables for its download. Nothing that could hold a credential is passed on.
fn kept_env() -> Vec<(String, std::ffi::OsString)> {
    ["SYSTEMROOT", "TEMP", "TMP", "TMPDIR"]
        .into_iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k.to_owned(), v)))
        .collect()
}

fn run(exe: &Path, install_dir: &Path, server: &MockServer, args: &[&str]) -> Output {
    Command::new(exe)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", install_dir)
        .env("USERPROFILE", install_dir)
        .env("IOHR_INSTALL_DIR", install_dir)
        .env("IOHR_UPGRADE_API", server.uri())
        .env("IOHR_UPGRADE_DOWNLOAD", format!("{}/dl", server.uri()))
        .envs(kept_env())
        .output()
        .unwrap()
}

async fn releases(server: &MockServer, tags: &[&str]) {
    let body: Vec<_> = tags
        .iter()
        .map(|t| {
            // A published release: the checksums and an archive for this host.
            let v = t.strip_prefix("iohr/v").unwrap_or(t);
            serde_json::json!({"tag_name": t, "assets": [
                {"name": "SHA256SUMS"},
                {"name": format!("iohr-{v}-{}.tar.gz", host_target())},
            ]})
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// The archive target name of this host, for the release listing (any value on Windows,
/// where no test installs).
fn host_target() -> String {
    #[cfg(not(windows))]
    return target();
    #[cfg(windows)]
    return String::from("none");
}

/// The release target of this host; there is none for Windows yet, so the tests that
/// serve a release only run elsewhere.
#[cfg(not(windows))]
fn target() -> String {
    let arch = std::env::consts::ARCH;
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        _ => "unknown-linux-musl",
    };
    format!("{arch}-{os}")
}

#[tokio::test]
async fn check_reports_a_newer_release_and_the_install_method() {
    let server = MockServer::start().await;
    releases(&server, &["iohr/v99.0.0", "iohr/v0.0.1", "go/v100.0.0"]).await;
    let (dir, exe) = installed();

    let o = run(&exe, dir.path(), &server, &["upgrade", "--check", "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["current"], THIS);
    assert_eq!(v["latest"], "99.0.0");
    assert_eq!(v["update_available"], true);
    assert_eq!(v["method"], "install_script");

    let text = run(&exe, dir.path(), &server, &["upgrade", "--check"]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("99.0.0") && text.contains("available"),
        "{text}"
    );
}

#[tokio::test]
async fn check_says_none_when_this_is_the_newest() {
    let server = MockServer::start().await;
    releases(&server, &[&format!("iohr/v{THIS}"), "iohr/v0.0.1"]).await;
    let (dir, exe) = installed();

    let o = run(&exe, dir.path(), &server, &["upgrade", "--check", "--json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["update_available"], false);

    let o = run(&exe, dir.path(), &server, &["upgrade"]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("newest release"));
}

#[tokio::test]
async fn a_copy_iohr_did_not_install_gets_instructions_and_no_change() {
    let server = MockServer::start().await;
    releases(&server, &["iohr/v99.0.0"]).await;
    let (dir, exe) = installed();
    let other = tempfile::tempdir().unwrap();

    // The install directory is somewhere else, so this copy is not managed.
    let o = run(&exe, other.path(), &server, &["upgrade"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("brew install inorbithr/tap/iohr"), "{err}");
    assert!(exe.exists());
    drop(dir);
}

/// An archive of a program that claims `version`, and its digest.
#[cfg(not(windows))]
fn archive(version: &str) -> (Vec<u8>, Digest) {
    let name = format!("iohr-{version}-{}", target());
    let program = format!("#!/bin/sh\necho 'iohr {version}'\n");
    let tgz = tar_gz(&[(format!("{name}/iohr").as_str(), b'0', program.as_bytes())]);
    let d = Digest::of(&tgz);
    (tgz, d)
}

#[cfg(not(windows))]
async fn serve_release(server: &MockServer, tgz: Vec<u8>, sum: &str, attestations: u16) {
    let file = format!("iohr-99.0.0-{}.tar.gz", target());
    Mock::given(method("GET"))
        .and(path(format!("/dl/iohr/v99.0.0/{file}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(tgz.clone()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dl/iohr/v99.0.0/SHA256SUMS"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{sum}  {file}\n")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/attestations/{}", Digest::of(&tgz).as_str())))
        .respond_with(
            ResponseTemplate::new(attestations)
                .set_body_json(serde_json::json!({"attestations": []})),
        )
        .mount(server)
        .await;
}

/// There is no Windows release yet: `upgrade` says so before it downloads anything, and
/// the installed binary and its directory stay as they were.
#[cfg(windows)]
#[tokio::test]
async fn a_platform_without_a_build_installs_nothing() {
    let server = MockServer::start().await;
    releases(&server, &["iohr/v99.0.0"]).await;
    let (dir, exe) = installed();
    let before = std::fs::read(&exe).unwrap();

    let o = run(&exe, dir.path(), &server, &["upgrade"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("there is no iohr build for"), "{err}");
    assert_eq!(std::fs::read(&exe).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_checksum_that_does_not_match_installs_nothing() {
    let server = MockServer::start().await;
    releases(&server, &["iohr/v99.0.0"]).await;
    let (tgz, _) = archive("99.0.0");
    serve_release(&server, tgz, &"0".repeat(64), 200).await;
    let (dir, exe) = installed();
    let before = std::fs::read(&exe).unwrap();

    let o = run(&exe, dir.path(), &server, &["upgrade"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("checksum mismatch"), "{err}");
    assert_eq!(std::fs::read(&exe).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_release_without_an_attestation_installs_nothing() {
    let server = MockServer::start().await;
    releases(&server, &["iohr/v99.0.0"]).await;
    let (tgz, d) = archive("99.0.0");
    serve_release(&server, tgz, d.hex(), 200).await;
    let (dir, exe) = installed();
    let before = std::fs::read(&exe).unwrap();

    let o = run(&exe, dir.path(), &server, &["upgrade"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("checksum ok") && err.contains("nothing was installed"),
        "{err}"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
