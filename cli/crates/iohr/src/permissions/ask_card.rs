//! The permission ask as an Io card (PRD 0197, PRD 0187.1): the answer card's frame (`╭─ ◆ Io`
//! header with the AI badge, violet rail) around what Io wants to do, and the numbered action
//! bar of the `?` menu under it. Drawn only where the look is fancy and the terminal is wide
//! enough ([`drawn_at`]); everywhere else the ask stays the one plain line of
//! [`super::broker::card_line`], which scripts and the tests read.
//!
//! [`template`] fills the one ask template ([`crate::ask_template`]) with this ask's words and
//! rows; nothing here draws a box or reads keys of its own. [`lines_with`] is pure (the width and
//! the look are arguments) so the layout is tested.

use console::{measure_text_width, truncate_str};

use super::broker::{Choice, Wanted};
use crate::ask_template::{IoHeader, Template};
use crate::card::Look;
use crate::ui;

/// The most lines the command takes on the card before it is cut with a "more" line.
const COMMAND_LINES: usize = 3;
/// What the command is indented by, under "Io (AI) wants to run".
const INDENT: &str = "  ";
/// The values of the rows, as [`ui::Choice`] carries them.
const ONCE: &str = "once";
const ALWAYS: &str = "always";
const NO: &str = "no";

/// What the card shows.
#[derive(Debug, Clone)]
pub(crate) struct Ask {
    pub(crate) wanted: Wanted,
    /// The folder's own name (clean: [`super::broker`] strips control characters).
    pub(crate) folder: String,
    /// Whether a rule can be saved for it, which adds the "Always" row.
    pub(crate) can_always: bool,
}

/// Whether the card is drawn on a terminal `cols` wide with this look, or the plain line is.
#[must_use]
pub(crate) fn drawn_at(cols: usize, look: Look) -> bool {
    ui::bar_boxed(cols, look)
}

fn row(value: &str, label: &str, hint: &str, key: char) -> ui::Choice {
    ui::Choice {
        value: value.to_owned(),
        label: label.to_owned(),
        hint: hint.to_owned(),
        available: true,
        key: Some(key),
    }
}

/// The rows: Allow once, Always in this project (only when a rule can be saved), No. The
/// letters are the old keys, so `y`, `a` and `n` still answer.
pub(crate) fn choices(can_always: bool) -> Vec<ui::Choice> {
    let mut out = vec![row(ONCE, "Allow once", "this time only", 'y')];
    if can_always {
        out.push(row(
            ALWAYS,
            "Always in this project",
            "saves a rule here",
            'a',
        ));
    }
    out.push(row(NO, "No", "Io carries on without it", 'n'));
    out
}

/// The rows of the folder-trust ask ([`super::trust`]): the same keys and answers as a
/// command's, with "always" meaning this folder is remembered as trusted.
pub(crate) fn trust_choices() -> Vec<ui::Choice> {
    vec![
        row(ONCE, "Allow once", "this time only", 'y'),
        row(ALWAYS, "Always trust this folder", "remembers it", 'a'),
        row(NO, "No", "Io answers without reading files", 'n'),
    ]
}

/// The row the bar starts on: No, so a stray Enter never allows anything.
pub(crate) const INITIAL: &str = NO;

/// The answer for the value of the row picked.
pub(crate) fn choice_for(value: &str) -> Choice {
    match value {
        ONCE => Choice::Once,
        ALWAYS => Choice::Always,
        _ => Choice::No,
    }
}

