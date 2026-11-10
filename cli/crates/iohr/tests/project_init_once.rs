//! `iohr project init` at a terminal asks where to keep the data and whether to copy once
//! across the review and the run, shows the crawl notice once, and takes a folder the person
//! types (PRD 0210): the binary on a pty with a private HOME, the crawl pointed at a port
//! nothing listens on so no test reaches the network.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

mod pty;
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use pty::Session;
use support::kept_env;

const DOWN: &[u8] = b"\x1b[B";
const ENTER: &[u8] = b"\r";

const PLACE: &str = "Where should this project keep its ADRs, notes and crawl?";
const COPY: &str = "Copy what is kept in the old place into the new one?";

fn cmd(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.env_clear()
        .current_dir(home)
        .args(args)
        .envs(kept_env())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TERM", "xterm-256color")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"));
    c
}

fn args<'a>(name: &'a str, dir: &'a str, more: &[&'a str]) -> Vec<&'a str> {
    let mut v = vec![
        "project",
        "init",
        name,
        "--template",
        "blank",
        "--lang",
        "rust",
        "--dir",
        dir,
        "--no-smoke",
        "--no-plan",
        "--no-sync",
    ];
    v.extend_from_slice(more);
    v
}

fn on_screen(s: &vt100::Screen, text: &str) -> bool {
    s.contents().contains(text)
}

/// How often `needle` was drawn as a finished prompt: a question answered prints `◇`, or `o`
/// where the locale is not UTF-8 and cliclack falls back to ASCII (CI, a bare server). The
/// question being asked prints another mark (`◆` or `*`), so it is never counted.
fn answered(s: &Session, needle: &str) -> usize {
    let text = plain(&s.transcript);
    ["\u{25c7}", "o"]
        .iter()
        .map(|mark| text.matches(&format!("{mark}  {needle}")).count())
        .sum()
}

/// The text without colour sequences.
fn plain(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A project whose old home place already holds a note, so the copy question is asked.
fn old_home(h: &Path, name: &str) {
    let at = h.join(".inorbit/projects").join(name).join("memory");
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join("old.md"), "an older note\n").unwrap();
}

/// Drives init to the end: place, copy (when asked), review, run, shape. Returns the session
/// once the process has ended.
fn drive(h: &Path, name: &str, more: &[&str], place_keys: &[&[u8]], path: Option<&str>) -> Session {
    let dir: PathBuf = h.join("w").join(name);
    let a = args(name, dir.to_str().unwrap(), more);
    let mut s = Session::spawn(cmd(h, &a));
    s.until("the place question", |sc| on_screen(sc, PLACE));
    for k in place_keys {
        s.send(k);
    }
    s.send(ENTER);
    if let Some(p) = path {
        s.until("the folder question", |sc| on_screen(sc, "Which folder?"));
        s.send(p.as_bytes());
        s.send(ENTER);
    }
    s.until("the copy question or the review", |sc| {
        on_screen(sc, COPY) || on_screen(sc, "Ready?")
    });
    if on_screen(s.screen.screen(), COPY) {
        s.send(ENTER);
        s.until("the review", |sc| on_screen(sc, "Ready?"));
    }
    s.send(ENTER);
    s.until("the shape question", |sc| {
        on_screen(sc, "What shape is this project?")
    });
    s.send(ENTER);
    let _ = s.exited();
    s
}

#[test]
fn place_and_copy_are_asked_once_across_the_review_and_the_run() {
    let h = tempfile::tempdir().unwrap();
    old_home(h.path(), "once");
    // The old place is the home folder (no choice recorded yet), the new one the project.
    let s = drive(h.path(), "once", &["--no-crawl"], &[], None);
    assert_eq!(answered(&s, PLACE), 1, "{}", s.transcript);
    assert_eq!(answered(&s, COPY), 1, "{}", s.transcript);
    let dir = h.path().join("w/once");
    assert!(
        dir.join(".inorbit/memory/old.md").is_file(),
        "the copy was made once: {}",
        s.transcript
    );
}

#[test]
fn the_crawl_notice_is_shown_once_across_the_review_and_the_run() {
    let h = tempfile::tempdir().unwrap();
    let s = drive(
        h.path(),
        "crawlonce",
        &["--crawl", "http://127.0.0.1:9/"],
        &[],
        None,
    );
    let notices = plain(&s.transcript)
        .matches("This reads the public pages of")
        .count();
    assert_eq!(notices, 1, "{}", s.transcript);
}

#[test]
fn somewhere_else_asks_for_a_folder_and_records_and_uses_it() {
    let h = tempfile::tempdir().unwrap();
    let s = drive(
        h.path(),
        "typed",
        &["--no-crawl"],
        &[DOWN, DOWN],
        Some("~/vault/typed"),
    );
    let dir = h.path().join("w/typed");
    let toml = std::fs::read_to_string(dir.join("inorbit.toml")).unwrap();
    assert!(
        toml.contains("data = \"~/vault/typed\""),
        "{toml}\n{}",
        s.transcript
    );
    assert!(
        h.path().join("vault/typed/memory").is_dir(),
        "{}",
        s.transcript
    );
    assert!(!dir.join(".inorbit/memory").exists(), "{}", s.transcript);
    assert_eq!(answered(&s, PLACE), 1, "{}", s.transcript);
}
