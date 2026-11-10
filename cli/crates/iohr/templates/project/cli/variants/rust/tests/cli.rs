//! The passes-on-init test: golden output in text and JSON, exit codes, and a version that
//! carries the commit. The library is driven in process; the binary is run once for real.

use std::process::Command;

use @@MODULE@@::{FAILURE, OK, USAGE, run};

const COMMIT: &str = "0123456789ab";

fn call(args: &[&str]) -> (String, String, u8) {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&args, &mut out, &mut err, COMMIT);
    (String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap(), code)
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(format!("{}/golden/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn golden_text() {
    for (name, args) in [
        ("help.txt", vec!["--help"]),
        ("greet.txt", vec!["greet", "Ada"]),
        ("version.txt", vec!["version"]),
    ] {
        let (out, _, code) = call(&args);
        assert_eq!((code, out), (OK, golden(name)), "{name}");
    }
}

#[test]
fn golden_json() {
    for (name, args) in [
        ("greet.json", vec!["--json", "greet", "Ada"]),
        ("version.json", vec!["version", "--json"]),
    ] {
        let (out, _, code) = call(&args);
        assert_eq!((code, out), (OK, golden(name)), "{name}");
    }
}

#[test]
fn exit_codes() {
    let long = "x".repeat(65);
    for (args, want) in [
        (vec![], USAGE),
        (vec!["greet"], OK),
        (vec!["nope"], USAGE),
        (vec!["greet", "a", "b"], USAGE),
        (vec!["greet", "   "], FAILURE),
        (vec!["greet", long.as_str()], FAILURE),
    ] {
        assert_eq!(call(&args).2, want, "{args:?}");
    }
    assert_eq!(call(&["nope"]).0, "", "a usage error writes to stderr, not stdout");
}

#[test]
fn the_real_binary_carries_the_commit_and_exits_with_the_codes() {
    let bin = env!("CARGO_BIN_EXE_@@NAME@@");
    let version = Command::new(bin).arg("version").output().unwrap();
    let line = String::from_utf8(version.stdout).unwrap();
    let commit = line.trim().rsplit_once("(commit ").map(|(_, c)| c.trim_end_matches(')'));
    let commit = commit.expect("the version line has a commit");
    assert!(
        commit == "unknown" || (commit.len() >= 7 && commit.chars().all(|c| c.is_ascii_hexdigit())),
        "{line}"
    );
    assert_eq!(Command::new(bin).arg("nope").status().unwrap().code(), Some(2));
    assert_eq!(Command::new(bin).args(["greet", " "]).status().unwrap().code(), Some(1));
}

#[test]
fn an_error_does_not_echo_a_hostile_argument() {
    let hostile = format!("\u{1b}[31m{}", "x".repeat(100));
    let (_, err, _) = call(&[hostile.as_str()]);
    assert!(!err.contains('\u{1b}') && err.len() < 200, "{err}");
}
