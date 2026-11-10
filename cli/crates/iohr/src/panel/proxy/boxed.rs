//! The chat box's side of the proxy (RFC 0212): the rows it reserves at the bottom of the
//! terminal, the reports it takes out of the shell's output, the keys and mouse clicks it
//! owns, and the hand-over of a question to the shell.
//!
//! The box is four rows. While it is open the proxy sets a scroll region over every row above
//! it and tells the shell's pty it is that much shorter, so the shell and the programs it runs
//! never write under the box; the box is drawn again after every chunk (a `clear` wipes it).
//! The reservation changes only between escape sequences, steps aside on the alternate screen
//! (a full screen program gets every row) and ends with the panel.

use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant as StdInstant;

use super::*;
use crate::panel::chat::{self, Chat, Do, Hit as ChatHit, Laid, Msg};

/// A scroll region is set on the terminal now: if the proxy ends without releasing it, the
/// terminal's guard does ([`release`]).
pub(super) static REGION: AtomicBool = AtomicBool::new(false);

/// Set in the shell's environment to the file a question is handed over in: the hook reports to
/// the chat box only when it is there.
pub(super) const FILE_ENV: &str = "IOHR_PANEL_CHAT_FILE";

/// `[panel] chat` (`IOHR_PANEL_CHAT` overrides it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Setting {
    /// The box is shown (the default).
    #[default]
    On,
    /// The box starts folded to its corner chip.
    Hidden,
    /// No box: the panel is as it was.
    Off,
}

impl Setting {
    /// The setting a word names; anything that is not `hidden` or `off` is the default.
    pub(super) fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("hidden") => Self::Hidden,
            Some("off" | "false" | "no" | "0") => Self::Off,
            _ => Self::On,
        }
    }
}

/// The bytes that give the box's rows back: the rows blanked (unless a program owns the
/// screen), the scroll region reset, the cursor where it was.
pub(super) fn release(rows: usize, blank: bool) -> Vec<u8> {
    let mut out = SAVE.to_vec();
    if blank {
        for r in rows.saturating_sub(chat::ROWS)..rows {
            out.extend_from_slice(format!("\x1b[{};1H\x1b[0m\x1b[2K", r + 1).as_bytes());
        }
    }
    out.extend_from_slice(b"\x1b[r");
    out.extend_from_slice(RESTORE);
    out
}

/// The private file a question is handed over in, removed when the panel ends.
pub(super) struct QuestionFile(pub(super) PathBuf);

impl QuestionFile {
    /// `~/.inorbit/term/chat-<pid>`, empty, readable by this person only.
    pub(super) fn create() -> Option<Self> {
        let dir = workloads::home_root()?.join("term");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .ok()?;
        let path = dir.join(format!("chat-{}", std::process::id()));
        write_private(&path, b"").ok()?;
        Some(Self(path))
    }
}

impl Drop for QuestionFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(shown_marker(&self.0));
    }
}

