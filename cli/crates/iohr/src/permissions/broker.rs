//! The live ask: the broker in the `iohr ask` process and the files it hands Claude Code.
//!
//! Claude Code is started with `--permission-prompt-tool mcp__iohrperm__approve`, an
//! `--mcp-config` that runs `iohr permission-server` (see [`super::server`]) and a plugin
//! folder with one `PreToolUse` hook that sends every `Bash` call to that tool, because a
//! read-only command such as `ls` otherwise never reaches it
//! (<https://code.claude.com/docs/en/cli-reference>, <https://code.claude.com/docs/en/hooks>).
//! The server relays each `{tool_name, input, tool_use_id}` over a unix socket in a folder only
//! this user can open; the broker answers `{behavior: allow, updatedInput}` or
//! `{behavior: deny, message}` (<https://code.claude.com/docs/en/agent-sdk/typescript>,
//! `PermissionResult`).
//!
//! One mechanism for every mode: the rules decide first ([`super::engine::decide`]; the secret
//! list beats everything), then in `prompt` mode the person is asked at the terminal and in
//! `rules` mode nobody is. No answer, Ctrl-C, a broken rule file and any failure here are a no.
//! Every decision goes to `.inorbit/permissions.log` (never output or file contents).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use console::Key;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::Mode;
use super::ask_card;
use super::engine::{self, Place, SECRET_RULE, Verdict};
use super::rule::Rule;
use super::store::{self, Files, Kind, Outcome, Scope};

/// The MCP server's name and its one tool, as Claude Code names them.
pub(crate) const SERVER: &str = "iohrperm";
pub(crate) const TOOL: &str = "approve";
/// The longest request line read from the socket.
const LINE_MAX: u64 = 1024 * 1024;
/// How much of a command is shown on the card, in characters.
const SHOWN_MAX: usize = 300;

/// `--permission-prompt-tool`'s value.
pub(crate) fn tool_flag() -> String {
    format!("mcp__{SERVER}__{TOOL}")
}

/// What the broker needs to decide.
#[derive(Debug, Clone)]
pub(crate) struct Opts {
    pub(crate) mode: Mode,
    pub(crate) files: Files,
    pub(crate) place: Place,
    /// The program's name for hints (`iohr permissions allow ...`).
    pub(crate) bin: String,
}

/// A call that was refused, for the lines printed after the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denied {
    /// What was wanted, in words (for example "run `ls -la`").
    pub(crate) what: String,
    /// The rule that would allow it, when there is a safe one.
    pub(crate) allow: Option<String>,
    /// A rule or the secret list said no (nothing to offer), against nobody could answer.
    pub(crate) by_rule: bool,
}

struct Shared {
    opts: Opts,
    denied: Mutex<Vec<Denied>>,
    /// Milliseconds spent waiting for the person, so the run's time limit leaves them out.
    waited_ms: AtomicU64,
    pending: AtomicUsize,
    /// One question on the terminal at a time.
    one_at_a_time: tokio::sync::Mutex<()>,
}