/// `text` broken into lines of at most `width` columns, at most `max` of them; when more is
/// left, the last line ends in "…". Cut by display width, not at spaces: a command is shown as
/// it is. With the lines, how many characters of `text` they leave out.
fn wrap_code(text: &str, width: usize, max: usize) -> (Vec<String>, usize) {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = measure_text_width(ch.encode_utf8(&mut [0; 4]));
        if used + w > width {
            lines.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(ch);
        used += w;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    let mut hidden = 0;
    if lines.len() > max {
        hidden = lines[max..].iter().map(|l| l.chars().count()).sum();
        lines.truncate(max);
        if let Some(last) = lines.last_mut() {
            // The "…" takes the room of the last character, which is hidden too.
            let kept: String = last
                .chars()
                .take(last.chars().count().saturating_sub(1))
                .collect();
            hidden += 1;
            *last = kept + "\u{2026}";
        }
    }
    (lines, hidden)
}

/// The card as the screen lines with row `at` highlighted: always the same count for every
/// `at`, none as wide as the terminal. `cols` is the terminal's width (0: unknown). The live
/// ask draws through [`lines_with`] with the rows it shows; this is the command ask's rows.
#[cfg(test)]
#[must_use]
pub(crate) fn lines(ask: &Ask, at: usize, cols: usize, look: Look) -> Vec<String> {
    lines_with(ask, &choices(ask.can_always), at, cols, look)
}

/// The ask template ([`ask_template`]) for this ask with these rows: a command's
/// ([`choices`]) or the folder-trust ask's ([`trust_choices`]). The body is built for a
/// terminal `cols` wide.
#[must_use]
pub(crate) fn template<'a>(
    ask: &Ask,
    rows: &'a [ui::Choice],
    cols: usize,
    look: Look,
) -> Template<'a> {
    let width = ui::bar_width(cols);
    let inner = width.saturating_sub(3);
    let mut body = Vec::new();
    let said = truncate_str(
        &format!("Io (AI) wants to {}", ask.wanted.verb),
        inner,
        "\u{2026}",
    )
    .into_owned();
    body.push(look.plain().bold().apply_to(said).to_string());
    if let Some(thing) = &ask.wanted.thing {
        let code = look.cyan();
        let (shown, hidden) = wrap_code(thing, inner.saturating_sub(INDENT.len()), COMMAND_LINES);
        for l in shown {
            body.push(format!("{INDENT}{}", code.apply_to(l)));
        }
        // What the card does not show is said, so a long command is never taken for a short one.
        let more = ask.wanted.more + hidden;
        if more > 0 {
            body.push(format!(
                "{INDENT}{}",
                look.dim()
                    .apply_to(format!("({more} more characters not shown)"))
            ));
        }
    }
    let place = truncate_str(&format!("in {}", ask.folder), inner, "\u{2026}").into_owned();
    body.push(look.dim().apply_to(place).to_string());
    Template {
        title: "Io (AI) asks",
        io: Some(IoHeader {
            badge: " AI request ",
            meta: "permission",
        }),
        body,
        rows,
    }
}

/// The card with these rows as screen lines (the layout the tests read); the live ask draws the
/// same [`template`] through [`ask_template::pick`].
#[cfg(test)]
#[must_use]
pub(crate) fn lines_with(
    ask: &Ask,
    rows: &[ui::Choice],
    at: usize,
    cols: usize,
    look: Look,
) -> Vec<String> {
    crate::ask_template::lines(&template(ask, rows, cols, look), at, cols, SCREEN, look)
}

