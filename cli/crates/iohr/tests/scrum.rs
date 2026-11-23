//! `iohr atlas scrum` with the real binary (RFC 0234): the board a project starts with, the
//! views and guards, `--watch` redrawing on a change, and `sync --dry-run`. A private HOME and
//! a temp project; nothing of the machine is read.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Kills the watcher when the test ends, pass or fail.
struct Watcher(Child);

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scrum(home: &Path, board: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("NO_COLOR", "1")
        .args(["atlas", "scrum", "--board", &board.display().to_string()])
        .args(args);
    c
}

fn run(home: &Path, board: &Path, args: &[&str]) -> Output {
    scrum(home, board, args).output().unwrap()
}

/// A project with the starter board and two cards added through the command itself.
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("atlas/scrum/board.json");
    std::fs::create_dir_all(board.parent().unwrap()).unwrap();
    let (path, text) = {
        let v = serde_json::json!({
            "board": "demo", "updated": "2026-11-23",
            "lists": ["backlog", "ready", "doing", "review", "done", "dropped"],
            "cards": []
        });
        (
            board.clone(),
            serde_json::to_string_pretty(&v).unwrap() + "\n",
        )
    };
    std::fs::write(path, text).unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    for id in ["first-card", "second-card"] {
        let out = run(
            &home,
            &board,
            &[
                "add",
                id,
                "--title",
                &format!("Card {id}"),
                "--size",
                "S",
                "--hours",
                "2",
                "--start",
                "2026-11-24",
                "--due",
                "2026-11-25",
                "--item",
                "do the thing",
                "--accept",
                "the thing works",
                "--test",
                "cargo test thing",
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    (dir, board)
}

#[test]
fn views_guards_and_render_work_on_a_fresh_board() {
    let (dir, board) = project();
    let home = dir.path().join("home");
    assert!(run(&home, &board, &["check"]).status.success());

    let out = run(&home, &board, &["board"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("first-card"));

    // done needs a PR or evidence; a refusal is exit 1 and changes nothing.
    let before = std::fs::read(&board).unwrap();
    let out = run(&home, &board, &["done", "first-card"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(std::fs::read(&board).unwrap(), before);

    // A card that is not on the board is a usage error.
    assert_eq!(run(&home, &board, &["show", "nope"]).status.code(), Some(2));

    // The README is made from the board; a stale one fails `render --check` until rendered.
    let readme = board.with_file_name("README.md");
    std::fs::write(
        &readme,
        "# Demo\n\n<!-- scrum:board:start -->\nstale\n<!-- scrum:board:end -->\n",
    )
    .unwrap();
    assert_eq!(
        run(&home, &board, &["render", "--check"]).status.code(),
        Some(1)
    );
    assert!(run(&home, &board, &["render"]).status.success());
    assert!(run(&home, &board, &["render", "--check"]).status.success());
}

#[test]
fn migrate_moves_section_and_ideas_under_x_and_the_board_passes_check() {
    let (dir, board) = project();
    let home = dir.path().join("home");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&board).unwrap()).unwrap();
    let c = v["cards"][0].as_object_mut().unwrap();
    c.insert("section".into(), "home".into());
    c.insert("ideas".into(), serde_json::json!([1, 2]));
    std::fs::write(&board, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    assert_eq!(
        run(&home, &board, &["check"]).status.code(),
        Some(1),
        "old keys are refused"
    );
    assert!(run(&home, &board, &["migrate"]).status.success());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&board).unwrap()).unwrap();
    assert_eq!(v["cards"][0]["x"]["section"], "home");
    assert_eq!(v["cards"][0]["x"]["ideas"], serde_json::json!([1, 2]));
    assert!(run(&home, &board, &["check"]).status.success());
}

#[test]
fn sync_dry_run_prints_the_commands_and_sends_nothing() {
    let (dir, board) = project();
    let home = dir.path().join("home");
    let out = run(&home, &board, &["sync", "--dry-run", "--repo", "acme/demo"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("scrum:acme/demo"), "{text}");
    assert!(text.contains("item add \"$PLAN\" 'first-card'"), "{text}");
}

#[test]
fn watch_prints_the_board_again_when_board_json_changes() {
    let (dir, board) = project();
    let home = dir.path().join("home");
    let mut child = scrum(&home, &board, &["--watch", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let _w = Watcher(child);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    let first = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&first).expect("one JSON line per frame");
    assert!(first.contains("first-card"), "{first}");
    assert!(
        rx.recv_timeout(Duration::from_millis(1500)).is_err(),
        "no redraw without a change"
    );

    let st = run(&home, &board, &["move", "first-card", "ready"]);
    assert!(
        st.status.success(),
        "{}",
        String::from_utf8_lossy(&st.stderr)
    );
    let second = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let w: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_ne!(v, w, "the new frame shows the move");
}

#[test]
fn watch_refuses_a_command_that_is_not_a_view() {
    let (dir, board) = project();
    let home = dir.path().join("home");
    let out = run(&home, &board, &["--watch", "show", "first-card"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("--watch works with board, list, next and plan")
    );
}