/// A running broker and the files Claude Code is pointed at. Dropping it stops the broker and
/// removes the folder, the socket with it.
pub(crate) struct Session {
    dir: PathBuf,
    mcp: PathBuf,
    plugin: PathBuf,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Session {
    /// Starts the broker. The folder is made with mode 0700 and a name nobody can guess, so
    /// no other user can reach the socket.
    pub(crate) fn start(opts: Opts) -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt as _;
        let exe = std::env::current_exe().map_err(|e| format!("cannot find iohr: {}", e.kind()))?;
        let mut seed = [0u8; 8];
        getrandom::fill(&mut seed).map_err(|_| "cannot make a private folder".to_owned())?;
        let tag: String = seed.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        let dir = std::env::temp_dir().join(format!("iop-{tag}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("cannot make a private folder: {}", e.kind()))?;
        let session = Self::lay_out(&dir, &exe, opts);
        if session.is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        session
    }

    fn lay_out(dir: &Path, exe: &Path, opts: Opts) -> Result<Self, String> {
        let socket = dir.join("s");
        let listener =
            UnixListener::bind(&socket).map_err(|e| format!("cannot listen: {}", e.kind()))?;
        let mcp = dir.join("mcp.json");
        let config = json!({"mcpServers": {SERVER: {
            "command": exe.to_string_lossy(),
            "args": ["permission-server", "--socket", socket.to_string_lossy()],
        }}});
        write(&mcp, &config.to_string())?;
        let plugin = dir.join("plugin");
        let hooks = plugin.join("hooks");
        std::fs::create_dir_all(&hooks).map_err(|e| format!("cannot write: {}", e.kind()))?;
        write(&hooks.join("hooks.json"), &hook_file())?;
        let shared = Arc::new(Shared {
            opts,
            denied: Mutex::new(Vec::new()),
            waited_ms: AtomicU64::new(0),
            pending: AtomicUsize::new(0),
            one_at_a_time: tokio::sync::Mutex::new(()),
        });
        let task = tokio::spawn(accept(listener, shared.clone()));
        Ok(Self {
            dir: dir.to_path_buf(),
            mcp,
            plugin,
            shared,
            task,
        })
    }

    /// The arguments that point Claude Code at this broker.
    pub(crate) fn args(&self) -> Vec<String> {
        vec![
            "--mcp-config".to_owned(),
            self.mcp.to_string_lossy().into_owned(),
            "--permission-prompt-tool".to_owned(),
            tool_flag(),
            "--plugin-dir".to_owned(),
            self.plugin.to_string_lossy().into_owned(),
        ]
    }

    /// Time spent waiting for the person, and whether a question is open now: the run's
    /// time limit counts neither.
    pub(crate) fn waiting(&self) -> (std::time::Duration, bool) {
        (
            std::time::Duration::from_millis(self.shared.waited_ms.load(Ordering::Relaxed)),
            self.shared.pending.load(Ordering::Relaxed) > 0,
        )
    }

    /// The calls that were refused so far.
    pub(crate) fn denied(&self) -> Vec<Denied> {
        self.shared
            .denied
            .lock()
            .map(|d| d.clone())
            .unwrap_or_default()
    }

    pub(crate) fn bin(&self) -> &str {
        &self.shared.opts.bin
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("cannot write: {}", e.kind()))
}

/// The plugin's hooks: every `Bash` call is sent to the permission prompt tool instead of
/// running because it looks read-only.
fn hook_file() -> String {
    let answer = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "ask",
        "permissionDecisionReason": "iohr asks before Io runs a command",
    }});
    json!({"hooks": {"PreToolUse": [{
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": format!("printf '%s' '{answer}'")}],
    }]}})
    .to_string()
}

async fn accept(listener: UnixListener, shared: Arc<Shared>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(serve(stream, shared.clone()));
    }
}

async fn serve(stream: UnixStream, shared: Arc<Shared>) {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    let mut reader = BufReader::new(read.take(LINE_MAX));
    let reply = match reader.read_line(&mut line).await {
        Ok(n) if n > 0 => match serde_json::from_str::<Value>(&line) {
            Ok(req) => decide_call(&shared, &req).await,
            Err(_) => refuse("iohr could not read the request"),
        },
        _ => refuse("iohr got no request"),
    };
    let mut text = reply.to_string();
    text.push('\n');
    let _ = write.write_all(text.as_bytes()).await;
}

use tokio::io::AsyncReadExt as _;

fn refuse(message: &str) -> Value {
    json!({"behavior": "deny", "message": message})
}

/// Decides one call. Never allows by accident: any doubt is a deny.
async fn decide_call(sh: &Arc<Shared>, req: &Value) -> Value {
    let Some(tool) = req.get("tool_name").and_then(Value::as_str) else {
        return refuse("iohr got no tool name");
    };
    let input = req.get("input").cloned().unwrap_or_else(|| json!({}));
    let o = &sh.opts;
    let files = &o.files;
    let rules = match store::rule_set(files) {
        Ok(r) => r,
        Err(why) => {
            audit(files, tool, &input, "(rule file)", Outcome::DeniedByRule);
            note_denied(sh, tool, &input, &o.place, true);
            return refuse(&format!(
                "Nothing is allowed while the permission rules cannot be read: {why}"
            ));
        }
    };
    match engine::decide(&rules, tool, &input, &o.place) {
        Verdict::Allow(rule) => {
            audit(files, tool, &input, &rule, Outcome::AllowedByRule);
            json!({"behavior": "allow", "updatedInput": input})
        }
        Verdict::Deny(rule) => {
            audit(files, tool, &input, &rule, Outcome::DeniedByRule);
            note_denied(sh, tool, &input, &o.place, true);
            if rule == SECRET_RULE {
                refuse("That file is never opened. Do not try another way to read it.")
            } else {
                refuse(&format!(
                    "The rule {rule} does not allow this. Do not try another way to do the same."
                ))
            }
        }
        Verdict::Ask if o.mode == Mode::Prompt => ask(sh, tool, input).await,
        Verdict::Ask => {
            audit(files, tool, &input, tool, Outcome::DeniedNoAnswer);
            note_denied(sh, tool, &input, &o.place, false);
            refuse("The person did not allow this and cannot be asked now. Carry on without it.")
        }
    }
}

