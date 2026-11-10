//! The chat box (RFC 0212, PRD 0202): a framed box of four rows at the bottom of the panel's
//! terminal where `?` and `??` questions are typed, sent with one Enter, followed with a
//! progress bar and stopped with Esc or a click.
//!
//! This module is the box's state and its drawing; no input or output happens here. The proxy
//! owns the rows (a scroll region keeps the shell out of them), feeds the person's keys to
//! [`Chat::feed`] while the box has focus, and takes the shell's reports out of its output with
//! [`extract`]. The shell side is `hook.zsh`: it says when the prompt is ready, when a command
//! runs, and (through the ask process) how far an answer has got, all as private operating
//! system commands (`ESC ] 7771 ; chat ; VERB BEL`) that no terminal shows.
//!
//! The box draws with the same cells as the floating panel ([`super::cells`]) and takes its
//! spinner from the answer card ([`crate::card::spinner`]), so there is one look.

use std::time::{Duration, Instant};

use super::cells::{
    ACC, AMBER, BoxSpec, Cell, FAINT, Floating, GREEN, KEY, Line, MUTE, PBG, RED, Rect, Span, TXT,
    bold, lay_out, one_wide, sp,
};
use super::input::HIDE_LABEL;

/// The rows the box takes: top border, the input, the status, the bottom border.
pub(super) const ROWS: usize = 4;
/// Below this many rows the box is not drawn: it would leave the shell too little room.
pub(super) const MIN_ROWS: usize = 16;
/// The key the proxy writes into the shell to deliver a question: one no keyboard sends, bound
/// by the hook to the widget that reads the question from the private file.
pub(super) const DELIVER: &[u8] = b"\x1b[27771~";
/// What the proxy types into the shell instead of `?` when the box is not there to take it
/// (folded, or the terminal too small): the shell puts a plain `?` on its line.
pub(super) const INLINE_QM: &[u8] = b"\x1b[27772~";
/// What the shell's reports start with.
const PREFIX: &[u8] = b"\x1b]7771;chat;";
/// The longest report body that is believed; a longer one is not ours.
const MAX_BODY: usize = 512;
/// The longest question sent.
const MAX_QUESTION: usize = 2000;
/// How long a delivered question may take to show as a running ask (or a prompt again) before
/// the box believes the shell is ready for a question again.
pub(super) const DELIVER_LIMIT: Duration = Duration::from_secs(3);

/// What the shell side reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Msg {
    /// `?` at an empty prompt: focus the box (a second one means `??`).
    Focus,
    /// The prompt is ready: the answer mode now and the modes Shift+Tab goes through.
    Idle { mode: String, ring: Vec<String> },
    /// A command runs.
    Busy,
    /// An ask started.
    Begin,
    /// An ask made progress: steps so far, the total when known, and what it is doing.
    Step {
        n: usize,
        total: Option<usize>,
        label: String,
    },
    /// The ask ended.
    End,
    /// The ask waits for the person (a question back); the box steps aside for the terminal.
    Back(bool),
    /// The shell's Shift+Tab changed the answer mode.
    Mode(String),
}

fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
}

