//! The one look of `iohr` for a person at a terminal: an intro and an outro, spinners that
//! end ok, warn or fail, a menu, a question, a confirmation and a note box (PRD 0088.1).
//! It wraps `cliclack` (which draws on stderr) so every command feels the same.
//!
//! The rule of [`crate::prompt`] holds: everything for the person goes to stderr, never to
//! stdout, so pipes get only data. When the look is not wanted (no terminal on stderr,
//! `--json`, `NO_COLOR`, `TERM=dumb`) every call falls back to a plain line on stderr, or
//! to nothing for the decoration, and a question is never asked: the caller reads
//! [`Ui::fancy`] and takes its own plain path, so scripts and CI keep exact, stable output.

use std::fmt::Display;
use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cliclack::Theme as _;
use console::Key;

use crate::ask_template;
use crate::card::{Look, nearest_256};
use crate::error::Error;
use crate::output::table_text;
use crate::prompt;

/// Whether the drawn look is wanted. Pure, so the rules are testable: a terminal on stderr,
/// not `--json`, `NO_COLOR` unset or empty (the no-color.org rule), and `TERM` neither unset
/// nor `dumb`.
#[must_use]
pub(crate) fn fancy_for(
    interactive: bool,
    json: bool,
    no_color: Option<&str>,
    term: Option<&str>,
) -> bool {
    interactive
        && !json
        && no_color.is_none_or(str::is_empty)
        && term.is_some_and(|t| !t.is_empty() && t != "dumb")
}

/// The help look: the one palette of this file (cyan headings, bold commands and flags, dim
/// placeholders) given to clap, so `--help` and the drawn output agree. `const`, so a derive
/// can take it as `styles = ui::clap_styles()`. Colour itself is clap's call: it follows
/// `NO_COLOR` and the terminal, and [`crate::cli::command`] turns it off for `--json`.
#[must_use]
pub(crate) const fn clap_styles() -> clap::builder::Styles {
    use clap::builder::styling::{AnsiColor, Effects, Style};
    let heading = Style::new()
        .fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Cyan)))
        .effects(Effects::BOLD);
    clap::builder::Styles::styled()
        .header(heading)
        .usage(heading)
        .literal(Style::new().effects(Effects::BOLD))
        .placeholder(Style::new().effects(Effects::DIMMED))
}

/// What kind of terminal stdout is, for the data commands print (tables, `show`): colour
/// from the same rule as the drawn look ([`fancy_for`], on stdout rather than stderr), and
/// whether a person is reading (hints are for them; a pipe gets data only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Paint {
    color: bool,
    tty: bool,
}

impl Paint {
    /// For this run's stdout: a terminal, `--json` off, `NO_COLOR` and `TERM` honoured.
    #[must_use]
    pub(crate) fn stdout(json: bool) -> Self {
        use std::io::IsTerminal as _;
        let tty = io::stdout().is_terminal() && !json;
        Self::from_parts(
            tty,
            json,
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// For this run's stderr, where errors go: the same rules as [`Paint::stdout`], on the
    /// stream the error is written to.
    #[must_use]
    pub(crate) fn stderr(json: bool) -> Self {
        use std::io::IsTerminal as _;
        Self::from_parts(
            io::stderr().is_terminal() && !json,
            json,
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// Pure, for tests: the same rules as [`Paint::stdout`] with the facts given.
    #[must_use]
    pub(crate) fn from_parts(
        tty: bool,
        json: bool,
        no_color: Option<&str>,
        term: Option<&str>,
    ) -> Self {
        Self {
            color: fancy_for(tty, json, no_color, term),
            tty: tty && !json,
        }
    }

    /// No colour, not a terminal: what a pipe gets.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn plain() -> Self {
        Self {
            color: false,
            tty: false,
        }
    }

    /// Whether output is coloured: the drawn look is for a person, plain lines for the rest.
    #[must_use]
    pub(crate) const fn color(self) -> bool {
        self.color
    }

    /// Whether a person is reading stdout (next-step hints are printed only then).
    #[must_use]
    pub(crate) const fn tty(self) -> bool {
        self.tty
    }

    fn apply(
        self,
        text: &str,
        f: impl FnOnce(console::StyledObject<String>) -> console::StyledObject<String>,
    ) -> String {
        if self.color {
            f(console::style(text.to_owned()).force_styling(true)).to_string()
        } else {
            text.to_owned()
        }
    }

    /// A column header or section title: bold cyan, as in help.
    #[must_use]
    pub(crate) fn head(self, text: &str) -> String {
        self.apply(text, |s| s.cyan().bold())
    }

    /// A name to act on: bold.
    #[must_use]
    pub(crate) fn name(self, text: &str) -> String {
        self.apply(text, console::StyledObject::bold)
    }

    /// Context that can be skipped: dim.
    #[must_use]
    pub(crate) fn dim(self, text: &str) -> String {
        self.apply(text, console::StyledObject::dim)
    }

    /// Something wrong, as the `error:` label: bold red.
    #[must_use]
    pub(crate) fn bad(self, text: &str) -> String {
        self.apply(text, |s| s.red().bold())
    }

    /// A mark that matters, such as the default's `*`: bold yellow.
    #[must_use]
    pub(crate) fn mark(self, text: &str) -> String {
        self.apply(text, |s| s.yellow().bold())
    }

    /// A block of a colour with its hex beside it (`show_hex`), for `iohr theme show`. Drawn
    /// with 24-bit colour where the terminal has it and the nearest of the 256 otherwise; with
    /// no colour, the hex alone, so a pipe and a screen reader get the value.
    #[must_use]
    pub(crate) fn swatch(
        self,
        rgb: (u8, u8, u8),
        hex: &str,
        show_hex: bool,
        truecolor: bool,
    ) -> String {
        if !self.color {
            return hex.to_owned();
        }
        let back = if truecolor {
            format!("48;2;{};{};{}", rgb.0, rgb.1, rgb.2)
        } else {
            format!("48;5;{}", nearest_256(rgb))
        };
        let block = format!("\u{1b}[{back}m    \u{1b}[0m");
        if show_hex {
            format!("{block} {hex}")
        } else {
            block
        }
    }

    /// A status: a green dot for live, a dim ring for coming, red for anything wrong. With no
    /// colour it is the word alone, so a pipe and a screen reader get plain text.
    #[must_use]
    pub(crate) fn status(self, word: &str) -> String {
        if !self.color {
            return word.to_owned();
        }
        match word {
            "live" => format!(
                "{} {word}",
                self.apply("\u{25cf}", console::StyledObject::green)
            ),
            "coming" => self.apply(&format!("\u{25cb} {word}"), console::StyledObject::dim),
            _ => format!(
                "{} {word}",
                self.apply("\u{25cf}", console::StyledObject::red)
            ),
        }
    }
}

/// One menu entry: the value returned, its label, a one-line hint, and whether it can be
/// chosen. An entry that cannot be chosen is shown dimmed, marked "coming", and refused.
/// `key` is a letter that picks it at once; a menu where any entry has one is drawn by
/// [`Ui::select_bar`].
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) hint: String,
    pub(crate) available: bool,
    pub(crate) key: Option<char>,
}

/// What a step of a wizard answers: the value, or "go back one step".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Nav<T> {
    Next(T),
    Back,
}

impl<T> Nav<T> {
    /// The value, turned into another; "back" stays "back".
    pub(crate) fn map<U>(self, f: impl FnOnce(T) -> U) -> Nav<U> {
        match self {
            Self::Next(v) => Nav::Next(f(v)),
            Self::Back => Nav::Back,
        }
    }
}

/// The value of the "Back" row of a menu (no real choice starts with `@`).
const BACK: &str = "@back";

/// The look, decided once per command.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ui {
    fancy: bool,
}

thread_local! {
    /// Whether the questions are closed on this thread: `init` closes them once the person has
    /// pressed Create (no general question after it), see [`Ui::close_questions`].
    static CLOSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Closes the questions until it is dropped. See [`Ui::close_questions`].
#[derive(Debug)]
pub(crate) struct Closed(());

impl Drop for Closed {
    fn drop(&mut self) {
        CLOSED.with(|c| c.set(false));
    }
}

/// The marks a finished step wears: a green check, a yellow bang, a red cross. cliclack's own
/// diamonds and squares say nothing a person can read at a glance (the owner's ask: "success if
/// it's success"). The theme sets only the log symbols: prompts keep theirs.
struct Marks;

/// A step that worked.
const CHECK: &str = "\u{2713}";
/// A step that did not fully happen: said plainly, with the fix.
const BANG: &str = "!";
/// A step that failed.
const CROSS: &str = "\u{2717}";

impl cliclack::Theme for Marks {
    fn active_symbol(&self) -> String {
        console::style(CHECK).green().bold().to_string()
    }
    fn warning_symbol(&self) -> String {
        console::style(BANG).yellow().bold().to_string()
    }
    fn error_symbol(&self) -> String {
        console::style(CROSS).red().bold().to_string()
    }
}

/// How a step ended, from the state a report gives it (`done`, `skipped`, `ask`, `not-yet`,
/// `failed`): a step that only did less than asked is a warning, never a success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Ok,
    Warn,
    Fail,
}

impl Mark {
    /// The mark of a report step's `state`. A state this does not know is a warning: nothing
    /// unknown is drawn as a success.
    #[must_use]
    pub(crate) fn of(state: &str) -> Self {
        match state {
            "done" => Self::Ok,
            "failed" => Self::Fail,
            _ => Self::Warn,
        }
    }

    /// The glyph a person reads at a glance: a check, a bang, a cross.
    #[must_use]
    pub(crate) const fn glyph(self) -> &'static str {
        match self {
            Self::Ok => CHECK,
            Self::Warn => BANG,
            Self::Fail => CROSS,
        }
    }

    /// Its colour from the shared palette: green, amber, red.
    fn style(self, look: Look) -> console::Style {
        match self {
            Self::Ok => look.green(),
            Self::Warn => look.amber(),
            Self::Fail => look.red(),
        }
    }
}

/// What a list holds, for the line under it on a terminal ([`Paint::list`]): the noun in
/// the singular and plural, and the command to run next (a label and the command).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Listed<'a> {
    one: &'a str,
    many: &'a str,
    next: Option<(&'a str, &'a str)>,
}

impl<'a> Listed<'a> {
    pub(crate) const fn new(one: &'a str, many: &'a str) -> Self {
        Self {
            one,
            many,
            next: None,
        }
    }

    /// The command to run next, shown after the count, and alone when the list is empty.
    pub(crate) const fn next(self, label: &'a str, command: &'a str) -> Self {
        Self {
            next: Some((label, command)),
            ..self
        }
    }
}

/// One row of a list of checks (`iohr doctor`): how it ended, its name, what was found, and
/// the lines under it as a label and a text (`fix`, `or`, `fixed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckRow {
    pub(crate) mark: Mark,
    pub(crate) id: String,
    pub(crate) what: String,
    pub(crate) notes: Vec<(&'static str, String)>,
}

impl Paint {
    /// `text` in a mark's colour, or as it is with no colour.
    #[must_use]
    pub(crate) fn tone(self, mark: Mark, text: &str) -> String {
        if self.color {
            mark.style(Look::detect(true)).apply_to(text).to_string()
        } else {
            text.to_owned()
        }
    }

    /// A list of checks drawn for a person: each row's mark in its colour, the names in one
    /// column, a passing finding dimmed so the problems stand out, the lines under a row
    /// indented under its text, then a count (`3 ok · 1 to fix`). No box, no rule: the
    /// colours and the column carry it. Commands call this only with colour on; their plain
    /// lines stay what scripts read.
    #[must_use]
    pub(crate) fn checks(self, rows: &[CheckRow]) -> String {
        self.card(rows, None)
    }

