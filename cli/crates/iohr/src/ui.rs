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

fn cancelled(e: &io::Error) -> Error {
    if e.kind() == io::ErrorKind::Interrupted {
        Error::Failed("cancelled".into())
    } else {
        Error::Failed(format!("cannot ask on the terminal: {e}"))
    }
}

fn plain(line: &str) {
    let _ = writeln!(io::stderr().lock(), "{line}");
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
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass the choice as a flag"
            )));
        }
        if choices.iter().any(|c| c.key.is_some()) {
            return Self::select_keyed(prompt, choices, initial)?
                .ok_or_else(|| Error::Failed("cancelled".into()));
        }
        loop {
            let mut menu = cliclack::select::<String>(prompt);
            for c in choices {
                let label = if c.available {
                    c.label.clone()
                } else {
                    console::style(format!("{} (coming)", c.label))
                        .dim()
                        .to_string()
                };
                menu = menu.item(c.value.clone(), label, c.hint.clone());
            }
            let picked = menu
                .initial_value(initial.to_owned())
                .interact()
                .map_err(|e| cancelled(&e))?;
            match choices.iter().find(|c| c.value == picked) {
                Some(c) if c.available => return Ok(picked),
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
        use console::{Key, Term, style};
        if !prompt::interactive() || choices.is_empty() {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on"
            )));
        }
        let term = Term::stderr();
        let mut at = choices.iter().position(|c| c.value == initial).unwrap_or(0);
        let io_err = |e: io::Error| cancelled(&e);
        loop {
            term.write_line(&style(prompt).bold().to_string())
                .map_err(io_err)?;
            for (i, c) in choices.iter().enumerate() {
                let key = c.key.map_or_else(String::new, |k| format!("{k}"));
                let line = format!(
                    "{} {} {}  {}",
                    if i == at { "\u{203a}" } else { " " },
                    style(key).cyan(),
                    c.label,
                    style(&c.hint).dim()
                );
                term.write_line(&line).map_err(io_err)?;
            }
            let key = term.read_key();
            term.clear_last_lines(choices.len() + 1).map_err(io_err)?;
            match key {
                Ok(Key::ArrowDown | Key::Tab) => at = (at + 1) % choices.len(),
                Ok(Key::ArrowUp | Key::BackTab) => at = (at + choices.len() - 1) % choices.len(),
                Ok(Key::Enter) => return Ok(Some(choices[at].value.clone())),
                Ok(Key::Escape | Key::CtrlC) | Err(_) => return Ok(None),
                Ok(Key::Char(k)) => {
                    let k = k.to_ascii_lowercase();
                    if k == 'q' {
                        return Ok(None);
                    }
                    if let Some(c) = choices.iter().find(|c| c.key == Some(k)) {
                        return Ok(Some(c.value.clone()));
                    }
                }
                Ok(_) => {}
            }
        }
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
        check: impl Fn(&str) -> Result<(), String> + 'static,
    ) -> Result<String, Error> {
        if !self.fancy {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on; pass it as an argument"
            )));
        }
        let mut q = cliclack::input(prompt).validate(move |v: &String| check(v.trim()));
        if !default.is_empty() {
            q = q.default_input(default);
        }
        q.interact::<String>()
            .map(|s| s.trim().to_owned())
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

impl Step {
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
    use super::fancy_for;

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
}