fn parse_msg(body: &[u8]) -> Option<Msg> {
    let body = std::str::from_utf8(body).ok()?;
    let mut f = body.splitn(4, ';');
    let verb = f.next()?;
    let (a, b, c) = (f.next(), f.next(), f.next());
    Some(match verb {
        "focus" => Msg::Focus,
        "busy" => Msg::Busy,
        "begin" => Msg::Begin,
        "end" => Msg::End,
        "idle" => Msg::Idle {
            mode: clean(a.unwrap_or(""), 16),
            ring: b
                .unwrap_or("")
                .split(',')
                .map(|m| clean(m, 16))
                .filter(|m| !m.is_empty())
                .take(8)
                .collect(),
        },
        "step" => Msg::Step {
            n: a?.parse().ok()?,
            total: b.and_then(|t| t.parse().ok()),
            label: clean(c.unwrap_or(""), 60),
        },
        "back" => Msg::Back(a == Some("1")),
        "mode" => Msg::Mode(clean(a.unwrap_or(""), 16)),
        _ => return None,
    })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// How many bytes at the end of `data` begin [`PREFIX`] without completing it.
fn partial(data: &[u8]) -> usize {
    (1..PREFIX.len().min(data.len() + 1))
        .rev()
        .find(|&n| data.ends_with(&PREFIX[..n]))
        .unwrap_or(0)
}

/// Takes the shell's reports out of its output. `held` carries a report (or the start of one)
/// a chunk ended in the middle of; the first value is the output without the reports.
pub(super) fn extract(held: &mut Vec<u8>, chunk: &[u8]) -> (Vec<u8>, Vec<Msg>) {
    held.extend_from_slice(chunk);
    let data = std::mem::take(held);
    let mut out = Vec::with_capacity(data.len());
    let mut msgs = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let Some(at) = find(&data[i..], PREFIX) else {
            let keep = partial(&data[i..]);
            out.extend_from_slice(&data[i..data.len() - keep]);
            *held = data[data.len() - keep..].to_vec();
            return (out, msgs);
        };
        out.extend_from_slice(&data[i..i + at]);
        let start = i + at + PREFIX.len();
        match data[start..].iter().position(|&b| b == 0x07) {
            Some(end) if end <= MAX_BODY => {
                msgs.extend(parse_msg(&data[start..start + end]));
                i = start + end + 1;
            }
            None if data.len() - start <= MAX_BODY => {
                *held = data[i + at..].to_vec();
                return (out, msgs);
            }
            // Too long to be ours: the prefix goes through as it came.
            _ => {
                out.extend_from_slice(&data[i + at..start]);
                i = start;
            }
        }
    }
    (out, msgs)
}

/// A key as the box reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChatKey {
    Char(char),
    Enter,
    Esc,
    CtrlC,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    /// Ctrl-W: the word before the cursor.
    Word,
    /// Ctrl-U: the line before the cursor.
    Kill,
    /// Shift+Tab: the next answer mode.
    BackTab,
}

/// What a key asks of the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Do {
    None,
    /// Hand this question to the shell, in this answer mode.
    Send {
        line: String,
        mode: String,
    },
    /// Stop the answer: the proxy writes Ctrl-C to the shell's terminal.
    Stop,
    /// Bytes for the shell as if the box were not there (Down opens the answer's menu).
    Forward(Vec<u8>),
}

/// Where a click landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    Outside,
    Hide,
    Chip,
    Stop,
    Input,
}

/// The shell's own answer to an ask, as far as the box knows.
#[derive(Debug, Clone)]
struct Run {
    since: Instant,
    steps: usize,
    total: Option<usize>,
    label: String,
}

/// The box.
#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent facts the shell and the person set"
)]
pub(super) struct Chat {
    /// `[panel] chat` is not `off`, and the shell is one the hook speaks to.
    pub(super) enabled: bool,
    /// The shell's hook has reported in: without it there is nothing to talk to.
    pub(super) hook_seen: bool,
    /// Folded to the corner chip.
    pub(super) hidden: bool,
    /// The box takes the keys.
    pub(super) focus: bool,
    /// `??`: the next question goes deeper.
    deeper: bool,
    text: Vec<char>,
    cursor: usize,
    /// The line was sent and its answer is being followed.
    sent: bool,
    run: Option<Run>,
    back: bool,
    /// The shell is at its prompt and can take a question.
    pub(super) at_prompt: bool,
    /// When the last question was handed over, until the shell shows it started.
    pub(super) delivered: Option<Instant>,
    modes: Vec<String>,
    mode: String,
    last: String,
    paste: bool,
    /// The start of a key sequence that was cut at the end of a read, kept for the next one.
    held: Vec<u8>,
    /// Nothing the person typed is on the shell's line: `? text` may open the box with its text.
    clean: bool,
    /// Keys went to the shell while it was not at its prompt (type-ahead): its next line is
    /// not clean.
    ahead: bool,
}