    /// A list of findings drawn as the checks card: only the problems are rows, and the count
    /// line says what was checked (`12 documents checked · 3 to fix`). With nothing found it is
    /// that line alone, under a green check.
    #[must_use]
    pub(crate) fn findings(self, rows: &[CheckRow], checked: &str) -> String {
        self.card(rows, Some(checked))
    }

    fn card(self, rows: &[CheckRow], checked: Option<&str>) -> String {
        use std::fmt::Write as _;
        let look = Look::detect(self.color);
        if rows.is_empty()
            && let Some(checked) = checked
        {
            let ok = Mark::Ok.style(look);
            return format!(
                "  {} {}\n",
                ok.clone().bold().apply_to(Mark::Ok.glyph()),
                ok.apply_to(format!("{checked}, nothing found"))
            );
        }
        let width = rows.iter().map(|r| r.id.chars().count()).max().unwrap_or(0);
        let label_width = rows
            .iter()
            .flat_map(|r| r.notes.iter().map(|(l, _)| l.chars().count()))
            .max()
            .unwrap_or(0);
        let mut s = String::new();
        for r in rows {
            let what = if r.mark == Mark::Ok {
                look.dim().apply_to(r.what.as_str()).to_string()
            } else {
                r.what.clone()
            };
            let _ = writeln!(
                s,
                "  {} {:<width$}  {what}",
                r.mark.style(look).bold().apply_to(r.mark.glyph()),
                r.id,
            );
            for (label, text) in &r.notes {
                let _ = writeln!(
                    s,
                    "    {:width$}  {}  {text}",
                    "",
                    look.dim().apply_to(format!("{label:<label_width$}")),
                );
            }
        }
        let count = |m: Mark| rows.iter().filter(|r| r.mark == m).count();
        let mut parts = Vec::new();
        if let Some(checked) = checked {
            parts.push(look.dim().apply_to(checked).to_string());
        }
        for (m, word) in [
            (Mark::Ok, "ok"),
            (Mark::Warn, "to check"),
            (Mark::Fail, "to fix"),
        ] {
            let n = count(m);
            let said = m == Mark::Ok && checked.is_some();
            if !said && (n > 0 || m == Mark::Ok) {
                parts.push(m.style(look).apply_to(format!("{n} {word}")).to_string());
            }
        }
        let sep = look.dim().apply_to(" \u{b7} ").to_string();
        let _ = writeln!(s, "\n  {}", parts.join(&sep));
        s
    }

    /// A failure drawn for a person: a red cross and what happened, then the lines under it,
    /// each with its label (`code`, `request`, `hint`) in one quiet column, or indented alone
    /// when it has none. Called only with colour on; the plain line stays `iohr: <message>`.
    #[must_use]
    pub(crate) fn failure(self, headline: &str, notes: &[(&'static str, String)]) -> String {
        use std::fmt::Write as _;
        let look = Look::detect(self.color);
        let red = Mark::Fail.style(look);
        let mut s = format!(
            "{} {}",
            red.bold().apply_to(Mark::Fail.glyph()),
            console::Style::new()
                .bold()
                .force_styling(self.color)
                .apply_to(headline),
        );
        let width = notes
            .iter()
            .map(|(l, _)| l.chars().count())
            .max()
            .unwrap_or(0);
        for (label, text) in notes {
            if label.is_empty() {
                let _ = write!(s, "\n  {text}");
            } else {
                let _ = write!(
                    s,
                    "\n  {}  {text}",
                    look.dim().apply_to(format!("{label:<width$}")),
                );
            }
        }
        s
    }

    /// Io's line under an outcome: something to type at the `?` prompt, bold, and what it
    /// does, quiet. Only for a colour terminal whose shell has the hook; never in plain or
    /// JSON output, where nothing may be added to the bytes scripts read.
    #[must_use]
    pub(crate) fn io_line(self, typed: &str, what: &str) -> String {
        format!("{}  {}", self.name(typed), self.dim(what))
    }

    /// A list: with colour, the header in the help colour, ids quiet, a `STATUS` or `STATE`
    /// cell as its mark and word, the rows indented as the checks card is. With no colour,
    /// exactly [`table_text`], the bytes scripts read.
    #[must_use]
    pub(crate) fn table(self, header: &[&str], rows: &[Vec<String>]) -> String {
        let width = stdout_width();
        self.table_within(header, rows, width)
    }

    /// [`Paint::table`] for a terminal `width` columns wide: with colour, the widest columns
    /// are cut (ending in `…`) until the indented table fits. With no colour, nothing is cut.
    fn table_within(self, header: &[&str], rows: &[Vec<String>], width: usize) -> String {
        if !self.color {
            return table_text(header, rows);
        }
        let state_col: Vec<bool> = header
            .iter()
            .map(|h| matches!(h.to_ascii_lowercase().as_str(), "status" | "state"))
            .collect();
        let head: Vec<String> = header.iter().map(|h| self.head(h)).collect();
        let head: Vec<&str> = head.iter().map(String::as_str).collect();
        let rows: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, cell)| {
                        if state_col.get(i).copied().unwrap_or(false) {
                            self.state(cell)
                        } else {
                            self.quiet_id(cell)
                        }
                    })
                    .collect()
            })
            .collect();
        let rows = fit_cells(&head, &rows, width.saturating_sub(2));
        indent(&table_text(&head, &rows))
    }

    /// A list with its footer: with colour, [`Paint::table`] and under it the count and the
    /// next command (`2 tokens · new one: iohr token create`); with no rows, one line in
    /// place of the header (`No tokens yet. Next: iohr token create`). With no colour,
    /// exactly [`table_text`], a header-only line when empty, as scripts have always read.
    #[must_use]
    pub(crate) fn list(self, header: &[&str], rows: &[Vec<String>], what: &Listed<'_>) -> String {
        if !self.color {
            return table_text(header, rows);
        }
        if rows.is_empty() {
            let none = format!("No {} yet.", what.many);
            return match what.next {
                Some((_, command)) => format!(
                    "  {} {}\n",
                    self.dim(&format!("{none} Next:")),
                    self.name(command)
                ),
                None => format!("  {}\n", self.dim(&none)),
            };
        }
        let n = rows.len();
        let count = format!("{n} {}", if n == 1 { what.one } else { what.many });
        let footer = match what.next {
            Some((label, command)) => format!(
                "{} {}",
                self.dim(&format!("{count} \u{b7} {label}:")),
                self.name(command)
            ),
            None => self.dim(&count),
        };
        format!("{}  {footer}\n", self.table(header, rows))
    }

    /// `key  value` lines: with colour, keys dim in one column and a `status` or `state`
    /// value as its mark and word, indented as a list is. With no colour, exactly the plain
    /// lines scripts read.
    #[must_use]
    pub(crate) fn pairs<K: AsRef<str>>(self, pairs: &[(K, String)]) -> String {
        use std::fmt::Write as _;
        let w = pairs
            .iter()
            .map(|(k, _)| k.as_ref().chars().count())
            .max()
            .unwrap_or(0);
        let mut s = String::new();
        for (k, v) in pairs {
            let k = k.as_ref();
            if self.color {
                let v = if matches!(k, "status" | "state") {
                    self.state(v)
                } else {
                    self.quiet_id(v)
                };
                let pad = " ".repeat(w - k.chars().count());
                let _ = writeln!(s, "  {}{pad}  {v}", self.dim(k));
            } else {
                let _ = writeln!(s, "{k:w$}  {v}");
            }
        }
        s
    }

    /// A state word as its mark and the word in the mark's colour (`✓ active`); a word with
    /// no known mark, or a cell already coloured, stays as it is.
    fn state(self, word: &str) -> String {
        match state_mark(word) {
            Some(m) if self.color && !word.contains('\u{1b}') => {
                self.tone(m, &format!("{} {word}", m.glyph()))
            }
            _ => word.to_owned(),
        }
    }

    /// An id dimmed, so the names beside it lead.
    fn quiet_id(self, cell: &str) -> String {
        if looks_like_id(cell) {
            self.dim(cell)
        } else {
            cell.to_owned()
        }
    }
}

/// The mark a state word reads as in a list: done and working green, waiting amber, broken
/// red. Words not named here get no mark, so nothing unknown looks like a success.
fn state_mark(word: &str) -> Option<Mark> {
    match word.trim().to_ascii_lowercase().as_str() {
        "active" | "live" | "online" | "ok" | "done" | "ready" | "connected" | "accepted"
        | "approved" | "enabled" | "healthy" | "running" | "merged" | "installed" | "passed"
        | "valid" => Some(Mark::Ok),
        "pending" | "waiting" | "queued" | "expiring" | "paused" | "draft" | "in_progress"
        | "question" | "stale" | "held" | "degraded" | "starting" | "proposed" => Some(Mark::Warn),
        "failed" | "revoked" | "expired" | "offline" | "error" | "denied" | "broken"
        | "invalid" | "rejected" | "down" => Some(Mark::Fail),
        _ => None,
    }
}

/// An InOrbit id (`tok_01M72X…`, a lowercase prefix, `_` and a ULID) or a UUID.
fn looks_like_id(s: &str) -> bool {
    let s = s.trim_end_matches('\u{2026}');
    if let Some((prefix, rest)) = s.split_once('_') {
        return !prefix.is_empty()
            && prefix.len() <= 6
            && prefix.bytes().all(|b| b.is_ascii_lowercase())
            && rest.len() >= 8
            && rest.len() <= 26
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase());
    }
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// Every line two spaces in, as the checks card is; blank lines stay blank.
fn indent(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        if line != "\n" {
            s.push_str("  ");
        }
        s.push_str(line);
    }
    s
}

/// How often a step that reports often (a crawl) is drawn: the display updates at most once a
/// second, the spinner included.
pub(crate) const REDRAW: Duration = Duration::from_secs(1);

/// A running step: a spinner on a terminal. Plain, it is silent: the command's own plain
/// report (the stable lines scripts read) says how each step went.
pub(crate) struct Step {
    bar: Option<Row>,
    /// The bar has a length (it is no longer a spinner): [`Step::progress`] sets it once.
    bounded: bool,
    /// How often the row may be drawn (zero: whenever it changes, and the spinner turns every
    /// 100 ms); one gate for the spinner and the bar that replaces it.
    gate: Arc<Mutex<Gate>>,
}

/// Decides, once per draw, whether the row is drawn now: at most once per `every` (zero: always),
/// the first draw and a forced one (the row's end) always. Time is passed in, so a test can
/// feed it a clock.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct Gate {
    every: Duration,
    last: Option<Instant>,
    /// The next draw is the row's end and always goes through.
    force: bool,
    /// The row's state is being changed: the draws that change causes are held back, and the
    /// next one (the spinner's, or the end's) shows the whole new state, never half of it.
    hold: bool,
    /// A draw is under way: its writes are all let through or all held back.
    open: bool,
    pass: bool,
}

impl Gate {
    fn new(every: Duration) -> Self {
        Self {
            every,
            last: None,
            force: false,
            hold: false,
            open: false,
            pass: false,
        }
    }

    /// Whether the draw that is under way (or starts now) goes to the terminal.
    fn admit(&mut self, now: Instant) -> bool {
        if !self.open {
            if self.hold && !self.force {
                return false;
            }
            self.open = true;
            let due = self
                .last
                .is_none_or(|last| now.saturating_duration_since(last) >= self.every);
            self.pass = std::mem::take(&mut self.force) || self.every.is_zero() || due;
            if self.pass {
                self.last = Some(now);
            }
        }
        self.pass
    }

