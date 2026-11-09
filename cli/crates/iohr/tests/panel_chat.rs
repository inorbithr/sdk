//! The panel's framed chat box (RFC 0212) on the real binary, around a real zsh with the hook
//! sourced and a fake `claude` on PATH: one Enter sends a `?` or `??` question, the progress row
//! shows while the answer runs, Esc and a click on Stop end it, Ctrl-] hides and shows the box,
//! a full-screen program has the screen to itself, and `chat = off` leaves the panel as it was.
//!
//! `IOHR_TEST_BIN` runs the same steps against another build of the binary (the build before the
//! box), which is how these tests are shown failing before the change.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

mod pty;

use pty::{ROWS, Session as Pty, any, row};

fn bin() -> PathBuf {
    std::env::var_os("IOHR_TEST_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_iohr")), PathBuf::from)
}

fn zsh() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    dirs.extend(["/usr/bin", "/bin"].map(PathBuf::from));
    dirs.iter()
        .map(|d| d.join("zsh"))
        .find(|z| z.is_file())
        .unwrap_or_else(|| panic!("zsh is not on PATH: install it (apt-get install zsh)"))
}

const FAKE_CLAUDE_HEAD: &str = r#"#!/bin/sh
O="$HOME/out"
printf '%s\n' "$@" > "$O/claude.args"
echo $$ > "$O/claude.pid"
cat > "$O/claude.stdin"
tool() { printf '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"%s","name":"%s","input":{"file_path":"x"}}]}}\n' "$1" "$2"; }
answer() { printf '{"type":"result","subtype":"success","is_error":false,"result":"%s","duration_ms":50,"num_turns":1}\n' "$1"; }
"#;

/// A throwaway HOME with the hook, a fake `claude` running `body`, and `iohr` on PATH.
struct Rig {
    home: PathBuf,
}

