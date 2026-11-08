//! The floating terminal panel (ADR 0225) on a real pseudo-terminal: the binary runs a command
//! under `panel --demo --variant float` (stub asks, no network), its output is read as a
//! terminal would draw it (the `vt100` model), and the panel must be at the top right while the
//! command's own lines fill the screen to the bottom row.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::process::Command;

mod pty;

use pty::{COLS, ROWS, Session as Pty, row};

/// The binary under a pty running `command` in the demo panel.
struct Session;

impl Session {
    fn start(command: &str) -> Pty {
        let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
        c.args([
            "panel",
            "--demo",
            "--variant",
            "float",
            "--",
            "sh",
            "-c",
            command,
        ])
        .env_clear()
        .env("HOME", tempfile_dir())
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor");
        Pty::spawn(c)
    }
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("iohr-panel-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 200 lines of output scroll the screen: the panel stays on row 0 at the right, the command's
/// last line is on the bottom row, and the left of row 0 is still the command's output.
#[test]
fn the_panel_floats_at_the_top_right_while_output_scrolls() {
    let mut s = Session::start("seq 1 200; printf END; sleep 5");
    s.until("the last line and the panel", |screen| {
        row(screen, ROWS - 1).starts_with("END") && row(screen, 1).contains("⌥i menu")
    });
    let screen = s.screen.screen();
    let top = row(screen, 0);
    assert!(top.contains("◆ iohr · stub"), "row 0: {top:?}");
    let body = row(screen, 1);
    assert!(body.contains("3 waiting"), "row 1: {body:?}");
    assert!(body.contains("⌥i menu"), "row 1: {body:?}");
    // Right-aligned: the box ends one column from the edge, so the pill starts well right of 0.
    let start = top.find('╭').map(|b| top[..b].chars().count()).unwrap();
    assert_eq!(start, usize::from(COLS) - 44 - 1, "row 0: {top:?}");
    // The command's own text is still on the left of row 0, and the last lines are intact.
    assert!(
        top.starts_with(|c: char| c.is_ascii_digit()),
        "row 0: {top:?}"
    );
    assert_eq!(row(screen, ROWS - 2).trim_end(), "200");
    assert_eq!(row(screen, ROWS - 1).trim_end(), "END");
    // The panel takes no rows: all of the screen is the command's, 23 numbers above END.
    let numbers: Vec<u32> = (0..ROWS - 1)
        .map(|r| {
            let line = row(screen, r);
            let left: String = line.chars().take(30).collect();
            left.trim().parse::<u32>().unwrap()
        })
        .collect();
    assert_eq!(numbers.last(), Some(&200));
    assert!(numbers.windows(2).all(|w| w[1] == w[0] + 1), "{numbers:?}");
}

/// Option-i (Esc i) opens the menu in place; Esc closes it and the output stays as it was.
#[test]
fn the_menu_opens_with_option_i_and_closes_with_esc() {
    let mut s = Session::start("seq 1 40; printf END; sleep 5");
    s.until("the panel at rest", |screen| {
        row(screen, ROWS - 1).starts_with("END") && row(screen, 1).contains("⌥i menu")
    });
    s.send(b"\x1bi");
    s.until("the menu", |screen| {
        (0..ROWS).any(|r| row(screen, r).contains("Approvals"))
            && (0..ROWS).any(|r| row(screen, r).contains("Stop panel"))
    });
    s.send(b"\x1b");
    s.until("the pill again", |screen| {
        row(screen, 1).contains("⌥i menu")
            && !(0..ROWS).any(|r| row(screen, r).contains("Stop panel"))
    });
    assert_eq!(row(s.screen.screen(), ROWS - 1).trim_end(), "END");
}

/// Workloads opens from the menu with `w`, one row opens in place, and Esc walks back: list,
/// menu, pill. The data under `--demo` is the labelled stub; nothing of the panel is left on
/// screen and the command's own last line is intact.
#[test]
fn workloads_open_one_and_esc_back_leaves_nothing_behind() {
    let mut s = Session::start("seq 1 40; printf END; sleep 5");
    s.until("the panel at rest", |screen| {
        row(screen, ROWS - 1).starts_with("END") && row(screen, 1).contains("⌥i menu")
    });
    let any = |screen: &vt100::Screen, text: &str| (0..ROWS).any(|r| row(screen, r).contains(text));
    s.send(b"\x1bi");
    s.until("the menu with Workloads", |screen| any(screen, "Workloads"));
    s.send(b"w");
    s.until("the Workloads list", |screen| {
        any(screen, "Workloads") && any(screen, "Coordinator") && any(screen, "need you")
    });
    assert!(any(s.screen.screen(), "stub"), "the stub is labelled");
    s.send(b"\r");
    s.until("one workload opened", |screen| {
        any(screen, "Why") && any(screen, "Latest") && any(screen, "Waits")
    });
    s.send(b"\x1b");
    s.until("the list again", |screen| {
        !any(screen, "Latest") && any(screen, "Coordinator")
    });
    s.send(b"\x1b");
    s.until("the menu again", |screen| {
        any(screen, "Stop panel") && !any(screen, "Coordinator service")
    });
    s.send(b"\x1b");
    s.until("the pill again", |screen| {
        row(screen, 1).contains("⌥i menu") && !any(screen, "Stop panel")
    });
    assert_eq!(row(s.screen.screen(), ROWS - 1).trim_end(), "END");
}

/// The Auto mode entry shows the policy (the stub's, labelled), and Approvals hand an approve
/// to the phone; on the stub nothing is sent or changed and the panel says so.
#[test]
fn auto_mode_and_approvals_act_on_the_stub_and_say_so() {
    let mut s = Session::start("seq 1 40; printf END; sleep 5");
    s.until("the panel at rest", |screen| {
        row(screen, ROWS - 1).starts_with("END") && row(screen, 1).contains("⌥i menu")
    });
    let any = |screen: &vt100::Screen, text: &str| (0..ROWS).any(|r| row(screen, r).contains(text));
    s.send(b"\x1bi");
    s.until("the menu", |screen| any(screen, "Auto mode"));
    s.send(b"a");
    s.until("the policy", |screen| {
        any(screen, "On until") && any(screen, "decided by it so far")
    });
    s.send(b"\r");
    s.until("the stub's answer", |screen| {
        any(screen, "stub: nothing was changed")
    });
    s.send(b"\x1b");
    s.until("the menu again", |screen| any(screen, "Stop panel"));
    s.send(b"p");
    s.until("the approvals", |screen| any(screen, "a approve"));
    s.send(b"a");
    s.until("the phone note", |screen| {
        any(screen, "approve on your phone")
    });
    s.send(b"q");
    s.until("the question line", |screen| any(screen, "ask ›"));
    s.send(b"why\r");
    s.until("the stub's answer", |screen| {
        any(screen, "stub: nothing was sent")
    });
    s.send(b"o");
    s.until("the details", |screen| {
        any(screen, "the real details come from")
    });
    assert_eq!(row(s.screen.screen(), ROWS - 1).trim_end(), "END");
}