    /// The draw that was under way is over.
    fn end(&mut self) {
        self.open = false;
    }
}

/// The terminal a [`Row`] draws on: every write of one redraw (indicatif ends each with a
/// flush) goes through or is held back as a whole, by the step's [`Gate`]. A held-back redraw
/// is not lost: the row keeps its latest state and the next redraw that passes shows it.
struct Paced<T> {
    inner: T,
    gate: Arc<Mutex<Gate>>,
    now: fn() -> Instant,
}

impl<T> std::fmt::Debug for Paced<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Paced")
    }
}

impl<T: indicatif::TermLike> Paced<T> {
    fn through(&self, write: impl FnOnce(&T) -> io::Result<()>) -> io::Result<()> {
        let pass = self.gate.lock().map_or(true, |mut g| g.admit((self.now)()));
        if pass { write(&self.inner) } else { Ok(()) }
    }
}

impl<T: indicatif::TermLike> indicatif::TermLike for Paced<T> {
    fn width(&self) -> u16 {
        self.inner.width()
    }
    fn height(&self) -> u16 {
        self.inner.height()
    }
    fn move_cursor_up(&self, n: usize) -> io::Result<()> {
        self.through(|t| t.move_cursor_up(n))
    }
    fn move_cursor_down(&self, n: usize) -> io::Result<()> {
        self.through(|t| t.move_cursor_down(n))
    }
    fn move_cursor_right(&self, n: usize) -> io::Result<()> {
        self.through(|t| t.move_cursor_right(n))
    }
    fn move_cursor_left(&self, n: usize) -> io::Result<()> {
        self.through(|t| t.move_cursor_left(n))
    }
    fn write_line(&self, s: &str) -> io::Result<()> {
        self.through(|t| t.write_line(s))
    }
    fn write_str(&self, s: &str) -> io::Result<()> {
        self.through(|t| t.write_str(s))
    }
    fn clear_line(&self) -> io::Result<()> {
        self.through(indicatif::TermLike::clear_line)
    }
    fn flush(&self) -> io::Result<()> {
        let r = self.through(indicatif::TermLike::flush);
        if let Ok(mut g) = self.gate.lock() {
            g.end();
        }
        r
    }
}

/// The spinner or bar of a step, drawn as `cliclack` draws it (same theme strings) but on a
/// [`Paced`] terminal, because `cliclack`'s own bar turns its spinner every 100 ms and redraws
/// on every change, with no way to slow either.
struct Row {
    bar: indicatif::ProgressBar,
    gate: Arc<Mutex<Gate>>,
    /// Paced rows use the short bar template and cut the message to the terminal's width, so a
    /// held-back redraw never leaves the row at another height than the one on screen.
    paced: bool,
}

impl Row {
    fn new(len: Option<u64>, what: &str, gate: &Arc<Mutex<Gate>>) -> Self {
        use cliclack::Theme as _;
        let paced = gate.lock().is_ok_and(|g| !g.every.is_zero());
        let term = Paced {
            inner: console::Term::stderr(),
            gate: Arc::clone(gate),
            now: Instant::now,
        };
        let bar = indicatif::ProgressBar::with_draw_target(
            len,
            indicatif::ProgressDrawTarget::term_like(Box::new(term)),
        );
        let template = match (len, paced) {
            (None, _) => Marks.default_spinner_template(),
            (Some(_), false) => Marks.default_progress_template(),
            (Some(_), true) => "{msg} {bar:20.magenta}".to_owned(),
        };
        bar.set_style(Self::style(&template));
        let row = Self {
            bar,
            gate: Arc::clone(gate),
            paced,
        };
        row.set_message(what);
        // A paced row wakes often so the latest state shows soon after the gate opens, but the
        // gate lets it draw once per `every`.
        row.bar.enable_steady_tick(if paced {
            Duration::from_millis(250)
        } else {
            Duration::from_millis(100)
        });
        row
    }

    /// Changes the row's state (all of it, or part) without drawing in between, and draws the
    /// result at once when `last` says this is the row's final state.
    fn update(&self, last: bool, change: impl FnOnce(&Self)) {
        let hold = |on: bool, force: bool| {
            if let Ok(mut g) = self.gate.lock() {
                g.hold = on && !g.every.is_zero();
                g.force |= force;
            }
        };
        hold(true, false);
        change(self);
        hold(false, last);
        if last {
            self.bar.force_draw();
        }
    }

    /// The theme's style for a template: the same marks and spinner as every other step.
    fn style(template: &str) -> indicatif::ProgressStyle {
        indicatif::ProgressStyle::with_template(
            &Marks.format_progress_start(template, false, false),
        )
        .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner())
        .tick_chars(&Marks.spinner_chars())
        .progress_chars(&Marks.progress_chars())
    }

    fn set_message(&self, what: &str) {
        if !self.paced {
            self.bar.set_message(Marks.format_progress_message(what));
            return;
        }
        // One line, whatever the message: a held-back redraw must never leave the row at
        // another height than the one on screen. The bar takes what the message leaves free
        // (up to 20 columns) and is left out when that is too little.
        let width = stderr_cols();
        let room = width.saturating_sub(4).max(20);
        let what = console::truncate_str(what, room, "\u{2026}").into_owned();
        if self.bar.length().is_some() {
            let free = room
                .saturating_sub(console::measure_text_width(&what) + 1)
                .min(20);
            self.bar.set_style(Self::style(&if free >= 8 {
                format!("{{msg}} {{bar:{free}.magenta}}")
            } else {
                "{msg}".to_owned()
            }));
        }
        self.bar.set_message(Marks.format_progress_message(&what));
    }

    /// Ends the row and leaves nothing on screen; the clearing redraw always goes through.
    fn clear(&self) {
        if let Ok(mut g) = self.gate.lock() {
            g.force = true;
        }
        self.bar.finish_and_clear();
    }
}

/// What a person who said "no, stop" gets: the same error whether they pressed Esc or Ctrl-C
/// (cliclack tells the two apart nowhere) or picked a "Cancel" row.
pub(crate) fn cancel() -> Error {
    Error::Failed("cancelled".into())
}

fn cancelled(e: &io::Error) -> Error {
    if e.kind() == io::ErrorKind::Interrupted {
        cancel()
    } else {
        Error::Failed(format!("cannot ask on the terminal: {e}"))
    }
}

/// Bytes that arrive within this of the one before are one burst from the terminal, such as
/// the tail of an escape sequence the key reader did not know, never a person typing.
const BURST: Duration = Duration::from_millis(25);

/// Reads that fail with `Interrupted` in a row before the menu gives up (a signal storm).
const MAX_INTERRUPTS: u32 = 1000;

/// What one decisive key does in [`run_keyed`].
enum Act {
    Down,
    Up,
    Pick,
    Cancel,
    Letter(usize),
    Fail(io::Error),
}

/// The loop of [`Ui::select_bar`], with the drawing and the key source passed in so it is
/// tested without a terminal. `numbered` is the action bar's: digits pick, `j`/`k` move. `draw` paints the menu with entry `at` highlighted, `clear`
/// removes it, and `read` gives the next key and whether it came in the same burst as the
/// key before.
///
/// The menu holds until a real choice: an Enter, a letter of an entry, Esc, `q` or Ctrl-C.
/// An interrupted read is read again, and so is a key it does not know. Keys the reader
/// cannot parse are skipped, with the rest of their bytes: a shell in application cursor
/// mode (zsh's line editor turns it on) sends `ESC O B` for Down, which `console` returns as
/// an unknown `ESC O` and then a plain `B`, and the `B` must not be taken for a hotkey (`ESC
/// O F`, the End key, would pick Follow up). Only a read that fails for real ends the menu.
fn run_keyed(
    choices: &[Choice],
    initial: &str,
    numbered: bool,
    mut draw: impl FnMut(usize) -> io::Result<()>,
    mut clear: impl FnMut() -> io::Result<()>,
    mut read: impl FnMut() -> (io::Result<Key>, bool),
) -> io::Result<Option<String>> {
    let mut at = choices.iter().position(|c| c.value == initial).unwrap_or(0);
    // After an unknown escape sequence: its remaining bytes are skipped; `ss3` when it was
    // `ESC O`, whose next byte names an arrow.
    let (mut tail, mut ss3) = (false, false);
    let (mut interrupts, mut blanks) = (0u32, 0u32);
    loop {
        draw(at)?;
        let act = loop {
            let (result, burst) = read();
            let key = match result {
                Ok(key) => key,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    interrupts += 1;
                    if interrupts >= MAX_INTERRUPTS {
                        break Act::Fail(e);
                    }
                    continue;
                }
                Err(e) => break Act::Fail(e),
            };
            interrupts = 0;
            if std::mem::take(&mut ss3) && burst {
                match key {
                    Key::Char('A') => break Act::Up,
                    Key::Char('B') => break Act::Down,
                    _ => continue,
                }
            }
            if tail && burst {
                continue;
            }
            tail = false;
            match key {
                Key::UnknownEscSeq(seq) => {
                    (tail, ss3) = (true, seq == ['O']);
                }
                Key::Unknown => {
                    // Not a terminal at all reads as this at once, for ever.
                    blanks += 1;
                    if blanks >= 100 {
                        break Act::Fail(io::Error::new(
                            io::ErrorKind::NotConnected,
                            "no keys can be read",
                        ));
                    }
                }
                Key::ArrowDown | Key::Tab => break Act::Down,
                Key::ArrowUp | Key::BackTab => break Act::Up,
                Key::Enter => break Act::Pick,
                Key::Escape | Key::CtrlC => break Act::Cancel,
                Key::Char(k) => {
                    let k = k.to_ascii_lowercase();
                    if k == 'q' {
                        break Act::Cancel;
                    }
                    if numbered {
                        // The action bar: `1`-`9` pick that row at once; `j` and `k` move.
                        if let Some(i) = k.to_digit(10).filter(|d| *d >= 1).map(|d| d as usize - 1)
                            && i < choices.len()
                        {
                            break Act::Letter(i);
                        }
                        if k == 'j' && !choices.iter().any(|c| c.key == Some('j')) {
                            break Act::Down;
                        }
                        if k == 'k' && !choices.iter().any(|c| c.key == Some('k')) {
                            break Act::Up;
                        }
                    }
                    if let Some(i) = choices.iter().position(|c| c.key == Some(k)) {
                        break Act::Letter(i);
                    }
                }
                _ => {}
            }
        };
        clear()?;
        match act {
            Act::Down => at = (at + 1) % choices.len(),
            Act::Up => at = (at + choices.len() - 1) % choices.len(),
            Act::Pick => return Ok(Some(choices[at].value.clone())),
            Act::Letter(i) => return Ok(Some(choices[i].value.clone())),
            Act::Cancel => return Ok(None),
            Act::Fail(e) => return Err(e),
        }
    }
}

fn plain(line: &str) {
    let _ = writeln!(io::stderr().lock(), "{line}");
}

/// The narrowest a column is cut to when a table is fitted to the terminal.
const FIT_MIN: usize = 12;

