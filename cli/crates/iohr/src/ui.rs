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
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) hint: String,
    pub(crate) available: bool,
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

fn plain(line: &str) {
    let _ = writeln!(io::stderr().lock(), "{line}");
}

/// The width of the terminal on stderr, 80 when it is not known.
fn terminal_width() -> usize {
    usize::from(console::Term::stderr().size().1).max(40)
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
            let _ = cliclack::log::info(line);
        } else {
            plain(line);
        }
    }

    /// A warning line between steps. Plain: the line with `warning:` in front.
    pub(crate) fn warn(self, line: &str) {
        if self.fancy {
            let _ = cliclack::log::warning(line);
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
    }
}

impl Step {
    /// The step is over and leaves nothing on screen.
    pub(crate) fn clear(self) {
        if let Some(b) = self.bar {
            b.clear();
        }
    }

    /// The step worked.
    pub(crate) fn ok(self, said: &str) {
        if let Some(b) = self.bar {
            b.stop(said);
        }
    }

    /// The step did not fully happen, and the reason is short.
    pub(crate) fn warn(self, said: &str) {
        if let Some(b) = self.bar {
            b.clear();
            let _ = cliclack::log::warning(said);
        }
    }

    /// The step failed, with a short reason.
    pub(crate) fn fail(self, said: &str) {
        if let Some(b) = self.bar {
            b.error(said);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, Paint, fancy_for, menu_rows};

    fn c(value: &str, label: &str, hint: &str, available: bool) -> Choice {
        Choice {
            value: value.into(),
            label: label.into(),
            hint: hint.into(),
            available,
        }
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
}
