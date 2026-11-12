//! The pty proxy: the shell runs in a pseudo-terminal this process owns, and the panel is
//! painted over the top right corner after each chunk of the shell's output (ADR 0225).
//!
//! The panel floats: it takes no rows, and it is on the normal screen, not the alternate one.
//! The chat box (RFC 0212, [`boxed`]) is the one exception: while it is open it reserves four
//! rows at the bottom with a scroll region and the shell's pty is that much shorter. Around every chunk the proxy puts back
//! what the panel covered (from a vt100 model of the child's screen, so nothing of the panel
//! reaches the scrollback), writes the chunk, and draws the panel again, all inside one
//! synchronized update. It paints only between escape sequences ([`Boundary`]) and never on
//! the alternate screen, where a full screen program owns every cell.
//!
//! Known limit: painting saves and restores the cursor with DECSC/DECRC, the one slot a
//! program may also use for its own saved cursor; a program that saved its cursor and has
//! the panel painted before it restores loses it.
//!
//! The child is started by running this binary again (`iohr panel -- CMD` with `IOHR_PANEL_LEADER` set)
//! on the pty's slave: it makes itself the session leader, takes the pty as its controlling
//! terminal and replaces itself with the command, so no unsafe code is needed.

use std::fs::File;
use std::future::Future;
use std::io::{IsTerminal as _, Write as _};
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use rustix::fs::{Mode, OFlags};
use rustix::termios::{OptionalActions, Termios, Winsize};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::cells::{self, Floating, Hit, Rect};
use super::chat::{self, Chat};
use super::input::{Input, MOUSE_OFF, MOUSE_ON, Mouse, MouseMode, OpenKey, Reader};
use super::model::{Ask, Effect, Key, Model, Source, View};
use super::screen::{Boundary, Screen};
use super::workloads::{self, Snapshot};
use super::{act, input};
use crate::card::Look;
use crate::cli::Global;
use crate::commands::asks::{self, PanelAsk, PanelAuto};
use crate::commands::coordinator;
use crate::commands::incident_settings::{self, PanelIncidents};
use crate::commands::session::{Joined, join_watched};
use crate::error::Error;
use crate::session::link;
use crate::{Env, ui};

/// How often the asks are read again while the panel runs.
const REFRESH: Duration = Duration::from_secs(20);
/// How long one read of the asks may take.
const FETCH_LIMIT: Duration = Duration::from_secs(8);
/// How long the shell's last output is drained after it exits.
const DRAIN: Duration = Duration::from_millis(150);

const SAVE: &[u8] = b"\x1b7";
const RESTORE: &[u8] = b"\x1b8";
const SYNC_ON: &[u8] = b"\x1b[?2026h";
const SYNC_OFF: &[u8] = b"\x1b[?2026l";

/// What `iohr stop` run inside the panel writes to the terminal to end the panel: an operating
/// system command no terminal shows, which the proxy takes out of the shell's output (the shell
/// and the proxy share no process, so a signal would need a pid the shell does not have).
pub(super) const STOP_SEQ: &[u8] = b"\x1b]7771;iohr-panel-stop\x07";

/// What bare `iohr` run inside a stopped panel writes to bring the panel back: the same kind of
/// sequence, taken out of the shell's output the same way.
pub(super) const RESUME_SEQ: &[u8] = b"\x1b]7771;iohr-panel-resume\x07";

mod boxed;

/// What to run and what to show.
pub(super) struct Config {
    /// Stub data (the demo): labelled so in the title.
    pub(super) stub: bool,
    /// The view the panel starts in.
    pub(super) initial: View,
    /// The command; empty means the person's shell.
    pub(super) command: Vec<String>,
    /// Answers when the session host this shell joined has stopped (`iohr stop`): the panel
    /// closes and the shell goes on. `None` when there is no host.
    pub(super) host_gone: Option<tokio::sync::oneshot::Receiver<()>>,
    /// The folder whose session host this panel was opened for: running `iohr` again after a
    /// stop joins (starting it when none runs) that host again, from here, because this
    /// process outlives the re-run and the host stops when its last session leaves. `None`
    /// when the panel has no session (`iohr panel`).
    pub(super) project: Option<std::path::PathBuf>,
}

fn command_or_shell(command: &[String]) -> Vec<String> {
    if command.is_empty() {
        vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())]
    } else {
        command.to_vec()
    }
}

fn io_err(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Failed(format!("{what}: {e}"))
}

/// Set (to anything) in the helper's environment: this run is the shell side of the pty.
pub(super) const LEADER: &str = "IOHR_PANEL_LEADER";
/// Set in the shell's environment to the pid of the panel proxy around it: the shell runs under
/// the panel, so a nested `iohr panel` runs plainly. It is only believed while that pid lives
/// (a shell that outlived its panel keeps the variable, and must be able to start a new one).
const INSIDE: &str = "IOHR_PANEL";

/// The shell side of the pty: become the session leader, take the pty (standard input) as the
/// controlling terminal, and become the command. Only returns on failure.
pub(super) fn session_leader(command: &[String]) -> Result<(), Error> {
    let command = command_or_shell(command);
    rustix::process::setsid().map_err(|e| io_err("cannot start a session", e))?;
    rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())
        .map_err(|e| io_err("cannot take the terminal", e))?;
    let e = std::process::Command::new(&command[0])
        .args(&command[1..])
        .env_remove(LEADER)
        .env(
            INSIDE,
            rustix::process::getppid()
                .map_or(1, |p| p.as_raw_nonzero().get())
                .to_string(),
        )
        .exec();
    Err(io_err(&format!("cannot run {}", command[0]), e))
}

/// Runs the command on this terminal as it is: no panel (not a terminal, plain output, or
/// already inside the panel).
fn run_plain(command: &[String]) -> Result<(), Error> {
    let command = command_or_shell(command);
    let status = std::process::Command::new(&command[0])
        .args(&command[1..])
        .status()
        .map_err(|e| io_err(&format!("cannot run {}", command[0]), e))?;
    exit_result(status)
}

fn exit_result(status: std::process::ExitStatus) -> Result<(), Error> {
    match status.code() {
        Some(0) => Ok(()),
        Some(c) => Err(Error::Child(u8::try_from(c).unwrap_or(1))),
        None => Err(Error::Child(128u8.saturating_add(
            u8::try_from(status.signal().unwrap_or(0)).unwrap_or(0),
        ))),
    }
}

/// The terminal put back as it was, when the proxy ends however it ends.
struct RawMode {
    saved: Termios,
}