fn audit(files: &Files, tool: &str, input: &Value, rule: &str, outcome: Outcome) {
    // A call no rule names is logged by its tool and, for the shell, its first word.
    let named = if rule == tool {
        input
            .get("command")
            .and_then(Value::as_str)
            .and_then(|c| c.split_whitespace().next())
            .map_or_else(|| tool.to_owned(), |w| format!("{tool}({w} ...)"))
    } else {
        rule.to_owned()
    };
    let _ = store::audit(files, tool, &named, outcome);
}

fn note_denied(sh: &Shared, tool: &str, input: &Value, at: &Place, by_rule: bool) {
    let allow = (!by_rule)
        .then(|| engine::suggest(tool, input, at))
        .flatten()
        .map(|rules| {
            rules
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        });
    let what = wanted(tool, input);
    if let Ok(mut d) = sh.denied.lock() {
        let item = Denied {
            what,
            allow,
            by_rule,
        };
        if !d.contains(&item) {
            d.push(item);
        }
    }
}

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    Once,
    Always,
    No,
}

/// The key to a choice; `None` keeps waiting. Enter, Esc, `q` and Ctrl-C are no.
pub(crate) fn choice_of(key: &Key, can_always: bool) -> Option<Choice> {
    match key {
        Key::Char('y' | 'Y') => Some(Choice::Once),
        Key::Char('a' | 'A') if can_always => Some(Choice::Always),
        Key::Char('n' | 'N' | 'q' | 'Q' | '\u{3}') | Key::Enter | Key::Escape | Key::CtrlC => {
            Some(Choice::No)
        }
        _ => None,
    }
}

async fn ask(sh: &Arc<Shared>, tool: &str, input: Value) -> Value {
    let o = &sh.opts;
    let _turn = sh.one_at_a_time.lock().await;
    sh.pending.fetch_add(1, Ordering::Relaxed);
    let began = Instant::now();
    let suggestion = engine::suggest(tool, &input, &o.place);
    let what = wanted(tool, &input);
    let folder = folder_name(&o.place.cwd);
    let line = card_line(
        &what,
        &folder,
        crate::card::stderr_width(),
        suggestion.is_some(),
    );
    let can_always = suggestion.is_some();
    // The drawn card where the look is fancy and the terminal wide enough; else the plain
    // line, which is what a script and the tests read.
    let look = crate::card::Look::detect(crate::ui::Ui::detect(false).fancy());
    let cols = console::Term::stderr()
        .size_checked()
        .map_or(0, |(_, c)| usize::from(c));
    let drawn = ask_card::drawn_at(cols, look).then(|| ask_card::Ask {
        wanted: wanted_parts(tool, &input),
        folder: folder.clone(),
        can_always,
    });
    crate::card::pause_spin(true);
    let key = tokio::task::spawn_blocking(move || {
        if let Some(card) = drawn {
            draw_and_read(&card, cols, look)
        } else {
            crate::card::say(&line);
            read_choice(can_always)
        }
    })
    .await
    .unwrap_or(Choice::No);
    let waited = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    sh.waited_ms.fetch_add(waited, Ordering::Relaxed);
    let reply = settle(sh, tool, &input, &what, suggestion.as_deref(), key);
    crate::card::pause_spin(false);
    sh.pending.fetch_sub(1, Ordering::Relaxed);
    reply
}

/// Draws the Io card over the spinner's line, reads the answer with the action bar's keys
/// (arrows and Enter, `1`-`9`, `y`, `a`, `n`; Esc, `q` and Ctrl-C are no), and erases the card,
/// so the one result line of [`settle`] is all that stays. A terminal that cannot be read is a
/// no.
fn draw_and_read(card: &ask_card::Ask, cols: usize, look: crate::card::Look) -> Choice {
    draw_and_read_rows(card, &ask_card::choices(card.can_always), cols, look)
}

