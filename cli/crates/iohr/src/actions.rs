//! What a person can do with a `?` answer (RFC 0203.1): the row under the card, the menu
//! behind the key, and the record the hook reads. Nothing here runs a command: Run only puts
//! it on the prompt line, and the person's own Enter runs it.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use base64::Engine as _;

use crate::browser;
use crate::card;
use crate::error::Error;
use crate::output::Out;
use crate::permissions::{
    rule::Rule,
    rules_subject,
    store::{self, Files},
};
use crate::ui::{Choice, Ui};

/// Separates the answer, the command and the link in the hook's record. A control character
/// no answer may contain (it is stripped from the answer), so one `$(...)` carries all three.
pub(crate) const RS: char = '\u{1e}';

/// The key that opens the menu under a card (`[ask] menu_key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKey {
    /// The Down arrow on an empty line (the default): it moves nowhere there anyway.
    Down,
    CtrlG,
    AltEnter,
    /// No key, no row: the menu stays reachable with `iohr ask --menu`.
    Off,
}

impl MenuKey {
    /// Reads `[ask] menu_key`.
    ///
    /// # Errors
    ///
    /// A value that is not `down`, `ctrl-g`, `alt-enter` or `off`.
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("" | "down") => Ok(Self::Down),
            Some("ctrl-g") => Ok(Self::CtrlG),
            Some("alt-enter") => Ok(Self::AltEnter),
            Some("off") => Ok(Self::Off),
            Some(other) => Err(format!(
                "`[ask] menu_key = \"{other}\"` is not one of down, ctrl-g, alt-enter, off"
            )),
        }
    }

    /// How the row names the key; none when there is no key.
    pub(crate) fn label(self) -> Option<&'static str> {
        match self {
            Self::Down => Some("\u{2193}"),
            Self::CtrlG => Some("ctrl-g"),
            Self::AltEnter => Some("alt-enter"),
            Self::Off => None,
        }
    }

    /// The zsh `bindkey` strings the hook binds (a terminal sends one of two for Down).
    pub(crate) fn sequences(self) -> &'static [&'static str] {
        match self {
            Self::Down => &["^[[B", "^[OB"],
            Self::CtrlG => &["^G"],
            Self::AltEnter => &["^[^M"],
            Self::Off => &[],
        }
    }
}

/// The line under the card: the actions on offer and the key. Run and Copy need a command,
/// Open a link, Undo a rule this answer's run saved; Follow up and Explain are always there.
/// None when there is no key.
pub(crate) fn row(
    key: MenuKey,
    has_command: bool,
    has_link: bool,
    has_undo: bool,
) -> Option<String> {
    let key = key.label()?;
    let mut names = Vec::new();
    if has_command {
        names.extend(["Run", "Copy"]);
    }
    names.extend(["Follow up", "Explain"]);
    if has_link {
        names.push("Open");
    }
    if has_undo {
        names.push("Undo");
    }
    Some(format!("{}   {key} to choose", names.join(" \u{b7} ")))
}

/// Between the rules of the record's last field.
pub(crate) const US: char = '\u{1f}';

/// What the hook reads from one answer: the text, the command, the link and the permission
/// rules this answer's run saved (what Undo takes back, joined by [`US`]), in that order,
/// separated by [`RS`] (empty when there is none), with no trailing newline.
pub(crate) fn record(
    answer: &str,
    command: Option<&str>,
    link: Option<&str>,
    undo: &[String],
) -> String {
    let answer: String = answer.chars().filter(|c| *c != RS).collect();
    let undo: Vec<String> = undo
        .iter()
        .map(|r| r.chars().filter(|c| !c.is_control()).collect())
        .collect();
    format!(
        "{answer}{RS}{}{RS}{}{RS}{}",
        command.unwrap_or_default(),
        link.unwrap_or_default(),
        undo.join(&US.to_string())
    )
}