/// The file that exists while the box's rows are on the screen, beside the question file: the
/// ask process looks for it to know whether the box shows its progress (else it draws its own
/// spinner, as it did before the box).
pub(crate) fn shown_marker(question_file: &Path) -> PathBuf {
    let mut name = question_file.as_os_str().to_owned();
    name.push(".shown");
    PathBuf::from(name)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

/// Whether the chat box's rows are on the screen of the panel around this process.
pub(super) fn shown() -> bool {
    inside()
        && std::env::var_os(FILE_ENV)
            .map(PathBuf::from)
            .is_some_and(|f| shown_marker(&f).exists())
}

/// One report to the chat box (`step;3;-;reading 3 files`), as the hook sends them: written to
/// the terminal, where the panel takes it out of the output. Only when the box is on the
/// screen; whether it was sent.
pub(super) fn report(body: &str) -> bool {
    let body: String = body
        .chars()
        .filter(|c| !c.is_control() || *c == ';')
        .collect();
    shown() && send_to_panel(format!("\x1b]7771;chat;{body}\x07").as_bytes())
}

/// Whether the shell to run is one the hook speaks to.
pub(super) fn speaks_zsh(command: &[String]) -> bool {
    command_or_shell(command)
        .first()
        .and_then(|c| Path::new(c).file_name())
        .is_some_and(|n| n == "zsh")
}

impl Proxy {
    /// The rows the shell has: the screen's, less the box's while it is reserved.
    pub(super) fn child_rows(&self) -> usize {
        if self.reserved {
            self.rows.saturating_sub(chat::ROWS)
        } else {
            self.rows
        }
    }

    /// The box wants its rows: open, and there is room, and nothing else owns the screen.
    fn want_region(&self) -> bool {
        self.chat.open()
            && !self.stopped
            && !self.screen.alt()
            && self.rows >= chat::MIN_ROWS
            && self.cols >= cells::MIN_COLS
    }

    /// Reserves the box's rows or gives them back, when that changed and the terminal is
    /// between escape sequences. Returns the bytes for the terminal.
    pub(super) fn region(&mut self) -> Vec<u8> {
        if !self.boundary.clean() {
            return Vec::new();
        }
        let want = self.want_region();
        if want == self.reserved {
            return Vec::new();
        }
        let alt = self.screen.alt();
        let mut out = self.lift();
        let child = self.rows.saturating_sub(chat::ROWS);
        if want {
            // The shell's cursor may be in the rows to be taken: scroll up first so it is not.
            let (row, col) = self.screen.cursor();
            let shift = (row + 1).saturating_sub(child);
            if shift > 0 {
                out.extend_from_slice(SAVE);
                out.extend_from_slice(format!("\x1b[{};1H", self.rows).as_bytes());
                out.extend(std::iter::repeat_n(b'\n', shift));
                out.extend_from_slice(RESTORE);
                out.extend_from_slice(format!("\x1b[{shift}A").as_bytes());
                let mut model = format!("\x1b[{};1H", self.rows).into_bytes();
                model.extend(std::iter::repeat_n(b'\n', shift));
                model
                    .extend_from_slice(format!("\x1b[{};{}H", row + 1 - shift, col + 1).as_bytes());
                self.screen.feed(&model);
            }
            // Setting the region homes the cursor; the saved one brings it back.
            out.extend_from_slice(SAVE);
            out.extend_from_slice(format!("\x1b[1;{child}r").as_bytes());
            out.extend_from_slice(RESTORE);
            self.reserved = true;
            self.screen.resize(child, self.cols);
        } else {
            out.extend(release(self.rows, !alt));
            self.reserved = false;
            self.screen.resize(self.rows, self.cols);
        }
        REGION.store(self.reserved, Ordering::Relaxed);
        self.mark_shown();
        out
    }

    /// Keeps [`shown_marker`] in step with the reserved rows.
    fn mark_shown(&self) {
        let Some(path) = &self.chat_file else {
            return;
        };
        let marker = shown_marker(path);
        if self.reserved {
            let _ = write_private(&marker, b"");
        } else {
            let _ = std::fs::remove_file(marker);
        }
    }

    /// The terminal reset itself (`ESC c`): the region is gone, so the next chance sets it again.
    pub(super) fn region_reset(&mut self, chunk: &[u8]) {
        if self.reserved && find(chunk, b"\x1bc").is_some() {
            self.reserved = false;
            self.screen.resize(self.rows, self.cols);
            REGION.store(false, Ordering::Relaxed);
            self.mark_shown();
        }
    }

    /// What the terminal gets of the shell's output: while the box's rows are reserved, a
    /// program's bare `ESC [ r` (reset the scroll region) means its own height, not the
    /// screen's, so it is rewritten to the region's.
    pub(super) fn terminal_copy(&self, chunk: &[u8]) -> Vec<u8> {
        if !self.reserved {
            return chunk.to_vec();
        }
        let region = format!("\x1b[1;{}r", self.child_rows()).into_bytes();
        let mut out = Vec::with_capacity(chunk.len());
        let mut i = 0;
        while i < chunk.len() {
            let rest = &chunk[i..];
            if rest.starts_with(b"\x1b[r") {
                out.extend_from_slice(&region);
                i += 3;
            } else if rest.starts_with(b"\x1b[;r") {
                out.extend_from_slice(&region);
                i += 4;
            } else {
                out.push(chunk[i]);
                i += 1;
            }
        }
        out
    }

    /// One report from the shell's hook.
    pub(super) fn chat_msg(&mut self, msg: Msg) {
        // A `?` the box cannot take (folded, no room) goes back to the line as a plain `?`.
        if matches!(msg, Msg::Focus) && !(self.reserved && self.chat.open()) {
            self.to_shell.extend_from_slice(chat::INLINE_QM);
            return;
        }
        self.chat.message(msg, StdInstant::now());
    }

    /// What the proxy owes the shell (bytes to type into it), taken.
    pub(super) fn take_to_shell(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.to_shell)
    }

    /// The open box, laid out, when it is on screen.
    pub(super) fn chat_laid(&self, now: StdInstant) -> Option<Laid> {
        (self.reserved && self.chat.open() && !self.stopped)
            .then(|| self.chat.layout(self.cols, self.child_rows(), now))
    }

    /// The folded box's chip: where it goes and what it shows.
    pub(super) fn chip_paint(&self, now: StdInstant) -> Option<(Rect, Vec<cells::Cell>)> {
        if !self.chat.folded() || self.stopped || self.reserved || self.rows < 6 {
            return None;
        }
        let cells = self.chat.chip_cells(now);
        let width = cells.len();
        if width + 2 > self.cols {
            return None;
        }
        let rect = Rect {
            top: self.rows - 1,
            left: self.cols - width - 1,
            height: 1,
            width,
        };
        Some((rect, cells))
    }

    /// The box's rows as paint segments.
    pub(super) fn box_segments(&self, now: StdInstant) -> Vec<(usize, usize, Vec<cells::Cell>)> {
        self.chat_laid(now).map_or_else(Vec::new, |laid| {
            let f = &laid.floating;
            f.cells
                .iter()
                .enumerate()
                .map(|(i, row)| (f.rect.top + i, f.rect.left, row.clone()))
                .collect()
        })
    }

    /// Hands a question to the shell: its mode and line in the private file.
    fn deliver(&self, line: &str, mode: &str) -> std::io::Result<()> {
        let Some(path) = &self.chat_file else {
            return Err(std::io::Error::other("no question file"));
        };
        write_private(path, format!("{mode}\n{line}\n").as_bytes())
    }
}

