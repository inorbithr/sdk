//! `iohr theme` (RFC 0212): the real binary with a private HOME, and a real zsh on a pty with the
//! hook and the theme turned on. Nothing here opens Terminal.app or touches the real home.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

mod pty;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pty::{COLS, ROWS, Session};

fn iohr() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_iohr"))
}

fn have(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// A private HOME with its own terminal folder.
struct Rig {
    home: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let r = Self {
            home: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(r.term()).unwrap();
        r
    }

    fn term(&self) -> PathBuf {
        self.home.path().join(".inorbit").join("term")
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(iohr());
        c.env_clear()
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join(".config"))
            .env("IOHR_TERM_DIR", self.term())
            .env("PATH", std::env::var_os("PATH").unwrap());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn files(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, base: &Path, into: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, into);
            } else {
                into.push(p.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

#[test]
fn list_show_install_and_off_work_and_say_what_they_write() {
    let rig = Rig::new();

    let o = rig.run(&["theme", "list", "--json"]);
    assert!(o.status.success(), "{o:?}");
    let v: serde_json::Value = serde_json::from_str(&text(&o)).unwrap();
    let ids: Vec<&str> = v["themes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["inorbit", "solarized-dark", "nord", "light"]);

    // Plain output (a pipe): the hex values, no escape codes.
    let o = rig.run(&["theme", "show", "inorbit"]);
    let shown = text(&o);
    assert!(o.status.success() && shown.contains("#ff9e1b"), "{shown}");
    assert!(!shown.contains('\u{1b}'), "{shown:?}");

    // Nothing written yet; install writes exactly the profile and the state.
    assert!(files(&rig.term()).is_empty());
    let o = rig.run(&["theme", "install", "nord", "--json"]);
    assert!(o.status.success(), "{o:?}");
    assert_eq!(files(&rig.term()), ["theme.json", "themes/nord.terminal"]);
    let v: serde_json::Value = serde_json::from_str(&text(&o)).unwrap();
    assert_eq!(v["profile_name"], "InOrbit Nord");
    assert_eq!(v["opened_in_terminal"], false);
    assert_eq!(v["prompt"], "off");

    // The profile is a property list macOS can read.
    if have("plutil") {
        let p = rig.term().join("themes").join("nord.terminal");
        let r = Command::new("plutil").arg("-lint").arg(&p).output().unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    }

    // An unknown theme is a usage error (exit 2) that writes nothing.
    let before = files(&rig.term());
    let o = rig.run(&["theme", "install", "dracula"]);
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(files(&rig.term()), before);

    // The hook has no prompt until it is asked for, then has it, then not again after `off`.
    let hook = |rig: &Rig| text(&rig.run(&["shell", "hook", "zsh"]));
    assert!(!hook(&rig).contains("_iohr_prompt_precmd"));
    assert!(rig.run(&["theme", "install", "nord", "--prompt"]).status.success());
    assert!(hook(&rig).contains("_iohr_prompt_precmd"));
    assert!(rig.run(&["theme", "off"]).status.success());
    assert!(!hook(&rig).contains("_iohr_prompt_precmd"));
}

/// The colour of the first cell on the screen that holds `glyph`.
fn colour_of(screen: &vt100::Screen, glyph: &str) -> Option<(u8, u8, u8)> {
    for r in 0..ROWS {
        for c in 0..COLS {
            let cell = screen.cell(r, c)?;
            if cell.contents() == glyph {
                return match cell.fgcolor() {
                    vt100::Color::Rgb(r, g, b) => Some((r, g, b)),
                    _ => None,
                };
            }
        }
    }
    None
}

fn zsh_session(rig: &Rig, work: &Path, rc: &str) -> Session {
    let bin = iohr();
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap()
    );
    let mut c = Command::new("zsh");
    c.args(["-f", "-i"])
        .env_clear()
        .current_dir(work)
        .env("HOME", rig.home.path())
        .env("IOHR_TERM_DIR", rig.term())
        .env("XDG_CONFIG_HOME", rig.home.path().join(".config"))
        .env("PATH", path)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("LANG", "en_US.UTF-8")
        .env("INORBIT_ASKS_PROMPT", "0")
        .env("INORBIT_ASK", "0");
    let mut s = Session::spawn(c);
    s.send(rc.as_bytes());
    s
}

#[test]
fn zsh_with_the_hook_and_the_theme_on_draws_the_prompt_in_the_themes_colours() {
    if !have("zsh") || !have("git") {
        return;
    }
    let rig = Rig::new();
    let work = rig.home.path().join("core");
    std::fs::create_dir_all(&work).unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git").args(args).current_dir(&work).output().unwrap();
        assert!(o.status.success(), "{o:?}");
    };
    git(&["init", "-q", "-b", "main"]);

    assert!(rig.run(&["theme", "install", "inorbit", "--prompt"]).status.success());
    let mut s = zsh_session(
        &rig,
        &work,
        "eval \"$(iohr shell hook zsh)\"; clear\n",
    );
    s.until("the themed prompt", |screen| {
        pty::any(screen, "core") && pty::any(screen, "main") && pty::any(screen, "❯")
    });
    // The folder is the theme's blue, the branch its magenta, the arrow the app's amber accent.
    let screen = s.screen.screen();
    assert_eq!(colour_of(screen, "❯"), Some((0xff, 0x9e, 0x1b)));
    assert_eq!(colour_of(screen, "m"), Some((0xc0, 0x84, 0xfc)), "the branch name");

    // A failed command shows the status in red and turns the arrow red.
    s.send(b"false\n");
    s.until("the failed status", |screen| pty::any(screen, "\u{2718} 1"));
    let screen = s.screen.screen();
    assert_eq!(colour_of(screen, "\u{2718}"), Some((0xf8, 0x71, 0x71)));
    s.send(b"exit\n");
}

#[test]
fn a_prompt_the_person_set_is_kept_unless_replace_prompt_was_asked() {
    if !have("zsh") {
        return;
    }
    let rig = Rig::new();
    let work = rig.home.path().to_path_buf();

    assert!(rig.run(&["theme", "install", "nord", "--prompt"]).status.success());
    let mut s = zsh_session(
        &rig,
        &work,
        "PROMPT='mine> '; eval \"$(iohr shell hook zsh)\"; clear\n",
    );
    s.until("the person's own prompt", |screen| pty::any(screen, "mine>"));
    s.send(b"true\n");
    s.until("still the person's prompt", |screen| {
        pty::any(screen, "mine>") && !pty::any(screen, "❯")
    });
    s.send(b"exit\n");

    assert!(rig.run(&["theme", "install", "nord", "--replace-prompt"]).status.success());
    let mut s = zsh_session(
        &rig,
        &work,
        "PROMPT='mine> '; eval \"$(iohr shell hook zsh)\"; clear\n",
    );
    s.until("the themed prompt", |screen| pty::any(screen, "❯"));
    assert!(!pty::any(s.screen.screen(), "mine>"));
    s.send(b"exit\n");
}