impl Rig {
    fn new(name: &str, body: &str) -> Self {
        // Under /tmp: a unix socket path holds 103 bytes.
        let home = PathBuf::from(format!("/tmp/iohr-chat-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["bin", "out", "data", "work"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let fake = home.join("bin/claude");
        std::fs::write(&fake, format!("{FAKE_CLAUDE_HEAD}{body}\n")).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The hook names the binary that printed it; it must find that name on PATH.
        std::os::unix::fs::symlink(bin(), home.join("bin/iohr")).unwrap();
        let hook = Command::new(bin())
            .args(["shell", "hook", "zsh"])
            .output()
            .unwrap();
        assert!(hook.status.success());
        std::fs::write(home.join("hook.zsh"), hook.stdout).unwrap();
        Self { home }
    }

    fn saw(&self, file: &str) -> String {
        std::fs::read_to_string(self.home.join("out").join(file)).unwrap_or_default()
    }

    /// The panel around `zsh -f` with the hook sourced and the prompt ready; `chat` is the
    /// `IOHR_PANEL_CHAT` value ("" leaves the default).
    fn panel(&self, chat: &str) -> Pty {
        let mut c = Command::new(bin());
        c.args(["panel", "--", zsh().to_str().unwrap(), "-f"])
            .current_dir(self.home.join("work"))
            .env_clear()
            .env("HOME", &self.home)
            .env("IOHR_DATA_DIR", self.home.join("data"))
            .env("IOHR_CONFIG_DIR", self.home.join("config"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.home.join("bin").display()),
            )
            .env("SHELL", zsh())
            .env("TERM", "xterm-256color")
            .env("TERM_PROGRAM", "Apple_Terminal")
            .env("INORBIT_ASKS_PROMPT", "0")
            .env("PS1", "READY> ");
        if !chat.is_empty() {
            c.env("IOHR_PANEL_CHAT", chat);
        }
        let mut s = Pty::spawn(c);
        s.until("the prompt", |sc| any(sc, "READY>"));
        line(
            &mut s,
            &format!("source {}/hook.zsh; echo HOO\"\"KED", self.home.display()),
        );
        s.until("the hook sourced", |sc| any(sc, "HOOKED"));
        s
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn line(s: &mut Pty, text: &str) {
    s.send(text.as_bytes());
    s.send(b"\r");
}

fn settle(s: &mut Pty, ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
    s.until("a quiet terminal", |_| true);
}

const CTRL_RB: &[u8] = &[0x1d];

fn boxed(sc: &vt100::Screen) -> bool {
    any(sc, "⌄ hide ^]")
}

fn folded(sc: &vt100::Screen) -> bool {
    any(sc, "shows the box")
}

fn ready(sc: &vt100::Screen) -> bool {
    any(sc, "?? deeper")
}

fn answering(sc: &vt100::Screen) -> bool {
    any(sc, "■ Stop esc")
}

/// Where the text is on the screen: (row, column), both 0-based, counted in characters.
fn find(sc: &vt100::Screen, text: &str) -> Option<(u16, u16)> {
    (0..ROWS).find_map(|r| {
        let line = row(sc, r);
        let at = line.find(text)?;
        Some((r, u16::try_from(line[..at].chars().count()).unwrap()))
    })
}

/// A left click (press and release) at the 0-based cell, in SGR form.
fn click(s: &mut Pty, (r, c): (u16, u16)) {
    s.send(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", c + 1, r + 1, c + 1, r + 1).as_bytes());
}

const SLOW: &str = r#"tool a Read; sleep 1; tool b Read; sleep 40; answer "never""#;
const QUICK: &str =
    r#"tool a Read; sleep 1; tool b Read; sleep 1; tool c Bash; sleep 1; answer "ANSWER-ONE""#;

#[test]
fn one_enter_sends_a_question_mark_question() {
    let rig = Rig::new("one", QUICK);
    let mut s = rig.panel("");
    s.until("the box at the prompt", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("the box taking the keys", |sc| {
        any(sc, "ask anything about this shell")
    });
    s.send(b"why is the roll slow");
    s.until("the line in the box", |sc| any(sc, "why is the roll slow"));
    s.send(b"\r"); // the one Enter
    s.until("the answer", |sc| any(sc, "ANSWER-ONE"));
    assert!(
        rig.saw("claude.stdin").contains("why is the roll slow"),
        "the question did not reach the agent: {}",
        rig.saw("claude.stdin")
    );
    assert!(
        !any(s.screen.screen(), "no matches found"),
        "a bare ? reached the shell as a glob:\n{}",
        s.screen.screen().contents()
    );
}

#[test]
fn one_enter_sends_a_double_question_mark_question_as_a_deeper_one() {
    let rig = Rig::new("deeper", QUICK);
    let mut s = rig.panel("");
    s.until("the box at the prompt", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("the box taking the keys", |sc| {
        any(sc, "ask anything about this shell")
    });
    s.send(b"?"); // the second ? means deeper
    s.until("the deeper mark", |sc| any(sc, "?? "));
    s.send(b"and the build");
    s.send(b"\r");
    s.until("the answer", |sc| any(sc, "ANSWER-ONE"));
    assert!(rig.saw("claude.stdin").contains("and the build"));
}

#[test]
fn the_progress_row_shows_the_files_read_while_the_answer_runs() {
    let rig = Rig::new("progress", QUICK);
    let mut s = rig.panel("");
    s.until("the box at the prompt", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("focus", |sc| any(sc, "ask anything about this shell"));
    s.send(b"what runs\r");
    s.until("the Stop button", answering);
    s.until("a counted step", |sc| any(sc, "reading 2 files"));
    s.until("the answer", |sc| any(sc, "ANSWER-ONE"));
    s.until("the progress row gone", |sc| !answering(sc));
    let args = rig.saw("claude.args");
    assert!(
        args.contains("stream-json") && args.contains("--verbose"),
        "the run did not stream: {args}"
    );
}

/// Whether the process in `claude.pid` is still there.
fn agent_alive(rig: &Rig) -> bool {
    let pid = rig.saw("claude.pid");
    Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn esc_stops_an_answer_and_the_agent_with_it() {
    let rig = Rig::new("esc", SLOW);
    let mut s = rig.panel("");
    s.until("the box at the prompt", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("focus", |sc| any(sc, "ask anything about this shell"));
    s.send(b"slow one\r");
    s.until("the Stop button", answering);
    s.until("the agent running", |_| agent_alive(&rig));
    s.send(b"\x1b");
    s.until("the answer stopped", |sc| !answering(sc) && ready(sc));
    s.until("the agent gone", |_| !agent_alive(&rig));
    assert!(!any(s.screen.screen(), "never"));
}

#[test]
fn a_click_on_stop_stops_the_answer() {
    let rig = Rig::new("click", SLOW);
    let mut s = rig.panel("");
    s.until("the box at the prompt", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("focus", |sc| any(sc, "ask anything about this shell"));
    s.send(b"slow one\r");
    s.until("the Stop button", answering);
    let at = find(s.screen.screen(), "■ Stop esc").unwrap();
    click(&mut s, (at.0, at.1 + 2));
    s.until("the answer stopped", |sc| !answering(sc) && ready(sc));
    s.until("the agent gone", |_| !agent_alive(&rig));
}

#[test]
fn ctrl_close_bracket_hides_the_box_and_brings_it_back() {
    let rig = Rig::new("hide", QUICK);
    let mut s = rig.panel("");
    s.until("the box", |sc| boxed(sc) && ready(sc));
    s.send(CTRL_RB);
    s.until("the box folded to its chip", |sc| folded(sc) && !boxed(sc));
    // The shell has its rows back and still works.
    line(&mut s, "echo STILL\"\"ALIVE");
    s.until("the shell answering", |sc| any(sc, "STILLALIVE"));
    s.send(CTRL_RB);
    s.until("the box back", |sc| boxed(sc) && !folded(sc));
    line(&mut s, "echo STILL\"\"HERE");
    s.until("the shell answering with the box", |sc| {
        any(sc, "STILLHERE")
    });
    assert!(boxed(s.screen.screen()));
}

#[test]
fn a_click_on_hide_and_on_the_chip_folds_and_shows_the_box() {
    let rig = Rig::new("clicks", QUICK);
    let mut s = rig.panel("");
    s.until("the box", |sc| boxed(sc) && ready(sc));
    let at = find(s.screen.screen(), "⌄ hide ^]").unwrap();
    click(&mut s, (at.0, at.1 + 2));
    s.until("the box folded", |sc| folded(sc) && !boxed(sc));
    let chip = find(s.screen.screen(), "shows the box").unwrap();
    click(&mut s, (chip.0, chip.1));
    s.until("the box back", |sc| boxed(sc) && !folded(sc));
}

#[test]
fn a_hidden_box_leaves_the_question_mark_in_the_line() {
    let rig = Rig::new("inline", QUICK);
    let mut s = rig.panel("hidden");
    s.until("the chip", folded);
    s.send(b"?");
    settle(&mut s, 300);
    assert!(
        any(s.screen.screen(), "READY> ?"),
        "the ? did not land in the line:\n{}",
        s.screen.screen().contents()
    );
    assert!(!boxed(s.screen.screen()));
}

#[test]
fn chat_off_leaves_the_panel_without_a_box_or_a_chip() {
    let rig = Rig::new("off", QUICK);
    let mut s = rig.panel("off");
    settle(&mut s, 500);
    assert!(!boxed(s.screen.screen()) && !folded(s.screen.screen()));
    s.send(CTRL_RB);
    settle(&mut s, 300);
    assert!(!boxed(s.screen.screen()) && !folded(s.screen.screen()));
}

#[test]
fn a_full_screen_program_has_the_screen_and_the_box_comes_back() {
    let rig = Rig::new("alt", QUICK);
    let mut s = rig.panel("");
    s.until("the box", |sc| boxed(sc) && ready(sc));
    line(
        &mut s,
        "printf '\\e[?1049h\\e[2JFULLSCREEN'; sleep 2; printf '\\e[?1049l'",
    );
    s.until("the program's screen without the box", |sc| {
        sc.alternate_screen() && any(sc, "FULLSCREEN") && !boxed(sc)
    });
    s.until("the box back after the program", |sc| {
        !sc.alternate_screen() && boxed(sc) && ready(sc)
    });
}

#[test]
fn shift_tab_changes_the_answer_mode_in_the_box() {
    let rig = Rig::new("mode", QUICK);
    let mut s = rig.panel("");
    s.until("the box", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("focus", |sc| any(sc, "ask anything about this shell"));
    let before = row(s.screen.screen(), ROWS - 4);
    s.send(b"\x1b[Z");
    s.until("another mode on the title", |sc| {
        row(sc, ROWS - 4) != before
    });
}

#[test]
fn a_question_stays_the_persons_after_a_stop_and_the_box_asks_again() {
    let rig = Rig::new("again", SLOW);
    let mut s = rig.panel("");
    s.until("the box", |sc| boxed(sc) && ready(sc));
    s.send(b"?");
    s.until("focus", |sc| any(sc, "ask anything about this shell"));
    s.send(b"first\r");
    s.until("the Stop button", answering);
    s.send(b"\x1b");
    s.until("stopped", |sc| !answering(sc) && ready(sc));
    // Up recalls the last question.
    s.send(b"?");
    s.until("focus", |sc| any(sc, "ask anything about this shell"));
    s.send(b"\x1b[A");
    s.until("the last question in the line", |sc| any(sc, "first"));
}