impl Chat {
    pub(super) fn new(enabled: bool, hidden: bool) -> Self {
        Self {
            enabled,
            hook_seen: false,
            hidden,
            focus: false,
            deeper: false,
            text: Vec::new(),
            cursor: 0,
            sent: false,
            run: None,
            back: false,
            at_prompt: false,
            delivered: None,
            modes: Vec::new(),
            mode: String::new(),
            last: String::new(),
            paste: false,
            held: Vec::new(),
            clean: false,
            ahead: false,
        }
    }

    /// The box can be shown or folded at all.
    pub(super) fn available(&self) -> bool {
        self.enabled && self.hook_seen
    }

    /// The box is open (its rows are wanted).
    pub(super) fn open(&self) -> bool {
        self.available() && !self.hidden
    }

    /// Only the corner chip shows.
    pub(super) fn folded(&self) -> bool {
        self.available() && self.hidden
    }

    /// An answer is being followed.
    pub(super) fn answering(&self) -> bool {
        self.run.is_some()
    }

    /// The ask waits for the person in the terminal.
    pub(super) fn waiting_for_person(&self) -> bool {
        self.back
    }

    /// Something on the box moves (the spinner, the clock, a delivery being waited on).
    pub(super) fn animating(&self) -> bool {
        self.available() && (self.run.is_some() || self.delivered.is_some())
    }

    /// A delivered question that showed no sign of starting within [`DELIVER_LIMIT`] (the key
    /// was lost, or the shell was busy after all): the box takes questions again. Whether it
    /// did.
    pub(super) fn expire(&mut self, now: Instant) -> bool {
        match self.delivered {
            Some(at) if now.saturating_duration_since(at) >= DELIVER_LIMIT => {
                self.delivered = None;
                self.at_prompt = true;
                self.sent = false;
                true
            }
            _ => false,
        }
    }

    pub(super) fn toggle(&mut self) {
        self.hidden = !self.hidden;
        if self.hidden {
            self.focus = false;
        }
    }

    /// Folds the box (the menu entry's other half, and `Hide`).
    pub(super) fn hide(&mut self) {
        self.hidden = true;
        self.focus = false;
    }

    pub(super) fn show(&mut self) {
        self.hidden = false;
    }

    fn reset_line(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.sent = false;
        self.deeper = false;
    }

    /// Takes the keys, with an empty line (a click on the input, or the shell's `?`).
    pub(super) fn focus_input(&mut self) {
        if !self.focus {
            self.reset_line();
        }
        self.focus = true;
    }

    /// The next answer mode of the ring (Shift+Tab, a click on the chip).
    pub(super) fn cycle_mode(&mut self) {
        if self.modes.is_empty() {
            return;
        }
        let at = self.modes.iter().position(|m| *m == self.mode);
        let next = at.map_or(0, |i| (i + 1) % self.modes.len());
        self.mode.clone_from(&self.modes[next]);
    }