/// [`draw_and_read`] with other rows under the card (the folder-trust ask's).
pub(crate) fn draw_and_read_rows(
    card: &ask_card::Ask,
    rows: &[crate::ui::Choice],
    cols: usize,
    look: crate::card::Look,
) -> Choice {
    use std::io::Write as _;
    // The spinner's line has no newline; the card starts on a clean one.
    let mut err = std::io::stderr().lock();
    let _ = write!(err, "\r\x1b[2K");
    let _ = err.flush();
    drop(err);
    let picked = crate::ui::Ui::select_card("Io asks", rows, ask_card::INITIAL, |at| {
        ask_card::lines_with(card, rows, at, cols, look)
    });
    // Esc, `q` and Ctrl-C leave with nothing picked, and a terminal that cannot be read
    // is an error: both are no.
    picked
        .ok()
        .flatten()
        .map_or(Choice::No, |v| ask_card::choice_for(&v))
}

/// Reads keys until one means something. A terminal that cannot be read is a no.
fn read_choice(can_always: bool) -> Choice {
    let term = console::Term::stderr();
    loop {
        match term.read_key_raw() {
            Ok(key) => {
                if let Some(c) = choice_of(&key, can_always) {
                    return c;
                }
            }
            Err(_) => return Choice::No,
        }
    }
}

/// Acts on the answer: the one result line, the rule written for "always", the audit, and
/// the reply for Claude Code.
fn settle(
    sh: &Shared,
    tool: &str,
    input: &Value,
    what: &str,
    suggestion: Option<&[Rule]>,
    key: Choice,
) -> Value {
    let o = &sh.opts;
    let named = suggestion.map_or_else(
        || tool.to_owned(),
        |r| {
            r.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        },
    );
    match key {
        Choice::Once => {
            audit(&o.files, tool, input, &named, Outcome::AllowedOnce);
            crate::card::say_done(&format!("Allowed once: {what}"));
            json!({"behavior": "allow", "updatedInput": input})
        }
        Choice::Always => {
            let saved = suggestion.unwrap_or_default();
            let mut failed = None;
            for rule in saved {
                if let Err(why) = store::add(&o.files, Scope::Project, Kind::Allow, rule, &o.place)
                {
                    failed = Some(why);
                    break;
                }
            }
            match failed {
                None => {
                    audit(&o.files, tool, input, &named, Outcome::AllowedAlways);
                    let file = o.files.path(Scope::Project);
                    let shown = file
                        .strip_prefix(&o.place.cwd)
                        .unwrap_or(&file)
                        .display()
                        .to_string();
                    crate::card::say_done(&format!(
                        "Allowed always: {named}, saved to {shown}. Undo: {} permissions remove '{named}'",
                        o.bin
                    ));
                }
                Some(why) => {
                    audit(&o.files, tool, input, &named, Outcome::AllowedOnce);
                    crate::card::say_done(&format!("Allowed once: {what} (not saved: {why})"));
                }
            }
            json!({"behavior": "allow", "updatedInput": input})
        }
        Choice::No => {
            audit(&o.files, tool, input, &named, Outcome::DeniedByPerson);
            crate::card::say_done(&format!("Not allowed: {what}"));
            refuse("The person denied this at the terminal. Carry on without it.")
        }
    }
}

/// The one line of the ask. Plain words, one line, cut to `width`: the command is shortened
/// and the keys stay.
pub(crate) fn card_line(what: &str, folder: &str, width: usize, can_always: bool) -> String {
    let keys = if can_always {
        "y once \u{b7} a always in this project \u{b7} n no"
    } else {
        "y once \u{b7} n no"
    };
    let head = format!("Io (AI) wants to {what} in {folder}");
    let tail = format!(" \u{b7} {keys}");
    let room = width.saturating_sub(console::measure_text_width(&tail) + 1);
    format!(
        "{}{tail}",
        console::truncate_str(&head, room.max(20), "...")
    )
}

/// What a call wants, split for drawing: the verb phrase ("run", or "use the X tool" when
/// there is nothing to show) and the thing, with how many characters were cut from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wanted {
    pub(crate) verb: String,
    /// The command, path or pattern, cut to [`SHOWN_MAX`] characters, with no control
    /// character in it.
    pub(crate) thing: Option<String>,
    /// How many characters of the thing are not shown.
    pub(crate) more: usize,
}

