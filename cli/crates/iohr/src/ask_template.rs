//! The one template of every terminal ask (PRD 0187.1, PRD 0197): a card with a title, body
//! lines, numbered rows with keys, and, when the question takes a typed answer, an input line
//! inside the same card. The permission ask ([`crate::permissions::ask_card`]), Io's own
//! questions ([`crate::askback`]), the action bar under an answer, the asks of `iohr asks` and
//! every wizard step (`init`, `project`: select, confirm, input, tick-list) are drawn here and
//! nowhere else, so an ask looks the same wherever it comes from.
//!
//! The frame is the answer card's: [`crate::card::header_line`] for an Io ask (the `◆ Io`
//! header with the AI badge), the bar's violet title line for a step of ours, then the rail,
//! the rows and the foot of [`crate::ui::bar_lines_in`]. Where the look is plain or the terminal
//! narrow there is no box: the same lines, one per row, numbered all the same. Where there is no
//! terminal at all the caller keeps its plain line (a script reads [`Template::plain_line`]).
//!
//! [`lines`] and [`input_lines`] are pure (the width and the look are arguments), so the layout
//! is tested; [`pick`] and [`type_in`] are the loops that read the keys.

use std::io;

use console::{Key, Term, measure_text_width, truncate_str};

use crate::card::{self, Look};
use crate::error::Error;
use crate::ui::{self, BarTop, Choice};

/// The value of the "type your own" row ([`other_row`]); no real answer is this.
pub(crate) const OTHER: &str = "\u{0}other";

/// Rows the screen keeps for what is not a row: the terminal's own prompt line and some air.
const SCREEN_SPARE: usize = 3;
/// The fewest rows a card shows of a long list.
const ROWS_MIN: usize = 3;
/// What the key reader may fail with in a row before the input gives up (a signal storm).
const MAX_INTERRUPTS: u32 = 1000;

/// The Io header of a card: the AI badge and the word at its right end.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IoHeader<'a> {
    pub(crate) badge: &'a str,
    pub(crate) meta: &'a str,
}

/// What an ask shows.
#[derive(Debug, Clone)]
pub(crate) struct Template<'a> {
    /// The question or the step: the title of a card of ours, the plain line's text, and the
    /// first line wherever there is no box.
    pub(crate) title: &'a str,
    /// Set for a question Io asks: the card then has Io's header and the title is part of the
    /// body the caller gives.
    pub(crate) io: Option<IoHeader<'a>>,
    /// The lines on the rail between the header and the rows, already coloured and fitted to
    /// [`body_width`]; an empty one is a blank rail line.
    pub(crate) body: Vec<String>,
    /// The rows; empty for a question that is only typed.
    pub(crate) rows: &'a [Choice],
}

impl<'a> Template<'a> {
    /// A card of ours: `title` in the frame, no body.
    #[must_use]
    pub(crate) fn new(title: &'a str, rows: &'a [Choice]) -> Self {
        Self {
            title,
            io: None,
            body: Vec::new(),
            rows,
        }
    }

    /// The ask as one plain line for a terminal that cannot ask: `title  [1] a  [2] b`.
    #[must_use]
    pub(crate) fn plain_line(&self) -> String {
        let rows: Vec<String> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, c)| format!("[{}] {}", i + 1, c.label))
            .collect();
        if rows.is_empty() {
            self.title.to_owned()
        } else {
            format!("{}  {}", self.title, rows.join("  "))
        }
    }
}

/// The width the body's lines may take on a terminal `cols` wide (inside the rail).
#[must_use]
pub(crate) fn body_width(cols: usize) -> usize {
    ui::bar_width(cols).saturating_sub(3)
}

/// `text` as body lines for a card: paragraphs, `code` and **bold** drawn as in an answer,
/// wrapped to [`body_width`], plain where `look` is.
#[must_use]
pub(crate) fn body_lines(text: &str, cols: usize, look: Look) -> Vec<String> {
    card::body(text, body_width(cols), look)
}

/// The "type your own" row: `key` picks it at once.
#[must_use]
pub(crate) fn other_row(key: char) -> Choice {
    Choice {
        value: OTHER.to_owned(),
        label: "Other".to_owned(),
        hint: "type your own".to_owned(),
        available: true,
        key: Some(key),
    }
}

/// The columns of the terminal (0: unknown).
pub(crate) fn term_cols() -> usize {
    usize::from(Term::stderr().size().1)
}