    /// What the shell reported.
    pub(super) fn message(&mut self, msg: Msg, now: Instant) {
        match msg {
            Msg::Focus => {
                // A folded box leaves `?` to the line (the proxy types it back), so a person
                // who hid the box still asks inline as before.
                if !self.open() {
                    return;
                }
                self.at_prompt = true;
                if self.focus {
                    self.deeper = true;
                } else {
                    self.focus_input();
                }
            }
            Msg::Idle { mode, ring } => {
                self.hook_seen = true;
                self.at_prompt = true;
                self.clean = !self.ahead;
                self.ahead = false;
                self.delivered = None;
                self.run = None;
                self.back = false;
                if !ring.is_empty() {
                    self.modes = ring;
                }
                self.mode = mode;
            }
            Msg::Busy => {
                self.hook_seen = true;
                self.at_prompt = false;
                self.focus = false;
            }
            Msg::Begin => {
                self.delivered = None;
                self.back = false;
                self.at_prompt = false;
                self.run = Some(Run {
                    since: now,
                    steps: 0,
                    total: None,
                    label: String::new(),
                });
            }
            Msg::Step { n, total, label } => {
                self.delivered = None;
                let run = self.run.get_or_insert_with(|| Run {
                    since: now,
                    steps: 0,
                    total: None,
                    label: String::new(),
                });
                run.steps = n;
                run.total = total.filter(|t| *t > 0);
                run.label = label;
            }
            Msg::End => {
                self.run = None;
                self.back = false;
            }
            Msg::Back(b) => {
                self.back = b;
                if b {
                    self.focus = false;
                    self.delivered = None;
                }
            }
            Msg::Mode(m) => self.mode = m,
        }
    }

    /// Sends the line: the shell is at its prompt and the line has text.
    fn send(&mut self, now: Instant) -> Do {
        let q: String = self.text.iter().collect::<String>().trim().to_owned();
        if q.is_empty() || !self.at_prompt || self.run.is_some() {
            return Do::None;
        }
        let q: String = q.chars().take(MAX_QUESTION).collect();
        let line = format!("{} {q}", if self.deeper { "??" } else { "?" });
        self.last = q;
        self.sent = true;
        self.focus = false;
        self.at_prompt = false;
        self.delivered = Some(now);
        Do::Send {
            line,
            mode: self.mode.clone(),
        }
    }

    /// The person's bytes while the box has focus. A key sequence cut at the end of the read
    /// (`ESC [`, a UTF-8 lead byte) waits for the rest in the next one; a lone `ESC` is the
    /// Esc key.
    pub(super) fn feed(&mut self, bytes: &[u8], now: Instant) -> Vec<Do> {
        let mut data = std::mem::take(&mut self.held);
        data.extend_from_slice(bytes);
        let cut = data.len() - unfinished(&data);
        self.held = data.split_off(cut);
        parse(&data, &mut self.paste)
            .into_iter()
            .map(|k| self.press(&k, now))
            .collect()
    }

    /// Bytes of the person's went to the shell: its line is no longer clean.
    pub(super) fn forwarded(&mut self) {
        if self.at_prompt {
            self.clean = false;
        } else {
            self.ahead = true;
        }
    }