impl Session {
    /// Writes what the proxy made for the terminal, then gives the shell the size it now has
    /// and whatever the proxy owes it.
    pub(super) fn emit(&mut self, bytes: &[u8]) {
        write_terminal(bytes);
        self.push_size();
        let owed = self.proxy.take_to_shell();
        if !owed.is_empty() {
            let _ = self.master.write_all(&owed);
        }
    }

    /// Draws the panel and the box as the model has them.
    pub(super) fn redraw(&mut self) {
        let bytes = self.proxy.redraw();
        self.emit(&bytes);
    }

    /// The shell's pty has the rows the box leaves it.
    pub(super) fn push_size(&mut self) {
        let want = (self.proxy.child_rows(), self.proxy.cols);
        if want != self.proxy.pty_size {
            let _ = rustix::termios::tcsetwinsize(&self.master, winsize(want.0, want.1));
            self.proxy.pty_size = want;
        }
    }

    /// A tick while something on the box moves: a question nobody started is given up, and
    /// the spinner and clock draw again.
    pub(super) fn chat_tick(&mut self) {
        self.proxy.chat.expire(StdInstant::now());
        self.redraw();
    }

    fn apply_do(&mut self, d: Do) {
        match d {
            Do::None => {}
            Do::Stop => {
                let _ = self.master.write_all(&[0x03]);
            }
            Do::Forward(bytes) => {
                let _ = self.master.write_all(&bytes);
            }
            Do::Send { line, mode } => {
                if self.proxy.deliver(&line, &mode).is_ok() {
                    let _ = self.master.write_all(chat::DELIVER);
                } else {
                    self.proxy.chat.at_prompt = true;
                    self.proxy.chat.delivered = None;
                }
            }
        }
    }