/// The rows of the terminal, 24 when unknown.
fn term_rows() -> usize {
    match usize::from(Term::stderr().size().0) {
        0 => 24,
        n => n,
    }
}

/// How many rows of a list fit on a screen `screen` lines high when the rest of the card takes
/// `fixed` lines: never fewer than [`ROWS_MIN`], so a list longer than the screen scrolls
/// inside its card instead of past the top, where erasing it would miss lines.
fn rows_that_fit(screen: usize, fixed: usize) -> usize {
    screen.saturating_sub(fixed + SCREEN_SPARE).max(ROWS_MIN)
}

/// The window of `total` rows, `fit` of them, that keeps row `at` in view: its first row.
fn window_start(at: usize, total: usize, fit: usize) -> usize {
    if total <= fit {
        return 0;
    }
    at.saturating_sub(fit / 2).min(total - fit)
}

/// The Io header of the card, if it has one, and its body with a blank rail line at the end
/// (the air before the rows or the input) unless the caller gave one.
fn top_of(t: &Template<'_>, cols: usize, look: Look) -> (Option<String>, Vec<String>) {
    let header =
        t.io.map(|h| card::header_line(h.badge, false, h.meta, ui::bar_width(cols), look));
    let mut body = t.body.clone();
    if body.last().is_some_and(|l| !l.is_empty()) {
        body.push(String::new());
    }
    (header, body)
}

/// The card as the screen lines with row `at` highlighted: always the same count for every
/// `at`, none as wide as the terminal. `cols` is the terminal's width (0: unknown) and `screen`
/// its height; a list longer than the screen shows the window around `at`. Boxed when
/// [`ui::bar_boxed`] says so, else plain lines.
#[must_use]
pub(crate) fn lines(
    t: &Template<'_>,
    at: usize,
    cols: usize,
    screen: usize,
    look: Look,
) -> Vec<String> {
    let (header, body) = top_of(t, cols, look);
    // The header (or title) and the foot, and the body between.
    let fit = rows_that_fit(screen, body.len() + 2);
    let total = t.rows.len();
    let first = window_start(at, total, fit);
    let end = (first + fit).min(total);
    let top = BarTop {
        header,
        lines: &body,
        window: (total > fit).then_some((first, total)),
    };
    ui::bar_lines_in(&top, t.title, &t.rows[first..end], at - first, cols, look)
}

/// The loop of an ask with rows: the key it returns is the value of the row picked, `None` for
/// Esc, `q` or Ctrl-C. Keys: the arrows, Tab, `j` and `k`, `1`-`9`, a row's own letter, Enter.
///
/// # Errors
///
/// No terminal on stderr, or the terminal cannot be read.
pub(crate) fn pick(t: &Template<'_>, initial: &str) -> Result<Option<String>, Error> {
    let look = Look::detect(ui::Ui::detect(false).fancy());
    let (cols, screen) = (term_cols(), term_rows());
    ui::Ui::select_drawn(t.title, t.rows, initial, true, |at| {
        lines(t, at, cols, screen, look)
    })
}

/// What a typed answer ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Typed {
    /// A line that passed the check.
    Text(String),
    /// `<` alone, where the step may go back.
    Back,
    /// Esc, Ctrl-C.
    Cancel,
}

/// What one key does to the input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Edit {
    Keep,
    Submit,
    Cancel,
}

/// Applies `key` to `typed`: characters append, Backspace removes one, Enter submits, Esc and
/// Ctrl-C cancel. A control character is never typed into the line.
pub(crate) fn edit(typed: &mut String, key: &Key) -> Edit {
    match key {
        Key::Enter => Edit::Submit,
        Key::Escape | Key::CtrlC => Edit::Cancel,
        Key::Backspace => {
            typed.pop();
            Edit::Keep
        }
        Key::Char(c) if !c.is_control() => {
            typed.push(*c);
            Edit::Keep
        }
        _ => Edit::Keep,
    }
}

/// The card of a typed answer with what is typed so far and the check's complaint (blank when
/// none): the same frame as [`lines`], the rail's body, then `› typed` and the complaint line
/// (always there, so the card keeps its height), then the foot. `back` adds `<` to the foot.
#[cfg(test)]
#[must_use]
pub(crate) fn input_lines(
    t: &Template<'_>,
    typed: &str,
    complaint: &str,
    back: bool,
    cols: usize,
    look: Look,
) -> Vec<String> {
    input_card(t, typed, "", complaint, back, cols, look)
}