/// The call's verb and thing. Every character that reaches a terminal from here is clean:
/// control characters (a command, a path or a tool name could carry escape codes) become
/// spaces, and a long thing is cut.
pub(crate) fn wanted_parts(tool: &str, input: &Value) -> Wanted {
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let (verb, thing) = match tool {
        "Bash" => ("run", text("command")),
        "Read" => ("read", text("file_path")),
        "Grep" => ("search", text("path").or_else(|| text("pattern"))),
        "Glob" => ("list", text("pattern")),
        "WebFetch" => ("open", text("url")),
        "Edit" | "Write" | "NotebookEdit" => ("change", text("file_path")),
        _ => ("use the", None),
    };
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    };
    let Some(thing) = thing else {
        return Wanted {
            verb: format!("{verb} {} tool", clean(tool)),
            thing: None,
            more: 0,
        };
    };
    let clean = clean(thing);
    let clean = clean.trim();
    let count = clean.chars().count();
    Wanted {
        verb: verb.to_owned(),
        thing: Some(clean.chars().take(SHOWN_MAX).collect()),
        more: count.saturating_sub(SHOWN_MAX),
    }
}

/// What the call wants, in words, safe to put on a terminal: control characters become
/// spaces (a command could carry escape codes) and a long one is cut.
pub(crate) fn wanted(tool: &str, input: &Value) -> String {
    let w = wanted_parts(tool, input);
    match w.thing {
        None => w.verb,
        Some(thing) if w.more > 0 => format!("{} `{thing}` ({} more characters)", w.verb, w.more),
        Some(thing) => format!("{} `{thing}`", w.verb),
    }
}

/// The folder's own name, for the line.
fn folder_name(cwd: &Path) -> String {
    cwd.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map_or_else(
            || "this folder".to_owned(),
            |n| n.chars().filter(|c| !c.is_control()).collect(),
        )
}

/// The lines to print after an answer for what was refused: at most three, then a count.
pub(crate) fn denied_lines(denied: &[Denied], bin: &str) -> Vec<String> {
    let mut out = Vec::new();
    for d in denied.iter().take(3) {
        let line = match (&d.allow, d.by_rule) {
            (Some(rule), _) => format!(
                "Io wanted to {} and could not ask. Allow it with: {bin} permissions allow '{rule}'",
                d.what
            ),
            (None, true) => format!("Io wanted to {}; a rule says no.", d.what),
            (None, false) => format!(
                "Io wanted to {} and could not ask; run it yourself if you want it.",
                d.what
            ),
        };
        out.push(line);
    }
    if denied.len() > 3 {
        out.push(format!("and {} more it could not do.", denied.len() - 3));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_line_says_who_asks_what_and_where_and_never_runs_over() {
        let l = card_line("run `ls -la`", "velebit-ai", 100, true);
        assert_eq!(
            l,
            "Io (AI) wants to run `ls -la` in velebit-ai \u{b7} y once \u{b7} a always in this project \u{b7} n no"
        );
        let long = wanted("Bash", &json!({"command": "x".repeat(400)}));
        let l = card_line(&long, "d", 80, true);
        assert!(console::measure_text_width(&l) <= 80, "{l}");
        assert!(l.ends_with("n no") && l.contains("..."), "{l}");
        assert!(!card_line("run `ls`", "d", 100, false).contains("always"));
    }

    #[test]
    fn a_command_cannot_smuggle_escape_codes_onto_the_card() {
        let w = wanted("Bash", &json!({"command": "echo \u{1b}[2J\nrm -rf x"}));
        assert!(!w.chars().any(char::is_control), "{w:?}");
    }

    #[test]
    fn keys_map_to_choices_and_unknown_keys_wait() {
        assert_eq!(choice_of(&Key::Char('y'), true), Some(Choice::Once));
        assert_eq!(choice_of(&Key::Char('a'), true), Some(Choice::Always));
        assert_eq!(choice_of(&Key::Char('a'), false), None);
        for no in [
            Key::Char('n'),
            Key::Enter,
            Key::Escape,
            Key::CtrlC,
            Key::Char('q'),
        ] {
            assert_eq!(choice_of(&no, true), Some(Choice::No), "{no:?}");
        }
        assert_eq!(choice_of(&Key::Char('x'), true), None);
        assert_eq!(choice_of(&Key::ArrowUp, true), None);
    }

    #[test]
    fn refusals_are_listed_with_the_rule_to_allow_them() {
        let d = vec![
            Denied {
                what: "run `ls -la`".into(),
                allow: Some("Bash(ls:*)".into()),
                by_rule: false,
            },
            Denied {
                what: "read `.env`".into(),
                allow: None,
                by_rule: true,
            },
        ];
        let l = denied_lines(&d, "iohr");
        assert_eq!(l.len(), 2);
        assert!(
            l[0].contains("iohr permissions allow 'Bash(ls:*)'"),
            "{l:?}"
        );
        assert!(l[1].contains("a rule says no") && !l[1].contains("allow '"));
    }
}