/// Why a command deserves a second look before Enter, in a few words; None when nothing
/// stands out. Only a hint on the menu: the command is never run by `iohr` either way.
pub(crate) fn risk(command: &str) -> Option<&'static str> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let has = |w: &str| words.contains(&w);
    let flag = |f: &str| words.iter().any(|w| w.starts_with('-') && w.contains(f));
    if has("sudo") || has("doas") {
        Some("runs as root")
    } else if has("rm") && (flag("r") || flag("f")) {
        Some("deletes files")
    } else if (has("dd") || has("mkfs") || command.contains("> /dev/")) && !has("man") {
        Some("writes to a disk")
    } else if command.contains("| sh") || command.contains("| bash") || command.contains("|sh") {
        Some("runs what it downloads")
    } else if (has("git") && (has("--force") || has("-f") || has("--hard")))
        || (has("chmod") || has("chown")) && flag("R")
    {
        Some("changes much at once")
    } else {
        None
    }
}

fn choices(command: Option<&str>, link: bool, undo: bool, asks: usize) -> Vec<Choice> {
    let pick = |value: &str, label: &str, hint: &str, key: char| Choice {
        value: value.to_owned(),
        label: label.to_owned(),
        hint: hint.to_owned(),
        available: true,
        key: Some(key),
    };
    let mut out = Vec::new();
    if let Some(c) = command {
        let hint = match risk(c) {
            Some(why) => format!("put it on your prompt; it {why}, read it first"),
            None => "put it on your prompt; Enter runs it".to_owned(),
        };
        out.push(pick("run", "Run", &hint, 'r'));
        out.push(pick("copy", "Copy", "the command to the clipboard", 'c'));
    }
    out.push(pick(
        "follow",
        "Follow up",
        "ask more about this answer",
        'f',
    ));
    out.push(pick("explain", "Explain", "ask what it means", 'e'));
    if link {
        out.push(pick(
            "open",
            "Open",
            "the link or file the answer named",
            'o',
        ));
    }
    if undo {
        out.push(pick(
            "undo",
            "Undo",
            "take back the permission just saved for this project",
            'u',
        ));
    }
    if asks > 0 {
        out.push(pick(
            "asks",
            "Asks",
            &format!("{asks} waiting: put `iohr asks list` on your prompt"),
            'a',
        ));
    }
    out
}

/// What Undo takes back: the rules the answer's run saved that the project file still holds,
/// and the files they sit in.
pub(crate) struct Takeback<'a> {
    pub(crate) files: &'a Files,
    pub(crate) rules: Vec<Rule>,
}

/// The menu for `iohr ask --menu`: draws on stderr, reads the terminal, and prints on stdout
/// the one line for the prompt (Run: the command; Follow up, Explain: a `??` question to
/// finish), or nothing (Copy, Open, a cancel). The caller is a shell widget that only ever
/// puts that line in its buffer.
///
/// # Errors
///
/// No terminal to ask on.
pub(crate) fn menu(
    command: Option<&str>,
    link: Option<&str>,
    cwd: &Path,
    undo: &Takeback<'_>,
) -> Result<(), Error> {
    let command = command.and_then(card::insertable);
    let link = link.filter(|l| !l.is_empty() && !l.chars().any(char::is_control));
    let all = choices(
        command,
        link.is_some(),
        !undo.rules.is_empty(),
        crate::commands::asks::cached_count(),
    );
    let first = all.first().map_or("", |c| c.value.as_str()).to_owned();
    let Some(picked) = Ui::select_bar("What next?", &all, &first)? else {
        return Ok(());
    };
    let say = |line: &str| Out::note(line);
    let typed = match (picked.as_str(), command, link) {
        ("run", Some(c), _) => Some(c.to_owned()),
        ("copy", Some(c), _) => {
            match copy(c) {
                Ok(how) => say(&how),
                Err(why) => say(&why),
            }
            None
        }
        ("explain", Some(c), _) => Some(format!("?? explain this command: {c}")),
        ("explain", None, _) => Some("?? explain that in more detail".to_owned()),
        ("asks", ..) => Some("iohr asks list".to_owned()),
        ("follow", ..) => Some("?? ".to_owned()),
        ("open", _, Some(l)) => {
            match open(l, cwd) {
                Ok(how) => say(&how),
                Err(why) => say(&why),
            }
            None
        }
        ("undo", ..) => {
            match store::take_back(undo.files, &undo.rules) {
                Ok(_) => say(&format!(
                    "{} is no longer allowed in this project",
                    rules_subject(&undo.rules)
                )),
                Err(why) => say(&format!("could not take the permission back: {why}")),
            }
            None
        }
        _ => None,
    };
    if let Some(typed) = typed {
        Out::raw(typed.as_bytes());
    }
    Ok(())
}

