//! What a person can do with a `?` answer (RFC 0203.1): the row under the card, the menu
//! behind the key, and the record the hook reads. Nothing here runs a command: Run only puts
//! it on the prompt line, and the person's own Enter runs it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine as _;

use crate::access;
use crate::attach;
use crate::browser;
use crate::card;
use crate::error::Error;
use crate::headless::OutputHow;
use crate::output::Out;
use crate::permissions::{
    rule::Rule,
    rules_subject,
    store::{self, Files},
    trust,
};
use crate::prompt;
use crate::steps::Step;
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

/// What the answer offers beyond its command and link (PRD 0187, RFC 0203): the folder Io
/// needs a yes to read, and the next steps the model suggested.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Next<'a> {
    /// Allow: the folder Io asked to read and did not get a yes for.
    pub(crate) allow: Option<&'a Path>,
    /// Numbered steps (Read a folder, Search, Run through the permission card).
    pub(crate) steps: &'a [Step],
}

/// The longest a step's name is drawn in the row under the card.
const STEP_ROW_MAX: usize = 28;

/// A step's name cut to [`STEP_ROW_MAX`] characters for the row.
fn short(label: &str) -> String {
    if label.chars().count() <= STEP_ROW_MAX {
        label.to_owned()
    } else {
        let cut: String = label.chars().take(STEP_ROW_MAX - 3).collect();
        format!("{cut}...")
    }
}

/// What an answer has beyond a command, for the row and the menu.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Extras {
    /// A file or `https://` link the answer named (Open).
    pub(crate) link: bool,
    /// A permission rule this answer's run saved (Undo).
    pub(crate) undo: bool,
    /// The last command failed and the question asks why: its output can be attached.
    pub(crate) attach: bool,
}

/// The line under the card: the actions on offer and the key. Run and Copy need a command,
/// Open a link, Undo a rule this answer's run saved, Attach output a failed command that the
/// question asks about, Allow a folder Io asked to read and the next steps it suggested; Follow
/// up, Explain and Add folder (one more folder for Io to read) are always there. None when
/// there is no key.
pub(crate) fn row(key: MenuKey, has_command: bool, has: Extras, next: &Next<'_>) -> Option<String> {
    let key = key.label()?;
    let mut names: Vec<String> = Vec::new();
    if has_command {
        names.extend(["Run".to_owned(), "Copy".to_owned()]);
    }
    if next.allow.is_some() {
        names.push("Allow".to_owned());
    }
    names.extend(next.steps.iter().map(|s| short(&s.label())));
    if has.attach {
        names.push("Attach output".to_owned());
    }
    names.extend(["Follow up".to_owned(), "Explain".to_owned()]);
    if has.link {
        names.push("Open".to_owned());
    }
    if has.undo {
        names.push("Undo".to_owned());
    }
    names.push("Add folder".to_owned());
    Some(format!("{}   {key} to choose", names.join(" \u{b7} ")))
}