/// The rows of a screen the card may take: a permission ask has three rows, so any height does.
#[cfg(test)]
const SCREEN: usize = 100;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    const FANCY: Look = Look {
        fancy: true,
        truecolor: false,
    };

    fn strip(s: &str) -> String {
        console::strip_ansi_codes(s).into_owned()
    }

    fn ask(command: &str, can_always: bool) -> Ask {
        Ask {
            wanted: Wanted {
                verb: "run".into(),
                thing: Some(command.into()),
                more: 0,
            },
            folder: "adr".into(),
            can_always,
        }
    }

    #[test]
    fn the_card_is_an_io_card_with_the_command_the_folder_and_three_numbered_rows() {
        let l: Vec<String> = lines(&ask("ls -la", true), 2, 80, FANCY)
            .iter()
            .map(|x| strip(x))
            .collect();
        assert!(
            l[0].starts_with("\u{256d}\u{2500} \u{25c6} Io  AI request "),
            "{l:?}"
        );
        assert!(l[0].ends_with(" permission"), "{l:?}");
        assert_eq!(l[1], "\u{2502}  Io (AI) wants to run");
        assert_eq!(l[2], "\u{2502}    ls -la");
        assert_eq!(l[3], "\u{2502}  in adr");
        assert_eq!(l[4], "\u{2502}");
        assert!(
            l[5].contains("[1] Allow once") && l[5].contains(" y "),
            "{l:?}"
        );
        assert!(l[6].contains("[2] Always in this project"), "{l:?}");
        assert!(
            l[7].starts_with("\u{2502}  \u{203a} [3] No"),
            "No is where it starts: {l:?}"
        );
        assert!(
            l[8].starts_with("\u{2570}\u{2500} \u{2191}\u{2193} move"),
            "{l:?}"
        );
        assert_eq!(l.len(), 9);
        println!("{}", l.join("\n"));
    }

    #[test]
    fn the_always_row_is_there_only_when_a_rule_can_be_saved_and_enter_starts_on_no() {
        let c = choices(false);
        assert_eq!(
            c.iter().map(|x| x.key.unwrap()).collect::<Vec<_>>(),
            ['y', 'n']
        );
        let c = choices(true);
        assert_eq!(
            c.iter().map(|x| x.key.unwrap()).collect::<Vec<_>>(),
            ['y', 'a', 'n']
        );
        assert!(c.iter().any(|x| x.value == INITIAL && x.label == "No"));
        assert_eq!(choice_for(INITIAL), Choice::No);
        assert_eq!(choice_for("once"), Choice::Once);
        assert_eq!(choice_for("always"), Choice::Always);
        let l = lines(&ask("ls", false), 1, 80, FANCY);
        assert!(!l.iter().any(|x| strip(x).contains("Always")));
    }

    #[test]
    fn at_widths_40_80_and_120_every_line_fits_and_the_height_never_changes() {
        let long = "cargo build --release --workspace --all-features && ".repeat(8);
        for cols in [41, 80, 120] {
            for command in ["ls -la", long.as_str()] {
                let a = ask(command, true);
                let h = lines(&a, 0, cols, FANCY).len();
                for at in 0..3 {
                    let l = lines(&a, at, cols, FANCY);
                    assert_eq!(l.len(), h, "{cols} {at}");
                    for x in &l {
                        assert!(measure_text_width(x) < cols, "{cols}: {}", strip(x));
                    }
                }
                let plain: Vec<String> =
                    lines(&a, 2, cols, FANCY).iter().map(|x| strip(x)).collect();
                println!("--- {cols} cols ---\n{}", plain.join("\n"));
            }
        }
        let a = ask(&long, true);
        let l: Vec<String> = lines(&a, 2, 80, FANCY).iter().map(|x| strip(x)).collect();
        assert!(
            l.iter().any(|x| x.ends_with('\u{2026}')),
            "a long command is cut: {l:?}"
        );
        assert!(
            l.iter().any(|x| x.contains("more characters not shown")),
            "and says so: {l:?}"
        );
    }

    #[test]
    fn a_command_cannot_put_escape_codes_on_the_card() {
        let w = crate::permissions::broker::wanted_parts(
            "Bash",
            &serde_json::json!({"command": "echo \u{1b}[2J\u{7}\nrm -rf x"}),
        );
        let a = Ask {
            wanted: w,
            folder: "adr".into(),
            can_always: true,
        };
        let plain = Look {
            fancy: false,
            truecolor: false,
        };
        // With no colour at all, any escape on a line came from the command.
        for x in lines(&a, 0, 80, plain) {
            assert!(!x.chars().any(char::is_control), "{x:?}");
        }
        let tool =
            crate::permissions::broker::wanted_parts("Evil\u{1b}[2J", &serde_json::json!({}));
        assert!(!tool.verb.chars().any(char::is_control), "{tool:?}");
    }

    #[test]
    fn the_card_is_drawn_only_when_fancy_and_wide_enough() {
        let plain = Look {
            fancy: false,
            truecolor: false,
        };
        assert!(drawn_at(80, FANCY));
        assert!(drawn_at(0, FANCY));
        assert!(!drawn_at(39, FANCY));
        assert!(!drawn_at(120, plain));
    }
}
