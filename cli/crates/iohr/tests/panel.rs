//! The floating terminal panel (ADR 0225) on a real pseudo-terminal: the binary runs a command
//! under `panel --demo --variant float` (stub asks, no network), its output is read as a
//! terminal would draw it (the `vt100` model), and the panel must be at the top right while the
//! command's own lines fill the screen to the bottom row.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags};
use rustix::termios::Winsize;

const ROWS: u16 = 24;
const COLS: u16 = 80;

/// The binary under a pty of `ROWS` x `COLS`: the master end and the process.
struct Session {
    master: File,
    child: Child,
    output: mpsc::Receiver<Vec<u8>>,
    screen: vt100::Parser,
}

impl Session {
    fn start(command: &str) -> Self {
        let master =
            rustix::pty::openpt(rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY)
                .unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let slave: OwnedFd = rustix::fs::open(
            name.to_str().unwrap(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        rustix::termios::tcsetwinsize(
            &master,
            Winsize {
                ws_row: ROWS,
                ws_col: COLS,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let home = tempfile_dir();
        let stdio = || Stdio::from(slave.try_clone().unwrap());
        let child = Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args([
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
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(stdio())
            .stdout(stdio())
            .stderr(stdio())
            .spawn()
            .unwrap();
        drop(slave);
        let mut read = File::from(master.try_clone().unwrap());
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = read.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            master: File::from(master),
            child,
            output,
            screen: vt100::Parser::new(ROWS, COLS, 0),
        }
    }

    /// Reads the terminal until `done` holds for what is on screen, or panics after ten seconds.
    fn until(&mut self, what: &str, done: impl Fn(&vt100::Screen) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(100)) {
                self.screen.process(&bytes);
            }
            // A frame is whole once the synchronized update closes; keep reading what is queued.
            while let Ok(bytes) = self.output.try_recv() {
                self.screen.process(&bytes);
            }
            if done(self.screen.screen()) {
                return;
            }
        }
        panic!(
            "timed out waiting for {what}; the screen is:\n{}",
            self.screen.screen().contents()
        );
    }

    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("iohr-panel-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn row(screen: &vt100::Screen, r: u16) -> String {
    screen.rows(0, COLS).nth(usize::from(r)).unwrap_or_default()
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