/// `text` without its control characters: it is typed on a prompt or drawn on a terminal.
fn plain(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Between the rules of the record's last field.
pub(crate) const US: char = '\u{1f}';

/// What the hook reads from one answer: the text, the command, the link, the permission
/// rules this answer's run saved (what Undo takes back, joined by [`US`]), the folder Io needs
/// a yes for and the next steps (`kind:value`, joined by [`US`]), in that order, separated by
/// [`RS`] (empty when there is none), with no trailing newline. A hook that reads only the
/// first four still works.
pub(crate) fn record(
    answer: &str,
    command: Option<&str>,
    link: Option<&str>,
    undo: &[String],
    next: &Next<'_>,
) -> String {
    let answer: String = answer.chars().filter(|c| *c != RS).collect();
    let undo: Vec<String> = undo.iter().map(|r| plain(r)).collect();
    let allow = next.allow.map(|f| plain(&f.display().to_string()));
    let allow = allow.unwrap_or_default();
    let steps: Vec<String> = next.steps.iter().map(Step::encode).collect();
    format!(
        "{answer}{RS}{}{RS}{}{RS}{}{RS}{allow}{RS}{}",
        command.unwrap_or_default(),
        link.unwrap_or_default(),
        undo.join(&US.to_string()),
        steps.join(&US.to_string())
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

fn choices(command: Option<&str>, has: Extras, asks: usize, next: &Next<'_>) -> Vec<Choice> {
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
    if let Some(folder) = next.allow {
        out.push(pick(
            "allow",
            "Allow",
            &format!(
                "let Io read {} from now on (secrets stay blocked), then ask again",
                plain(&folder.display().to_string())
            ),
            'l',
        ));
    }
    for (i, step) in next.steps.iter().enumerate() {
        out.push(Choice {
            value: format!("step{i}"),
            label: step.label(),
            hint: step.hint().to_owned(),
            available: true,
            key: None,
        });
    }
    if has.attach {
        out.push(pick(
            "rerun",
            "Re-run with output",
            "run the command again with its output captured; asks first",
            'w',
        ));
        out.push(pick(
            "paste",
            "Paste output",
            "paste what it printed; secrets are redacted before anything is sent",
            'p',
        ));
    }
    out.push(pick(
        "follow",
        "Follow up",
        "ask more about this answer",
        'f',
    ));
    out.push(pick("explain", "Explain", "ask what it means", 'e'));
    if has.link {
        out.push(pick(
            "open",
            "Open",
            "the link or file the answer named",
            'o',
        ));
    }
    if has.undo {
        out.push(pick(
            "undo",
            "Undo",
            "take back the permission just saved for this project",
            'u',
        ));
    }
    out.push(pick(
        "addfolder",
        "Add folder",
        "let Io also read another folder from now on (secrets stay blocked), then ask again",
        'd',
    ));
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

/// What Attach output works on: the failed command, the question to ask again and the private
/// file the output is written to (made by the hook, read by the next ask).
pub(crate) struct Attach<'a> {
    pub(crate) failed: &'a attach::Failed,
    pub(crate) asked: &'a str,
    pub(crate) to: &'a Path,
}

/// After Attach output the line is the question to ask again, then [`RS`] and how the output
/// came (`captured` or `pasted`), so the hook sends it with `--output-kind`.
fn asked_again(a: &Attach<'_>, how: OutputHow) -> String {
    let asked: String = a.asked.chars().filter(|c| !c.is_control()).collect();
    format!("? {asked}{RS}{}", how.name())
}

/// Writes the scrubbed output to the file; the line to put on the prompt, or nothing.
fn keep(a: &Attach<'_>, text: &str, how: OutputHow) -> Option<String> {
    let text = attach::scrub(text);
    if text.trim().is_empty() {
        Out::note("there was no output to send");
        return None;
    }
    match std::fs::write(a.to, &text) {
        Ok(()) => {
            Out::note(&format!(
                "{} lines of output kept, secrets redacted; Enter asks again with them",
                text.lines().count()
            ));
            Some(asked_again(a, how))
        }
        Err(e) => {
            Out::note(&format!("could not keep the output: {}", e.kind()));
            None
        }
    }
}

/// Re-run with output: shows the exact command, asks, and only after a yes runs it once in the
/// folder, for at most [`attach::RERUN_LIMIT`], keeping the end of what it printed.
fn rerun(a: &Attach<'_>, cwd: &Path) -> Option<String> {
    let Some(command) = card::insertable(&a.failed.command) else {
        Out::note("cannot re-run it: the command has control characters");
        return None;
    };
    Out::note(&format!(
        "It would run again in {}: {command}",
        cwd.display()
    ));
    let hint = match risk(command) {
        Some(why) => format!("it {why}; it runs again, read it first"),
        None => format!(
            "runs once, stopped after {} s; the end of its output is sent, secrets redacted",
            attach::RERUN_LIMIT.as_secs()
        ),
    };
    let choice = |value: &str, label: &str, hint: &str, key: char| Choice {
        value: value.to_owned(),
        label: label.to_owned(),
        hint: hint.to_owned(),
        available: true,
        key: Some(key),
    };
    let rows = [
        choice("no", "No", "leave it as it is", 'n'),
        choice("yes", "Yes, run it again", &hint, 'y'),
    ];
    // Esc, a cancel or no terminal are all a no: nothing is run.
    if !matches!(
        Ui::select_bar("Run it again to capture its output?", &rows, "no"),
        Ok(Some(v)) if v == "yes"
    ) {
        return None;
    }
    match attach::rerun(command, cwd, attach::RERUN_LIMIT) {
        Ok(got) => {
            if got.timed_out {
                Out::note(&format!(
                    "stopped after {} s; kept the end of what it printed",
                    attach::RERUN_LIMIT.as_secs()
                ));
            } else if got.exit_code != Some(a.failed.exit_code) {
                let now = got
                    .exit_code
                    .map_or("no status".to_owned(), |c| c.to_string());
                Out::note(&format!(
                    "it exited {now} this time (it exited {} before)",
                    a.failed.exit_code
                ));
            }
            keep(a, &got.text, OutputHow::Captured)
        }
        Err(why) => {
            Out::note(&why);
            None
        }
    }
}

/// Paste output: reads what the person pastes, to Ctrl-D, from the terminal.
fn paste(a: &Attach<'_>) -> Option<String> {
    Out::note("Paste the output, then Ctrl-D on an empty line (Ctrl-C leaves).");
    match attach::paste(std::io::stdin().lock()) {
        Ok(text) => keep(a, &text, OutputHow::Pasted),
        Err(why) => {
            Out::note(&why);
            None
        }
    }
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
    (command, link): (Option<&str>, Option<&str>),
    cwd: &Path,
    undo: &Takeback<'_>,
    attach: Option<&Attach<'_>>,
    (next, asked, user_dir): (&Next<'_>, &str, &Path),
) -> Result<(), Error> {
    let command = command.and_then(card::insertable);
    let link = link.filter(|l| !l.is_empty() && !l.chars().any(char::is_control));
    let has = Extras {
        link: link.is_some(),
        undo: !undo.rules.is_empty(),
        attach: attach.is_some(),
    };
    let all = choices(command, has, crate::commands::asks::cached_count(), next);
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
        ("rerun", ..) => attach.and_then(|a| rerun(a, cwd)),
        ("paste", ..) => attach.and_then(paste),
        ("follow", ..) => Some("?? ".to_owned()),
        ("open", _, Some(l)) => {
            match open(l, cwd) {
                Ok(how) => say(&how),
                Err(why) => say(&why),
            }
            None
        }
        ("allow", ..) => allow(next.allow, asked, user_dir, false),
        ("addfolder", ..) => add_folder(cwd, asked, user_dir),
        (step, ..) if step.starts_with("step") => step[4..]
            .parse::<usize>()
            .ok()
            .and_then(|i| next.steps.get(i))
            .map(Step::question),
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

/// Allow: keeps the yes for `folder` (the same `trusted.toml` the terminal card writes) and
/// returns the line that asks the question again; nothing when it could not be kept. The yes
/// is for that question's folder only (`standing` is false); Add folder keeps it for every ask.
fn allow(folder: Option<&Path>, asked: &str, user_dir: &Path, standing: bool) -> Option<String> {
    let folder = folder?;
    let kept = if standing {
        trust::add_standing(user_dir, folder)
    } else {
        trust::remember(user_dir, folder)
    };
    match kept {
        Ok(file) => Out::note(&format!(
            "Io may read {} {}, secrets stay blocked; take it back by removing it from {}",
            plain(&folder.display().to_string()),
            if standing {
                "in every ask from now on"
            } else {
                "when you ask about it from now on"
            },
            file.display()
        )),
        Err(why) => {
            Out::note(&format!("could not keep the permission: {why}"));
            return None;
        }
    }
    let q = plain(asked);
    (!q.trim().is_empty()).then(|| format!("? {}", q.trim()))
}

/// Add folder: asks for a folder (a leading `~/` is the home folder; one that is missing,
/// cannot be listed or is a secret place is refused where it is typed) and keeps the yes the
/// way Allow does. A cancelled question gives nothing.
fn add_folder(cwd: &Path, asked: &str, user_dir: &Path) -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let (at, h) = (cwd.to_path_buf(), home.clone());
    let ui = Ui::detect(false);
    let typed = if ui.fancy() {
        ui.input("Folder for Io to read", "", move |v| {
            if v.is_empty() {
                return Err("type a folder".to_owned());
            }
            folder_to_add(v, &at, h.as_deref()).map(|_| ())
        })
        .ok()?
    } else {
        prompt::line("Folder for Io to read").ok()?
    };
    let real = match folder_to_add(&typed, cwd, home.as_deref()) {
        Ok(real) => real,
        Err(why) => {
            Out::note(&format!("{}: {why}", plain(&typed)));
            return None;
        }
    };
    allow(Some(&real), asked, user_dir, true)
}

/// The folder `typed` names, as Io may read it, or why not: the same check
/// ([`access::allowable_at`]) every other way of granting a folder goes through.
pub(crate) fn folder_to_add(
    typed: &str,
    cwd: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let at = access::expand_home(Path::new(typed.trim()), home);
    access::allowable_at(&at, cwd, home, access::Source::Trust).map_err(|why| why.to_string())
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
            choices(None, Extras::default(), n, &Next::default())
                .iter()
                .filter_map(|c| c.key)
                .collect()
        };
        assert_eq!(keys(0), ['f', 'e', 'd']);
        assert_eq!(keys(3), ['f', 'e', 'd', 'a']);
        assert!(
            choices(None, Extras::default(), 3, &Next::default())
                .last()
                .unwrap()
                .hint
                .starts_with("3 waiting")
        );
    }

    #[test]
    fn the_row_offers_what_the_answer_has() {
        assert_eq!(
            row(
                MenuKey::Down,
                true,
                Extras {
                    link: true,
                    ..Extras::default()
                },
                &Next::default()
            )
            .as_deref(),
            Some(
                "Run \u{b7} Copy \u{b7} Follow up \u{b7} Explain \u{b7} Open \u{b7} Add folder   \u{2193} to choose"
            )
        );
        assert_eq!(
            row(
                MenuKey::Down,
                false,
                Extras {
                    undo: true,
                    ..Extras::default()
                },
                &Next::default()
            )
            .as_deref(),
            Some("Follow up \u{b7} Explain \u{b7} Undo \u{b7} Add folder   \u{2193} to choose")
        );
        assert_eq!(
            row(MenuKey::CtrlG, false, Extras::default(), &Next::default()).as_deref(),
            Some("Follow up \u{b7} Explain \u{b7} Add folder   ctrl-g to choose")
        );
        assert_eq!(
            row(
                MenuKey::Off,
                true,
                Extras {
                    link: true,
                    undo: true,
                    attach: true
                },
                &Next::default()
            ),
            None
        );
    }

    #[test]
    fn a_record_has_six_fields_and_no_record_separator_in_the_answer() {
        assert_eq!(
            record("hi", Some("ls"), None, &[], &Next::default()),
            "hi\u{1e}ls\u{1e}\u{1e}\u{1e}\u{1e}"
        );
        assert_eq!(
            record("a\u{1e}b", None, Some("x.rs"), &[], &Next::default()),
            "ab\u{1e}\u{1e}x.rs\u{1e}\u{1e}\u{1e}"
        );
        let rules = ["Bash(ls:*)".to_owned(), "Read(a\nb)".to_owned()];
        assert_eq!(
            record("hi", None, None, &rules, &Next::default()),
            "hi\u{1e}\u{1e}\u{1e}Bash(ls:*)\u{1f}Read(ab)\u{1e}\u{1e}"
        );
    }

    #[test]
    fn the_row_and_the_bar_offer_allow_and_the_next_steps_as_actions() {
        let steps = [Step::Read("api".into()), Step::Search("TODO".into())];
        let folder = Path::new("/home/me");
        let next = Next {
            allow: Some(folder),
            steps: &steps,
        };
        assert_eq!(
            row(MenuKey::Down, false, Extras::default(), &next).as_deref(),
            Some(
                "Allow \u{b7} Read api/ \u{b7} Search for TODO \u{b7} Follow up \u{b7} Explain \u{b7} Add folder   \u{2193} to choose"
            )
        );
        let c = choices(None, Extras::default(), 0, &next);
        let values: Vec<&str> = c.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(
            values,
            ["allow", "step0", "step1", "follow", "explain", "addfolder"]
        );
        assert_eq!(c[0].key, Some('l'));
        assert!(c[0].hint.contains("/home/me") && c[0].hint.contains("secrets stay blocked"));
        assert_eq!(c[1].label, "Read api/");
        assert_eq!(c[2].label, "Search for TODO");
        // The bar numbers them; a step is never a bare shell line to copy.
        assert!(c.iter().all(|c| !c.hint.contains("grep")));
        // Nothing to allow and no step: the bar is as it was.
        let none = choices(None, Extras::default(), 0, &Next::default());
        assert_eq!(none.len(), 3);
    }

    #[test]
    fn the_folder_to_add_is_a_real_readable_one_and_a_tilde_is_home() {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let docs = home.path().join("docs");
        std::fs::create_dir(&docs).unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        std::fs::write(home.path().join("file.txt"), "x").unwrap();
        let add = |typed: &str| folder_to_add(typed, work.path(), Some(home.path()));
        assert_eq!(add("~/docs"), Ok(docs.canonicalize().unwrap()));
        assert_eq!(
            add(&format!("  {}  ", docs.display())),
            Ok(docs.canonicalize().unwrap())
        );
        assert!(add("~/missing").is_err());
        assert!(add("~/file.txt").is_err(), "a file is not a folder");
        assert!(add("~/.ssh").is_err(), "a secret place is never added");
    }

    #[test]
    fn a_long_step_is_cut_in_the_row_but_whole_in_the_bar() {
        let long = "x".repeat(60);
        let steps = [Step::Search(long.clone())];
        let next = Next {
            allow: None,
            steps: &steps,
        };
        let r = row(MenuKey::Down, false, Extras::default(), &next).unwrap();
        assert!(r.contains("...") && !r.contains(&long), "{r}");
        assert_eq!(
            choices(None, Extras::default(), 0, &next)[0].label,
            format!("Search for {long}")
        );
    }

    #[test]
    fn the_record_carries_the_folder_and_the_steps_for_the_menu() {
        let steps = [Step::Read("api".into()), Step::Run("cargo test".into())];
        let next = Next {
            allow: Some(Path::new("/home/me\u{7}")),
            steps: &steps,
        };
        assert_eq!(
            record("hi", None, None, &[], &next),
            "hi\u{1e}\u{1e}\u{1e}\u{1e}/home/me\u{1e}read:api\u{1f}run:cargo test"
        );
    }

    #[test]
    fn allow_keeps_the_yes_and_asks_the_question_again_and_a_blank_question_gives_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("data");
        std::fs::create_dir(&folder).unwrap();
        let user = dir.path().join("cfg");
        assert!(!trust::is_trusted(&user, &folder));
        assert_eq!(
            allow(Some(&folder), "explain\u{7} this directory", &user, false).as_deref(),
            Some("? explain this directory")
        );
        assert!(trust::is_trusted(&user, &folder));
        assert_eq!(allow(Some(&folder), "  ", &user, false), None);
        assert_eq!(allow(None, "q", &user, false), None);
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

    #[test]
    fn add_folder_refuses_the_root_the_home_folder_and_what_holds_secrets_with_a_reason() {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".config/gh")).unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        std::fs::create_dir(home.path().join("docs")).unwrap();
        let add = |typed: &str| folder_to_add(typed, work.path(), Some(home.path()));
        for typed in ["/", "~", "~/", "~/.config", "~/.ssh"] {
            let why = add(typed).unwrap_err();
            assert!(
                why.contains("too broad") || why.contains("secret"),
                "{typed}: {why}"
            );
        }
        assert!(add("~/docs").is_ok());
    }

    #[test]
    fn allow_keeps_a_yes_for_the_question_and_add_folder_one_for_every_ask() {
        let user = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let folder = folder.path().canonicalize().unwrap();
        let _ = allow(Some(&folder), "q", user.path(), false);
        assert!(trust::is_trusted(user.path(), &folder));
        assert_eq!(trust::standing(user.path()).len(), 0);
        let _ = allow(Some(&folder), "q", user.path(), true);
        assert_eq!(trust::standing(user.path()), [folder]);
    }
}