    /// `? text`, `?? text`, as one write or a bracketed paste, at a clean prompt: the shell's
    /// `?` key never sees it (a paste skips key bindings), so the box takes it: it gets the
    /// focus with the text, and nothing goes to the shell's line. Returns the box's answers
    /// when it took the bytes.
    pub(super) fn open_with(&mut self, bytes: &[u8], now: Instant) -> Option<Vec<Do>> {
        if self.focus || !self.open() || !self.at_prompt || !self.clean || self.run.is_some() {
            return None;
        }
        let (paste, rest) = match bytes.strip_prefix(b"\x1b[200~") {
            Some(rest) => (true, rest),
            None => (false, bytes),
        };
        let rest = rest.strip_prefix(b"?")?;
        let (deeper, rest) = match rest.strip_prefix(b"?") {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let rest = rest.strip_prefix(b" ")?;
        self.focus_input();
        self.deeper = deeper;
        self.paste = paste;
        Some(self.feed(rest, now))
    }

    /// One key.
    pub(super) fn press(&mut self, key: &ChatKey, now: Instant) -> Do {
        let editing = self.focus && self.run.is_none();
        match key {
            ChatKey::CtrlC | ChatKey::Esc if self.run.is_some() && !self.back => Do::Stop,
            ChatKey::BackTab => {
                self.cycle_mode();
                Do::None
            }
            _ if !editing => Do::None,
            ChatKey::Enter => {
                if self.text.is_empty() {
                    self.focus = false;
                    Do::None
                } else {
                    self.send(now)
                }
            }
            ChatKey::Esc | ChatKey::CtrlC => {
                if self.text.is_empty() && !self.deeper {
                    self.focus = false;
                } else {
                    self.reset_line();
                }
                Do::None
            }
            ChatKey::Up => {
                if !self.last.is_empty() {
                    self.text = self.last.chars().collect();
                    self.cursor = self.text.len();
                    self.sent = false;
                }
                Do::None
            }
            ChatKey::Down => {
                self.focus = false;
                Do::Forward(b"\x1b[B".to_vec())
            }
            ChatKey::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                Do::None
            }
            ChatKey::Right => {
                self.cursor = (self.cursor + 1).min(self.text.len());
                Do::None
            }
            ChatKey::Home => {
                self.cursor = 0;
                Do::None
            }
            ChatKey::End => {
                self.cursor = self.text.len();
                Do::None
            }
            ChatKey::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.text.remove(self.cursor);
                } else if self.text.is_empty() && self.deeper {
                    self.deeper = false;
                }
                Do::None
            }
            ChatKey::Delete => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                }
                Do::None
            }
            ChatKey::Word => {
                let mut start = self.cursor;
                while start > 0 && self.text[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && self.text[start - 1] != ' ' {
                    start -= 1;
                }
                self.text.drain(start..self.cursor);
                self.cursor = start;
                Do::None
            }
            ChatKey::Kill => {
                self.text.drain(..self.cursor);
                self.cursor = 0;
                Do::None
            }
            ChatKey::Char(c) => {
                // `?` first on an empty line is the second `?`: deeper.
                if *c == '?' && self.text.is_empty() && !self.deeper {
                    self.deeper = true;
                } else if !c.is_control() && self.text.len() < MAX_QUESTION {
                    self.text.insert(self.cursor, *c);
                    self.cursor += 1;
                    self.sent = false;
                }
                Do::None
            }
        }
    }

    /// The chip's text: what a question will be, and in which mode.
    fn chip(&self) -> String {
        let kind = if self.deeper { "deeper" } else { "ask" };
        if self.mode.is_empty() {
            kind.to_owned()
        } else {
            format!("{kind} · {}", self.mode)
        }
    }

    fn mode_color(&self) -> crate::card::Rgb {
        match self.mode.as_str() {
            "auto" => AMBER,
            "plan" => GREEN,
            _ => KEY,
        }
    }

    /// The box as cells at `top` for a screen `width` wide, with what a click can reach.
    pub(super) fn layout(&self, width: usize, top: usize, now: Instant) -> Laid {
        let width = width.max(24);
        let inner = width - 2;
        let running = self.run.is_some();

        // Title: the mark, the name and the chip.
        let chip = self.chip();
        let title = vec![
            sp("◆ ", ACC),
            bold("iohr", ACC),
            sp("  ", MUTE),
            bold(&chip, self.mode_color()),
        ];
        let title_len = 2 + 4 + 2 + chip.chars().count();
        let right_text = format!("⌄ hide {HIDE_LABEL}");
        let right_text = right_text.as_str();
        let right = vec![sp(right_text, MUTE)];
        let right_len = right_text.chars().count();
        // The same test `lay_out` makes: the right header goes first when the title is long.
        let right_fits = 3 + title_len + right_len + 4 <= inner;
        let chip_fits = 3 + title_len < inner;

        // The input row: prompt, mark, the visible part of the line, the sent tag.
        let mark = if self.deeper { "?? " } else { "? " };
        let tag = if self.sent { "sent ✓" } else { "" };
        let spans_room = width - 4;
        let prefix = 2 + mark.chars().count();
        let tag_room = if tag.is_empty() {
            0
        } else {
            tag.chars().count() + 1
        };
        let room = spans_room.saturating_sub(prefix + tag_room + 1).max(1);
        let from = if self.cursor >= room {
            self.cursor + 1 - room
        } else {
            0
        };
        let shown: String = one_wide(&self.text.iter().skip(from).take(room).collect::<String>());
        let mut input = vec![sp("❯ ", ACC), bold(mark, KEY)];
        if self.text.is_empty() && !self.sent {
            input.push(sp(
                if self.focus {
                    "ask anything about this shell"
                } else {
                    "press ? to ask"
                },
                FAINT,
            ));
        } else {
            input.push(sp(&shown, TXT));
        }
        let used: usize = input.iter().map(|s| s.text.chars().count()).sum();
        if !tag.is_empty() {
            input.push(sp(
                &" ".repeat(spans_room.saturating_sub(used + tag_room)),
                TXT,
            ));
            input.push(sp(tag, GREEN));
        }
        let cursor_col = (self.focus && !running).then(|| 3 + prefix + (self.cursor - from));

        // The status row.
        let (footer, stop) = self.status(width - 4, now);

        let spec = BoxSpec {
            width,
            title,
            right: if right_fits { right } else { Vec::new() },
            body: vec![Line {
                spans: input,
                selected: false,
            }],
            footer: Some(footer),
        };
        let mut f = lay_out(&spec, top, 0);
        if let Some(col) = cursor_col
            && let Some(cell) = f.cells.get_mut(1).and_then(|r| r.get_mut(col))
        {
            *cell = Cell::new(cell.ch, PBG, TXT, false);
        }
        Laid {
            floating: f,
            top,
            chip: chip_fits.then_some((3 + 8, 3 + title_len)),
            hide: right_fits.then_some((width - 3 - right_len, width - 3)),
            stop: stop.map(|(a, b)| (a + 3, b + 3)),
        }
    }

    /// The status row (as spans, `room` cells wide) and the columns of its Stop button within
    /// it.
    fn status(&self, room: usize, now: Instant) -> (Vec<Span>, Option<(usize, usize)>) {
        let key = |k: &str| bold(k, KEY);
        // The ask waits for the person (a trust question, a question back, a command to allow),
        // before a run began as well as during one.
        if self.back {
            return (
                vec![
                    bold("✎ ", AMBER),
                    sp("Io asks you something: answer in the terminal above", TXT),
                ],
                None,
            );
        }
        let Some(run) = &self.run else {
            let spans = if !self.at_prompt && self.delivered.is_none() {
                vec![sp(
                    "a command is running; the box waits for the prompt",
                    MUTE,
                )]
            } else if self.focus {
                vec![
                    key("↵"),
                    sp(" send  ", MUTE),
                    key("esc"),
                    sp(" clear  ", MUTE),
                    key("⇧tab"),
                    sp(&format!(" mode: {}  ", self.mode_name()), MUTE),
                    key("↑"),
                    sp(" last question", MUTE),
                ]
            } else {
                vec![
                    key("?"),
                    sp(" ask  ", MUTE),
                    key("??"),
                    sp(" deeper  ", MUTE),
                    key("⇧tab"),
                    sp(&format!(" mode: {}  ", self.mode_name()), MUTE),
                    key(HIDE_LABEL),
                    sp(" hide", MUTE),
                ]
            };
            return (spans, None);
        };
        let elapsed = now.saturating_duration_since(run.since);
        let frame = (elapsed.as_millis() / 90) as usize;
        let label = if run.label.is_empty() {
            "thinking".to_owned()
        } else {
            one_wide(&run.label)
        };
        let count = match run.total {
            Some(t) => format!("{}/{t}", run.steps.min(t)),
            None if run.steps > 0 => format!("{} steps", run.steps),
            None => String::new(),
        };
        let time = format!("{:.1}s", elapsed.as_secs_f64());
        let stop_text = "■ Stop esc";
        let stop_len = stop_text.chars().count();
        let mut spans = vec![
            bold(crate::card::spinner(frame), ACC),
            sp(" ", TXT),
            sp(&label, TXT),
            sp("  ", TXT),
        ];
        let tail = format!(
            "  {count}{}{time}",
            if count.is_empty() { "" } else { "  " }
        );
        let used: usize = spans.iter().map(|s| s.text.chars().count()).sum();
        // The bar takes what is left between the label and the count, at most 24 cells.
        let bar_room = room.saturating_sub(used + tail.chars().count() + stop_len + 2);
        let bar_w = bar_room.min(24);
        if bar_w >= 6 {
            spans.extend(bar(bar_w, run, elapsed));
        }
        spans.push(sp(&tail, MUTE));
        let used: usize = spans.iter().map(|s| s.text.chars().count()).sum();
        let pad = room.saturating_sub(used + stop_len + 1);
        spans.push(sp(&" ".repeat(pad), TXT));
        let stop_at = used + pad;
        spans.push(bold(stop_text, RED));
        (spans, Some((stop_at, stop_at + stop_len)))
    }

    fn mode_name(&self) -> &str {
        if self.mode.is_empty() {
            "config"
        } else {
            &self.mode
        }
    }

    /// The corner chip shown while the box is folded: what it is, a spinner and the step count
    /// while an answer runs, and the key that brings the box back.
    pub(super) fn chip_cells(&self, now: Instant) -> Vec<Cell> {
        let mut spans = vec![sp(" ◆ ", ACC)];
        if let Some(run) = &self.run {
            let elapsed = now.saturating_duration_since(run.since);
            let frame = (elapsed.as_millis() / 90) as usize;
            spans.push(bold(crate::card::spinner(frame), ACC));
            spans.push(sp(
                &if run.steps > 0 {
                    format!(" {} steps ", run.steps)
                } else {
                    " thinking ".to_owned()
                },
                TXT,
            ));
        } else {
            spans.push(bold("iohr ", ACC));
        }
        spans.push(bold(HIDE_LABEL, KEY));
        spans.push(sp(" shows the box ", MUTE));
        spans
            .iter()
            .flat_map(|s| {
                s.text
                    .chars()
                    .map(|ch| Cell::new(ch, s.fg, PBG, s.bold))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Where a click at (`col`, `row`) landed, for a box laid out by [`Self::layout`] at
    /// `top`.
    pub(super) fn hit(laid: &Laid, col: usize, row: usize) -> Hit {
        let r = laid.floating.rect;
        if row < r.top || row >= r.top + r.height || col >= r.left + r.width {
            return Hit::Outside;
        }
        let within =
            |span: Option<(usize, usize)>| span.is_some_and(|(a, b)| (a..b).contains(&col));
        match row - laid.top {
            0 if within(laid.hide) => Hit::Hide,
            0 if within(laid.chip) => Hit::Chip,
            1 => Hit::Input,
            2 if within(laid.stop) => Hit::Stop,
            _ => Hit::Outside,
        }
    }
}

/// A laid out box and the columns a click can reach.
#[derive(Debug, Clone)]
pub(super) struct Laid {
    pub(super) floating: Floating,
    top: usize,
    chip: Option<(usize, usize)>,
    hide: Option<(usize, usize)>,
    stop: Option<(usize, usize)>,
}

impl Laid {
    pub(super) fn rect(&self) -> Rect {
        self.floating.rect
    }
}

/// The progress bar: filled for a known total; with none, a short bar that travels, so it never
/// reads as a percentage.
fn bar(width: usize, run: &Run, elapsed: Duration) -> Vec<Span> {
    if let Some(total) = run.total {
        let done = (run.steps.min(total) * width).div_ceil(total).min(width);
        return vec![
            sp(&"━".repeat(done), ACC),
            sp(&"─".repeat(width - done), FAINT),
        ];
    }
    let head = 4.min(width);
    let travel = width - head;
    let tick = (elapsed.as_millis() / 80) as usize;
    let period = (2 * travel).max(1);
    let p = tick % period;
    let at = if p <= travel { p } else { period - p };
    vec![
        sp(&"─".repeat(at), FAINT),
        sp(&"━".repeat(head), ACC),
        sp(&"─".repeat(travel - at), FAINT),
    ]
}

/// How many bytes at the end of `bytes` are the start of a key that the next read finishes:
/// `ESC [` or `ESC O` and its parameters with no final byte, or a UTF-8 character with its
/// tail missing. A lone `ESC` is not one: it is the Esc key.
fn unfinished(bytes: &[u8]) -> usize {
    const MAX_HELD: usize = 32;
    if let Some(esc) = bytes.iter().rposition(|b| *b == 0x1b) {
        let tail = &bytes[esc..];
        if tail.len() >= 2
            && matches!(tail[1], b'[' | b'O')
            && !tail[2..].iter().any(|b| (0x40..=0x7e).contains(b))
            && tail.len() <= MAX_HELD
        {
            return tail.len();
        }
    }
    let n = bytes.len();
    for back in 1..=3.min(n) {
        let b = bytes[n - back];
        let need = match b {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            0x80..=0xbf => continue,
            _ => return 0,
        };
        return if back < need { back } else { 0 };
    }
    0
}

/// The keys in `bytes`; `paste` is whether a bracketed paste is open (kept between reads).
#[allow(clippy::too_many_lines, reason = "one arm per key sequence")]
pub(super) fn parse(bytes: &[u8], paste: &mut bool) -> Vec<ChatKey> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x1b => {
                let rest = &bytes[i + 1..];
                match rest.first() {
                    None => {
                        out.push(ChatKey::Esc);
                        i += 1;
                    }
                    Some(b'[' | b'O') => {
                        let ss3 = rest[0] == b'O';
                        let mut j = i + 2;
                        while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                            j += 1;
                        }
                        if j >= bytes.len() {
                            break;
                        }
                        let params = &bytes[i + 2..j];
                        let key = match (bytes[j], params) {
                            (b'A', _) => Some(ChatKey::Up),
                            (b'B', _) => Some(ChatKey::Down),
                            (b'C', _) => Some(ChatKey::Right),
                            (b'D', _) => Some(ChatKey::Left),
                            (b'H', _) | (b'~', b"1" | b"7") => Some(ChatKey::Home),
                            (b'F', _) | (b'~', b"4" | b"8") => Some(ChatKey::End),
                            (b'Z', _) if !ss3 => Some(ChatKey::BackTab),
                            (b'~', b"3") => Some(ChatKey::Delete),
                            (b'~', b"200") => {
                                *paste = true;
                                None
                            }
                            (b'~', b"201") => {
                                *paste = false;
                                None
                            }
                            _ => None,
                        };
                        out.extend(key);
                        i = j + 1;
                    }
                    // Alt and a key: neither is the box's.
                    Some(_) => i += 2,
                }
            }
            b'\r' | b'\n' => {
                out.push(if *paste {
                    ChatKey::Char(' ')
                } else {
                    ChatKey::Enter
                });
                i += 1;
            }
            0x03 => {
                out.push(ChatKey::CtrlC);
                i += 1;
            }
            0x7f | 0x08 => {
                out.push(ChatKey::Backspace);
                i += 1;
            }
            0x17 => {
                out.push(ChatKey::Word);
                i += 1;
            }
            0x15 => {
                out.push(ChatKey::Kill);
                i += 1;
            }
            0x01 => {
                out.push(ChatKey::Home);
                i += 1;
            }
            0x05 => {
                out.push(ChatKey::End);
                i += 1;
            }
            0x02 => {
                out.push(ChatKey::Left);
                i += 1;
            }
            0x06 => {
                out.push(ChatKey::Right);
                i += 1;
            }
            0x00..=0x1f => i += 1,
            _ => {
                let len = match b {
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    _ => 1,
                };
                let end = (i + len).min(bytes.len());
                if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
                    out.extend(s.chars().next().map(ChatKey::Char));
                }
                i = end;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