/// `rows` with their widest columns cut, each cell ending in `…`, until the table (columns
/// two apart) is no wider than `room`. A column is never cut below [`FIT_MIN`] or its header,
/// and an `ID` column is never cut, so a table with many columns may still be wider.
fn fit_cells(header: &[&str], rows: &[Vec<String>], room: usize) -> Vec<Vec<String>> {
    let shown = console::measure_text_width;
    let mut width: Vec<usize> = header.iter().map(|h| shown(h)).collect();
    for row in rows {
        for (w, cell) in width.iter_mut().zip(row) {
            *w = (*w).max(shown(cell));
        }
    }
    // An id is copied whole, so its column is never cut.
    let floor: Vec<usize> = header
        .iter()
        .zip(&width)
        .map(|(h, w)| {
            let h = console::strip_ansi_codes(h);
            if h == "ID" || h.ends_with(" ID") {
                *w
            } else {
                shown(&h).max(FIT_MIN)
            }
        })
        .collect();
    let total = |w: &[usize]| w.iter().sum::<usize>() + 2 * w.len().saturating_sub(1);
    while total(&width) > room {
        let Some((i, &w)) = width
            .iter()
            .enumerate()
            .filter(|(i, w)| **w > floor[*i])
            .max_by_key(|(_, w)| **w)
        else {
            break;
        };
        let next = width.iter().filter(|v| **v < w).max().copied().unwrap_or(0);
        let over = total(&width) - room;
        width[i] = floor[i].max(next).max(w.saturating_sub(over));
    }
    // A path keeps its end (the file it names) and loses its start.
    let path: Vec<bool> = header
        .iter()
        .map(|h| {
            let h = console::strip_ansi_codes(h);
            h == "PATH" || h.ends_with(" PATH")
        })
        .collect();
    rows.iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .zip(width.iter().chain(std::iter::repeat(&usize::MAX)))
                .map(|((i, cell), w)| {
                    if path.get(i).copied().unwrap_or(false) {
                        cut_front(cell, *w)
                    } else {
                        console::truncate_str(cell, *w, "\u{2026}").into_owned()
                    }
                })
                .collect()
        })
        .collect()
}

/// `text` no wider than `width` columns, cut at its start behind a `…` when it is wider.
fn cut_front(text: &str, width: usize) -> String {
    if console::measure_text_width(text) <= width {
        return text.to_owned();
    }
    let plain = console::strip_ansi_codes(text);
    let mut room = width.saturating_sub(1);
    let mut tail: Vec<char> = Vec::new();
    for c in plain.chars().rev() {
        let w = console::measure_text_width(c.encode_utf8(&mut [0; 4]));
        if w > room {
            break;
        }
        room -= w;
        tail.push(c);
    }
    tail.reverse();
    format!("\u{2026}{}", tail.into_iter().collect::<String>())
}

/// The columns of the terminal on stderr as `console` reports them (its default when it
/// cannot tell), with no floor.
fn stderr_cols() -> usize {
    usize::from(console::Term::stderr().size().1)
}

/// The width of the terminal on stderr, at least 40 columns.
pub(crate) fn terminal_width() -> usize {
    stderr_cols().max(40)
}

/// The width of the terminal on stdout, at least 40 columns: what results are drawn at.
pub(crate) fn stdout_width() -> usize {
    usize::from(console::Term::stdout().size().1).max(40)
}

/// `text` broken at spaces into lines no wider than `width` columns, so a long line sits inside
/// the rail (each line gets its `│` from the log) instead of wrapping past it. A word wider than
/// `width` is left whole. Lines already in `text` stay lines.
fn wrapped(text: &str, width: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut row = String::new();
        for word in line.split(' ') {
            if !row.is_empty()
                && console::measure_text_width(&row) + 1 + console::measure_text_width(word) > width
            {
                out.push(std::mem::take(&mut row));
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(word);
        }
        out.push(row);
    }
    out.join("\n")
}

/// A numbered list for a box: `1. text`, a long item wrapped under its own text, every line at
/// most `width` columns.
fn numbered(items: &[String], width: usize) -> String {
    let lead = items.len().to_string().len() + 2;
    let mut out: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let n = format!("{}.", i + 1);
        let text = wrapped(item, width.saturating_sub(lead).max(10));
        for (j, line) in text.lines().enumerate() {
            if j == 0 {
                out.push(format!("{n:<lead$}{line}"));
            } else {
                out.push(format!("{}{line}", " ".repeat(lead)));
            }
        }
    }
    out.join("\n")
}

/// The room a log line has inside the rail: the terminal's width less the rail and its gap.
fn log_width() -> usize {
    terminal_width().saturating_sub(4).max(30)
}

/// The columns below which the action bar is one plain line per row, with no box.
const BAR_BOX_MIN: usize = 40;

/// The action bar as lines (the same count for every `at`): in a box that matches the card's
/// frame (`╭─` header, `│` rail, `╰─` footer, its width and colours) when the look is drawn
/// and the terminal is `cols` wide enough; otherwise one line per row, numbered all the same.
/// Each row is `› [1] Name  k  hint`: the chip, the name, its letter and the dim hint; the
/// highlighted row has the `›` and is reversed. `cols` of 0 is unknown, taken as 80.
#[cfg(test)]
fn bar_lines(prompt: &str, choices: &[Choice], at: usize, cols: usize, look: Look) -> Vec<String> {
    bar_lines_in(&BarTop::default(), prompt, choices, at, cols, look)
}

/// What a bar carries above its rows when it is the foot of a card of its own (the permission
/// ask, PRD 0197): the card's header line in place of the prompt's, and lines on the rail
/// between them. Both are drawn by the caller at [`bar_width`]; the bar only places them, and
/// only in the box (a plain bar has no rail to put them on, so the caller draws its plain line).
#[derive(Debug, Default)]
pub(crate) struct BarTop<'a> {
    pub(crate) header: Option<String>,
    pub(crate) lines: &'a [String],
    /// When the rows are a window of a longer list: how many rows come before the first one
    /// shown (the chips keep their place in the whole list) and how many rows there are in all.
    pub(crate) window: Option<(usize, usize)>,
}

/// Whether the bar is drawn as a box at `cols` columns (0 is unknown, taken as 80).
#[must_use]
pub(crate) fn bar_boxed(cols: usize, look: Look) -> bool {
    look.fancy && (if cols == 0 { 80 } else { cols }) >= BAR_BOX_MIN
}

/// The width a bar line may take on a terminal `cols` wide: the card's rule (the terminal
/// less one column, so no line fills the last), but never the card's floor of 40 on a terminal
/// narrower than that.
#[must_use]
pub(crate) fn bar_width(cols: usize) -> usize {
    (if cols == 0 { 80 } else { cols })
        .saturating_sub(1)
        .max(20)
}

/// [`bar_lines`] with a [`BarTop`]: the same rows, the same foot.
pub(crate) fn bar_lines_in(
    top: &BarTop<'_>,
    prompt: &str,
    choices: &[Choice],
    at: usize,
    cols: usize,
    look: Look,
) -> Vec<String> {
    use console::{measure_text_width, truncate_str};
    const FOOT: &str = "\u{2191}\u{2193} move \u{b7} 1-9 or letter pick \u{b7} enter \u{b7} esc";
    let boxed = bar_boxed(cols, look);
    let width = bar_width(cols);
    let lead = if boxed { "\u{2502}  " } else { "" };
    let pad = choices
        .iter()
        .map(|c| measure_text_width(&c.label))
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    if boxed && let Some(header) = &top.header {
        out.push(header.clone());
    } else if boxed {
        let fixed = measure_text_width("\u{256d}\u{2500} ") + measure_text_width(prompt) + 1;
        let fill = width.saturating_sub(fixed).max(2);
        out.push(format!(
            "{}{} {}",
            look.violet().apply_to("\u{256d}\u{2500} "),
            look.violet().bold().apply_to(prompt),
            look.violet().apply_to("\u{2500}".repeat(fill)),
        ));
    } else {
        out.push(look.plain().bold().apply_to(prompt).to_string());
    }
    for l in top.lines {
        if !boxed {
            // No rail to put it on: the line as it is.
            out.push(l.clone());
        } else if l.is_empty() {
            out.push(look.violet().apply_to("\u{2502}").to_string());
        } else {
            out.push(format!("{}{l}", look.violet().apply_to(lead)));
        }
    }
    let first = top.window.map_or(0, |(first, _)| first);
    let chip_w = format!("[{}]", first + choices.len()).len();
    for (i, c) in choices.iter().enumerate() {
        let on = i == at;
        let chip = format!("{:<chip_w$}", format!("[{}]", first + i + 1));
        let name = format!(
            "{}{}",
            c.label,
            " ".repeat(pad - measure_text_width(&c.label))
        );
        let key = c.key.map_or_else(|| " ".to_owned(), String::from);
        // "› " + chip + " " + name + "  " + key + "  ": what is left is the hint's.
        let used = measure_text_width(lead) + 2 + measure_text_width(&chip) + 1 + pad + 2 + 1 + 2;
        let hint = truncate_str(&c.hint, width.saturating_sub(used), "\u{2026}");
        let head = format!("{chip} {name}");
        let head = if on {
            look.cyan().bold().reverse().apply_to(head).to_string()
        } else {
            format!(
                "{} {}",
                look.cyan().apply_to(chip),
                look.plain().bold().apply_to(name)
            )
        };
        let mark = if on {
            look.violet().bold().apply_to("\u{203a}").to_string()
        } else {
            " ".to_owned()
        };
        out.push(format!(
            "{}{mark} {head}  {}  {}",
            look.violet().apply_to(lead),
            look.cyan().apply_to(key),
            look.dim().apply_to(hint),
        ));
    }
    let foot = match top.window {
        Some((first, total)) => format!(
            "{}\u{2013}{} of {total} \u{b7} {FOOT}",
            first + 1,
            first + choices.len()
        ),
        None => FOOT.to_owned(),
    };
    let foot = truncate_str(&foot, width.saturating_sub(3), "\u{2026}");
    if boxed {
        out.push(format!(
            "{}{}",
            look.violet().apply_to("\u{2570}\u{2500} "),
            look.dim().apply_to(foot)
        ));
    } else {
        out.push(look.dim().apply_to(foot).to_string());
    }
    out
}

impl Ui {
    /// Closes the questions: while the guard lives, a question put to a terminal is a bug (a
    /// panic in a debug build, which the tests run). `init` holds it from Create to the end: the flow only acts and prints
    /// progress after that, every answer having been taken before the review.
    pub(crate) fn close_questions() -> Closed {
        CLOSED.with(|c| c.set(true));
        Closed(())
    }

    /// Every prompt calls this before it draws anything.
    fn asking(self) {
        if self.fancy && CLOSED.with(std::cell::Cell::get) {
            debug_assert!(false, "a question was asked after Create");
        }
    }

    /// The look for this run: drawn only when [`fancy_for`] says so.
    #[must_use]
    pub(crate) fn detect(json: bool) -> Self {
        let fancy = fancy_for(
            prompt::interactive(),
            json,
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        );
        if fancy {
            static MARKS: std::sync::Once = std::sync::Once::new();
            MARKS.call_once(|| cliclack::set_theme(Marks));
        }
        Self { fancy }
    }

    /// Whether the drawn look and its questions are on.
    #[must_use]
    pub(crate) fn fancy(self) -> bool {
        self.fancy
    }

    /// The first line of a command. Plain: nothing.
    pub(crate) fn intro(self, title: impl Display) {
        if self.fancy {
            let _ = cliclack::intro(console::style(format!(" {title} ")).on_cyan().black());
        }
    }

    /// The last line of a command. Plain: the message as a plain line.
    pub(crate) fn outro(self, message: impl Display) {
        if self.fancy {
            let _ = cliclack::outro(message);
        } else {
            plain(&message.to_string());
        }
    }

