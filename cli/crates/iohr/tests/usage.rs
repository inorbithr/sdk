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
    assert!(err.contains("got a flag it does not have"), "{err}");
    assert!(err.contains("iohr api GET /v1/me"), "{err}");
    assert!(!err.contains("For more information"), "{err}");
}

#[test]
fn an_unknown_command_falls_back_to_the_top_level_examples() {
    let o = run(&[("NO_COLOR", "1")], &["whoamii"]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("not an iohr command"), "{err}");
    assert!(!err.contains("whoamii"), "{err}");
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

/// A value that looks like a secret, typed into a slot that does not take it.
const SECRET: &str = "sk_live_SECRET";

/// The line `iohr ask '...'` of a text report.
fn ask_line(err: &str) -> &str {
    err.lines()
        .find(|l| l.trim_start().starts_with("iohr ask '"))
        .unwrap_or_else(|| panic!("no AI line in:\n{err}"))
}

/// Runs `args` as text and as `--json`; the secret is in neither the AI line nor anywhere in
/// the JSON, and the exit code is 2. Returns the text stderr for more checks.
fn never_echoed(secret: &str, args: &[&str]) -> String {
    let text = run(&[("NO_COLOR", "1")], args);
    assert_eq!(text.status.code(), Some(2), "{}", stderr(&text));
    assert!(text.stdout.is_empty(), "{:?}", text.stdout);
    let err = stderr(&text);
    assert!(
        !ask_line(&err).contains(secret),
        "the secret is in the AI line:\n{err}"
    );
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let json = run(&[], &with_json);
    assert_eq!(json.status.code(), Some(2), "{}", stderr(&json));
    assert!(json.stdout.is_empty(), "{:?}", json.stdout);
    let out = stderr(&json);
    assert!(!out.contains(secret), "the secret is in the JSON:\n{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["code"], "usage", "{out}");
    err
}

#[test]
fn an_unexpected_positional_is_not_repeated() {
    let err = never_echoed(SECRET, &["whoami", SECRET]);
    // Not on the terminal either: one rule for every usage error clap finds.
    assert!(!err.contains(SECRET), "{err}");
    assert!(
        err.contains("`iohr whoami` got an unexpected argument"),
        "{err}"
    );
}

#[test]
fn an_invalid_flag_value_is_not_repeated() {
    let err = never_echoed(
        SECRET,
        &[
            "token", "create", "--name", "x", "--scope", "s", "--days", SECRET,
        ],
    );
    assert!(!err.contains(SECRET), "{err}");
    assert!(err.contains("invalid value for --days <DAYS>"), "{err}");
    // The values clap accepts are ours to list; the typed one is not.
    let err = never_echoed(SECRET, &["auth", "token", "--format", SECRET]);
    assert!(err.contains("invalid value for --format <FORMAT>"), "{err}");
    assert!(err.contains("possible values: text and json"), "{err}");
}

#[test]
fn an_unknown_command_is_not_repeated() {
    let err = never_echoed(SECRET, &[SECRET]);
    assert!(!err.contains(SECRET), "{err}");
    assert!(err.contains("not an iohr command"), "{err}");
    // A nested one, clap's own error.
    let err = never_echoed(SECRET, &["token", SECRET]);
    assert!(!err.contains(SECRET), "{err}");
    assert!(err.contains("`iohr token` has no such subcommand"), "{err}");
}

#[test]
fn a_usage_error_a_command_raises_is_not_repeated_in_the_ai_line_or_the_json() {
    // `profile use` raises "there is no profile NAME" at run time; the terminal shows it as
    // raised, the line meant to be pasted and the JSON do not.
    let err = never_echoed("sk_live_secret", &["profile", "use", "sk_live_secret"]);
    assert!(err.contains("there is no profile"), "{err}");
    assert!(
        ask_line(&err).contains("the arguments were refused"),
        "{err}"
    );
}

#[test]
fn a_command_that_needs_a_subcommand_gets_the_report_not_clap_help() {
    let o = run(&[("NO_COLOR", "1")], &["token"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty(), "{:?}", o.stdout);
    let err = stderr(&o);
    for part in [
        "error: `iohr token` needs a subcommand",
        "create  Create an API token",
        "revoke  Revoke an API token",
        "Usage: iohr token [OPTIONS] <COMMAND>",
        "Examples:",
        "iohr token create --name ci --scope decisions:read",
        "More: iohr token --help",
        "iohr ask 'I ran `iohr token` and iohr said:",
    ] {
        assert!(err.contains(part), "{part:?} missing from:\n{err}");
    }
    let json = run(&[], &["token", "--json"]);
    assert_eq!(json.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&stderr(&json)).unwrap();
    assert_eq!(v["error"], "`iohr token` needs a subcommand");
}

#[test]
fn the_panel_help_describes_the_live_panel_not_a_design_round() {
    let o = run(&[("NO_COLOR", "1")], &["panel", "--help"]);
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8(o.stdout).unwrap();
    assert_eq!(
        out.lines().next(),
        Some(
            "Runs your shell with the floating InOrbit panel over it (menu, approvals, workloads)."
        ),
        "{out}"
    );
    assert!(!out.contains("design round"), "{out}");
}
