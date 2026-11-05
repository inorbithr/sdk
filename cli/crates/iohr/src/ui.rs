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

/// The loop of [`Ui::select_keyed`], with the drawing and the key source passed in so it is
/// tested without a terminal. `draw` paints the menu with entry `at` highlighted, `clear`
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
        use console::{Term, style};
        if !prompt::interactive() || choices.is_empty() {
            return Err(Error::Usage(format!(
                "{prompt}: there is no terminal to ask on"
            )));
        }
        let term = Term::stderr();
        let mut last = Instant::now();
        run_keyed(
            choices,
            initial,
            |at| {
                term.write_line(&style(prompt).bold().to_string())?;
                for (i, c) in choices.iter().enumerate() {
                    let key = c.key.map_or_else(String::new, |k| format!("{k}"));
                    let line = format!(
                        "{} {} {}  {}",
                        if i == at { "\u{203a}" } else { " " },
                        style(key).cyan(),
                        c.label,
                        style(&c.hint).dim()
                    );
                    term.write_line(&line)?;
                }
                Ok(())
            },
            || term.clear_last_lines(choices.len() + 1),
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
    use std::cell::{Cell, RefCell};
    use std::io;

    use console::Key;

    use super::{Choice, fancy_for, run_keyed};

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
        let drawn = RefCell::new(Vec::new());
        let reads = Cell::new(0);
        let script = RefCell::new(script.into_iter());
        let out = run_keyed(
            &choices(),
            "run",
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
}