    /// A boxed note with a title (the next commands, say). Plain: the title, then the lines.
    pub(crate) fn note(self, title: &str, body: &str) {
        if self.fancy {
            let _ = cliclack::note(title, body);
        } else {
            plain(title);
            for l in body.lines() {
                plain(&format!("  {l}"));
            }
        }
    }

    /// A line of information between steps.
    pub(crate) fn info(self, line: &str) {
        if self.fancy {
            let _ = cliclack::log::info(wrapped(line, log_width()));
        } else {
            plain(line);
        }
    }

    /// What a run will do, as one numbered box under `title`. Drawn only: plain, each action
    /// said so already as it was planned (`would: ...`), the lines scripts read.
    pub(crate) fn plan(self, title: &str, items: &[String]) {
        if self.fancy && !items.is_empty() {
            let _ = cliclack::note(title, numbered(items, log_width().saturating_sub(4)));
        }
    }

    /// A line that says something worked, with a check mark. Drawn only: plain, the command's
    /// own report says it.
    pub(crate) fn done(self, line: &str) {
        if self.fancy {
            let _ = cliclack::log::success(wrapped(line, log_width()));
        }
    }

    /// A line that says something failed, with a cross. Drawn only, like [`Ui::done`].
    pub(crate) fn failed(self, line: &str) {
        if self.fancy {
            let _ = cliclack::log::error(wrapped(line, log_width()));
        }
    }

    /// A warning line between steps. Plain: the line with `warning:` in front.
    pub(crate) fn warn(self, line: &str) {
        if self.fancy {
            let _ = cliclack::log::warning(wrapped(line, log_width()));
        } else {
            plain(&format!("warning: {line}"));
        }
    }

    /// Starts a step. Plain: nothing until it ends.
    pub(crate) fn step(self, what: &str) -> Step {
        self.step_every(what, Duration::ZERO)
    }

    /// [`Ui::step`] for work that reports often (a crawl): its row is drawn at most once per
    /// `every`, the spinner included, showing the latest state each time, and its end always
    /// draws.
    pub(crate) fn step_every(self, what: &str, every: Duration) -> Step {
        let gate = Arc::new(Mutex::new(Gate::new(every)));
        if !self.fancy {
            return Step {
                bar: None,
                bounded: false,
                gate,
            };
        }
        Step {
            bar: Some(Row::new(None, what, &gate)),
            bounded: false,
            gate,
        }
    }

    /// A menu. Entries that are not available are dimmed and re-asked when picked; `initial`
    /// is the entry the cursor starts on.
    ///
    /// # Errors
    ///
    /// No terminal, or the person cancelled.
    pub(crate) fn select(
        self,
        prompt: &str,
        choices: &[Choice],
        initial: &str,
    ) -> Result<String, Error> {
        match self.select_step(prompt, choices, initial, false)? {
            Nav::Next(v) => Ok(v),
            Nav::Back => Err(cancel()),
        }
    }

    /// A menu of a wizard: [`Ui::select`] with a last row "Back" when `back` is set (every step
    /// but the first).
    ///
    /// # Errors
    ///
    /// No terminal, or the person cancelled.
    pub(crate) fn select_step(
        self,
        prompt: &str,
        choices: &[Choice],
        initial: &str,
        back: bool,
    ) -> Result<Nav<String>, Error> {
        self.asking();
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass the choice as a flag"
            )));
        }
        let mut all: Vec<Choice> = choices
            .iter()
            .map(|c| {
                let mut c = c.clone();
                if !c.available {
                    c.label = format!("{} (coming)", c.label);
                }
                c
            })
            .collect();
        if back {
            all.push(back_row());
        }
        loop {
            let picked = ask_template::pick(&ask_template::Template::new(prompt, &all), initial)?
                .ok_or_else(cancel)?;
            match all.iter().find(|c| c.value == picked) {
                Some(c) if c.available => {
                    if picked != BACK {
                        settled(prompt, &c.label);
                    }
                    return Ok(if back && picked == BACK {
                        Nav::Back
                    } else {
                        Nav::Next(picked)
                    });
                }
                _ => self.warn(&format!("{picked} is coming; pick a live one")),
            }
        }
    }

    /// The action bar under a card (PRD 0187.1) and every list of rows with keys: the rows of
    /// `choices` in the ask template ([`ask_template`]), `1`-`9` pick a row at once, a row's
    /// letter picks it, `j` and `k` move, Esc, `q` and Ctrl-C leave (`None`) and the card is
    /// erased.
    ///
    /// # Errors
    ///
    /// No terminal on stderr, or the terminal cannot be read.
    pub(crate) fn select_bar(
        prompt: &str,
        choices: &[Choice],
        initial: &str,
    ) -> Result<Option<String>, Error> {
        ask_template::pick(&ask_template::Template::new(prompt, choices), initial)
    }

    /// The loop of every menu: `rows(at)` is the screen with entry `at` highlighted, always
    /// the same number of lines, so erasing it leaves nothing behind. Drawn with the bar's keys
    /// (arrows, `1`-`9` when `numbered`, a row's letter, Enter, Esc, `q`, Ctrl-C); returns the
    /// value of the row picked, or `None` for a cancel. The bar under a card of the caller's own
    /// (the permission ask) calls it with its rows.
    ///
    /// # Errors
    ///
    /// No terminal on stderr, or the terminal cannot be read.
    pub(crate) fn select_drawn(
        prompt: &str,
        choices: &[Choice],
        initial: &str,
        numbered: bool,
        rows: impl Fn(usize) -> Vec<String>,
    ) -> Result<Option<String>, Error> {
        use console::Term;
        if !prompt::interactive() || choices.is_empty() {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on"
            )));
        }
        let term = Term::stderr();
        let mut last = Instant::now();
        let height = rows(0).len();
        run_keyed(
            choices,
            initial,
            numbered,
            |at| {
                for line in rows(at) {
                    term.write_line(&line)?;
                }
                Ok(())
            },
            || term.clear_last_lines(height),
            || {
                // `read_key_raw`, not `read_key`: `read_key` raises SIGINT on this very
                // process for Ctrl-C (and for any interrupted read) and the menu dies with
                // its lines on the screen; raw hands Ctrl-C back as a key.
                let result = term.read_key_raw();
                let now = Instant::now();
                let burst = now.duration_since(last) < BURST;
                last = now;
                (result, burst)
            },
        )
        .map_err(|e| cancelled(&e))
    }

    /// A question with a default and a check.
    ///
    /// # Errors
    ///
    /// No terminal, or the person cancelled.
    pub(crate) fn input(
        self,
        prompt: &str,
        default: &str,
        check: impl Fn(&str) -> Result<(), String>,
    ) -> Result<String, Error> {
        match self.input_step(prompt, default, check, false)? {
            Nav::Next(v) => Ok(v),
            Nav::Back => Err(cancel()),
        }
    }

    /// A question of a wizard: [`Ui::input`], and when `back` is set typing `<` alone goes back
    /// one step (the prompt says so; it is read before the check).
    ///
    /// # Errors
    ///
    /// No terminal, or the person cancelled.
    pub(crate) fn input_step(
        self,
        prompt: &str,
        default: &str,
        check: impl Fn(&str) -> Result<(), String>,
        back: bool,
    ) -> Result<Nav<String>, Error> {
        self.asking();
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass it as an argument"
            )));
        }
        let t = ask_template::Template::new(prompt, &[]);
        match ask_template::type_in(&t, default, back, &check)? {
            ask_template::Typed::Text(v) => {
                settled(prompt, &v);
                Ok(Nav::Next(v))
            }
            ask_template::Typed::Back => Ok(Nav::Back),
            ask_template::Typed::Cancel => Err(cancel()),
        }
    }

    /// Any of the choices, as a list; `initial` are ticked first.
    ///
    /// # Errors
    ///
    /// No terminal, or the person cancelled.
    pub(crate) fn multiselect(
        self,
        prompt: &str,
        choices: &[String],
        initial: &[String],
    ) -> Result<Vec<String>, Error> {
        let items: Vec<(String, String)> = choices.iter().map(|c| (c.clone(), c.clone())).collect();
        self.multiselect_items(prompt, &items, initial)
    }

    /// The list of [`Ui::multiselect`] with a label apart from each value, in the ask template:
    /// each row toggles its tick, the last row ("Done") ends the list.
    fn multiselect_items(
        self,
        prompt: &str,
        items: &[(String, String)],
        initial: &[String],
    ) -> Result<Vec<String>, Error> {
        self.asking();
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass it with --set"
            )));
        }
        let mut ticked: Vec<String> = initial.to_vec();
        let mut at = ticked_rows(items, &ticked)
            .first()
            .map_or_else(|| DONE.to_owned(), |c| c.value.clone());
        loop {
            let rows = ticked_rows(items, &ticked);
            let t = ask_template::Template::new(prompt, &rows);
            let picked = ask_template::pick(&t, &at)?.ok_or_else(cancel)?;
            if picked == BACK {
                return Ok(vec![BACK.to_owned()]);
            }
            if picked == DONE {
                return Ok(items
                    .iter()
                    .map(|(v, _)| v.clone())
                    .filter(|v| ticked.contains(v))
                    .collect());
            }
            if let Some(i) = ticked.iter().position(|v| *v == picked) {
                ticked.remove(i);
            } else {
                ticked.push(picked.clone());
            }
            at = picked;
        }
    }

    /// A yes or no. Plain: the default, asked of no one.
    ///
    /// # Errors
    ///
    /// The person cancelled.
    pub(crate) fn confirm(self, prompt: &str, default: bool) -> Result<bool, Error> {
        self.asking();
        if !self.fancy {
            return Ok(default);
        }
        let rows = yes_no_rows();
        let picked = ask_template::pick(
            &ask_template::Template::new(prompt, &rows),
            if default { "yes" } else { "no" },
        )?
        .ok_or_else(cancel)?;
        settled(prompt, if picked == "yes" { "Yes" } else { "No" });
        Ok(picked == "yes")
    }
}

/// What a wizard step leaves on the screen once answered, in the wizard's rail: `◇  question`
/// and `│  answer`, so the scrollback reads as a record of what was chosen (the card itself is
/// erased).
fn settled(prompt: &str, answer: &str) {
    let look = Look::detect(true);
    let term = console::Term::stderr();
    let _ = term.write_line(&format!("{}  {prompt}", look.green().apply_to("\u{25c7}")));
    let _ = term.write_line(&format!(
        "{}  {}",
        look.dim().apply_to("\u{2502}"),
        look.dim().apply_to(answer)
    ));
    let _ = term.write_line(&look.dim().apply_to("\u{2502}").to_string());
}

impl Ui {
    /// [`Ui::multiselect`] for a wizard: with `back` set the list ends with a "Back" entry, and
    /// ticking it goes back one step (a list has no row to pick, so it is an entry to tick).
    ///
    /// # Errors
    ///
    /// No terminal, or the person cancelled.
    pub(crate) fn multiselect_step(
        self,
        prompt: &str,
        choices: &[String],
        initial: &[String],
        back: bool,
    ) -> Result<Nav<Vec<String>>, Error> {
        if !back {
            return self.multiselect(prompt, choices, initial).map(Nav::Next);
        }
        let mut items: Vec<(String, String)> =
            choices.iter().map(|c| (c.clone(), c.clone())).collect();
        items.push((BACK.to_owned(), "Back (to the step before)".to_owned()));
        let mut picked = self.multiselect_items(prompt, &items, initial)?;
        if picked.iter().any(|p| p == BACK) {
            return Ok(Nav::Back);
        }
        picked.retain(|p| p != BACK);
        Ok(Nav::Next(picked))
    }

