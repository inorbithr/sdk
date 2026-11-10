//! A pseudo-terminal harness for the tests that run the binary on a terminal: the binary's
//! output is read as a terminal would draw it (the `vt100` model). Shared by `panel.rs` and
//! `runner.rs` (`mod pty;`).

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![allow(dead_code, reason = "each test file uses part of the harness")]

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags};
use rustix::termios::Winsize;

pub(crate) const ROWS: u16 = 24;
pub(crate) const COLS: u16 = 80;

/// The binary under a pty of `ROWS` x `COLS`: the master end and the process.
pub(crate) struct Session {
    master: File,
    pub child: Child,
    output: mpsc::Receiver<Vec<u8>>,
    pub screen: vt100::Parser,
    /// Everything the process wrote, as text: for counting how often a line was drawn (the
    /// screen holds only what is visible now).
    pub transcript: String,
}

impl Session {
    /// Runs `command` with the pty as its stdin, stdout and stderr.
    pub(crate) fn spawn(mut command: Command) -> Self {
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
        let stdio = || Stdio::from(slave.try_clone().unwrap());
        let child = command
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
            transcript: String::new(),
        }
    }

    /// Reads the terminal until `done` holds for what is on screen, or panics after ten seconds.
    pub(crate) fn until(&mut self, what: &str, done: impl Fn(&vt100::Screen) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(100)) {
                self.take(&bytes);
            }
            // A frame is whole once the synchronized update closes; keep reading what is queued.
            while let Ok(bytes) = self.output.try_recv() {
                self.take(&bytes);
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

    fn take(&mut self, bytes: &[u8]) {
        self.screen.process(bytes);
        self.transcript.push_str(&String::from_utf8_lossy(bytes));
    }

    pub(crate) fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    /// The exit status once the process has ended (reading the terminal meanwhile), or `None`
    /// after ten seconds.
    pub(crate) fn exited(&mut self) -> Option<ExitStatus> {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            while let Ok(bytes) = self.output.try_recv() {
                self.take(&bytes);
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn row(screen: &vt100::Screen, r: u16) -> String {
    screen.rows(0, COLS).nth(usize::from(r)).unwrap_or_default()
}

/// Whether any row of the screen holds `text`.
pub(crate) fn any(screen: &vt100::Screen, text: &str) -> bool {
    (0..ROWS).any(|r| row(screen, r).contains(text))
}