/// [`input_lines`] with `placeholder`, the dim default Enter keeps while nothing is typed.
fn input_card(
    t: &Template<'_>,
    typed: &str,
    placeholder: &str,
    complaint: &str,
    back: bool,
    cols: usize,
    look: Look,
) -> Vec<String> {
    let boxed = ui::bar_boxed(cols, look);
    let width = ui::bar_width(cols);
    let (header, body) = top_of(t, cols, look);
    let mut out = Vec::new();
    let lead = if boxed { "\u{2502}  " } else { "" };
    let rail = |s: String| {
        if boxed {
            format!("{}{s}", look.violet().apply_to(lead))
        } else {
            s
        }
    };
    if boxed {
        out.push(header.unwrap_or_else(|| {
            let fixed = measure_text_width("\u{256d}\u{2500} ") + measure_text_width(t.title) + 1;
            format!(
                "{}{} {}",
                look.violet().apply_to("\u{256d}\u{2500} "),
                look.violet().bold().apply_to(t.title),
                look.violet()
                    .apply_to("\u{2500}".repeat(width.saturating_sub(fixed).max(2))),
            )
        }));
    } else {
        out.push(look.plain().bold().apply_to(t.title).to_string());
    }
    for l in &body {
        if l.is_empty() && boxed {
            out.push(look.violet().apply_to("\u{2502}").to_string());
        } else if boxed {
            out.push(rail(l.clone()));
        } else {
            out.push(l.clone());
        }
    }
    // "› " and the caret take 3 columns; a longer answer shows its end.
    let room = width.saturating_sub(measure_text_width(lead) + 3).max(1);
    let shown = if measure_text_width(typed) > room {
        let skip = typed.chars().count() - room.min(typed.chars().count()) + 1;
        format!("\u{2026}{}", typed.chars().skip(skip).collect::<String>())
    } else {
        typed.to_owned()
    };
    let ghost = if typed.is_empty() {
        look.dim()
            .apply_to(truncate_str(
                placeholder,
                room.saturating_sub(1),
                "\u{2026}",
            ))
            .to_string()
    } else {
        String::new()
    };
    out.push(rail(format!(
        "{} {}{}{ghost}",
        look.violet().bold().apply_to("\u{203a}"),
        look.plain().apply_to(shown),
        look.plain().reverse().apply_to(" "),
    )));
    let complaint = truncate_str(
        complaint,
        width.saturating_sub(measure_text_width(lead) + 2),
        "\u{2026}",
    );
    if boxed && complaint.is_empty() {
        out.push(look.violet().apply_to("\u{2502}").to_string());
    } else if complaint.is_empty() {
        out.push(String::new());
    } else {
        out.push(rail(format!(
            "{} {}",
            look.amber().apply_to("\u{2717}"),
            look.amber().apply_to(complaint)
        )));
    }
    let foot = if back {
        "enter send \u{b7} < back \u{b7} esc cancel"
    } else {
        "enter send \u{b7} esc cancel"
    };
    let foot = truncate_str(foot, width.saturating_sub(3), "\u{2026}");
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

/// A typed answer inside the card: `default` is shown dim while nothing is typed and an empty
/// line takes it (like the prompts it replaces),
/// `check` says what is wrong with a line (shown on the card, the line kept), and with `back`
/// a lone `<` goes back one step.
///
/// # Errors
///
/// No terminal on stderr, or the terminal cannot be read.
pub(crate) fn type_in(
    t: &Template<'_>,
    default: &str,
    back: bool,
    check: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Typed, Error> {
    let fancy = ui::Ui::detect(false).fancy();
    if !crate::prompt::interactive() {
        return Err(Error::Usage(format!(
            "{}: there is no terminal to ask on",
            t.title
        )));
    }
    let look = Look::detect(fancy);
    let cols = term_cols();
    let term = Term::stderr();
    let mut typed = String::new();
    let mut complaint = String::new();
    let height = input_card(t, &typed, default, &complaint, back, cols, look).len();
    let _ = term.hide_cursor();
    let mut interrupts = 0u32;
    let result = loop {
        let drawn = input_card(t, &typed, default, &complaint, back, cols, look);
        if let Err(e) = drawn.iter().try_for_each(|l| term.write_line(l)) {
            break Err(e);
        }
        let key = loop {
            match term.read_key_raw() {
                Ok(Key::Unknown | Key::UnknownEscSeq(_)) => {}
                Ok(k) => break Ok(k),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    interrupts += 1;
                    if interrupts >= MAX_INTERRUPTS {
                        break Err(e);
                    }
                }
                Err(e) => break Err(e),
            }
        };
        let key = match key {
            Ok(k) => k,
            Err(e) => {
                let _ = term.clear_last_lines(height);
                break Err(e);
            }
        };
        let _ = term.clear_last_lines(height);
        match edit(&mut typed, &key) {
            Edit::Keep => complaint.clear(),
            Edit::Cancel => break Ok(Typed::Cancel),
            Edit::Submit => {
                let line = match typed.trim() {
                    "" => default,
                    l => l,
                };
                if back && line == "<" {
                    break Ok(Typed::Back);
                }
                match check(line) {
                    Ok(()) => break Ok(Typed::Text(line.to_owned())),
                    Err(why) => complaint = why,
                }
            }
        }
    };
    let _ = term.show_cursor();
    result.map_err(|e| {
        if e.kind() == io::ErrorKind::Interrupted {
            ui::cancel()
        } else {
            Error::Failed(format!("cannot ask on the terminal: {e}"))
        }
    })
}

/// What [`ask_or_type`] answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The value of the row picked.
    Row(String),
    /// What the person typed on the "type your own" row.
    Typed(String),
    /// Esc, `q` or Ctrl-C.
    Dismissed,
}

