//! The live panel (no `--demo`) around a real shell on a pseudo-terminal, the way a person
//! meets it in a terminal window: the keys the terminal sends reach the menu whatever form they
//! come in, `iohr stop` lifts the panel and running `iohr` again gives a live one, nothing stray
//! (`^G`) reaches the terminal, and a session host that cannot start says why.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

mod pty;

use pty::{ROWS, Session as Pty, any, row};

const BIN: &str = env!("CARGO_BIN_EXE_iohr");

/// The zsh the tests run in, found on PATH (and the two system folders), not at a fixed place:
/// the panel is a zsh feature, so a machine without one fails these tests loudly (CI installs
/// it) instead of skipping the bugs they guard.
fn zsh() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    dirs.extend(["/usr/bin", "/bin"].map(PathBuf::from));
    dirs.iter()
        .map(|d| d.join("zsh"))
        .find(|z| z.is_file())
        .unwrap_or_else(|| panic!("zsh is not on PATH: install it (apt-get install zsh)"))
}

/// A fresh HOME: signed out, no project, no host.
fn home(name: &str) -> PathBuf {
    // Under /tmp, not the long per-user temp folder: a unix socket path holds 103 bytes.
    let dir = PathBuf::from(format!("/tmp/iohr-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A bare `iohr` runs the person's own zsh, whose start the machine can interrupt with a
    // question that eats what the test types next: with no rc file, the new-user menu (Linux);
    // and Ubuntu's /etc/zsh/zshrc runs `compinit`, which asks "Ignore insecure directories?"
    // when a completion directory is group-writable (a CI runner's are). A one-line `.zshrc`
    // and `skip_global_compinit` (the knob that file documents) keep both away; `zsh -f`
    // shells ignore the files.
    std::fs::write(dir.join(".zshenv"), "skip_global_compinit=1\n").unwrap();
    std::fs::write(dir.join(".zshrc"), "PS1='$ '\n").unwrap();
    dir
}

/// The real binary in a pty, signed out, the way Apple's Terminal names itself, with `args`.
fn iohr(home: &Path, args: &[&str]) -> Pty {
    let mut c = Command::new(BIN);
    c.args(args)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("IOHR_DATA_DIR", home.join("data"))
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", zsh())
        .env("TERM", "xterm-256color")
        .env("TERM_PROGRAM", "Apple_Terminal")
        .env("PS1", "READY> ");
    Pty::spawn(c)
}

/// `iohr panel -- zsh -f`: the live panel with no session host, and a shell with a prompt.
fn panel(home: &Path) -> Pty {
    let mut s = iohr(home, &["panel", "--", zsh().to_str().unwrap(), "-f"]);
    s.until("the prompt", |sc| any(sc, "READY>"));
    s
}

fn menu_is_open(sc: &vt100::Screen) -> bool {
    any(sc, "Approvals")
}

/// The text of the menu row the selection bar is on.
fn selected(screen: &vt100::Screen) -> Option<String> {
    (0..ROWS).map(|r| row(screen, r)).find_map(|line| {
        let at = line.find('▎')?;
        let rest = &line[at + '▎'.len_utf8()..];
        Some(rest.split('│').next().unwrap_or("").trim().to_owned())
    })
}

fn type_line(s: &mut Pty, line: &str) {
    s.send(line.as_bytes());
    s.send(b"\r");
}

/// Reads the terminal for `ms` more, so a key that should do nothing has had time to.
fn settle(s: &mut Pty, ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
    s.until("a quiet terminal", |_| true);
}

/// The menu, then one arrow in the form `seq`: the selection must be on another row, and the
/// matching up arrow must bring it back.
fn arrow_moves(seq: &[u8], back: &[u8]) {
    let mut s = panel(&home("arrows"));
    s.send(b"\x1bi");
    s.until("the menu", menu_is_open);
    let first = selected(s.screen.screen()).unwrap();
    s.send(seq);
    s.until("the selection moving down", |sc| {
        selected(sc).is_some_and(|row| row != first)
    });
    s.send(back);
    s.until("the selection moving back up", |sc| {
        selected(sc).as_deref() == Some(first.as_str())
    });
}

#[test]
fn the_arrows_move_the_selection_in_every_form_a_terminal_sends() {
    arrow_moves(b"\x1b[B", b"\x1b[A"); // plain
    arrow_moves(b"\x1bOB", b"\x1bOA"); // application cursor keys
    arrow_moves(b"\x1b[1;2B", b"\x1b[1;2A"); // Shift
    arrow_moves(b"\x1b[1;3B", b"\x1b[1;3A"); // Option
    arrow_moves(b"\x1b[1;1:1B", b"\x1b[1;1:2A"); // kitty keyboard: press, repeat
}

#[test]
fn a_released_arrow_does_not_move_the_selection_twice() {
    let mut s = panel(&home("release"));
    s.send(b"\x1bi");
    s.until("the menu", menu_is_open);
    let first = selected(s.screen.screen()).unwrap();
    s.send(b"\x1b[1;1:3B"); // kitty: the key was released
    settle(&mut s, 500);
    assert_eq!(selected(s.screen.screen()), Some(first));
}

#[test]
fn stop_then_run_again_gives_a_live_panel_not_the_already_in_note() {
    let home = home("restart");
    let mut s = panel(&home);
    s.send(b"\x1bi");
    s.until("the menu", menu_is_open);
    s.send(b"\x1b"); // Esc closes it
    s.until("the menu closed", |sc| !menu_is_open(sc));

    type_line(&mut s, &format!("{BIN} stop"));
    s.until("the panel closed by stop", |sc| {
        any(sc, "the panel is closed")
    });
    // Stopped: the panel is gone, the menu key is not the panel's.
    s.send(b"\x1bi");
    settle(&mut s, 500);
    assert!(!menu_is_open(s.screen.screen()), "the stopped panel opened");

    type_line(&mut s, BIN);
    s.until("the run again", |sc| any(sc, "the panel is on"));
    // The owner's report: after a stop the re-run said "already in" over a dead panel.
    assert!(
        !any(s.screen.screen(), "already in"),
        "{}",
        s.screen.screen().contents()
    );
    s.send(b"\x1bi");
    s.until("a live menu after the re-run", menu_is_open);
}

#[test]
fn running_iohr_in_a_panel_that_is_not_stopped_only_says_it_is_on() {
    let mut s = panel(&home("inside"));
    type_line(&mut s, BIN);
    s.until("the note", |sc| any(sc, "the panel is on"));
    s.send(b"\x1bi");
    s.until("the menu", menu_is_open);
}

#[test]
fn a_stray_ctrl_g_is_not_echoed_under_a_command_while_the_panel_is_stopped() {
    let home = home("bell");
    let mut s = panel(&home);
    type_line(&mut s, &format!("{BIN} stop"));
    s.until("the panel closed by stop", |sc| {
        any(sc, "the panel is closed")
    });
    // `cat` reads in cooked mode: the terminal echoes a control character as `^G`.
    type_line(&mut s, "cat");
    settle(&mut s, 300);
    s.send(&[0x07]);
    settle(&mut s, 500);
    assert!(
        !any(s.screen.screen(), "^G"),
        "Ctrl-G reached the shell:\n{}",
        s.screen.screen().contents()
    );
}

#[test]
fn a_session_host_that_stops_at_once_says_why_not_just_its_exit_status() {
    let home = home("nohost");
    // Bare `iohr` in a terminal joins the session host, starting it; signed out, it cannot run.
    let mut s = iohr(&home, &[]);
    s.until("the note", |sc| any(sc, "no session host"));
    // Read only once the panel's box is painted: it covers the top right corner, and a note
    // printed there is lost under it. A slow machine paints before the read, a fast one after.
    s.until("the box", |sc| any(sc, "◆ iohr"));
    let screen = s.screen.screen().contents();
    assert!(screen.contains("stopped at once"), "{screen}");
    let log = screen
        .split("see ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("host.log");
    assert!(log.contains("host.log"), "{screen}");
    // The reason is on the line, between the status and the log path: the host's own words.
    // The host is a runner of its own (the `session` profile), never the live runner's: with
    // none, its last line names the commands that make it, and the register one in full.
    assert!(
        screen.contains("session host has no profile of its own"),
        "the host's own last line is missing:\n{screen}"
    );
    assert!(
        screen.contains("login --profile session"),
        "the sign-in command is missing:\n{screen}"
    );
    assert!(
        screen.contains("--profile session runner register --name"),
        "the register command is missing:\n{screen}"
    );
    assert!(
        !screen.contains("(exit status: 3); see"),
        "only the exit status, no cause:\n{screen}"
    );
}

#[test]
fn a_rerun_after_stop_that_cannot_start_a_host_says_why_again() {
    let home = home("rerun-nohost");
    // Signed out, the host cannot start: the panel opens without one, with the note.
    let mut s = iohr(&home, &[]);
    s.until("the first note", |sc| any(sc, "no session host:"));
    s.until("the box", |sc| any(sc, "◆ iohr"));
    // The person's own zsh is the shell here (its prompt is theirs): give it a moment to be
    // ready for what is typed next.
    settle(&mut s, 800);
    let notes = |s: &Pty| {
        s.screen
            .screen()
            .contents()
            .matches("no session host:")
            .count()
    };
    assert_eq!(notes(&s), 1, "{}", s.screen.screen().contents());

    type_line(&mut s, &format!("{BIN} stop"));
    s.until("the panel closed by stop", |sc| {
        any(sc, "the panel is closed")
    });
    // The re-run brings the panel back, and the panel tries the host again from its own
    // process: the same note, written into the shell's output.
    type_line(&mut s, BIN);
    s.until("the panel back", |sc| any(sc, "the panel is on"));
    s.until("the note again", |sc| {
        sc.contents().matches("no session host:").count() == 2
    });
}