impl RawMode {
    fn enter() -> Result<Self, Error> {
        let fd = rustix::stdio::stdin();
        let saved = rustix::termios::tcgetattr(fd).map_err(|e| io_err("not a terminal", e))?;
        let mut raw = saved.clone();
        raw.make_raw();
        rustix::termios::tcsetattr(fd, OptionalActions::Drain, &raw)
            .map_err(|e| io_err("cannot set the terminal", e))?;
        Ok(Self { saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Whatever ended the proxy (an error, a panic), the terminal is not left reporting the
        // mouse to a shell that never asked for it.
        write_terminal(MOUSE_OFF);
        if boxed::REGION.swap(false, std::sync::atomic::Ordering::Relaxed) {
            write_terminal(&boxed::release(host_size().0, true));
        }
        let _ =
            rustix::termios::tcsetattr(rustix::stdio::stdin(), OptionalActions::Drain, &self.saved);
    }
}

fn host_size() -> (usize, usize) {
    match rustix::termios::tcgetwinsize(std::io::stdout().as_fd()) {
        Ok(w) if w.ws_row > 0 && w.ws_col > 0 => (usize::from(w.ws_row), usize::from(w.ws_col)),
        _ => (24, 80),
    }
}

fn winsize(rows: usize, cols: usize) -> Winsize {
    Winsize {
        ws_row: u16::try_from(rows).unwrap_or(u16::MAX),
        ws_col: u16::try_from(cols).unwrap_or(u16::MAX),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// A pseudo-terminal: the master this process holds and the slave the shell gets.
struct Pty {
    master: OwnedFd,
    slave: OwnedFd,
}

fn open_pty(rows: usize, cols: usize) -> Result<Pty, Error> {
    let fail = |e: rustix::io::Errno| io_err("cannot open a pseudo-terminal", e);
    let master =
        rustix::pty::openpt(rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY)
            .map_err(fail)?;
    // The shell must not inherit the master (macOS has no O_CLOEXEC for it).
    rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC).map_err(fail)?;
    rustix::pty::grantpt(&master).map_err(fail)?;
    rustix::pty::unlockpt(&master).map_err(fail)?;
    let name = rustix::pty::ptsname(&master, Vec::new()).map_err(fail)?;
    let path = std::ffi::OsStr::new(
        name.to_str()
            .map_err(|e| io_err("cannot open a pseudo-terminal", e))?,
    )
    .to_owned();
    let slave = rustix::fs::open(
        &path,
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(fail)?;
    rustix::termios::tcsetwinsize(&master, winsize(rows, cols)).map_err(fail)?;
    Ok(Pty { master, slave })
}

type Fetch<T> = Pin<Box<dyn Future<Output = T>>>;

async fn poll<T>(slot: &mut Option<Fetch<T>>) -> T {
    match slot.as_mut() {
        Some(f) => {
            let out = f.await;
            *slot = None;
            out
        }
        None => std::future::pending().await,
    }
}

/// The stub asks the demo shows: labelled stub in the title.
fn stub_asks() -> Vec<Ask> {
    [
        ("Merge sdk #443 to main", "pull request", "4m"),
        ("Roll runner to 0.7.21", "roll", "11m"),
        ("Apply migration 20261106", "migration", "1h"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (title, kind, age))| Ask {
        id: format!("stub-{i}"),
        title: (*title).to_owned(),
        kind: (*kind).to_owned(),
        age: (*age).to_owned(),
        // The stub session of this name is the one that waits on the first ask.
        agent: if i == 0 { "Fix sdk #448 reads" } else { "" }.to_owned(),
    })
    .collect()
}

/// The proxy's state: the model, the child's screen, and what is painted on the terminal.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent facts about what is on the terminal"
)]
struct Proxy {
    model: Model,
    screen: Screen,
    boundary: Boundary,
    look: Look,
    rows: usize,
    cols: usize,
    /// The rectangle (box and shadow) painted on the terminal now.
    on_screen: Option<Rect>,
    /// The panel changed while painting was not allowed: paint at the next chance.
    dirty: bool,
    /// The panel was stopped from the menu: the shell goes on without it.
    stopped: bool,
    /// When the panel asks the terminal for mouse reports.
    mouse: MouseMode,
    /// The terminal reports the mouse to the panel now.
    mouse_on: bool,
    /// The chat box (RFC 0212).
    chat: Chat,
    /// The box's rows are reserved on the terminal now (a scroll region over the rest).
    reserved: bool,
    /// The folded box's corner chip painted on the terminal now.
    chip_on: Option<Rect>,
    /// The start of a report from the shell a chunk ended in the middle of.
    osc_held: Vec<u8>,
    /// The start of a `CSI` the shell's output ended in the middle of, held back from the
    /// terminal until the rest comes (so a scroll-region reset is never seen half).
    csi_held: Vec<u8>,
    /// Bytes the proxy owes the shell's input (a `?` the box could not take).
    to_shell: Vec<u8>,
    /// The size the shell's pty has now: the rows are the screen's less the box's.
    pty_size: (usize, usize),
    /// Where a question is handed to the shell.
    chat_file: Option<std::path::PathBuf>,
}

impl Proxy {
    fn new(model: Model, rows: usize, cols: usize, look: Look, mouse: MouseMode) -> Self {
        Self {
            model,
            screen: Screen::new(rows, cols),
            boundary: Boundary::new(),
            look,
            rows,
            cols,
            on_screen: None,
            dirty: false,
            stopped: false,
            mouse,
            mouse_on: false,
            chat: Chat::new(false, false),
            reserved: false,
            chip_on: None,
            osc_held: Vec::new(),
            csi_held: Vec::new(),
            to_shell: Vec::new(),
            pty_size: (rows, cols),
            chat_file: None,
        }
    }

    /// Turns the chat box on, with the file a question is handed over in.
    fn with_chat(mut self, setting: boxed::Setting, file: Option<std::path::PathBuf>) -> Self {
        let on = setting != boxed::Setting::Off && file.is_some();
        self.chat = Chat::new(on, setting == boxed::Setting::Hidden);
        self.chat_file = file.filter(|_| on);
        self
    }

    /// Where the panel is now: one place for the painting and for the hit-test.
    fn layout(&self) -> Option<Floating> {
        let width = self.model.width().min(self.cols.saturating_sub(2));
        cells::place(&self.model.spec(width), self.cols)
    }

    /// Where a mouse report landed relative to the panel's box.
    fn hit(&self, col: usize, row: usize) -> Hit {
        self.layout().map_or(Hit::Outside, |f| f.rect.hit(col, row))
    }

    /// Whether the panel wants mouse reports now: while a view is open (always, unless off), and
    /// at rest only when `always` is set and the program has no mouse mode of its own; never
    /// once the panel is lifted or a program is on the alternate screen.
    fn wants_mouse(&self) -> bool {
        if self.stopped || self.screen.alt() {
            return false;
        }
        match self.mouse {
            MouseMode::Off => false,
            MouseMode::Menu => {
                // The box and its chip are clicked, so they ask for the mouse like a view does,
                // unless the program has its own mouse mode.
                self.model.open()
                    || ((self.reserved || self.chat.folded()) && !self.screen.uses_mouse())
            }
            MouseMode::Always => self.model.open() || !self.screen.uses_mouse(),
        }
    }

    /// The bytes that bring the terminal's mouse reports in line with [`Self::wants_mouse`]:
    /// on, or off with the program's own mode put back. Only between escape sequences.
    fn mouse_sync(&mut self) -> Vec<u8> {
        let want = self.wants_mouse();
        if want == self.mouse_on || !self.boundary.clean() {
            return Vec::new();
        }
        self.mouse_on = want;
        if want {
            MOUSE_ON.to_vec()
        } else {
            let mut out = MOUSE_OFF.to_vec();
            out.extend(self.screen.own_mouse());
            out
        }
    }

    fn can_paint(&self) -> bool {
        self.boundary.clean() && !self.screen.alt()
    }

    /// Puts back what the panel covers.
    fn lift(&mut self) -> Vec<u8> {
        if (self.on_screen.is_none() && self.chip_on.is_none()) || !self.can_paint() {
            return Vec::new();
        }
        let mut out = SAVE.to_vec();
        for rect in [self.on_screen.take(), self.chip_on.take()]
            .into_iter()
            .flatten()
        {
            out.extend(self.screen.restore(rect));
        }
        out.extend_from_slice(RESTORE);
        out
    }

    /// Draws the panel as the model has it now, removing it from where it was if that moved.
    fn sync(&mut self) -> Vec<u8> {
        if !self.can_paint() {
            self.dirty = true;
            return Vec::new();
        }
        self.dirty = false;
        if self.stopped {
            return self.lift();
        }
        let rows = self.child_rows();
        let placed = self.layout();
        let new_rect = placed.as_ref().map(|f| f.rect.with_shadow(rows, self.cols));
        let now = std::time::Instant::now();
        // The folded box's chip, unless the panel's box is over its place.
        let chip = self.chip_paint(now).filter(|(r, _)| {
            placed
                .as_ref()
                .is_none_or(|f| f.rect.top + f.rect.height + 1 < r.top)
        });
        let new_chip = chip.as_ref().map(|(r, _)| *r);
        let mut out = SAVE.to_vec();
        if let Some(old) = self.on_screen
            && Some(old) != new_rect
        {
            out.extend(self.screen.restore(old));
        }
        if let Some(old) = self.chip_on
            && Some(old) != new_chip
        {
            out.extend(self.screen.restore(old));
        }
        let mut segments = Vec::new();
        if let Some(f) = &placed {
            let screen = &self.screen;
            segments = cells::segments(f, rows, self.cols, &|r, c| screen.char_at(r, c));
        }
        if let Some((r, cells)) = chip {
            segments.push((r.top, r.left, cells));
        }
        segments.extend(self.box_segments(now));
        out.extend(cells::paint(self.look, &segments));
        out.extend_from_slice(RESTORE);
        self.on_screen = new_rect;
        self.chip_on = new_chip;
        out
    }

    /// One chunk of the shell's output, as the bytes for the terminal: the panel lifted, the
    /// chunk, the panel drawn again, in one synchronized update.
    fn child_output(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut out = SYNC_ON.to_vec();
        out.extend(self.lift());
        out.extend(self.terminal_copy(chunk));
        self.boundary.feed(chunk);
        self.region_reset(chunk);
        self.screen.feed(chunk);
        out.extend(self.region());
        out.extend(self.sync());
        out.extend(self.mouse_sync());
        out.extend_from_slice(SYNC_OFF);
        out
    }

    /// One chunk of the shell's output, with the stop sequence (`iohr stop` run inside the panel)
    /// taken out: the panel is lifted and the shell goes on.
    fn shell_output(&mut self, chunk: &[u8]) -> Vec<u8> {
        // The hook's reports to the chat box come out first (a start a chunk ended in the middle
        // of is held for the next one, which is also where a stop sequence's start is rejoined).
        let (mut chunk, msgs) = if self.chat.enabled {
            chat::extract(&mut self.osc_held, chunk)
        } else {
            (chunk.to_vec(), Vec::new())
        };
        for msg in msgs {
            self.chat_msg(msg);
        }
        // Whichever sequence comes first is taken first, so the last one written wins.
        while let Some((at, seq, stop)) = [(STOP_SEQ, true), (RESUME_SEQ, false)]
            .into_iter()
            .filter_map(|(seq, stop)| find(&chunk, seq).map(|at| (at, seq, stop)))
            .min_by_key(|(at, ..)| *at)
        {
            self.stopped = stop;
            chunk.drain(at..at + seq.len());
        }
        self.child_output(&chunk)
    }

    /// A change of the panel with no output from the shell.
    fn redraw(&mut self) -> Vec<u8> {
        self.redraw_lifting(false)
    }

    /// `redraw`, putting back what the panel covers first when `lift` (a resize).
    fn redraw_lifting(&mut self, lift: bool) -> Vec<u8> {
        let mut out = SYNC_ON.to_vec();
        if lift {
            out.extend(self.lift());
        }
        out.extend(self.region());
        out.extend(self.sync());
        out.extend(self.mouse_sync());
        out.extend_from_slice(SYNC_OFF);
        out
    }

    fn resize(&mut self, rows: usize, cols: usize) -> Vec<u8> {
        let mut out = Vec::new();
        if self.reserved {
            // The old region is for the old size: end it, and `redraw` sets the new one.
            out.extend_from_slice(SAVE);
            out.extend_from_slice(b"\x1b[r");
            out.extend_from_slice(RESTORE);
            self.reserved = false;
            boxed::REGION.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        self.rows = rows;
        self.cols = cols;
        self.screen.resize(rows, cols);
        // The terminal keeps the painted cells where they were and the shell redraws only its
        // prompt, so the old panel and chip rects are put back from the resized model (clipped
        // to the new size) in the same update that paints them again.
        out.extend(self.redraw_lifting(true));
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn write_terminal(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

fn reader<R: std::io::Read + Send + 'static>(mut from: R) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel(64);
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    rx
}

fn source_of<T>(r: Result<T, Error>) -> Source<T> {
    match r {
        Ok(v) => Source::Ready(v),
        Err(e) => Source::Unavailable(e.to_string().lines().next().unwrap_or("").to_owned()),
    }
}

/// The shell started on a pty: its master side (for keys in, output out) and its handle.
struct Shell {
    master: File,
    child: std::process::Child,
}

fn launch(
    command: &[String],
    rows: usize,
    cols: usize,
    chat_file: Option<&Path>,
) -> Result<Shell, Error> {
    let pty = open_pty(rows, cols)?;
    let exe = std::env::current_exe().map_err(|e| io_err("cannot find iohr", e))?;
    let stdio = |fd: &OwnedFd| -> Result<Stdio, Error> {
        Ok(Stdio::from(
            fd.try_clone()
                .map_err(|e| io_err("cannot share the pty", e))?,
        ))
    };
    let mut shell = std::process::Command::new(exe);
    shell.args(["panel", "--"]).args(command).env(LEADER, "1");
    // The hook speaks to the chat box only when there is one: the file is its sign.
    match chat_file {
        Some(file) => shell.env(boxed::FILE_ENV, file),
        None => shell.env_remove(boxed::FILE_ENV),
    };
    let child = shell
        .stdin(stdio(&pty.slave)?)
        .stdout(stdio(&pty.slave)?)
        .stderr(stdio(&pty.slave)?)
        .spawn()
        .map_err(|e| io_err(&format!("cannot run {}", command[0]), e))?;
    Ok(Shell {
        master: File::from(pty.master),
        child,
    })
}

/// How long a lone Esc is held for an `h` that follows it (Option+h written in two reads).
const ESC_HOLD: Duration = Duration::from_millis(40);

/// What the loop holds besides the proxy: the shell's input and the reads in flight.
struct Session {
    proxy: Proxy,
    master: File,
    /// The person's input, with a report a read ended in the middle of held for the next.
    reader: Reader,
    stub: bool,
    g: Global,
    env: Env,
    asks_fetch: Option<Fetch<Result<Vec<PanelAsk>, Error>>>,
    deny_fetch: Option<Fetch<(String, Result<(), Error>)>>,
    workloads_fetch: Option<Fetch<Result<Snapshot, Error>>>,
    auto_fetch: Option<Fetch<Result<PanelAuto, Error>>>,
    incidents_fetch: Option<Fetch<Result<PanelIncidents, Error>>>,
    /// One action at a time: a question, a message, a stop, auto mode on or off.
    act_fetch: Option<Fetch<Result<String, String>>>,
    /// One page read at a time: an ask's details, a transcript.
    read_fetch: Option<Fetch<Result<Vec<String>, String>>>,
    host_gone: Option<Fetch<()>>,
    /// The host stopped while the panel was open.
    host_stopped: bool,
    /// See [`Config::project`].
    project: Option<std::path::PathBuf>,
    /// The session this process rejoined with after a stop: the host counts it until it is
    /// dropped, which is when the panel ends.
    held: Option<link::Session>,
    /// A rejoin in flight (starting a host takes seconds).
    rejoin_fetch: Option<Fetch<Joined>>,
    /// A lone Esc held until this moment, to see whether `h` follows in the next read (Option+h
    /// from a terminal that writes Esc and `h` apart).
    esc_held: Option<Instant>,
}

impl Session {
    fn refresh(&mut self) {
        if !self.stub && self.asks_fetch.is_none() {
            self.asks_fetch = Some(read_asks(self.g.clone(), self.env.clone()));
        }
        if !self.stub && self.workloads_fetch.is_none() {
            self.workloads_fetch = Some(read_workloads(self.g.clone(), self.env.clone()));
        }
        if !self.stub && self.auto_fetch.is_none() {
            self.auto_fetch = Some(read_auto(self.g.clone(), self.env.clone()));
        }
        // Only while the menu or its view is open: the setting changes rarely.
        if !self.stub
            && self.incidents_fetch.is_none()
            && matches!(self.proxy.model.view, View::Menu | View::Incidents)
        {
            self.incidents_fetch = Some(read_incidents(self.g.clone(), self.env.clone()));
        }
    }

    /// Starts an action, unless one is still running.
    fn act(&mut self, f: Fetch<Result<String, String>>) {
        if self.act_fetch.is_some() {
            self.proxy
                .model
                .acted(Err("still working on the last action".into()));
        } else {
            self.act_fetch = Some(f);
        }
    }

    /// Runs a local action (the registry, a process, a file) off the loop.
    fn local(
        &mut self,
        f: impl FnOnce(&std::path::Path) -> Result<String, String> + Send + 'static,
    ) {
        let fut: Fetch<Result<String, String>> = Box::pin(async move {
            let run = move || match workloads::home_root() {
                Some(root) => f(&root),
                None => Err("cannot find your home folder".into()),
            };
            tokio::task::spawn_blocking(run)
                .await
                .unwrap_or_else(|e| Err(format!("the action failed: {e}")))
        });
        self.act(fut);
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::Refresh => self.refresh(),
            Effect::Deny(id) => {
                if self.stub {
                    self.proxy.model.denied(&id);
                } else if self.deny_fetch.is_none() {
                    self.deny_fetch = Some(deny(self.g.clone(), self.env.clone(), id));
                }
            }
            Effect::Stop => self.proxy.stopped = true,
            Effect::ChatToggle => self.proxy.chat.toggle(),
            Effect::Question { id, text } => {
                if self.stub {
                    self.proxy.model.acted(Ok("stub: nothing was sent".into()));
                } else {
                    let (g, env) = (self.g.clone(), self.env.clone());
                    self.act(limited(async move {
                        asks::question_quiet(&g, &env, &id, &text)
                            .await
                            .map(|()| "question sent".to_owned())
                    }));
                }
            }
            Effect::Detail(id) => {
                if self.stub {
                    self.proxy.model.reader_done(Ok(stub_detail(&id)));
                } else if self.read_fetch.is_none() {
                    let (g, env) = (self.g.clone(), self.env.clone());
                    self.read_fetch = Some(read_limited(async move {
                        asks::detail_quiet(&g, &env, &id).await.map(|d| {
                            let mut lines: Vec<String> =
                                d.pairs.iter().map(|(k, v)| format!("{k}  {v}")).collect();
                            if !d.open {
                                lines.push("this ask is no longer waiting".into());
                            }
                            lines
                        })
                    }));
                }
            }
            Effect::AutoOn => {
                if self.stub {
                    self.proxy.model.acted(Ok("stub: nothing was asked".into()));
                } else {
                    let (g, env) = (self.g.clone(), self.env.clone());
                    self.act(limited(async move {
                        asks::auto_request(&g, &env)
                            .await
                            .map(|_| "asked your phone: approve there to turn it on".to_owned())
                    }));
                }
            }
            Effect::AutoOff => {
                if self.stub {
                    self.proxy
                        .model
                        .acted(Ok("stub: nothing was changed".into()));
                } else {
                    let (g, env) = (self.g.clone(), self.env.clone());
                    self.act(limited(async move {
                        asks::auto_revoke(&g, &env)
                            .await
                            .map(|()| "auto mode is off".to_owned())
                    }));
                }
            }
            Effect::IncidentsAutoResolve(on) => {
                if self.stub {
                    self.proxy
                        .model
                        .acted(Ok("stub: nothing was changed".into()));
                } else {
                    let (g, env) = (self.g.clone(), self.env.clone());
                    self.act(limited(async move {
                        incident_settings::set_auto_resolve(&g, &env, on)
                            .await
                            .map(|()| format!("auto-resolve is {}", if on { "on" } else { "off" }))
                    }));
                }
            }
            other => self.apply_work(other),
        }
    }

    /// The Workloads actions: they act on this machine's sessions, not on the account.
    fn apply_work(&mut self, effect: Effect) {
        match effect {
            Effect::Message { id, text } => {
                if self.stub {
                    self.proxy.model.acted(Ok("stub: nothing was sent".into()));
                } else {
                    self.local(move |root| {
                        act::message(root, &id, &text, OffsetDateTime::now_utc())
                    });
                }
            }
            Effect::Terminal(id) => {
                if self.stub {
                    self.proxy
                        .model
                        .acted(Ok("stub: no terminal to show".into()));
                } else {
                    self.local(move |root| act::terminal(root, &id));
                }
            }
            Effect::Transcript(id) => {
                if self.stub {
                    self.proxy.model.reader_done(Ok(vec![
                        "stub: a session's transcript is read from its file".into(),
                    ]));
                } else if self.read_fetch.is_none() {
                    self.read_fetch = Some(Box::pin(async move {
                        let run = move || match workloads::home_root() {
                            Some(root) => act::transcript(&root, &id),
                            None => Err("cannot find your home folder".into()),
                        };
                        tokio::task::spawn_blocking(run)
                            .await
                            .unwrap_or_else(|e| Err(format!("the read failed: {e}")))
                    }));
                }
            }
            Effect::Pr { id, pr } => {
                if self.stub {
                    self.proxy
                        .model
                        .acted(Ok("stub: no pull request to open".into()));
                } else {
                    self.local(move |root| act::pr(root, &id, pr.as_deref()));
                }
            }
            Effect::StopWork(id) => {
                if self.stub {
                    self.proxy
                        .model
                        .acted(Ok("stub: nothing was stopped".into()));
                } else {
                    self.local(move |root| act::stop(root, &id));
                }
            }
            _ => {}
        }
    }

    /// Input from the person: the panel's while it is open (keys and mouse reports), or for
    /// its own key, else the shell's. A mouse report goes to the panel or to a program that
    /// asked for the mouse itself, never to a shell that did not.
    fn user_input(&mut self, bytes: &[u8]) {
        // A lone Esc waits a moment: `h` right after it is Option+h, any other read releases it
        // with that read (the shell gets Esc and the key together, as a terminal sends Meta).
        if self.esc_held.take().is_some() {
            let mut joined = vec![0x1b];
            joined.extend_from_slice(bytes);
            self.route_input(&joined);
            return;
        }
        if OpenKey::starts_hide(bytes) && self.can_hold_esc() {
            self.esc_held = Some(Instant::now() + ESC_HOLD);
            return;
        }
        self.route_input(bytes);
    }

    /// Whether a lone Esc could still be the box's key: the box is there to take it and nothing
    /// else reads Esc (the menu, a cut sequence).
    fn can_hold_esc(&self) -> bool {
        let p = &self.proxy;
        !p.stopped
            && !p.screen.alt()
            && p.chat.available()
            && !p.model.open()
            && !self.reader.pending()
    }

    /// The held Esc's time is up: it was a key of its own.
    fn esc_released(&mut self) {
        if self.esc_held.take().is_some() {
            self.route_input(&[0x1b]);
        }
    }

    fn route_input(&mut self, bytes: &[u8]) {
        // A stopped panel owns no key; Ctrl-G, the menu key on terminals without ⌥, is not the
        // shell's either, and sent on it would echo as `^G` under a command in cooked mode.
        if self.proxy.stopped && bytes == [0x07] && self.proxy.model.open_key().is(bytes) {
            return;
        }
        if self.chat_input(bytes) {
            return;
        }
        let panel_keys = !self.proxy.stopped && !self.proxy.screen.alt();
        let report = input::is_mouse_report(bytes);
        let routed = panel_keys
            && (self.proxy.model.open()
                || self.reader.pending()
                || (self.proxy.mouse_on && report));
        let inputs = if routed {
            self.reader.feed(bytes)
        } else if panel_keys && self.proxy.model.open_key().is(bytes) {
            vec![Input::Key(Key::Menu)]
        } else {
            if !report || self.proxy.screen.uses_mouse() {
                if !report {
                    self.proxy.chat.forwarded();
                }
                let _ = self.master.write_all(bytes);
            }
            return;
        };
        for input in inputs {
            let effect = match input {
                Input::Key(k) => self.proxy.model.key(k),
                Input::Mouse(m) if self.chat_mouse(m) => Effect::None,
                Input::Mouse(m) => self.mouse(m),
            };
            self.apply(effect);
        }
        self.redraw();
    }

    /// A mouse report for the panel. Closed (at rest, `always`): a press on the pill opens the
    /// menu. Open: a press on a row selects and opens it, a press outside closes the menu, the
    /// wheel over the box moves the selection.
    fn mouse(&mut self, m: Mouse) -> Effect {
        match m {
            Mouse::Press { col, row } => {
                match (self.proxy.model.open(), self.proxy.hit(col, row)) {
                    (false, Hit::Outside) | (true, Hit::Frame) => Effect::None,
                    (false, _) | (true, Hit::Outside) => self.proxy.model.key(Key::Menu),
                    (true, Hit::Line(line)) => self.proxy.model.click(line),
                }
            }
            Mouse::WheelUp { col, row } | Mouse::WheelDown { col, row } => {
                let key = if matches!(m, Mouse::WheelUp { .. }) {
                    Key::Up
                } else {
                    Key::Down
                };
                if self.proxy.model.open() && self.proxy.hit(col, row) != Hit::Outside {
                    self.proxy.model.key(key)
                } else {
                    Effect::None
                }
            }
            Mouse::Release { .. } => Effect::None,
        }
    }

    fn asks_done(&mut self, result: Result<Vec<PanelAsk>, Error>) {
        let now = OffsetDateTime::now_utc();
        self.proxy.model.set_asks(source_of(
            result.map(|rows| rows.iter().map(|a| Ask::from(a, now)).collect::<Vec<_>>()),
        ));
        self.redraw();
    }

    fn workloads_done(&mut self, result: Result<Snapshot, Error>) {
        self.proxy.model.set_work(source_of(result));
        self.redraw();
    }

    fn deny_done(&mut self, id: &str, result: Result<(), Error>) {
        match result {
            Ok(()) => self.proxy.model.denied(id),
            Err(e) => self
                .proxy
                .model
                .deny_failed(e.to_string().lines().next().unwrap_or("")),
        }
        self.redraw();
    }

    fn incidents_done(&mut self, result: Result<PanelIncidents, Error>) {
        self.proxy.model.set_incidents(source_of(result));
        write_terminal(&self.proxy.redraw());
    }

    fn auto_done(&mut self, result: Result<PanelAuto, Error>) {
        self.proxy.model.set_auto(source_of(result));
        self.redraw();
    }

    /// An action ended: say what it did, and read everything again (a question or a stop
    /// changes the lists, auto mode changes the pill).
    fn act_done(&mut self, result: Result<String, String>) {
        self.proxy.model.acted(result);
        self.refresh();
        self.redraw();
    }

    fn read_done(&mut self, result: Result<Vec<String>, String>) {
        self.proxy.model.reader_done(result);
        self.redraw();
    }

    /// One chunk of the shell's output: written to the terminal, and when it brought a stopped
    /// panel back, a host is joined again.
    fn shell_wrote(&mut self, bytes: &[u8]) {
        let was_stopped = self.proxy.stopped;
        let out = self.proxy.shell_output(bytes);
        self.emit(&out);
        if was_stopped && !self.proxy.stopped {
            self.resumed();
        }
    }

    /// The shell ran `iohr` again in a stopped panel and the panel is back. With no host
    /// joined (`iohr stop` ended it) one is joined again here, starting it when none runs:
    /// this process lives as long as the panel, the re-run does not, and the host stops when
    /// its last session leaves.
    fn resumed(&mut self) {
        if self.host_gone.is_some() || self.rejoin_fetch.is_some() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let g = self.g.clone();
        self.rejoin_fetch = Some(Box::pin(async move { join_watched(&g, &project).await }));
    }

    /// A rejoin ended: the new session and its watcher replace the stopped host's, or the same
    /// note `iohr` gives when no host can start is written into the shell's output.
    fn rejoined(&mut self, joined: Joined) {
        if let Some(note) = joined.note {
            let out = self
                .proxy
                .child_output(format!("\r\n{note}\r\n").as_bytes());
            self.emit(&out);
            return;
        }
        self.held = joined.session;
        self.host_gone = joined.gone.map(host_closed);
        self.host_stopped = false;
    }

    /// The session host stopped (`iohr stop`): the panel closes, the shell goes on.
    fn host_ended(&mut self) {
        self.host_stopped = true;
        self.proxy.stopped = true;
        self.redraw();
    }

    fn resized(&mut self) {
        let (rows, cols) = host_size();
        let bytes = self.proxy.resize(rows, cols);
        self.emit(&bytes);
    }
}

fn initial_model(config: &Config) -> Model {
    let mut model = Model::new(config.stub);
    model.view = config.initial;
    if config.stub {
        model.set_asks(Source::Ready(stub_asks()));
        model.set_work(Source::Ready(workloads::stub()));
        model.set_auto(Source::Ready(PanelAuto {
            active: true,
            expires_at: "2099-01-01T18:00:00Z".into(),
            approved: 3,
            never_kinds: vec!["pull_request".into(), "roll".into(), "migration".into()],
        }));
    }
    model
}

/// What the panel reads from the person's setup: its mouse mode and the key that opens it.
struct Settings {
    mouse: MouseMode,
    open_key: OpenKey,
    chat: boxed::Setting,
}

impl Settings {
    /// `[panel] mouse` (or `IOHR_PANEL_MOUSE`), `[ask] menu_key`, and `$TERM_PROGRAM`. A config
    /// that cannot be read is the defaults.
    fn read(g: &Global) -> Self {
        let config = crate::context::Ctx::load(g).ok().map(|c| c.config);
        let mouse = std::env::var("IOHR_PANEL_MOUSE")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.panel.mouse.clone()));
        let ask_ctrl_g = config.as_ref().is_some_and(|c| {
            matches!(
                crate::actions::MenuKey::parse(c.ask.menu_key.as_deref()),
                Ok(crate::actions::MenuKey::CtrlG)
            )
        });
        let chat = std::env::var("IOHR_PANEL_CHAT")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.panel.chat.clone()));
        Self {
            mouse: MouseMode::parse(mouse.as_deref()),
            open_key: OpenKey::here(ask_ctrl_g),
            chat: boxed::Setting::parse(chat.as_deref()),
        }
    }
}