    /// [`Ui::confirm`] for a wizard: with `back` set it is a menu of Yes, No and Back.
    ///
    /// # Errors
    ///
    /// The person cancelled.
    pub(crate) fn confirm_step(
        self,
        prompt: &str,
        default: bool,
        back: bool,
    ) -> Result<Nav<bool>, Error> {
        if !back || !self.fancy {
            return self.confirm(prompt, default).map(Nav::Next);
        }
        Ok(
            match self.select_step(
                prompt,
                &yes_no_rows(),
                if default { "yes" } else { "no" },
                true,
            )? {
                Nav::Next(v) => Nav::Next(v == "yes"),
                Nav::Back => Nav::Back,
            },
        )
    }
}

/// The rows of a yes or no: `y` and `n` pick them.
fn yes_no_rows() -> [Choice; 2] {
    let row = |value: &str, label: &str, key: char| Choice {
        value: value.into(),
        label: label.into(),
        hint: String::new(),
        available: true,
        key: Some(key),
    };
    [row("yes", "Yes", 'y'), row("no", "No", 'n')]
}

/// The value of the row that ends a tick-list.
const DONE: &str = "@done";

/// The rows of a tick-list: `[x]` or `[ ]` before each label, then "Done".
fn ticked_rows(items: &[(String, String)], ticked: &[String]) -> Vec<Choice> {
    let mut rows: Vec<Choice> = items
        .iter()
        .map(|(value, label)| Choice {
            value: value.clone(),
            label: format!(
                "[{}] {label}",
                if ticked.contains(value) { "x" } else { " " }
            ),
            hint: String::new(),
            available: true,
            key: None,
        })
        .collect();
    rows.push(Choice {
        value: DONE.into(),
        label: "Done".into(),
        hint: "keep what is ticked".into(),
        available: true,
        key: None,
    });
    rows
}

fn back_row() -> Choice {
    Choice {
        value: BACK.into(),
        label: "Back".into(),
        hint: "to the step before".into(),
        available: true,
        key: None,
    }
}

impl Step {
    /// The step is over and leaves nothing on screen.
    pub(crate) fn clear(self) {
        if let Some(b) = self.bar {
            b.clear();
        }
    }

    /// Says what the running step is doing now, on its spinner.
    pub(crate) fn say(&self, what: &str) {
        if let Some(b) = &self.bar {
            b.update(false, |r| r.set_message(what));
        }
    }

    /// Says how far the running step is, as `done` of `total` with `what` beside it. Until a
    /// `total` is known the step stays a spinner that shows `what`; the first `total` swaps it
    /// for a bar that fills (a later one, or a longer one, moves its end). Plain it is silent,
    /// like the rest of a step: the command's own report says how it went.
    pub(crate) fn progress(&mut self, done: u64, total: Option<u64>, what: &str) {
        let Some(old) = &self.bar else {
            return;
        };
        let Some(total) = total else {
            if !self.bounded {
                old.update(false, |r| r.set_message(what));
            }
            return;
        };
        if !self.bounded {
            old.clear();
            self.bar = Some(Row::new(Some(total), what, &self.gate));
            self.bounded = true;
        }
        if let Some(row) = &self.bar {
            row.update(done >= total, |r| {
                r.bar.set_length(total);
                r.bar.set_position(done.min(total));
                r.set_message(what);
            });
        }
    }

    /// The step worked, with a check mark. The spinner is cleared and the result is a log line
    /// of its own: the spinner's own finish pads its line to the terminal width and relies on
    /// the terminal to wrap, which drew a stray rail mark and a second, wrapped line.
    pub(crate) fn ok(self, said: &str) {
        if let Some(b) = self.bar {
            b.clear();
            let _ = cliclack::log::success(wrapped(said, log_width()));
        }
    }

    /// The step ended as `mark` says, with `said`.
    pub(crate) fn end(self, mark: Mark, said: &str) {
        match mark {
            Mark::Ok => self.ok(said),
            Mark::Warn => self.warn(said),
            Mark::Fail => self.fail(said),
        }
    }

    /// The step did not fully happen, and the reason is short.
    pub(crate) fn warn(self, said: &str) {
        if let Some(b) = self.bar {
            b.clear();
            let _ = cliclack::log::warning(wrapped(said, log_width()));
        }
    }

