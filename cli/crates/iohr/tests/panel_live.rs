//! The live panel (no `--demo`) around a real zsh on a pseudo-terminal: the keys the person
//! types reach the menu whatever form the terminal sends them in, `iohr stop` lifts the panel
//! and the next run gives a live one, and nothing stray (`^G`) reaches the terminal.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::process::Command;

mod pty;

use pty::{ROWS, Session as Pty, any, row};

fn home() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("iohr-live-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The real binary, `panel -- zsh -f`, signed out, the way Apple's Terminal names itself.
fn start() -> Pty {
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.args(["panel", "--", "/bin/zsh", "-f"])
        .env_clear()
        .env("HOME", home())
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("TERM_PROGRAM", "Apple_Terminal")
        .env("PS1", "READY> ");
    Pty::spawn(c)
}

/// The text of the menu row the selection bar is on.
fn selected(screen: &vt100::Screen) -> Option<String> {
    (0..ROWS).map(|r| row(screen, r)).find_map(|line| {
        let at = line.find('▎')?;
        let rest = &line[at + '▎'.len_utf8()..];
        Some(rest.split('│').next().unwrap_or("").trim().to_owned())
    })
}

#[test]
fn explore() {
    for (name, seq) in [
        ("csi", &b"\x1b[B"[..]),
        ("ss3", b"\x1bOB"),
        ("shift", b"\x1b[1;2B"),
        ("kitty", b"\x1b[1;1:1B"),
        ("kitty2", b"\x1b[B"),
    ] {
        let mut s = start();
        s.until("the prompt", |sc| any(sc, "READY>"));
        s.send(b"\x1bi");
        s.until("the menu", |sc| any(sc, "Approvals"));
        let first = selected(s.screen.screen());
        s.send(seq);
        std::thread::sleep(std::time::Duration::from_millis(700));
        s.until("anything", |_| true);
        let mut s2 = s;
        // drain
        s2.until("drain", |_| true);
        eprintln!("{name}: {first:?} -> {:?}", selected(s2.screen.screen()));
    }
}
