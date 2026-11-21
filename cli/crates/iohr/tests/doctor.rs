//! `iohr doctor` end to end: the local problems it finds, the codes it reports, and that a
//! sign-in token never reaches its output (PRD 0208).

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::process::{Command, Output};
use support::kept_env;

/// A home and a config directory of their own: the real machine's bridge, disk and
/// settings never decide a test.
fn doctor(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .arg("doctor")
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("cfg"))
        .envs(kept_env())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "not json ({e}): {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

fn finding<'a>(v: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no `{id}` finding in {v}"))
}

#[test]
fn a_signed_out_machine_fails_with_the_fix_for_it() {
    let home = tempfile::tempdir().unwrap();
    let o = doctor(home.path(), &["--json"]);
    assert_eq!(o.status.code(), Some(1));
    let v = json(&o);
    assert_eq!(v["ok"], false);
    assert_eq!(finding(&v, "auth")["status"], "signed_out");
    assert_eq!(finding(&v, "auth")["offer"], "iohr login");
    assert_eq!(finding(&v, "config")["status"], "ok");
    // Free space is read with statvfs, so only Unix reports it.
    if cfg!(unix) {
        assert_eq!(finding(&v, "disk")["status"], "ok");
    }
    // No bridge is installed in this home, so there is nothing to report about it.
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["id"] != "bridge")
    );
}

#[test]
fn a_config_that_does_not_parse_is_found_and_never_fixed_unasked() {
    let home = tempfile::tempdir().unwrap();
    let cfg = home.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("config.toml"), "[ask\nmode =").unwrap();
    // Even with the standing opt-in nothing edits the file: the fix is an offer.
    let before = std::fs::read(cfg.join("config.toml")).unwrap();
    let o = doctor(home.path(), &["--json", "--fix"]);
    let v = json(&o);
    assert_eq!(finding(&v, "config")["status"], "invalid");
    assert!(
        finding(&v, "config")["offer"]
            .as_str()
            .unwrap()
            .contains("config.toml")
    );
    assert_eq!(std::fs::read(cfg.join("config.toml")).unwrap(), before);
    assert_eq!(v["fixed"].as_array().unwrap().len(), 0);
}

#[test]
fn the_plain_output_names_each_failure_and_its_fix() {
    let home = tempfile::tempdir().unwrap();
    let o = doctor(home.path(), &[]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("FAIL  auth"), "{out}");
    assert!(out.contains("fix: iohr login"), "{out}");
    assert!(out.contains("ok    config"), "{out}");
}

#[test]
fn a_token_in_the_environment_never_reaches_the_output() {
    let home = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(["doctor", "--json", "--verbose"])
        .env_clear()
        .env("HOME", home.path())
        .env("IOHR_CONFIG_DIR", home.path().join("cfg"))
        .env("IOHR_TOKEN", "aaa.bbb.TOKENSIGNATUREMARKER")
        .envs(kept_env())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!all.contains("TOKENSIGNATUREMARKER"), "{all}");
}