    /// The step failed, with a short reason.
    pub(crate) fn fail(self, said: &str) {
        if let Some(b) = self.bar {
            b.clear();
            let _ = cliclack::log::error(wrapped(said, log_width()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::time::{Duration, Instant};

    use super::{Ui, wrapped};
    use console::Key;

    #[test]
    #[should_panic(expected = "a question was asked after Create")]
    fn a_question_after_create_is_a_bug() {
        let _closed = Ui::close_questions();
        let _ = Ui { fancy: true }.confirm("Install it?", false);
    }

    #[test]
    fn questions_open_again_when_the_latch_drops() {
        drop(Ui::close_questions());
        assert!(!super::CLOSED.get());
    }

    use super::{Choice, Look, Marks, Paint, bar_lines, fancy_for, run_keyed};
    use super::{ask_template, back_row, ticked_rows, yes_no_rows};
    use cliclack::Theme;

    /// One layout test per kind of wizard step: each is the ask template, so each shows the
    /// title, numbered rows and (a step after the first) the Back row.
    fn template_text(title: &str, rows: &[Choice]) -> String {
        let t = ask_template::Template::new(title, rows);
        let look = Look {
            fancy: true,
            truecolor: false,
        };
        ask_template::lines(&t, 0, 80, 40, look)
            .iter()
            .map(|l| console::strip_ansi_codes(l).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_wizard_select_with_back_is_one_template_card() {
        let rows = vec![
            Choice {
                value: "a".into(),
                label: "Alpha".into(),
                hint: "first".into(),
                available: true,
                key: None,
            },
            back_row(),
        ];
        let out = template_text("Which one?", &rows);
        assert!(
            out.contains("Which one?") && out.contains("[1] Alpha"),
            "{out}"
        );
        assert!(out.contains("[2]") && out.contains("Back"), "{out}");
    }

    #[test]
    fn a_confirm_is_two_keyed_rows_in_the_template() {
        let out = template_text("Copy it?", &yes_no_rows());
        assert!(out.contains("[1] Yes") && out.contains("[2] No"), "{out}");
    }

    #[test]
    fn a_tick_list_is_rows_with_boxes_and_a_done_row() {
        let items = [
            ("a".to_owned(), "Alpha".to_owned()),
            ("b".to_owned(), "Beta".to_owned()),
        ];
        let out = template_text("Pick any", &ticked_rows(&items, &["b".to_owned()]));
        assert!(
            out.contains("Alpha") && out.contains("Beta") && out.contains("Done"),
            "{out}"
        );
    }

    thread_local! {
        static CLOCK: (Instant, Cell<Duration>) = (Instant::now(), Cell::new(Duration::ZERO));
    }

    fn at(ms: u64) -> Instant {
        CLOCK.with(|(base, off)| {
            off.set(Duration::from_millis(ms));
            *base + off.get()
        })
    }

    fn clock() -> Instant {
        CLOCK.with(|(base, off)| *base + off.get())
    }

    /// A terminal that keeps what was flushed to it: one entry per redraw that got through.
    #[derive(Debug, Default, Clone)]
    struct Rec(
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        std::sync::Arc<std::sync::Mutex<String>>,
    );

    impl indicatif::TermLike for Rec {
        fn width(&self) -> u16 {
            200
        }
        fn move_cursor_up(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn move_cursor_down(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn move_cursor_right(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn move_cursor_left(&self, _: usize) -> io::Result<()> {
            Ok(())
        }
        fn write_line(&self, s: &str) -> io::Result<()> {
            self.1.lock().unwrap().push_str(s);
            Ok(())
        }
        fn write_str(&self, s: &str) -> io::Result<()> {
            self.1.lock().unwrap().push_str(s);
            Ok(())
        }
        fn clear_line(&self) -> io::Result<()> {
            Ok(())
        }
        fn flush(&self) -> io::Result<()> {
            let drawn = std::mem::take(&mut *self.1.lock().unwrap());
            self.0.lock().unwrap().push(drawn.trim().to_owned());
            Ok(())
        }
    }

    fn row_on(
        every: Duration,
    ) -> (
        indicatif::ProgressBar,
        Rec,
        std::sync::Arc<std::sync::Mutex<super::Gate>>,
    ) {
        let rec = Rec::default();
        let gate = std::sync::Arc::new(std::sync::Mutex::new(super::Gate::new(every)));
        let term = super::Paced {
            inner: rec.clone(),
            gate: std::sync::Arc::clone(&gate),
            now: clock,
        };
        let bar = indicatif::ProgressBar::with_draw_target(
            Some(100),
            indicatif::ProgressDrawTarget::term_like(Box::new(term)),
        );
        bar.set_style(indicatif::ProgressStyle::with_template("{msg}").unwrap());
        (bar, rec, gate)
    }

    #[test]
    fn a_paced_row_is_drawn_once_a_second_with_the_latest_state_and_its_end_always() {
        // Every progress line redrew the row before: five lines inside one second, five draws.
        at(0);
        let (bar, rec, _) = row_on(Duration::ZERO);
        for (i, ms) in [0, 250, 500, 750, 900].into_iter().enumerate() {
            at(ms);
            bar.set_message(format!("line {i}"));
        }
        assert_eq!(rec.0.lock().unwrap().len(), 5);

        // Paced, the same five lines draw once; a line after a second draws the latest.
        at(0);
        let (bar, rec, gate) = row_on(super::REDRAW);
        for (i, ms) in [0, 250, 500, 750, 900].into_iter().enumerate() {
            at(ms);
            bar.set_message(format!("line {i}"));
        }
        assert_eq!(
            *rec.0.lock().unwrap(),
            ["line 0"],
            "one draw inside the second"
        );
        at(1000);
        bar.set_message("line 5");
        bar.set_message("line 6");
        assert_eq!(
            *rec.0.lock().unwrap(),
            ["line 0", "line 5"],
            "one more after it"
        );

        // The spinner's own tick obeys the same gate: a tick inside the second draws nothing.
        at(1400);
        bar.tick();
        bar.set_message("line 7");
        assert_eq!(rec.0.lock().unwrap().len(), 2);

        // The row's end always draws, inside the second too, and it shows the latest state.
        at(1500);
        gate.lock().unwrap().force = true;
        bar.set_message("line 8");
        assert_eq!(rec.0.lock().unwrap().last().unwrap(), "line 8");
        assert_eq!(rec.0.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_numbered_plan_wraps_under_its_own_text() {
        let items = vec![
            "check your sign-in".to_owned(),
            "write 81 files into ./bg and then some more words to wrap".to_owned(),
        ];
        let out = super::numbered(&items, 30);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "1. check your sign-in");
        assert!(lines[1].starts_with("2. write 81 files"), "{out}");
        assert!(lines[2].starts_with("   "), "{out}");
        assert!(lines.iter().all(|l| l.chars().count() <= 30), "{out}");
    }

    #[test]
    fn a_report_state_is_a_success_only_when_it_is_done() {
        use super::Mark;
        assert_eq!(Mark::of("done"), Mark::Ok);
        for warned in ["skipped", "ask", "not-yet", "something-new"] {
            assert_eq!(Mark::of(warned), Mark::Warn, "{warned}");
        }
        assert_eq!(Mark::of("failed"), Mark::Fail);
    }

    #[test]
    fn a_finished_step_wears_a_check_a_bang_or_a_cross() {
        assert_eq!(strip(&Marks.active_symbol()), "\u{2713}");
        assert_eq!(strip(&Marks.warning_symbol()), "!");
        assert_eq!(strip(&Marks.error_symbol()), "\u{2717}");
    }

    fn check_rows() -> Vec<super::CheckRow> {
        use super::{CheckRow, Mark};
        vec![
            CheckRow {
                mark: Mark::Fail,
                id: "auth".into(),
                what: "signed out".into(),
                notes: vec![("fix", "iohr login".into())],
            },
            CheckRow {
                mark: Mark::Ok,
                id: "config".into(),
                what: "reads".into(),
                notes: vec![],
            },
        ]
    }

    #[test]
    fn checks_line_up_names_hang_the_fix_under_its_row_and_count_what_needs_doing() {
        let out =
            Paint::from_parts(true, false, None, Some("xterm-256color")).checks(&check_rows());
        assert!(out.contains('\u{1b}'), "a person gets colour: {out:?}");
        let lines: Vec<String> = strip(&out).lines().map(str::to_owned).collect();
        assert_eq!(lines[0], "  \u{2717} auth    signed out");
        assert_eq!(lines[1], "            fix  iohr login");
        assert_eq!(lines[2], "  \u{2713} config  reads");
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], "  1 ok \u{b7} 1 to fix");
    }

    #[test]
    fn findings_count_what_was_checked_and_say_so_when_nothing_was_found() {
        let tty = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let rows = check_rows();
        let out = strip(&tty.findings(&rows[..1], "4 documents checked"));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "  \u{2717} auth  signed out");
        assert_eq!(lines[1], "          fix  iohr login");
        assert_eq!(lines[3], "  4 documents checked \u{b7} 1 to fix");
        assert_eq!(
            strip(&tty.findings(&[], "4 documents checked")),
            "  \u{2713} 4 documents checked, nothing found\n"
        );
    }

    #[test]
    fn a_tone_is_colour_for_a_person_and_the_same_text_for_a_pipe() {
        use super::Mark;
        let tty = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let toned = tty.tone(Mark::Fail, "down");
        assert_ne!(toned, "down");
        assert_eq!(strip(&toned), "down");
        assert_eq!(Paint::plain().tone(Mark::Fail, "down"), "down");
        assert_eq!(
            Paint::from_parts(true, false, Some("1"), None).tone(Mark::Ok, "up"),
            "up"
        );
    }

    fn strip(s: &str) -> String {
        console::strip_ansi_codes(s).into_owned()
    }

    #[test]
    fn a_long_log_line_is_broken_at_spaces_inside_the_width() {
        let text = "iohr project init: AI-built project, generated by an AI model (claude) under your own subscription; the folder goes to that provider. Check it before you rely on it.";
        let broke = wrapped(text, 60);
        assert!(broke.lines().count() >= 3, "{broke}");
        assert!(
            broke.lines().all(|l| console::measure_text_width(l) <= 60),
            "{broke}"
        );
        assert_eq!(broke.replace('\n', " "), text, "no word is lost or cut");
        assert_eq!(wrapped("short", 60), "short");
        assert_eq!(wrapped("a\nb", 60), "a\nb");
        assert_eq!(
            wrapped("unbroken-word-wider-than-ten", 10),
            "unbroken-word-wider-than-ten"
        );
    }

    #[test]
    fn the_look_needs_a_terminal_and_a_colour_capable_term() {
        assert!(fancy_for(true, false, None, Some("xterm-256color")));
        assert!(fancy_for(true, false, Some(""), Some("xterm")));
    }

    #[test]
    fn no_terminal_means_the_plain_path() {
        assert!(!fancy_for(false, false, None, Some("xterm")));
    }

    #[test]
    fn json_means_the_plain_path() {
        assert!(!fancy_for(true, true, None, Some("xterm")));
    }

    #[test]
    fn no_color_means_the_plain_path() {
        assert!(!fancy_for(true, false, Some("1"), Some("xterm")));
    }

    #[test]
    fn a_dumb_or_missing_term_means_the_plain_path() {
        assert!(!fancy_for(true, false, None, Some("dumb")));
        assert!(!fancy_for(true, false, None, Some("")));
        assert!(!fancy_for(true, false, None, None));
    }

    #[test]
    fn a_plain_step_never_draws_a_bar_whatever_the_agent_reports() {
        // Plain is every case `fancy_for` refuses: no terminal, --json, NO_COLOR, a dumb TERM.
        for (tty, json, no_color, term) in [
            (false, false, None, Some("xterm")),
            (true, true, None, Some("xterm")),
            (true, false, Some("1"), Some("xterm")),
            (true, false, None, Some("dumb")),
        ] {
            let ui = Ui {
                fancy: fancy_for(tty, json, no_color, term),
            };
            let mut step = ui.step("Crawling");
            step.progress(1, None, "1 page");
            step.progress(3, Some(6), "3 of 6");
            step.progress(6, Some(6), "6 of 6");
            assert!(step.bar.is_none(), "{tty} {json} {no_color:?} {term:?}");
            assert!(!step.bounded, "{tty} {json} {no_color:?} {term:?}");
        }
    }

    #[test]
    fn paint_is_plain_without_a_terminal_colour_or_with_json() {
        for p in [
            Paint::from_parts(false, false, None, Some("xterm")),
            Paint::from_parts(true, false, Some("1"), Some("xterm")),
            Paint::from_parts(true, true, None, Some("xterm")),
            Paint::plain(),
        ] {
            assert_eq!(p.head("NAME"), "NAME");
            assert_eq!(p.status("live"), "live");
            assert_eq!(p.dim("x"), "x");
        }
    }

    #[test]
    fn paint_colours_on_a_terminal_and_marks_status() {
        let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
        assert!(p.tty());
        assert!(p.head("NAME").contains('\u{1b}'));
        let strip = |s: String| console::strip_ansi_codes(&s).into_owned();
        assert_eq!(strip(p.status("live")), "\u{25cf} live");
        assert_eq!(strip(p.status("coming")), "\u{25cb} coming");
        assert_eq!(strip(p.status("invalid")), "\u{25cf} invalid");
    }

    fn token_rows() -> Vec<Vec<String>> {
        vec![
            vec![
                "tok_01M72XABCDEF".to_owned(),
                "ci-deploy".to_owned(),
                "active".to_owned(),
            ],
            vec![
                "tok_01M6ZQWERTYU".to_owned(),
                "laptop".to_owned(),
                "something".to_owned(),
            ],
        ]
    }

    #[test]
    fn plain_tables_and_pairs_keep_their_bytes() {
        let rows = token_rows();
        let header = ["ID", "NAME", "STATE"];
        assert_eq!(
            Paint::plain().table(&header, &rows),
            crate::output::table_text(&header, &rows)
        );
        let dumb = Paint::from_parts(true, false, None, Some("dumb"));
        assert_eq!(
            dumb.table(&header, &rows),
            crate::output::table_text(&header, &rows)
        );
        let pairs = [
            ("id", "tok_01M72XABCDEF".to_owned()),
            ("state", "active".to_owned()),
        ];
        assert_eq!(
            Paint::plain().pairs(&pairs),
            "id     tok_01M72XABCDEF\nstate  active\n"
        );
    }

    #[test]
    fn drawn_tables_mark_states_and_quiet_ids() {
        let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let text = p.table(&["ID", "NAME", "STATE"], &token_rows());
        let lines: Vec<String> = text.lines().map(strip).collect();
        assert_eq!(lines[0], "  ID                NAME       STATE");
        assert_eq!(lines[1], "  tok_01M72XABCDEF  ci-deploy  \u{2713} active");
        assert_eq!(lines[2], "  tok_01M6ZQWERTYU  laptop     something");
        assert!(text.contains(&p.dim("tok_01M72XABCDEF")), "ids are quiet");
        assert!(!text.contains(&p.dim("ci-deploy")), "names lead");
        let shown = p.pairs(&[
            ("state", "failed".to_owned()),
            ("name", "laptop".to_owned()),
        ]);
        assert_eq!(strip(&shown), "  state  \u{2717} failed\n  name   laptop\n");
        assert!(shown.contains(&p.dim("state")), "keys are dim");
    }

    #[test]
    fn a_drawn_table_cuts_its_widest_column_to_fit_the_terminal_and_plain_keeps_every_byte() {
        let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let id = "60fb098e-0403-4aac-a6db-7ed64b151e21".to_owned();
        let rows = vec![vec![id.clone(), "a:read ".repeat(30), "active".to_owned()]];
        let header = ["ID", "SCOPES", "STATE"];
        let text = p.table_within(&header, &rows, 66);
        for line in text.lines() {
            assert!(console::measure_text_width(line) <= 66, "{line:?} fits");
        }
        assert!(strip(&text).contains('\u{2026}'), "the cut is shown");
        assert!(
            strip(&text).contains("active"),
            "the narrow columns stay whole"
        );
        assert!(strip(&text).contains(&id), "an id is never cut");
        let plain = Paint::from_parts(false, false, None, None);
        assert_eq!(
            plain.table_within(&header, &rows, 66),
            crate::output::table_text(&header, &rows)
        );
    }

    #[test]
    fn a_drawn_table_cuts_a_path_at_its_start_so_the_file_stays() {
        let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let path = format!("/{}/folder/IO.md", "deep/".repeat(20));
        let rows = vec![vec!["IO.md".to_owned(), "8".to_owned(), path.clone()]];
        let header = ["FILE", "BYTES", "PATH"];
        let text = strip(&p.table_within(&header, &rows, 50));
        let line = text.lines().nth(1).unwrap_or_default();
        assert!(console::measure_text_width(line) <= 50, "{line:?} fits");
        assert!(line.ends_with("/folder/IO.md"), "the file stays: {line:?}");
        assert!(line.contains('\u{2026}'), "the cut is shown");
        assert_eq!(super::cut_front("short", 10), "short");
        assert_eq!(super::cut_front("/a/b/c/IO.md", 8), "\u{2026}c/IO.md");
    }

    #[test]
    fn a_drawn_list_ends_with_its_count_and_next_command() {
        let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let tokens = super::Listed::new("token", "tokens").next("new one", "iohr token create");
        let text = p.list(&["ID", "NAME", "STATE"], &token_rows(), &tokens);
        let lines: Vec<String> = text.lines().map(strip).collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[3], "  2 tokens \u{b7} new one: iohr token create");
        assert!(
            text.contains(&p.name("iohr token create")),
            "the command stands out"
        );
        let one = p.list(&["ID", "NAME", "STATE"], &token_rows()[..1], &tokens);
        assert!(strip(&one).ends_with("  1 token \u{b7} new one: iohr token create\n"));
        let bare = p.list(
            &["ID"],
            &token_rows(),
            &super::Listed::new("token", "tokens"),
        );
        assert!(strip(&bare).ends_with("\n  2 tokens\n"));
    }

    #[test]
    fn an_empty_list_says_so_on_a_terminal_and_keeps_its_header_line_in_plain_output() {
        let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let tokens = super::Listed::new("token", "tokens").next("new one", "iohr token create");
        assert_eq!(
            strip(&p.list(&["ID", "NAME"], &[], &tokens)),
            "  No tokens yet. Next: iohr token create\n"
        );
        let grants = super::Listed::new("grant", "grants");
        assert_eq!(strip(&p.list(&["ID"], &[], &grants)), "  No grants yet.\n");
        let plain = Paint::plain();
        assert_eq!(plain.list(&["ID", "NAME"], &[], &tokens), "ID  NAME\n");
        assert_eq!(
            plain.list(&["ID", "NAME", "STATE"], &token_rows(), &tokens),
            plain.table(&["ID", "NAME", "STATE"], &token_rows()),
            "plain output is the table alone"
        );
    }

    #[test]
    fn ids_and_states_are_read_narrowly() {
        assert!(super::looks_like_id("apr_01M74R2BRNZTTKWNYK94DTQHTR"));
        assert!(super::looks_like_id("2779ba0b-df69-4cfc-89fe-101441554413"));
        assert!(!super::looks_like_id("ci-deploy"));
        assert!(!super::looks_like_id("snake_case_name"));
        assert_eq!(super::state_mark("Active"), Some(super::Mark::Ok));
        assert_eq!(super::state_mark("in_progress"), Some(super::Mark::Warn));
        assert_eq!(super::state_mark("revoked"), Some(super::Mark::Fail));
        assert_eq!(super::state_mark("coming"), None);
    }

    fn choice(value: &str, key: char) -> Choice {
        Choice {
            value: value.to_owned(),
            label: value.to_owned(),
            hint: String::new(),
            available: true,
            key: Some(key),
        }
    }

    fn choices() -> Vec<Choice> {
        vec![
            choice("run", 'r'),
            choice("copy", 'c'),
            choice("follow", 'f'),
            choice("explain", 'e'),
        ]
    }

    /// Runs the menu on a scripted key source (`true` = the key came in the same burst as
    /// the one before). Gives what it returned and the entry highlighted at each draw. A
    /// script that runs out ends the test: the menu never read past what was scripted.
    fn keyed(script: Vec<(io::Result<Key>, bool)>) -> (io::Result<Option<String>>, Vec<usize>) {
        keyed_with(script, false)
    }

    fn keyed_with(
        script: Vec<(io::Result<Key>, bool)>,
        numbered: bool,
    ) -> (io::Result<Option<String>>, Vec<usize>) {
        let drawn = RefCell::new(Vec::new());
        let reads = Cell::new(0);
        let script = RefCell::new(script.into_iter());
        let out = run_keyed(
            &choices(),
            "run",
            numbered,
            |at| {
                drawn.borrow_mut().push(at);
                Ok(())
            },
            || Ok(()),
            || {
                reads.set(reads.get() + 1);
                script
                    .borrow_mut()
                    .next()
                    .unwrap_or_else(|| (Err(io::Error::other("script ended")), false))
            },
        );
        (out, drawn.into_inner())
    }

    fn typed(k: Key) -> (io::Result<Key>, bool) {
        (Ok(k), false)
    }

    fn burst(k: Key) -> (io::Result<Key>, bool) {
        (Ok(k), true)
    }

    fn interrupted() -> (io::Result<Key>, bool) {
        (Err(io::ErrorKind::Interrupted.into()), false)
    }

    // The owner's bug: an interrupted read (a SIGWINCH, the SIGCHLD of an async prompt's
    // worker) ended the menu before any choice.
    #[test]
    fn an_interrupted_read_is_read_again_and_the_letter_after_it_picks() {
        let (out, _) = keyed(vec![interrupted(), typed(Key::Char('r'))]);
        assert_eq!(out.unwrap().as_deref(), Some("run"));
        let (out, _) = keyed(vec![
            interrupted(),
            interrupted(),
            interrupted(),
            typed(Key::Enter),
        ]);
        assert_eq!(
            out.unwrap().as_deref(),
            Some("run"),
            "Enter picks the highlighted"
        );
    }

    #[test]
    fn the_menu_holds_through_unknown_keys_and_picks_only_on_a_real_choice() {
        let (out, drawn) = keyed(vec![
            typed(Key::Char('x')),
            typed(Key::Home),
            typed(Key::Backspace),
            typed(Key::Unknown),
            typed(Key::ArrowDown),
            typed(Key::Tab),
            typed(Key::ArrowUp),
            typed(Key::Enter),
        ]);
        assert_eq!(out.unwrap().as_deref(), Some("copy"));
        assert_eq!(drawn, [0, 1, 2, 1], "redrawn only when the highlight moves");
    }

    #[test]
    fn esc_q_and_ctrl_c_leave_with_nothing_and_a_letter_is_case_blind() {
        for k in [Key::Escape, Key::CtrlC, Key::Char('q'), Key::Char('Q')] {
            let (out, _) = keyed(vec![typed(k)]);
            assert_eq!(out.unwrap(), None);
        }
        let (out, _) = keyed(vec![typed(Key::Char('E'))]);
        assert_eq!(out.unwrap().as_deref(), Some("explain"));
    }

    // A shell's line editor puts the terminal in application cursor mode: Down is `ESC O B`,
    // which `console` reads as an unknown `ESC O` and a plain `B`; End is `ESC O F`, whose
    // `F` used to pick Follow up, and Right is `ESC O C`, whose `C` used to pick Copy.
    #[test]
    fn application_cursor_keys_move_and_never_pick_a_hotkey() {
        let ss3 = |last: char| vec![typed(Key::UnknownEscSeq(vec!['O'])), burst(Key::Char(last))];
        let (out, drawn) = keyed(
            [ss3('B'), ss3('B'), ss3('A'), vec![typed(Key::Enter)]]
                .into_iter()
                .flatten()
                .collect(),
        );
        assert_eq!(out.unwrap().as_deref(), Some("copy"));
        assert_eq!(drawn, [0, 1, 2, 1]);
        for last in ['C', 'D', 'F', 'H', 'P'] {
            let (out, drawn) = keyed(
                ss3(last)
                    .into_iter()
                    .chain([typed(Key::Char('e'))])
                    .collect(),
            );
            assert_eq!(out.unwrap().as_deref(), Some("explain"), "ESC O {last}");
            assert_eq!(drawn, [0], "ESC O {last} did nothing");
        }
    }

    #[test]
    fn the_rest_of_an_unknown_sequence_is_skipped_but_a_person_typing_is_not() {
        // `ESC [ 1 ; 5 C` (Ctrl-Right): the reader returns `ESC [ 1 ;`, then `5`, `C`.
        let (out, drawn) = keyed(vec![
            typed(Key::UnknownEscSeq(vec!['[', '1', ';'])),
            burst(Key::Char('5')),
            burst(Key::Char('C')),
            typed(Key::Char('f')),
        ]);
        assert_eq!(out.unwrap().as_deref(), Some("follow"));
        assert_eq!(drawn, [0]);
        // The same letter typed a moment later, not in the burst, is a choice.
        let (out, _) = keyed(vec![
            typed(Key::UnknownEscSeq(vec!['[', '1', ';'])),
            typed(Key::Char('c')),
        ]);
        assert_eq!(out.unwrap().as_deref(), Some("copy"));
    }

    #[test]
    fn only_a_real_failure_ends_the_menu_with_an_error() {
        let (out, _) = keyed(vec![(
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            false,
        )]);
        assert_eq!(out.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        // Not a terminal: keys that never come.
        let script = (0..200).map(|_| typed(Key::Unknown)).collect();
        let (out, _) = keyed(script);
        assert_eq!(out.unwrap_err().kind(), io::ErrorKind::NotConnected);
        // A signal storm that never lets a read finish ends it too, as a cancel.
        let script = (0..2000).map(|_| interrupted()).collect();
        let (out, _) = keyed(script);
        assert_eq!(out.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn in_the_bar_a_digit_picks_that_row_and_j_k_move_but_not_in_other_menus() {
        let (out, _) = keyed_with(vec![typed(Key::Char('3'))], true);
        assert_eq!(out.unwrap().as_deref(), Some("follow"));
        // A digit past the last row, and 0, do nothing; the next key decides.
        let (out, drawn) = keyed_with(
            vec![
                typed(Key::Char('9')),
                typed(Key::Char('0')),
                typed(Key::Char('j')),
                typed(Key::Char('j')),
                typed(Key::Char('k')),
                typed(Key::Enter),
            ],
            true,
        );
        assert_eq!(out.unwrap().as_deref(), Some("copy"));
        assert_eq!(drawn, [0, 1, 2, 1]);
        // The plain menu has no numbers.
        let (out, drawn) = keyed_with(
            vec![
                typed(Key::Char('3')),
                typed(Key::Char('j')),
                typed(Key::Char('r')),
            ],
            false,
        );
        assert_eq!(out.unwrap().as_deref(), Some("run"));
        assert_eq!(drawn, [0]);
    }

    fn bar_choices() -> Vec<Choice> {
        let mut c = choices();
        for (c, hint) in c.iter_mut().zip([
            "put it on your prompt",
            "to the clipboard",
            "ask more about this answer",
            "ask what it means",
        ]) {
            c.label = format!("{}{}", c.value[..1].to_uppercase(), &c.value[1..]);
            c.hint = hint.to_owned();
        }
        c
    }

    const FANCY: Look = Look {
        fancy: true,
        truecolor: false,
    };
    const PLAIN: Look = Look {
        fancy: false,
        truecolor: false,
    };

    #[test]
    fn the_bar_is_a_card_box_with_numbered_rows_the_letter_and_a_footer() {
        let lines: Vec<String> = bar_lines("What next?", &bar_choices(), 2, 80, FANCY)
            .iter()
            .map(|l| strip(l))
            .collect();
        assert!(
            lines[0].starts_with("\u{256d}\u{2500} What next? \u{2500}"),
            "{lines:?}"
        );
        assert_eq!(
            console::measure_text_width(&lines[0]),
            79,
            "card width: cols less one"
        );
        assert!(
            lines[1].starts_with("\u{2502}    [1] Run      r  put it"),
            "{lines:?}"
        );
        assert!(
            lines[3].starts_with("\u{2502}  \u{203a} [3] Follow   f  ask more"),
            "{lines:?}"
        );
        assert_eq!(lines.len(), 6);
        assert!(lines[5].starts_with("\u{2570}\u{2500} \u{2191}\u{2193} move \u{b7} 1-9 or letter pick \u{b7} enter \u{b7} esc"), "{lines:?}");
        // The same height for every highlight, and nothing is as wide as the terminal.
        for cols in [40, 41, 55, 80, 200] {
            for at in 0..4 {
                let l = bar_lines("What next?", &bar_choices(), at, cols, FANCY);
                assert_eq!(l.len(), 6);
                assert!(
                    l.iter().all(|x| console::measure_text_width(x) < cols),
                    "{cols} {l:?}"
                );
            }
        }
    }

    #[test]
    fn plain_or_narrow_the_bar_is_one_numbered_line_per_row_with_no_box_or_escape_codes() {
        for (look, cols) in [(PLAIN, 80), (FANCY, 39)] {
            let lines = bar_lines("What next?", &bar_choices(), 0, cols, look);
            let text: Vec<String> = lines.iter().map(|l| strip(l)).collect();
            assert_eq!(text[0], "What next?");
            assert!(text[1].starts_with("\u{203a} [1] Run"), "{text:?}");
            assert!(text[2].starts_with("  [2] Copy"), "{text:?}");
            assert!(
                text.iter()
                    .all(|l| !l.contains('\u{2502}') && !l.contains('\u{256d}')),
                "{text:?}"
            );
            assert!(
                text.iter().all(|l| console::measure_text_width(l) < cols),
                "{text:?}"
            );
            if !look.fancy {
                assert_eq!(lines, text, "no colour at all");
            }
        }
    }
}