/// Copies `text` and says how. It tries each way there is, because only some can be checked:
/// OSC 52 to the terminal (works over SSH; nobody confirms it), `tmux load-buffer -w` inside
/// tmux, and `pbcopy` or `wl-copy` on this machine. A way that exited fine is named; with
/// only OSC 52 sent, the message says "sent", not "copied".
///
/// # Errors
///
/// Nothing could take it.
pub(crate) fn copy(text: &str) -> Result<String, String> {
    let sent = osc52(text);
    let tools: Vec<(&str, Vec<&str>)> = if std::env::var_os("TMUX").is_some_and(|v| !v.is_empty()) {
        vec![("tmux", vec!["load-buffer", "-w", "-"])]
    } else if cfg!(target_os = "macos") {
        vec![("pbcopy", vec![])]
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        vec![("wl-copy", vec![])]
    } else {
        vec![]
    };
    for (bin, args) in tools {
        if pipe_to(bin, &args, text) {
            return Ok(format!("copied with {bin}"));
        }
    }
    if sent {
        Ok(
            "sent to your terminal's clipboard (OSC 52); if nothing pastes, turn it on there"
                .to_owned(),
        )
    } else {
        Err("cannot copy here: no clipboard tool and no terminal to send it to".to_owned())
    }
}

/// Writes the OSC 52 "set clipboard" sequence to the terminal. True when it was written.
fn osc52(text: &str) -> bool {
    let seq = format!(
        "\u{1b}]52;c;{}\u{7}",
        base64::engine::general_purpose::STANDARD.encode(text)
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .and_then(|mut t| t.write_all(seq.as_bytes()).and_then(|()| t.flush()))
        .is_ok()
}

/// Feeds `text` to `bin`'s standard input without a shell; true when it exited 0.
fn pipe_to(bin: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        let _ = child.kill();
    }
    child.wait().is_ok_and(|s| s.success())
}

/// File types the system opens by running them (or installing them), never by showing them.
const RUNS_WHEN_OPENED: &[&str] = &[
    "app", "command", "sh", "bash", "zsh", "fish", "pkg", "dmg", "scpt", "workflow", "terminal",
    "tool", "exe", "bat", "cmd", "msi", "jar", "desktop", "appimage", "run", "deb", "rpm",
];