    /// The person's keys, when the box has a say in them. Returns whether it took them.
    /// Ctrl-] folds and brings back the box (on the alternate screen it is the program's);
    /// Esc stops a running answer; with the focus, the box reads the keys, except the
    /// panel's own menu key and mouse reports.
    pub(super) fn chat_input(&mut self, bytes: &[u8]) -> bool {
        let p = &self.proxy;
        if p.stopped || p.screen.alt() || !p.chat.available() {
            return false;
        }
        if bytes == [0x1d] {
            self.proxy.chat.toggle();
            self.redraw();
            return true;
        }
        let now = StdInstant::now();
        if bytes == [0x1b] && p.chat.answering() && !p.chat.waiting_for_person() {
            let d = self.proxy.chat.press(&chat::ChatKey::Esc, now);
            self.apply_do(d);
            self.redraw();
            return true;
        }
        // The box has the keys only while it is on the screen: with its rows given back (a
        // small terminal, a resize) nothing shows that it holds them.
        if p.chat.focus && !p.reserved {
            self.proxy.chat.focus = false;
            return false;
        }
        let p = &self.proxy;
        if !p.chat.focus
            && p.reserved
            && !p.model.open()
            && !self.reader.pending()
            && !input::is_mouse_report(bytes)
            && !p.model.open_key().is(bytes)
            && let Some(dos) = self.proxy.chat.open_with(bytes, now)
        {
            for d in dos {
                self.apply_do(d);
            }
            self.redraw();
            return true;
        }
        let p = &self.proxy;
        if !p.chat.focus
            || p.model.open()
            || self.reader.pending()
            || input::is_mouse_report(bytes)
            || p.model.open_key().is(bytes)
        {
            return false;
        }
        for d in self.proxy.chat.feed(bytes, now) {
            self.apply_do(d);
        }
        self.redraw();
        true
    }

    /// A mouse report on the box (or its chip). Returns whether it was on it.
    pub(super) fn chat_mouse(&mut self, m: Mouse) -> bool {
        if self.proxy.model.open() {
            return false;
        }
        let (col, row, press) = match m {
            Mouse::Press { col, row } => (col, row, true),
            Mouse::Release { col, row }
            | Mouse::WheelUp { col, row }
            | Mouse::WheelDown { col, row } => (col, row, false),
        };
        let now = StdInstant::now();
        if let Some(laid) = self.proxy.chat_laid(now) {
            let r = laid.rect();
            if row < r.top || row >= r.top + r.height || col >= r.left + r.width {
                return false;
            }
            if press {
                let chat = &mut self.proxy.chat;
                match Chat::hit(&laid, col, row) {
                    ChatHit::Hide => chat.hide(),
                    ChatHit::Chip => chat.cycle_mode(),
                    ChatHit::Stop if chat.answering() && !chat.waiting_for_person() => {
                        let _ = self.master.write_all(&[0x03]);
                    }
                    ChatHit::Input if chat.at_prompt && !chat.answering() => chat.focus_input(),
                    _ => {}
                }
            }
            return true;
        }
        if let Some((r, _)) = self.proxy.chip_paint(now) {
            if row != r.top || col < r.left || col >= r.left + r.width {
                return false;
            }
            if press {
                self.proxy.chat.show();
            }
            return true;
        }
        false
    }
}