/// Runs `config.command` under the panel until it exits; its exit code is ours.
/// Whether the panel can be drawn here: a terminal on both ends, colour allowed, no `--json`.
/// Bare `iohr` uses the same test to decide between entering the session and the usage report.
pub(super) fn fancy(g: &Global) -> bool {
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    interactive
        && ui::fancy_for(
            true,
            g.json,
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
}

/// Whether this process already runs inside a panel.
pub(super) fn inside() -> bool {
    std::env::var(INSIDE).is_ok_and(|v| proxy_lives(&v))
}

/// Whether the panel proxy `var` names (a pid) still runs. A value that is not a pid counts as
/// alive: it was set by a version that wrote `1`.
fn proxy_lives(var: &str) -> bool {
    let Some(pid) = var
        .parse::<i32>()
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return true;
    };
    // Signal 0 only asks whether the process exists; no permission to signal it means it does.
    !matches!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

/// What bare `iohr` says inside a panel, instead of an error: the panel is on in this shell,
/// and which key opens the menu. A panel that was stopped (`iohr stop`, "Stop panel" or
/// its host ending) is brought back first, so running `iohr` again gives a live panel. `None`
/// outside a panel.
pub(super) fn bare_inside_message(g: &Global) -> Option<String> {
    inside().then(|| {
        send_to_panel(RESUME_SEQ);
        panel_is_on(Settings::read(g).open_key)
    })
}

/// True both when the panel was live and when this run just brought a stopped one back.
fn panel_is_on(key: OpenKey) -> String {
    format!("the panel is on; {} opens the menu", key.label())
}

/// Tells the panel proxy around this process to end, so `iohr stop` inside it closes the panel
/// with the session host. The proxy reads the sequence from the shell's output and lifts the
/// panel (restoring its terminal modes). Returns whether it was sent: only inside a panel, and
/// only when there is a terminal to send it to.
pub(super) fn end_from_inside() -> bool {
    send_to_panel(STOP_SEQ)
}

/// Whether the chat box's rows are on the screen (see [`boxed::shown`]).
pub(super) fn chat_shown() -> bool {
    boxed::shown()
}

/// One report to the chat box; whether it was sent (see [`boxed::report`]).
pub(super) fn chat_report(body: &str) -> bool {
    boxed::report(body)
}

/// Writes `seq` to the controlling terminal for the panel proxy to read, only when a proxy
/// runs around this process: with none, the sequence would reach the real terminal, which may
/// print its bell as `^G`.
fn send_to_panel(seq: &[u8]) -> bool {
    use std::io::Write as _;
    if !inside() {
        return false;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .and_then(|mut tty| tty.write_all(seq).and_then(|()| tty.flush()))
        .is_ok()
}

/// Runs the shell (or the command) under the panel. `Ok(true)` means the session host stopped
/// while the panel was open.
#[allow(clippy::too_many_lines, reason = "the setup and the one select loop")]
pub(super) async fn run(g: &Global, env: Env, mut config: Config) -> Result<bool, Error> {
    if !fancy(g) || inside() {
        return run_plain(&config.command).map(|()| false);
    }
    let command = command_or_shell(&config.command);
    let (rows, cols) = host_size();
    let settings = Settings::read(g);
    // The chat box speaks to zsh's hook only (RFC 0212); the file is where its questions go.
    let question_file = (settings.chat != boxed::Setting::Off && boxed::speaks_zsh(&command))
        .then(boxed::QuestionFile::create)
        .flatten();
    let Shell { master, mut child } = launch(
        &command,
        rows,
        cols,
        question_file.as_ref().map(|f| f.0.as_path()),
    )?;

    let _raw = RawMode::enter()?;
    let mut from_child = reader(
        master
            .try_clone()
            .map_err(|e| io_err("cannot share the pty", e))?,
    );
    let mut from_user = reader(std::io::stdin());
    let (exit_tx, mut exit_rx) = mpsc::channel::<Result<(), Error>>(1);
    std::thread::spawn(move || {
        let r = child
            .wait()
            .map_err(|e| io_err("cannot wait for the shell", e))
            .and_then(exit_result);
        let _ = exit_tx.blocking_send(r);
    });
    let mut winch = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
        .map_err(|e| io_err("cannot watch the window size", e))?;

    let mut model = initial_model(&config);
    model.set_open_key(settings.open_key);
    let mut session = Session {
        proxy: Proxy::new(model, rows, cols, Look::detect(true), settings.mouse)
            .with_chat(settings.chat, question_file.as_ref().map(|f| f.0.clone())),
        master,
        reader: Reader::default(),
        stub: config.stub,
        g: g.clone(),
        env,
        asks_fetch: None,
        deny_fetch: None,
        workloads_fetch: None,
        auto_fetch: None,
        incidents_fetch: None,
        act_fetch: None,
        read_fetch: None,
        host_gone: config.host_gone.take().map(host_closed),
        host_stopped: false,
        project: config.project.take(),
        held: None,
        rejoin_fetch: None,
        esc_held: None,
    };
    session.redraw();
    session.refresh();

    let mut tick = tokio::time::interval(REFRESH);
    // The box's spinner and clock, only while something on it moves.
    let mut beat = tokio::time::interval(Duration::from_millis(100));
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut exit: Option<Result<(), Error>> = None;
    let mut exit_at: Option<Instant> = None;
    let mut user_open = true;

    loop {
        let esc_at = session.esc_held;
        tokio::select! {
            chunk = from_child.recv() => match chunk {
                Some(bytes) => session.shell_wrote(&bytes),
                None => break,
            },
            bytes = from_user.recv(), if user_open => match bytes {
                Some(bytes) => session.user_input(&bytes),
                None => user_open = false,
            },
            result = exit_rx.recv(), if exit.is_none() => {
                exit = Some(result.unwrap_or(Ok(())));
                exit_at = Some(Instant::now() + DRAIN);
            },
            () = async {
                match exit_at {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending().await,
                }
            } => break,
            () = async {
                match esc_at {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending().await,
                }
            } => session.esc_released(),
            _ = winch.recv() => session.resized(),
            _ = beat.tick(), if session.proxy.chat.animating() => session.chat_tick(),
            _ = tick.tick(), if !config.stub => session.refresh(),
            result = poll(&mut session.asks_fetch) => session.asks_done(result),
            result = poll(&mut session.workloads_fetch) => session.workloads_done(result),
            (id, result) = poll(&mut session.deny_fetch) => session.deny_done(&id, result),
            result = poll(&mut session.auto_fetch) => session.auto_done(result),
            result = poll(&mut session.incidents_fetch) => session.incidents_done(result),
            result = poll(&mut session.act_fetch) => session.act_done(result),
            result = poll(&mut session.read_fetch) => session.read_done(result),
            () = poll(&mut session.host_gone) => session.host_ended(),
            joined = poll(&mut session.rejoin_fetch) => session.rejoined(joined),
        }
    }

    // Leave the screen as the shell had it, without the panel and with the mouse reports off.
    session.proxy.stopped = true;
    let last = session.proxy.redraw();
    write_terminal(&last);
    if session.proxy.mouse_on {
        write_terminal(MOUSE_OFF);
    }
    let host_stopped = session.host_stopped;
    match exit {
        Some(r) => r,
        None => exit_rx.recv().await.unwrap_or(Ok(())),
    }
    .map(|()| host_stopped)
}

/// Resolves when the host closes; a dropped sender is not a stop, so that never resolves.
fn host_closed(rx: tokio::sync::oneshot::Receiver<()>) -> Fetch<()> {
    Box::pin(async move {
        if rx.await.is_err() {
            std::future::pending::<()>().await;
        }
    })
}

/// An action under the same limit as every other read.
fn limited(
    f: impl Future<Output = Result<String, Error>> + 'static,
) -> Fetch<Result<String, String>> {
    Box::pin(async move {
        match tokio::time::timeout(FETCH_LIMIT, f).await {
            Ok(r) => r.map_err(|e| e.to_string().lines().next().unwrap_or("").to_owned()),
            Err(_) => Err("it took too long".to_owned()),
        }
    })
}

fn read_limited(
    f: impl Future<Output = Result<Vec<String>, Error>> + 'static,
) -> Fetch<Result<Vec<String>, String>> {
    Box::pin(async move {
        match tokio::time::timeout(FETCH_LIMIT, f).await {
            Ok(r) => r.map_err(|e| e.to_string().lines().next().unwrap_or("").to_owned()),
            Err(_) => Err("it took too long".to_owned()),
        }
    })
}

fn read_incidents(g: Global, env: Env) -> Fetch<Result<PanelIncidents, Error>> {
    Box::pin(async move {
        match tokio::time::timeout(FETCH_LIMIT, incident_settings::read(&g, &env)).await {
            Ok(r) => r,
            Err(_) => Err(Error::Failed("the incident settings took too long".into())),
        }
    })
}

fn read_auto(g: Global, env: Env) -> Fetch<Result<PanelAuto, Error>> {
    Box::pin(async move {
        match tokio::time::timeout(FETCH_LIMIT, asks::auto_state(&g, &env)).await {
            Ok(r) => r,
            Err(_) => Err(Error::Failed("the auto mode took too long".into())),
        }
    })
}

/// What the demo shows for an ask's details: labelled stub.
fn stub_detail(id: &str) -> Vec<String> {
    vec![
        format!("id  {id}"),
        "stub: the real details come from the approvals service".into(),
    ]
}

fn read_asks(g: Global, env: Env) -> Fetch<Result<Vec<PanelAsk>, Error>> {
    Box::pin(async move {
        match tokio::time::timeout(FETCH_LIMIT, asks::pending_asks(&g, &env)).await {
            Ok(r) => r,
            Err(_) => Err(Error::Failed("the asks took too long".into())),
        }
    })
}

/// The sessions on this Mac and the coordinator's queue, read together: the registry and `ps`
/// run off the loop, the picture under the same limit as the asks. A queue that cannot be read
/// says why in the snapshot; the sessions are local and still shown.
fn read_workloads(g: Global, env: Env) -> Fetch<Result<Snapshot, Error>> {
    Box::pin(async move {
        let now = OffsetDateTime::now_utc();
        let local = tokio::task::spawn_blocking(move || {
            workloads::home_root().map_or_else(Vec::new, |root| {
                workloads::sessions(&root, now, &workloads::pid_alive, &workloads::ps_uses)
            })
        });
        let picture = tokio::time::timeout(FETCH_LIMIT, coordinator::picture(&g, &env));
        let (local, picture) = tokio::join!(local, picture);
        let sessions =
            local.map_err(|e| Error::Failed(format!("the sessions read failed: {e}")))?;
        let queue = match picture {
            Ok(Ok(v)) => Ok(workloads::queue(&v, now)),
            Ok(Err(e)) => Err(e.to_string().lines().next().unwrap_or("").to_owned()),
            Err(_) => Err("the queue took too long".to_owned()),
        };
        Ok(Snapshot { sessions, queue })
    })
}

fn deny(g: Global, env: Env, id: String) -> Fetch<(String, Result<(), Error>)> {
    Box::pin(async move {
        let r = match tokio::time::timeout(FETCH_LIMIT, asks::deny_quiet(&g, &env, &id)).await {
            Ok(r) => r,
            Err(_) => Err(Error::Failed("the deny took too long".into())),
        };
        (id, r)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(mouse: MouseMode) -> Proxy {
        let look = Look {
            fancy: true,
            truecolor: true,
        };
        Proxy::new(Model::new(true), 24, 100, look, mouse)
    }

    /// The rows of a terminal of `rows` x `cols` that has been fed `bytes`.
    fn rows_of(rows: u16, cols: u16, bytes: &[&[u8]]) -> Vec<String> {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        for b in bytes {
            parser.process(b);
        }
        parser.screen().rows(0, cols).collect()
    }

    #[test]
    fn a_resize_puts_back_what_the_old_panel_covered() {
        let (mut p, mut term_bytes) = (proxy(MouseMode::Menu), Vec::new());
        p.resize(24, 60);
        // The shell's output fills the top row; the terminal gets it and the panel on top.
        term_bytes.extend(p.child_output(&b"x".repeat(60)));
        assert!(p.on_screen.is_some(), "the panel is painted");
        let narrow = rows_of(24, 60, &[&term_bytes]);
        let badge = narrow.iter().position(|r| r.contains("iohr")).unwrap();
        let old_at = narrow[badge].find("iohr").unwrap();
        // A wider terminal keeps the cells as they were.
        let mut wide = vt100::Parser::new(24, 60, 0);
        wide.process(&term_bytes);
        wide.screen_mut().set_size(24, 100);
        let mut after = Vec::new();
        after.extend(p.resize(24, 100));
        wide.process(&after);
        let rows: Vec<String> = wide.screen().rows(0, 100).collect();
        assert_eq!(
            rows.iter().filter(|r| r.contains("iohr")).count(),
            1,
            "one badge, the live one: {rows:?}"
        );
        let new_at = rows[badge].find("iohr").unwrap();
        assert!(new_at > old_at, "at the new corner, not the old one");
    }

    /// A proxy with the chat box open (the hook has spoken), on a terminal of `rows` x `cols`.
    fn boxed_proxy(rows: usize, cols: usize) -> Proxy {
        let look = Look {
            fancy: true,
            truecolor: true,
        };
        let mut p = Proxy::new(Model::new(true), rows, cols, look, MouseMode::Menu).with_chat(
            boxed::Setting::On,
            Some(std::path::PathBuf::from("/nonexistent/q")),
        );
        p.chat.message(
            chat::Msg::Idle {
                mode: "manual".into(),
                ring: vec!["manual".into()],
            },
            std::time::Instant::now(),
        );
        p
    }

    /// Every line the terminal holds, scrolled-off ones first, then the screen's rows.
    fn history_and_screen(term: &mut vt100::Parser) -> (Vec<String>, Vec<String>) {
        let (rows, cols) = term.screen().size();
        let mut off = Vec::new();
        let mut at = usize::MAX;
        term.screen_mut().set_scrollback(usize::MAX);
        let top = term.screen().scrollback();
        // Read the scrolled-off lines a screen's height at a time from the oldest.
        let mut seen = 0;
        while seen < top {
            at = top - seen;
            term.screen_mut().set_scrollback(at);
            let page: Vec<String> = term.screen().rows(0, cols).collect();
            let take = usize::from(rows).min(top - seen);
            off.extend(page.into_iter().take(take));
            seen += take;
        }
        let _ = at;
        term.screen_mut().set_scrollback(0);
        (off, term.screen().rows(0, cols).collect())
    }

    fn count(rows: &[String], needle: &str) -> usize {
        rows.iter().filter(|r| r.contains(needle)).count()
    }

    #[test]
    fn card_output_scrolling_past_the_box_never_stacks_it() {
        let mut p = boxed_proxy(24, 100);
        let mut term = vt100::Parser::new(24, 100, 1000);
        let feed = |term: &mut vt100::Parser, bytes: Vec<u8>| term.process(&bytes);
        // The hook's first report opens the box and reserves its rows.
        feed(&mut term, p.shell_output(b"$ "));
        assert!(p.reserved, "the box's rows are reserved");
        // A question was sent: the box shows it and the command that runs.
        p.chat.message(chat::Msg::Busy, std::time::Instant::now());
        feed(&mut term, p.redraw());
        // Cards print line by line, a few times the screen's height, in small chunks.
        for round in 0..5 {
            for i in 0..30 {
                let line = format!("\x1b[1mcard {round}.{i}\x1b[0m\r\n");
                feed(&mut term, p.shell_output(line.as_bytes()));
            }
            // The tick repaints the box between chunks.
            feed(&mut term, p.redraw());
        }
        let (off, screen) = history_and_screen(&mut term);
        assert_eq!(count(&screen, "ask · manual"), 1, "one header: {screen:?}");
        assert_eq!(count(&screen, "a command is running"), 1, "{screen:?}");
        assert_eq!(count(&off, "ask · manual"), 0, "none scrolled off: {off:?}");
        assert_eq!(count(&off, "a command is running"), 0, "{off:?}");
    }

    #[test]
    fn a_scroll_region_reset_never_frees_the_box_s_rows() {
        let mut p = boxed_proxy(24, 100);
        let mut term = vt100::Parser::new(24, 100, 1000);
        term.process(&p.shell_output(b"$ "));
        assert!(p.reserved);
        p.chat.message(chat::Msg::Busy, std::time::Instant::now());
        term.process(&p.redraw());
        // A reset in one piece, split across two reads, with the full height named, and with
        // a top row: none may reach the terminal as the whole screen.
        let chunks: [&[u8]; 6] = [b"\x1b[r", b"\x1b[", b"r", b"\x1b[1;24r", b"\x1b[2;", b"30r"];
        for (n, c) in chunks.iter().enumerate() {
            term.process(&p.shell_output(c));
            for i in 0..40 {
                let line = format!("line {n}.{i}\r\n");
                term.process(&p.shell_output(line.as_bytes()));
            }
        }
        term.process(&p.redraw());
        let (off, screen) = history_and_screen(&mut term);
        assert_eq!(count(&screen, "ask · manual"), 1, "{screen:?}");
        assert_eq!(count(&off, "ask · manual"), 0, "{off:?}");
        assert_eq!(count(&screen, "a command is running"), 1, "{screen:?}");
        assert_eq!(count(&off, "a command is running"), 0, "{off:?}");
    }

    fn has(bytes: &[u8], needle: &[u8]) -> bool {
        find(bytes, needle).is_some()
    }

    #[test]
    fn mouse_reports_are_on_only_while_a_view_is_open() {
        let mut p = proxy(MouseMode::Menu);
        assert!(!has(&p.redraw(), MOUSE_ON), "off at rest");
        assert!(!p.mouse_on);

        p.model.key(Key::Menu);
        let opened = p.redraw();
        assert!(has(&opened, MOUSE_ON), "on when the menu opens");
        assert!(!has(&opened, b"?1002"), "never the drag reports");
        assert!(p.mouse_on);
        assert!(
            !has(&p.redraw(), MOUSE_ON),
            "turned on once, not at every redraw"
        );

        p.model.key(Key::Menu);
        let closed = p.redraw();
        assert!(has(&closed, MOUSE_OFF), "off again when the menu closes");
        assert!(!p.mouse_on);
    }

    #[test]
    fn the_mouse_is_off_for_a_program_on_the_alternate_screen_and_when_the_panel_stops() {
        let mut p = proxy(MouseMode::Menu);
        p.model.key(Key::Menu);
        p.redraw();
        assert!(p.mouse_on);

        let alt = p.child_output(b"\x1b[?1049h");
        assert!(
            has(&alt, MOUSE_OFF),
            "the program on the alternate screen owns the mouse"
        );
        assert!(!p.mouse_on);
        let back = p.child_output(b"\x1b[?1049l");
        assert!(
            has(&back, MOUSE_ON),
            "and the panel has it again when the program leaves"
        );

        let out = p.shell_output(b"a\x1b]7771;iohr-panel-stop\x07b");
        assert!(p.stopped, "`iohr stop` inside the panel lifts the panel");
        assert!(
            !has(&out, STOP_SEQ),
            "the sequence never reaches the terminal"
        );
        assert!(has(&out, MOUSE_OFF), "and the mouse reports end with it");
        assert!(has(&out, b"ab") || (has(&out, b"a") && has(&out, b"b")));
        assert!(!p.mouse_on);
    }

    #[test]
    fn the_resume_sequence_brings_a_stopped_panel_back_and_never_reaches_the_terminal() {
        let mut p = proxy(MouseMode::Menu);
        p.shell_output(STOP_SEQ);
        assert!(p.stopped);
        let out = p.shell_output(b"a\x1b]7771;iohr-panel-resume\x07b");
        assert!(!p.stopped, "running `iohr` again lifts the stop");
        assert!(!has(&out, RESUME_SEQ), "the sequence is the proxy's own");
        // Both in one read: the last one written wins.
        p.shell_output(&[STOP_SEQ, RESUME_SEQ].concat());
        assert!(!p.stopped);
        p.shell_output(&[RESUME_SEQ, STOP_SEQ].concat());
        assert!(p.stopped);
    }

    #[test]
    fn the_panel_is_believed_only_while_its_proxy_lives() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id().to_string();
        assert!(proxy_lives(&std::process::id().to_string()));
        assert!(proxy_lives("1"), "the old value, written before pids");
        assert!(proxy_lives("not a pid"));
        child.wait().unwrap();
        assert!(!proxy_lives(&pid), "a proxy that ended is no panel");
    }

    #[test]
    fn a_program_s_own_mouse_mode_is_put_back_when_the_panel_gives_the_mouse_up() {
        let mut p = proxy(MouseMode::Menu);
        p.child_output(b"\x1b[?1000h\x1b[?1006h");
        p.model.key(Key::Menu);
        assert!(has(&p.redraw(), MOUSE_ON));
        p.model.key(Key::Menu);
        let closed = p.redraw();
        assert!(has(&closed, MOUSE_OFF));
        assert!(
            has(&closed, b"\x1b[?1000h\x1b[?1006h"),
            "the program's reports come back"
        );
    }

    #[test]
    fn at_rest_the_mouse_follows_the_setting() {
        let mut off = proxy(MouseMode::Off);
        off.model.key(Key::Menu);
        assert!(!has(&off.redraw(), MOUSE_ON), "never with off");

        let mut always = proxy(MouseMode::Always);
        assert!(has(&always.redraw(), MOUSE_ON), "on at rest with always");
        let own = always.child_output(b"\x1b[?1000h");
        assert!(
            has(&own, MOUSE_OFF),
            "but not over a program that asked for the mouse itself"
        );
    }

    #[test]
    fn the_mouse_is_not_switched_inside_an_escape_sequence() {
        let mut p = proxy(MouseMode::Menu);
        p.model.key(Key::Menu);
        let cut = p.child_output(b"\x1b[3");
        assert!(!has(&cut, MOUSE_ON), "the chunk ends inside a sequence");
        let whole = p.child_output(b"1mx");
        assert!(has(&whole, MOUSE_ON), "so it comes with the next one");
    }

    #[test]
    fn a_click_lands_on_the_row_the_box_paints_there() {
        let mut p = proxy(MouseMode::Menu);
        p.model.key(Key::Menu);
        let f = p.layout().expect("fits");
        let (top, left, width) = (f.rect.top, f.rect.left, f.rect.width);
        // Body line `i` is painted on row `top + 1 + i`; its text is in the cells of that row.
        for (i, name) in ["Auto mode", "Approvals", "Workloads"].iter().enumerate() {
            let row: String = f.cells[1 + i].iter().map(|c| c.ch).collect();
            assert!(row.contains(name), "row {i} of the box is {name}: {row}");
            assert_eq!(p.hit(left + 3, top + 1 + i), Hit::Line(i));
        }
        assert_eq!(p.hit(left, top + 1), Hit::Frame, "the side border");
        assert_eq!(p.hit(left + width - 1, top + 1), Hit::Frame);
        assert_eq!(p.hit(left + 3, top), Hit::Frame, "the title row");
        assert_eq!(p.hit(left.saturating_sub(1), top + 1), Hit::Outside);
        assert_eq!(p.hit(left + 3, top + f.rect.height), Hit::Outside);

        // A click on a body line is the same as Enter on it.
        let mut by_key = proxy(MouseMode::Menu);
        by_key.model.key(Key::Menu);
        by_key.model.key(Key::Down);
        by_key.model.key(Key::Enter);
        p.model.click(1);
        assert_eq!(p.model.view, by_key.model.view);
    }

    #[test]
    fn bare_inside_says_the_panel_is_on_and_names_the_key() {
        let msg = panel_is_on(OpenKey::detect(Some("Apple_Terminal"), false));
        assert_eq!(msg, "the panel is on; ^G opens the menu");
        let msg = panel_is_on(OpenKey::detect(Some("xterm"), false));
        assert!(msg.contains("⌥i"), "{msg}");
    }

    #[test]
    fn the_pill_is_where_a_click_at_rest_lands() {
        let p = proxy(MouseMode::Always);
        let f = p.layout().expect("fits");
        assert_eq!(p.hit(f.rect.left + 3, f.rect.top + 1), Hit::Line(0));
        assert_eq!(p.hit(0, 10), Hit::Outside);
    }
}