/// A question whose rows include [`other_row`]: a row picks its value; "Other" turns the card
/// into an input line (the same card, the same title and body) and Esc there returns to the
/// rows.
///
/// # Errors
///
/// No terminal on stderr, or the terminal cannot be read.
pub(crate) fn ask_or_type(
    t: &Template<'_>,
    initial: &str,
    check: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Answer, Error> {
    loop {
        match pick(t, initial)? {
            None => return Ok(Answer::Dismissed),
            Some(v) if v == OTHER => match type_in(t, "", false, check)? {
                Typed::Text(s) => return Ok(Answer::Typed(s)),
                Typed::Back | Typed::Cancel => {}
            },
            Some(v) => return Ok(Answer::Row(v)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    const FANCY: Look = Look {
        fancy: true,
        truecolor: false,
    };
    const PLAIN: Look = Look {
        fancy: false,
        truecolor: false,
    };

    fn row(value: &str, label: &str, key: Option<char>) -> Choice {
        Choice {
            value: value.into(),
            label: label.into(),
            hint: String::new(),
            available: true,
            key,
        }
    }

    fn strip(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|l| console::strip_ansi_codes(l).into_owned())
            .collect()
    }

    #[test]
    fn an_io_question_is_inside_io_s_card_with_its_rows_and_an_other_row() {
        let rows = vec![
            row("a", "An HTTP API service", Some('1')),
            row("b", "A background worker", Some('2')),
            other_row('o'),
        ];
        let mut t = Template::new("Io asks", &rows);
        t.io = Some(IoHeader {
            badge: " AI question ",
            meta: "keeps nothing unless you answer",
        });
        t.body = body_lines("**What is this project for?**", 80, FANCY);
        let l = strip(&lines(&t, 0, 80, 40, FANCY));
        assert!(
            l[0].starts_with("\u{256d}\u{2500} \u{25c6} Io  AI question "),
            "{l:?}"
        );
        assert_eq!(l[1], "\u{2502}  What is this project for?");
        assert_eq!(l[2], "\u{2502}");
        assert!(
            l[3].starts_with("\u{2502}  \u{203a} [1] An HTTP API service"),
            "{l:?}"
        );
        assert!(
            l[5].contains("[3] Other") && l[5].contains("type your own"),
            "{l:?}"
        );
        assert!(l.last().unwrap().starts_with("\u{2570}\u{2500} "), "{l:?}");
        assert!(
            !l.iter().any(|x| x.starts_with("\u{203a} 1")),
            "never the bare menu: {l:?}"
        );
    }

    #[test]
    fn the_height_never_changes_with_the_highlight_and_nothing_is_as_wide_as_the_terminal() {
        let rows: Vec<Choice> = (0..30)
            .map(|i| row(&format!("v{i}"), &format!("Row {i}"), None))
            .collect();
        let t = Template::new("Which template?", &rows);
        for cols in [41, 80, 120] {
            for screen in [10, 24, 60] {
                let h = lines(&t, 0, cols, screen, FANCY).len();
                for at in 0..30 {
                    let l = lines(&t, at, cols, screen, FANCY);
                    assert_eq!(l.len(), h, "{cols} {screen} {at}");
                    assert!(
                        h <= screen.max(ROWS_MIN + 5),
                        "fits the screen: {h} {screen}"
                    );
                    assert!(l.iter().all(|x| measure_text_width(x) < cols));
                }
            }
        }
    }

    #[test]
    fn a_list_longer_than_the_screen_scrolls_in_its_card_and_keeps_its_numbers() {
        let rows: Vec<Choice> = (0..30)
            .map(|i| row(&format!("v{i}"), &format!("Row {i}"), None))
            .collect();
        let t = Template::new("Which?", &rows);
        let l = strip(&lines(&t, 20, 80, 12, FANCY));
        assert!(
            l.iter()
                .any(|x| x.contains("[21] Row 20") && x.contains('\u{203a}')),
            "{l:?}"
        );
        assert!(!l.iter().any(|x| x.contains("Row 0")), "{l:?}");
        assert!(l.last().unwrap().contains(" of 30 "), "{l:?}");
        assert!(l.len() <= 12);
    }

    #[test]
    fn plain_or_narrow_the_card_is_lines_with_no_box_and_the_body_stays() {
        let rows = vec![row("a", "One", None), row("b", "Two", None)];
        let mut t = Template::new("Pick one", &rows);
        t.body = vec!["Some context".to_owned()];
        for (look, cols) in [(PLAIN, 80), (FANCY, 39)] {
            let l = strip(&lines(&t, 0, cols, 40, look));
            assert_eq!(l[0], "Pick one");
            assert_eq!(l[1], "Some context");
            assert!(l.iter().any(|x| x.starts_with("\u{203a} [1] One")), "{l:?}");
            assert!(
                l.iter()
                    .all(|x| !x.contains('\u{2502}') && !x.contains('\u{256d}')),
                "{l:?}"
            );
        }
        assert_eq!(t.plain_line(), "Pick one  [1] One  [2] Two");
    }

    #[test]
    fn typing_goes_inside_the_card_with_a_complaint_line_that_keeps_the_height() {
        let rows = [other_row('o')];
        let mut t = Template::new("Io asks", &rows);
        t.io = Some(IoHeader {
            badge: " AI question ",
            meta: "",
        });
        t.body = body_lines("**What is this project for?**", 80, FANCY);
        let ok = strip(&input_lines(&t, "an API", "", false, 80, FANCY));
        let bad = strip(&input_lines(
            &t,
            "",
            "one to 200 characters",
            true,
            80,
            FANCY,
        ));
        assert_eq!(ok.len(), bad.len());
        assert!(ok[0].starts_with("\u{256d}\u{2500} \u{25c6} Io"), "{ok:?}");
        assert!(
            ok.iter()
                .any(|x| x.starts_with("\u{2502}  \u{203a} an API")),
            "{ok:?}"
        );
        assert!(
            bad.iter()
                .any(|x| x.contains("\u{2717} one to 200 characters")),
            "{bad:?}"
        );
        assert!(bad.last().unwrap().contains("< back"), "{bad:?}");
        assert!(
            ok.last()
                .unwrap()
                .starts_with("\u{2570}\u{2500} enter send"),
            "{ok:?}"
        );
        let long = "x".repeat(300);
        for x in input_lines(&t, &long, "", false, 60, FANCY) {
            assert!(measure_text_width(&x) < 60, "{x}");
        }
    }

    #[test]
    fn the_keys_edit_the_line_enter_submits_and_escape_cancels() {
        let mut s = String::new();
        assert_eq!(edit(&mut s, &Key::Char('a')), Edit::Keep);
        assert_eq!(edit(&mut s, &Key::Char('\u{7}')), Edit::Keep);
        assert_eq!(edit(&mut s, &Key::Char('b')), Edit::Keep);
        assert_eq!(s, "ab");
        assert_eq!(edit(&mut s, &Key::Backspace), Edit::Keep);
        assert_eq!(s, "a");
        assert_eq!(edit(&mut s, &Key::Enter), Edit::Submit);
        assert_eq!(edit(&mut s, &Key::Escape), Edit::Cancel);
        assert_eq!(edit(&mut s, &Key::CtrlC), Edit::Cancel);
    }
}