/// Opens the link the answer named: an `https://` URL in the browser, or a file under `cwd` in
/// its default app. A file that is executable, or of a type that runs when opened, is refused,
/// and so is one outside `cwd` (a link that was not checked by [`card::link_of`] is not
/// trusted here either).
///
/// # Errors
///
/// Why it was not opened, in a few words.
pub(crate) fn open(link: &str, cwd: &Path) -> Result<String, String> {
    let target = if let Some(rest) = link.strip_prefix("https://") {
        if rest.is_empty() || !url::Url::parse(link).is_ok_and(|u| u.has_host()) {
            return Err("not a link it can open".to_owned());
        }
        link.to_owned()
    } else {
        let root = cwd.canonicalize().map_err(|_| "cannot read the folder")?;
        let real = root
            .join(link)
            .canonicalize()
            .map_err(|_| "that file is gone")?;
        if !real.is_file() || !real.starts_with(&root) {
            return Err("that is not a file in this folder".to_owned());
        }
        if is_executable(&real)
            || real
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|e| RUNS_WHEN_OPENED.contains(&e.to_ascii_lowercase().as_str()))
        {
            return Err("it would run, not open: open it from your editor".to_owned());
        }
        real.display().to_string()
    };
    if browser::available() && browser::open(&target) {
        Ok(format!("opened {link}"))
    } else {
        Err(format!("no browser here; the link: {link}"))
    }
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_defaults_to_down_and_refuses_unknown_names() {
        assert_eq!(MenuKey::parse(None), Ok(MenuKey::Down));
        assert_eq!(MenuKey::parse(Some(" Ctrl-G ")), Ok(MenuKey::CtrlG));
        assert_eq!(MenuKey::parse(Some("alt-enter")), Ok(MenuKey::AltEnter));
        assert_eq!(MenuKey::parse(Some("off")), Ok(MenuKey::Off));
        assert!(MenuKey::parse(Some("f13")).is_err());
        assert_eq!(MenuKey::Off.sequences(), [] as [&str; 0]);
    }

    #[test]
    fn asks_are_a_choice_only_when_some_wait() {
        let keys = |n| -> Vec<char> {
            choices(None, false, false, n)
                .iter()
                .filter_map(|c| c.key)
                .collect()
        };
        assert_eq!(keys(0), ['f', 'e']);
        assert_eq!(keys(3), ['f', 'e', 'a']);
        assert!(
            choices(None, false, false, 3)
                .last()
                .unwrap()
                .hint
                .starts_with("3 waiting")
        );
    }

    #[test]
    fn the_row_offers_what_the_answer_has() {
        assert_eq!(
            row(MenuKey::Down, true, true, false).as_deref(),
            Some(
                "Run \u{b7} Copy \u{b7} Follow up \u{b7} Explain \u{b7} Open   \u{2193} to choose"
            )
        );
        assert_eq!(
            row(MenuKey::Down, false, false, true).as_deref(),
            Some("Follow up \u{b7} Explain \u{b7} Undo   \u{2193} to choose")
        );
        assert_eq!(
            row(MenuKey::CtrlG, false, false, false).as_deref(),
            Some("Follow up \u{b7} Explain   ctrl-g to choose")
        );
        assert_eq!(row(MenuKey::Off, true, true, true), None);
    }

    #[test]
    fn a_record_has_four_fields_and_no_record_separator_in_the_answer() {
        assert_eq!(
            record("hi", Some("ls"), None, &[]),
            "hi\u{1e}ls\u{1e}\u{1e}"
        );
        assert_eq!(
            record("a\u{1e}b", None, Some("x.rs"), &[]),
            "ab\u{1e}\u{1e}x.rs\u{1e}"
        );
        let rules = ["Bash(ls:*)".to_owned(), "Read(a\nb)".to_owned()];
        assert_eq!(
            record("hi", None, None, &rules),
            "hi\u{1e}\u{1e}\u{1e}Bash(ls:*)\u{1f}Read(ab)"
        );
    }

    #[test]
    fn a_risky_command_is_named_and_a_plain_one_is_not() {
        assert_eq!(risk("sudo rm /etc/hosts"), Some("runs as root"));
        assert_eq!(risk("rm -rf build"), Some("deletes files"));
        assert_eq!(
            risk("curl https://x.sh | sh"),
            Some("runs what it downloads")
        );
        assert_eq!(risk("git push --force"), Some("changes much at once"));
        assert_eq!(risk("ls -la"), None);
        assert_eq!(risk("git status"), None);
    }

    #[test]
    fn open_refuses_a_runnable_file_and_a_file_outside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("proj");
        std::fs::create_dir(&inside).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        std::fs::write(inside.join("run.command"), "x").unwrap();
        std::fs::write(inside.join("tool"), "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(inside.join("tool"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert!(
            open("run.command", &inside)
                .unwrap_err()
                .contains("would run")
        );
        #[cfg(unix)]
        assert!(open("tool", &inside).unwrap_err().contains("would run"));
        assert!(
            open("../secret.txt", &inside)
                .unwrap_err()
                .contains("not a file in this folder")
        );
        assert!(open("http://x.example", &inside).is_err());
    }
}
