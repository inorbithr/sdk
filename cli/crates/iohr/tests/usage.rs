//! A usage error helps: it says what was missing in plain words, shows real examples for the
//! command that failed, points at `--help` and offers one line to get AI help. It exits 2,
//! as before; under `--json` it is the platform's error envelope on stderr, and with no
//! terminal or `NO_COLOR` it is plain text. Stdout stays empty.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::process::{Command, Output, Stdio};
use support::kept_env;

fn run(env: &[(&str, &str)], args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.args(args)
        .env_clear()
        .envs(kept_env())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("IOHR_CONFIG_DIR", home.path().join("config"))
        .env("COLUMNS", "100")
        .stdin(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

#[test]
fn api_with_no_arguments_says_what_is_missing_and_shows_examples() {
    let o = run(&[("NO_COLOR", "1")], &["api"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty(), "{:?}", o.stdout);
    let err = stderr(&o);
    for part in [
        "error: `iohr api` is missing <METHOD> and <PATH>",
        "A path on the API, such as /v1/me",
        "Usage: iohr api [OPTIONS] <METHOD> <PATH>",
        "Examples:",
        "iohr api GET /v1/me",
        "iohr api POST /v1/webhooks/endpoints --input body.json",
        "More: iohr api --help",
        "Ask AI (your own agent). Nothing is sent unless you run this line:",
        "iohr ask 'I ran `iohr api` and iohr said:",
    ] {
        assert!(err.contains(part), "{part:?} missing from:\n{err}");
    }
    assert!(!err.contains('\u{1b}'), "{err:?}");
}

#[test]
fn json_gives_the_error_envelope_on_stderr_and_nothing_on_stdout() {
    let o = run(&[], &["api", "--json"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty(), "{:?}", o.stdout);
    let err = stderr(&o);
    assert_eq!(err.lines().count(), 1, "{err}");
    let v: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(v["code"], "usage");
    assert_eq!(v["error"], "`iohr api` is missing <METHOD> and <PATH>");
    let d = &v["details"][0];
    assert_eq!(d["type"], "usage");
    assert_eq!(d["command"], "iohr api");
    assert_eq!(d["help"], "iohr api --help");
    assert_eq!(d["examples"][0]["command"], "iohr api GET /v1/me");
    assert!(
        d["ask_ai"]
            .as_str()
            .unwrap()
            .starts_with("iohr ask 'I ran `iohr api` and iohr said:")
    );
}

#[test]
fn a_pipe_gets_plain_text_even_with_colour_asked_for() {
    // Not a terminal: no escape codes, `NO_COLOR` or not.
    let o = run(&[("CLICOLOR_FORCE", "1")], &["token", "create"]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(!err.contains('\u{1b}'), "{err:?}");
    assert!(err.contains("`iohr token create` is missing --name <NAME> and --scope <SCOPES>"));
    assert!(err.contains("iohr token create --name ci --scope decisions:read"));
}

#[test]
fn an_unknown_flag_gets_the_same_help() {
    let o = run(
        &[("NO_COLOR", "1")],
        &["--profile", "work", "api", "--nope"],
    );
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("unexpected argument '--nope' found"), "{err}");
    assert!(err.contains("iohr api GET /v1/me"), "{err}");
    assert!(!err.contains("For more information"), "{err}");
}

#[test]
fn an_unknown_command_falls_back_to_the_top_level_examples() {
    let o = run(&[("NO_COLOR", "1")], &["whoamii"]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("whoamii"), "{err}");
    assert!(err.contains("Examples:"), "{err}");
    assert!(err.contains("More: iohr --help"), "{err}");
}

#[test]
fn help_and_version_are_not_errors() {
    let o = run(&[("NO_COLOR", "1")], &["api", "--help"]);
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.contains("Examples:"), "{out}");
    assert!(out.contains("iohr api GET /v1/me"), "{out}");
    assert!(o.stderr.is_empty(), "{:?}", o.stderr);
    let o = run(&[], &["--version"]);
    assert_eq!(o.status.code(), Some(0));
}
