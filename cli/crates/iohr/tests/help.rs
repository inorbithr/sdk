//! The help the command line prints is a contract: three screens are kept as files under
//! `tests/snapshots/`, and a change to them is a change to the CLI that shows up in review.
//! Colour is off (`NO_COLOR`) so the files are plain text. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo nextest run -p iohr --test help`.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn run(env: &[(&str, &str)], args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("IOHR_CONFIG_DIR", home.path().join("config"))
        .env("COLUMNS", "100")
        .stdin(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn plain(args: &[&str]) -> String {
    let o = run(&[("NO_COLOR", "1")], args);
    assert!(o.status.success(), "{args:?}: {:?}", o.stderr);
    String::from_utf8(o.stdout).unwrap()
}

fn snapshot(name: &str, args: &[&str]) {
    let got = plain(args);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; run with UPDATE_SNAPSHOTS=1", path.display()));
    assert_eq!(
        got, want,
        "{name} changed; review it, then UPDATE_SNAPSHOTS=1"
    );
}

#[test]
fn iohr_help_is_grouped() {
    snapshot("iohr_help.txt", &["--help"]);
}

#[test]
fn iohr_project_help() {
    snapshot("iohr_project_help.txt", &["project", "--help"]);
}

#[test]
fn iohr_project_template_help() {
    snapshot(
        "iohr_project_template_help.txt",
        &["project", "template", "--help"],
    );
}

#[test]
fn the_short_help_is_one_line_per_command_and_the_long_help_is_longer() {
    let short = plain(&["-h"]);
    let long = plain(&["--help"]);
    assert!(long.len() > short.len());
    assert!(long.contains("A token is never an argument"), "{long}");
    assert!(!short.contains("A token is never an argument"), "{short}");
    for line in short
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
    {
        assert!(line.len() <= 100, "one line each: {line}");
    }
}

#[test]
fn no_help_line_names_an_rfc_prd_or_adr_number() {
    let t = plain(&["--help"]);
    for word in ["RFC 0", "PRD 0", "ADR 0"] {
        assert!(!t.contains(word), "{word}: {t}");
    }
}

#[test]
fn global_options_sit_under_one_heading_and_packages_is_hidden() {
    let t = plain(&["project", "init", "--help"]);
    assert!(t.contains("\nGlobal options:\n"), "{t}");
    let root = plain(&["--help"]);
    assert!(!root.contains("packages"), "{root}");
}

#[test]
fn colour_follows_no_color_json_and_the_terminal() {
    let esc = '\u{1b}';
    let forced = run(&[("CLICOLOR_FORCE", "1")], &["--help"]);
    assert!(
        String::from_utf8_lossy(&forced.stdout).contains(esc),
        "colour when forced"
    );
    for (why, env, args) in [
        (
            "NO_COLOR",
            vec![("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")],
            vec!["--help"],
        ),
        (
            "--json",
            vec![("CLICOLOR_FORCE", "1")],
            vec!["--json", "--help"],
        ),
        ("a pipe", vec![], vec!["--help"]),
    ] {
        let o = run(&env, &args);
        assert!(!String::from_utf8_lossy(&o.stdout).contains(esc), "{why}");
    }
}

#[test]
fn the_old_names_still_run() {
    // `packages` is hidden, and `project templates` is `project template list`: both parse.
    let o = run(&[("NO_COLOR", "1")], &["project", "templates", "--help"]);
    assert!(o.status.success(), "{:?}", o.stderr);
    let o = run(&[("NO_COLOR", "1")], &["packages", "--help"]);
    assert!(o.status.success(), "{:?}", o.stderr);
}

#[test]
fn usage_mistakes_exit_2() {
    let o = run(&[("NO_COLOR", "1")], &["project", "template", "show"]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let o = run(&[("NO_COLOR", "1")], &["--no-such-flag"]);
    assert_eq!(o.status.code(), Some(2));
}
