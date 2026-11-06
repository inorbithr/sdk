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
use std::time::{Duration, Instant};

use console::Key;

use crate::card::Look;
use crate::error::Error;
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

    /// A mark that matters, such as the default's `*`: bold yellow.
    #[must_use]
    pub(crate) fn mark(self, text: &str) -> String {
        self.apply(text, |s| s.yellow().bold())
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
/// [`Ui::select_keyed`].
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

/// What typed into a text step goes back one step. cliclack 0.5 reports Esc and Ctrl-C as the
/// same `Interrupted` error, so Esc cannot be the way back: this is, and the prompt says so.
const BACK_WORD: &str = "<";

/// The look, decided once per command.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ui {
    fancy: bool,
}

/// A running step: a spinner on a terminal. Plain, it is silent: the command's own plain
/// report (the stable lines scripts read) says how each step went.
pub(crate) struct Step {
    bar: Option<cliclack::ProgressBar>,
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

/// The loop of [`Ui::select_keyed`] and [`Ui::select_bar`], with the drawing and the key source passed in so it is
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

/// The width of the terminal on stderr, 80 when it is not known.
pub(crate) fn terminal_width() -> usize {
    usize::from(console::Term::stderr().size().1).max(40)
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

/// The room a log line has inside the rail: the terminal's width less the rail and its gap.
fn log_width() -> usize {
    terminal_width().saturating_sub(4).max(30)
}

/// The text of each menu entry: labels padded to one width so the reasons line up in a
/// column, the reason dim, and the whole line cut to `width` (less the menu's own margin) so
/// it never wraps. An entry that cannot be chosen is dim and says "coming".
fn menu_rows(choices: &[Choice], width: usize) -> Vec<String> {
    let label = |c: &Choice| {
        if c.available {
            c.label.clone()
        } else {
            format!("{} (coming)", c.label)
        }
    };
    let pad = choices
        .iter()
        .map(|c| console::measure_text_width(&label(c)))
        .max()
        .unwrap_or(0);
    let room = width.saturating_sub(6);
    choices
        .iter()
        .map(|c| {
            let l = label(c);
            let gap = pad - console::measure_text_width(&l);
            let plain_line = if c.hint.is_empty() {
                l.clone()
            } else {
                format!("{l}{}  {}", " ".repeat(gap), c.hint)
            };
            let cut = console::truncate_str(&plain_line, room, "...").into_owned();
            let head: String = cut.chars().take(l.chars().count()).collect();
            let tail: String = cut.chars().skip(l.chars().count()).collect();
            let head = if c.available {
                head
            } else {
                console::style(head).dim().to_string()
            };
            format!("{head}{}", console::style(tail).dim())
        })
        .collect()
}

/// The columns below which the action bar is one plain line per row, with no box.
const BAR_BOX_MIN: usize = 40;

/// The action bar as lines (the same count for every `at`): in a box that matches the card's
/// frame (`╭─` header, `│` rail, `╰─` footer, its width and colours) when the look is drawn
/// and the terminal is `cols` wide enough; otherwise one line per row, numbered all the same.
/// Each row is `› [1] Name  k  hint`: the chip, the name, its letter and the dim hint; the
/// highlighted row has the `›` and is reversed. `cols` of 0 is unknown, taken as 80.
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
    if boxed {
        for l in top.lines {
            if l.is_empty() {
                out.push(look.violet().apply_to("\u{2502}").to_string());
            } else {
                out.push(format!("{}{l}", look.violet().apply_to(lead)));
            }
        }
    }
    for (i, c) in choices.iter().enumerate() {
        let on = i == at;
        let chip = format!("[{}]", i + 1);
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
    let foot = truncate_str(FOOT, width.saturating_sub(3), "\u{2026}");
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
    /// The look for this run: drawn only when [`fancy_for`] says so.
    #[must_use]
    pub(crate) fn detect(json: bool) -> Self {
        Self {
            fancy: fancy_for(
                prompt::interactive(),
                json,
                std::env::var("NO_COLOR").ok().as_deref(),
                std::env::var("TERM").ok().as_deref(),
            ),
        }
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
        if !self.fancy {
            return Step { bar: None };
        }
        let bar = cliclack::spinner();
        bar.start(what);
        Step { bar: Some(bar) }
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
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass the choice as a flag"
            )));
        }
        let mut all = choices.to_vec();
        if back {
            all.push(back_row());
        }
        if all.iter().any(|c| c.key.is_some()) {
            let picked = Self::select_keyed(prompt, &all, initial)?
                .ok_or_else(|| Error::Failed("cancelled".into()))?;
            return Ok(if back && picked == BACK {
                Nav::Back
            } else {
                Nav::Next(picked)
            });
        }
        let rows = menu_rows(&all, terminal_width());
        loop {
            let mut menu = cliclack::select::<String>(prompt);
            for (c, label) in all.iter().zip(&rows) {
                menu = menu.item(c.value.clone(), label.clone(), "");
            }
            let picked = menu
                .initial_value(initial.to_owned())
                .interact()
                .map_err(|e| cancelled(&e))?;
            match all.iter().find(|c| c.value == picked) {
                Some(c) if c.available => {
                    return Ok(if back && picked == BACK {
                        Nav::Back
                    } else {
                        Nav::Next(picked)
                    });
                }
                _ => {
                    let _ = cliclack::log::warning(format!("{picked} is coming; pick a live one"));
                }
            }
        }
    }

    /// A menu of entries with a letter each: the letter picks, the arrows and Enter pick the
    /// highlighted one, Esc, `q` and Ctrl-C leave (`None`). `cliclack`'s menu has no hotkeys,
    /// so this one is drawn on stderr with `console` and works on any terminal, with or
    /// without colour (`console` honours `NO_COLOR`). Entries that are not available are left
    /// out by the caller.
    ///
    /// # Errors
    ///
    /// No terminal on stderr, or the terminal cannot be read.
    pub(crate) fn select_keyed(
        prompt: &str,
        choices: &[Choice],
        initial: &str,
    ) -> Result<Option<String>, Error> {
        use console::style;
        Self::select_drawn(prompt, choices, initial, false, |at| {
            let mut rows = vec![style(prompt).bold().to_string()];
            for (i, c) in choices.iter().enumerate() {
                let key = c.key.map_or_else(String::new, |k| format!("{k}"));
                rows.push(format!(
                    "{} {} {}  {}",
                    if i == at { "\u{203a}" } else { " " },
                    style(key).cyan(),
                    c.label,
                    style(&c.hint).dim()
                ));
            }
            rows
        })
    }

    /// The action bar under a card (PRD 0187.1): [`Ui::select_keyed`]'s keys, plus a number
    /// for each row (`1`-`9` pick it at once, `j` and `k` move), drawn by [`bar_lines`] in a
    /// box that reads as part of the card. Esc, `q` and Ctrl-C leave (`None`) and the bar is
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
        let look = Look::detect(Ui::detect(false).fancy());
        let cols = usize::from(console::Term::stderr().size().1);
        Self::select_drawn(prompt, choices, initial, true, |at| {
            bar_lines(prompt, choices, at, cols, look)
        })
    }

    /// The bar as the foot of a card of the caller's own (the permission ask): `rows(at)` is
    /// the whole screen with row `at` highlighted, always the same number of lines, drawn
    /// with the bar's keys (arrows, `1`-`9`, a row's letter, Enter, Esc, `q`, Ctrl-C). Returns
    /// the value of the row picked, or `None` for a cancel; the card is erased either way.
    ///
    /// # Errors
    ///
    /// No terminal on stderr, or the terminal cannot be read.
    #[cfg(unix)]
    pub(crate) fn select_card(
        prompt: &str,
        choices: &[Choice],
        initial: &str,
        rows: impl Fn(usize) -> Vec<String>,
    ) -> Result<Option<String>, Error> {
        Self::select_drawn(prompt, choices, initial, true, rows)
    }

    /// The loop of both menus: `rows(at)` is the screen with entry `at` highlighted, always
    /// the same number of lines, so erasing it leaves nothing behind.
    fn select_drawn(
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
    #[allow(
        dead_code,
        reason = "the plain form of input_step: other commands (and the card-actions work) ask with it"
    )]
    pub(crate) fn input(
        self,
        prompt: &str,
        default: &str,
        check: impl Fn(&str) -> Result<(), String> + 'static,
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
        check: impl Fn(&str) -> Result<(), String> + 'static,
        back: bool,
    ) -> Result<Nav<String>, Error> {
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass it as an argument"
            )));
        }
        let shown = if back {
            format!("{prompt}  {}", console::style("(< goes back)").dim())
        } else {
            prompt.to_owned()
        };
        let mut q = cliclack::input(shown).validate(move |v: &String| {
            let v = v.trim();
            if back && v == BACK_WORD {
                Ok(())
            } else {
                check(v)
            }
        });
        if !default.is_empty() {
            q = q.default_input(default);
        }
        let typed = q
            .interact::<String>()
            .map(|s| s.trim().to_owned())
            .map_err(|e| cancelled(&e))?;
        Ok(if back && typed == BACK_WORD {
            Nav::Back
        } else {
            Nav::Next(typed)
        })
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

    /// The list of [`Ui::multiselect`] with a label apart from each value.
    fn multiselect_items(
        self,
        prompt: &str,
        items: &[(String, String)],
        initial: &[String],
    ) -> Result<Vec<String>, Error> {
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass it with --set"
            )));
        }
        let mut menu = cliclack::multiselect::<String>(prompt).required(false);
        for (value, label) in items {
            menu = menu.item(value.clone(), label.clone(), "");
        }
        menu.initial_values(initial.to_vec())
            .interact()
            .map_err(|e| cancelled(&e))
    }

    /// A yes or no. Plain: the default, asked of no one.
    ///
    /// # Errors
    ///
    /// The person cancelled.
    pub(crate) fn confirm(self, prompt: &str, default: bool) -> Result<bool, Error> {
        if !self.fancy {
            return Ok(default);
        }
        cliclack::confirm(prompt)
            .initial_value(default)
            .interact()
            .map_err(|e| cancelled(&e))
    }
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
        items.push((
            BACK.to_owned(),
            "Back (tick it to go to the step before)".to_owned(),
        ));
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
        let yes_no = |value: &str, label: &str| Choice {
            value: value.into(),
            label: label.into(),
            hint: String::new(),
            available: true,
            key: None,
        };
        let rows = [yes_no("yes", "Yes"), yes_no("no", "No")];
        Ok(
            match self.select_step(prompt, &rows, if default { "yes" } else { "no" }, true)? {
                Nav::Next(v) => Nav::Next(v == "yes"),
                Nav::Back => Nav::Back,
            },
        )
    }
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

    /// The step worked. The spinner is cleared and the result is a log line of its own: the
    /// spinner's own finish pads its line to the terminal width and relies on the terminal to
    /// wrap, which drew a stray rail mark and a second, wrapped line.
    pub(crate) fn ok(self, said: &str) {
        if let Some(b) = self.bar {
            b.clear();
            let _ = cliclack::log::step(wrapped(said, log_width()));
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

    use super::wrapped;
    use console::Key;

    use super::{Choice, Look, Paint, bar_lines, fancy_for, menu_rows, run_keyed};

    fn strip(s: &str) -> String {
        console::strip_ansi_codes(s).into_owned()
    }

    fn c(value: &str, label: &str, hint: &str, available: bool) -> Choice {
        Choice {
            value: value.into(),
            label: label.into(),
            hint: hint.into(),
            available,
            key: None,
        }
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
    fn menu_reasons_start_in_one_column() {
        console::set_colors_enabled(false);
        let rows = menu_rows(
            &[
                c("a", "./my-shop", "in the folder you are in", true),
                c("b", "~/code/my-shop", "your projects folder", true),
            ],
            120,
        );
        assert_eq!(rows[0].find("in the"), rows[1].find("your"));
        assert!(rows[0].starts_with("./my-shop"));
    }

    #[test]
    fn a_long_reason_is_cut_to_the_width_and_a_coming_entry_says_so() {
        console::set_colors_enabled(false);
        let rows = menu_rows(
            &[
                c("a", "go", &"x".repeat(200), true),
                c("b", "zig", "later", false),
            ],
            50,
        );
        assert!(console::measure_text_width(&rows[0]) <= 50, "{}", rows[0]);
        assert!(rows[1].contains("zig (coming)"), "{}", rows[1]);
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
