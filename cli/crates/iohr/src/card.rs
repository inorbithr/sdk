//! The answer card `?` draws at a terminal (PRD 0187): a violet rail with the answer set as
//! short paragraphs, lists and one suggested command, under a header that says it is an AI
//! answer and from which agent, and over a footer that says what was read and that nothing
//! was run (AI Act Art. 50). It goes to stderr, like the rest of `iohr`'s look; stdout keeps
//! the plain answer for a pipe or `??`.
//!
//! [`render`] is pure (the width and the look are arguments) so the layout is tested; the
//! caller decides the look from [`crate::ui::fancy_for`]. Plain, the card is the label line
//! and the answer as they are, with no escape codes. A suggested command is only shown;
//! nothing here runs it.

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Duration;

use console::{Style, measure_text_width};

/// The widest a card is drawn, however wide the terminal.
pub(crate) const WIDTH_MAX: usize = 100;
/// The narrowest: below this the terminal wraps the card anyway.
const WIDTH_MIN: usize = 40;
/// The rail before each body line.
const RAIL: &str = "│  ";
/// The braille spinner, one frame per tick.
const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How the card is drawn: colour or not, and 24-bit colour where the terminal has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) fancy: bool,
    pub(crate) truecolor: bool,
}

impl Look {
    /// The look for stderr: drawn when [`crate::ui::Ui`] would draw, 24-bit when
    /// `COLORTERM` says so.
    #[must_use]
    pub(crate) fn detect(fancy: bool) -> Self {
        let ct = std::env::var("COLORTERM").unwrap_or_default();
        Self {
            fancy,
            truecolor: matches!(ct.as_str(), "truecolor" | "24bit"),
        }
    }

    fn style(self, rgb: (u8, u8, u8), ansi: u8) -> Style {
        let s = Style::new().force_styling(self.fancy);
        if self.truecolor {
            s.true_color(rgb.0, rgb.1, rgb.2)
        } else {
            s.color256(ansi)
        }
    }
    fn violet(self) -> Style {
        self.style((167, 139, 250), 141)
    }
    fn green(self) -> Style {
        self.style((74, 222, 128), 114)
    }
    fn cyan(self) -> Style {
        self.style((125, 211, 252), 117)
    }
    fn dim(self) -> Style {
        self.style((139, 144, 163), 246)
    }
    fn amber(self) -> Style {
        self.style((251, 191, 36), 214)
    }
    fn badge(self) -> Style {
        let s = Style::new().force_styling(self.fancy).black().bold();
        if self.truecolor {
            s.on_true_color(167, 139, 250)
        } else {
            s.on_color256(141)
        }
    }
    fn plain(self) -> Style {
        Style::new().force_styling(self.fancy)
    }
}

/// What a card says.
#[derive(Debug, Clone)]
pub(crate) enum Body<'a> {
    /// The agent's answer, as it wrote it.
    Answer(&'a str),
    /// Why there is no answer (never the question or the agent's own text).
    Failed(&'a str),
}

/// One card.
#[derive(Debug, Clone)]
pub(crate) struct Card<'a> {
    /// The agent, as people read it: "Claude Code (local)".
    pub(crate) agent: &'a str,
    pub(crate) model: &'a str,
    pub(crate) elapsed: Option<Duration>,
    /// The earlier question this one follows up, if any.
    pub(crate) follows: Option<&'a str>,
    /// What was read and that nothing was run ([`crate::headless::read_line`]).
    pub(crate) read: &'a str,
    /// The disclosure sentences that close the card.
    pub(crate) notice: &'a str,
    /// The one-line label printed instead of a card when the look is plain.
    pub(crate) label: &'a str,
    /// A hint at the end of the footer, such as "`??` follows up".
    pub(crate) hint: Option<&'a str>,
    pub(crate) body: Body<'a>,
}

/// The width to draw at for a terminal `cols` wide (unknown: 80): one column less than the
/// terminal, so no line ever fills the last column (a terminal that wraps eagerly, or one
/// that counts a glyph wider than we do, would break it and drop the rail).
#[must_use]
pub(crate) fn width_for(cols: Option<usize>) -> usize {
    cols.map_or(80, |c| c.saturating_sub(1))
        .clamp(WIDTH_MIN, WIDTH_MAX)
}

/// The width of stderr's terminal, as [`width_for`] draws it.
#[must_use]
pub(crate) fn stderr_width() -> usize {
    width_for(
        console::Term::stderr()
            .size_checked()
            .map(|(_, c)| usize::from(c)),
    )
}

/// The card as text, every line ending in a newline.
#[must_use]
pub(crate) fn render(card: &Card<'_>, width: usize, look: Look) -> String {
    if !look.fancy {
        return plain(card);
    }
    let width = width.max(WIDTH_MIN);
    let mut out = String::new();
    header(&mut out, card, width, look);
    let rail = look.violet().apply_to(RAIL).to_string();
    let rail_end = look.violet().apply_to("│").to_string();
    let _ = writeln!(out, "{rail_end}");
    if let Some(prior) = card.follows {
        let q = console::truncate_str(prior, width.saturating_sub(RAIL.len() + 20), "…");
        let _ = writeln!(
            out,
            "{rail}{}",
            look.dim().apply_to(format!("↳ following up “{q}”"))
        );
        let _ = writeln!(out, "{rail_end}");
    }
    let inner = width - measure_text_width(RAIL);
    let lines = match card.body {
        Body::Answer(text) => body(text, inner, look),
        Body::Failed(why) => failed(why, inner, look),
    };
    for l in lines {
        if l.is_empty() {
            let _ = writeln!(out, "{rail_end}");
        } else {
            let _ = writeln!(out, "{rail}{l}");
        }
    }
    let _ = writeln!(out, "{rail_end}");
    footer(&mut out, card, width, look);
    out
}

/// Plain: the label, then the answer (or the reason), exactly as written.
fn plain(card: &Card<'_>) -> String {
    let mut out = format!("{} {}\n", card.label, card.read);
    match card.body {
        Body::Answer(text) => {
            out.push_str(text.trim_end());
            out.push('\n');
        }
        Body::Failed(why) => {
            let _ = writeln!(out, "no answer: {why}");
        }
    }
    out
}

fn header(out: &mut String, card: &Card<'_>, width: usize, look: Look) {
    let badge = if matches!(card.body, Body::Failed(_)) {
        " no answer "
    } else {
        " AI answer "
    };
    let mut meta = vec![card.agent.to_owned(), card.model.to_owned()];
    if let Some(d) = card.elapsed {
        meta.push(format!("{:.1}s", d.as_secs_f64()));
    }
    let mut meta = meta.join(" · ");
    // "╭─ ◆ Io " + badge + " " + filler + " " + meta
    let fixed = measure_text_width("╭─ ◆ Io ") + measure_text_width(badge) + 2;
    let room = width.saturating_sub(fixed + 2);
    if measure_text_width(&meta) > room {
        meta = console::truncate_str(&meta, room, "…").into_owned();
    }
    let fill = width
        .saturating_sub(fixed + measure_text_width(&meta))
        .max(2);
    let badge = if matches!(card.body, Body::Failed(_)) {
        look.amber().bold().reverse().apply_to(badge).to_string()
    } else {
        look.badge().apply_to(badge).to_string()
    };
    let _ = writeln!(
        out,
        "{}{}{badge} {} {}",
        look.violet().apply_to("╭─ "),
        look.violet().bold().apply_to("◆ Io "),
        look.violet().apply_to("─".repeat(fill)),
        look.dim().apply_to(meta),
    );
}

fn footer(out: &mut String, card: &Card<'_>, width: usize, look: Look) {
    let text = format!("{} {}", card.read, card.notice);
    let inner = width - 3;
    let mut lines = wrap(&spans(&text), inner, look, Kind::Dim);
    // The hint is a line of its own, so it is never cut in half by the notice's wrap.
    if let Some(h) = card.hint {
        lines.extend(wrap(&spans(h), inner, look, Kind::Dim));
    }
    for (i, l) in lines.iter().enumerate() {
        let lead = if i == 0 {
            look.violet().apply_to("╰─ ").to_string()
        } else {
            "   ".to_owned()
        };
        let _ = writeln!(out, "{lead}{l}");
    }
}

fn failed(why: &str, inner: usize, look: Look) -> Vec<String> {
    let mut lines = wrap(&spans(why), inner, look, Kind::Amber);
    lines.push(String::new());
    lines.extend(wrap(
        &spans("Nothing was run. Try again, or `iohr ask --interactive` to open the agent."),
        inner,
        look,
        Kind::Text,
    ));
    lines
}

/// A block of the answer, in the order written.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Para(String),
    /// A list item: its mark ("•", "1.") and its text.
    Item(String, String),
    Head(String),
    /// A suggested command, one line, shown and never run.
    Command(String),
    /// Code that is not a command, kept as written.
    Code(Vec<String>),
}

/// Shell fences: what is in them is a command to show, not code to read.
fn is_shell(lang: &str) -> bool {
    matches!(
        lang,
        "" | "sh" | "bash" | "zsh" | "shell" | "console" | "fish"
    )
}

/// "- x", "* x", "• x" → ("•", "x"); "1. x", "2) x" → ("1.", "x").
fn item(line: &str) -> Option<(String, String)> {
    for m in ["- ", "* ", "• "] {
        if let Some(rest) = line.strip_prefix(m) {
            return Some(("•".to_owned(), rest.trim().to_owned()));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let rest = &line[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((format!("{}.", &line[..digits]), r.trim().to_owned()));
        }
    }
    None
}

fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut cur: Option<Block> = None;
    let mut fence: Option<(String, Vec<String>)> = None;
    let flush = |cur: &mut Option<Block>, out: &mut Vec<Block>| {
        if let Some(b) = cur.take() {
            out.push(b);
        }
    };
    let close = |lang: &str, body: Vec<String>, out: &mut Vec<Block>| {
        if is_shell(lang) {
            for l in body {
                let l = l.trim();
                let l = l.strip_prefix("$ ").unwrap_or(l).trim();
                if !l.is_empty() {
                    out.push(Block::Command(l.to_owned()));
                }
            }
        } else if body.iter().any(|l| !l.trim().is_empty()) {
            out.push(Block::Code(body));
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("```") {
            if let Some((lang, body)) = fence.take() {
                close(&lang, body, &mut out);
            } else {
                flush(&mut cur, &mut out);
                fence = Some((rest.trim().to_ascii_lowercase(), Vec::new()));
            }
            continue;
        }
        if let Some((_, body)) = fence.as_mut() {
            body.push(raw.trim_end().to_owned());
            continue;
        }
        if line.is_empty() {
            flush(&mut cur, &mut out);
        } else if line.starts_with('#') {
            flush(&mut cur, &mut out);
            out.push(Block::Head(line.trim_start_matches('#').trim().to_owned()));
        } else if let Some((mark, rest)) = item(line) {
            flush(&mut cur, &mut out);
            cur = Some(Block::Item(mark, rest));
        } else {
            match cur.as_mut() {
                Some(Block::Para(p) | Block::Item(_, p)) => {
                    p.push(' ');
                    p.push_str(line);
                }
                _ => cur = Some(Block::Para(line.to_owned())),
            }
        }
    }
    flush(&mut cur, &mut out);
    if let Some((lang, body)) = fence {
        close(&lang, body, &mut out);
    }
    out
}

/// How a piece of text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Bold,
    Code,
    Dim,
    Amber,
}

/// Inline `code` and **bold**, the marks dropped; everything else is text.
fn spans(text: &str) -> Vec<(Kind, String)> {
    let mut out: Vec<(Kind, String)> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let tick = rest.find('`');
        let star = rest.find("**");
        let (at, mark, kind) = match (tick, star) {
            (Some(t), Some(s)) if s < t => (s, "**", Kind::Bold),
            (Some(t), _) => (t, "`", Kind::Code),
            (None, Some(s)) => (s, "**", Kind::Bold),
            (None, None) => {
                out.push((Kind::Text, rest.to_owned()));
                break;
            }
        };
        let after = &rest[at + mark.len()..];
        if let Some(end) = after.find(mark).filter(|&e| e > 0) {
            if at > 0 {
                out.push((Kind::Text, rest[..at].to_owned()));
            }
            out.push((kind, after[..end].to_owned()));
            rest = &after[end + mark.len()..];
        } else {
            // An unclosed mark is text.
            out.push((Kind::Text, rest[..at + mark.len()].to_owned()));
            rest = after;
        }
    }
    out
}

fn paint(kind: Kind, s: &str, look: Look) -> String {
    match kind {
        Kind::Text => look.plain().apply_to(s).to_string(),
        Kind::Bold => look.plain().bold().apply_to(s).to_string(),
        Kind::Code => look.cyan().apply_to(s).to_string(),
        Kind::Dim => look.dim().apply_to(s).to_string(),
        Kind::Amber => look.amber().apply_to(s).to_string(),
    }
}

/// A word: the pieces of each kind it is made of (`` `x`, `` is code then text).
type Word = Vec<(Kind, String)>;

fn words(spans: &[(Kind, String)], base: Kind) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut cur: Word = Vec::new();
    for (kind, text) in spans {
        let kind = if *kind == Kind::Text { base } else { *kind };
        let mut piece = String::new();
        for c in text.chars() {
            if c.is_whitespace() {
                if !piece.is_empty() {
                    cur.push((kind, std::mem::take(&mut piece)));
                }
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            } else {
                piece.push(c);
            }
        }
        if !piece.is_empty() {
            cur.push((kind, piece));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn word_width(w: &Word) -> usize {
    w.iter().map(|(_, s)| measure_text_width(s)).sum()
}

/// A word wider than a whole line, cut into pieces that fit.
fn split_long(w: Word, width: usize) -> Vec<Word> {
    let mut out = Vec::new();
    let mut cur: Word = Vec::new();
    let mut used = 0;
    for (kind, s) in w {
        let mut piece = String::new();
        for c in s.chars() {
            let cw = measure_text_width(c.encode_utf8(&mut [0; 4]));
            if used + cw > width && used > 0 {
                if !piece.is_empty() {
                    cur.push((kind, std::mem::take(&mut piece)));
                }
                out.push(std::mem::take(&mut cur));
                used = 0;
            }
            piece.push(c);
            used += cw;
        }
        if !piece.is_empty() {
            cur.push((kind, piece));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Word-wrapped lines, each at most `width` columns wide.
fn wrap(spans: &[(Kind, String)], width: usize, look: Look, base: Kind) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Word>> = vec![Vec::new()];
    let mut used = 0;
    for w in words(spans, base) {
        let ww = word_width(&w);
        let pieces = if ww > width {
            split_long(w, width)
        } else {
            vec![w]
        };
        for p in pieces {
            let pw = word_width(&p);
            let need = if used == 0 { pw } else { used + 1 + pw };
            if need > width && used > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            used = if used == 0 { pw } else { used + 1 + pw };
            if let Some(last) = lines.last_mut() {
                last.push(p);
            }
        }
    }
    lines
        .into_iter()
        .filter(|l| !l.is_empty())
        .map(|l| {
            l.iter()
                .map(|w| {
                    w.iter()
                        .map(|(k, s)| paint(*k, s, look))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// The answer's lines (without the rail), a blank line between blocks.
fn body(text: &str, width: usize, look: Look) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<&Block> = None;
    let all = blocks(text);
    for b in &all {
        // Items of one list and commands of one fence sit together; other blocks get air.
        let together = matches!(
            (last, b),
            (Some(Block::Item(..)), Block::Item(..))
                | (Some(Block::Command(_)), Block::Command(_))
                | (Some(Block::Head(_)), _)
        );
        if last.is_some() && !together {
            out.push(String::new());
        }
        match b {
            Block::Para(p) => out.extend(wrap(&spans(p), width, look, Kind::Text)),
            Block::Head(h) => out.extend(wrap(&spans(h), width, look, Kind::Bold)),
            Block::Item(mark, t) => {
                let lead = measure_text_width(mark) + 1;
                let lines = wrap(&spans(t), width.saturating_sub(lead), look, Kind::Text);
                for (i, l) in lines.into_iter().enumerate() {
                    if i == 0 {
                        out.push(format!("{} {l}", look.violet().apply_to(mark)));
                    } else {
                        out.push(format!("{}{l}", " ".repeat(lead)));
                    }
                }
            }
            Block::Command(c) => {
                // Never wrapped: it has to stay one line a person can copy.
                out.push(format!(
                    "{} {}",
                    look.green().bold().apply_to("❯"),
                    look.green().bold().apply_to(c)
                ));
            }
            Block::Code(lines) => {
                for l in lines {
                    out.push(format!("  {}", look.dim().apply_to(l)));
                }
            }
        }
        last = Some(b);
    }
    if out.is_empty() {
        out.push(look.dim().apply_to("(the agent gave no text)").to_string());
    }
    out
}

/// One spinner frame on stderr, over the line before it.
pub(crate) fn spin(frame: usize, elapsed: Duration, agent: &str, look: Look) {
    let f = FRAMES[frame % FRAMES.len()];
    let _ = write!(
        std::io::stderr().lock(),
        "\r\x1b[2K{} {} {}",
        look.violet().bold().apply_to(f),
        look.violet().apply_to("Io is thinking"),
        look.dim()
            .apply_to(format!("{:.1}s · {agent}", elapsed.as_secs_f64())),
    );
    let _ = std::io::stderr().flush();
}

/// Writes a rendered card to stderr.
pub(crate) fn draw(card: &str) {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(card.as_bytes()).and_then(|()| err.flush());
}

/// Clears the spinner's line.
pub(crate) fn spin_clear() {
    let _ = write!(std::io::stderr().lock(), "\r\x1b[2K");
    let _ = std::io::stderr().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    const FANCY: Look = Look {
        fancy: true,
        truecolor: false,
    };
    const PLAIN: Look = Look {
        fancy: false,
        truecolor: false,
    };

    fn card(text: &str) -> Card<'_> {
        Card {
            agent: "Claude Code (local)",
            model: "haiku",
            elapsed: Some(Duration::from_millis(1840)),
            follows: None,
            read: "Read: your question, the folder's path; nothing was run.",
            notice: "An AI model under your own subscription. Check it before you rely on it.",
            label: "Io · AI answer -- Io running on Claude Code (local).",
            hint: Some("`??` follows up."),
            body: Body::Answer(text),
        }
    }

    fn strip(s: &str) -> String {
        console::strip_ansi_codes(s).into_owned()
    }

    #[test]
    fn the_answer_is_set_as_paragraphs_lists_and_one_command() {
        let text = "The helper sets `HOME` only. On Windows the home comes from\n\
                    **USERPROFILE**, so the folder lands outside.\n\n\
                    - set both\n- like tests/ext.rs\n\n\
                    ```sh\n$ cargo nextest run -p iohr --test project\n```";
        let out = strip(&render(&card(text), 60, FANCY));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("╭─ ◆ Io  AI answer  ─"), "{out}");
        assert!(
            lines[0].ends_with("Claude Code (local) · haiku · 1.8s"),
            "{out}"
        );
        assert_eq!(measure_text_width(lines[0]), 60, "{out}");
        assert!(out.contains("│  The helper sets HOME only."), "{out}");
        assert!(!out.contains('`'), "{out}");
        assert!(!out.contains("**"), "{out}");
        assert!(
            out.contains("│  • set both\n│  • like tests/ext.rs\n"),
            "{out}"
        );
        assert!(
            out.contains("│  ❯ cargo nextest run -p iohr --test project\n"),
            "{out}"
        );
        assert!(
            lines.last().is_some_and(|l| l.contains("follows up")),
            "{out}"
        );
        assert!(out.contains("╰─ Read: your question"), "{out}");
        for l in &lines[1..] {
            assert!(measure_text_width(l) <= 60, "too wide: {l:?}");
        }
    }

    #[test]
    fn a_long_command_is_never_wrapped_and_a_long_word_is_cut() {
        let cmd = format!("echo {}", "x".repeat(120));
        let text = format!("see {}\n\n```bash\n{cmd}\n```", "y".repeat(90));
        let out = strip(&render(&card(&text), 50, FANCY));
        assert!(out.contains(&format!("❯ {cmd}\n")), "{out}");
        let ys: usize = out
            .lines()
            .filter(|l| l.contains("yyy"))
            .map(|l| l.matches('y').count())
            .sum();
        assert_eq!(ys, 90, "{out}");
        for l in out.lines().filter(|l| !l.contains('❯')) {
            assert!(measure_text_width(l) <= 50, "too wide: {l:?}");
        }
    }

    #[test]
    fn other_code_stays_as_written_and_unclosed_marks_are_text() {
        let out = strip(&render(
            &card("a `b and **c\n\n```rust\nfn main() {}\n```"),
            60,
            FANCY,
        ));
        assert!(out.contains("a `b and **c"), "{out}");
        assert!(out.contains("│    fn main() {}"), "{out}");
        assert!(!out.contains('❯'), "{out}");
    }

    #[test]
    fn plain_is_the_label_and_the_answer_with_no_escape_codes() {
        let out = render(&card("Hi. What are you working on?"), 60, PLAIN);
        assert_eq!(
            out,
            "Io · AI answer -- Io running on Claude Code (local). Read: your question, the folder's path; nothing was run.\nHi. What are you working on?\n"
        );
        let mut c = card("");
        c.body = Body::Failed("`claude` is not signed in");
        assert!(render(&c, 60, PLAIN).ends_with("no answer: `claude` is not signed in\n"));
    }

    #[test]
    fn a_failure_says_no_answer_and_that_nothing_was_run() {
        let mut c = card("");
        c.body = Body::Failed("`claude` is not signed in");
        let out = strip(&render(&c, 60, FANCY));
        assert!(out.starts_with("╭─ ◆ Io  no answer  ─"), "{out}");
        assert!(out.contains("│  claude is not signed in"), "{out}");
        assert!(out.contains("Nothing was run."), "{out}");
    }

    #[test]
    fn a_follow_up_names_the_earlier_question() {
        let mut c = card("Yes.");
        c.follows = Some("why did that fail");
        let out = strip(&render(&c, 60, FANCY));
        assert!(
            out.contains("│  ↳ following up “why did that fail”"),
            "{out}"
        );
    }

    #[test]
    fn the_width_follows_the_terminal_within_bounds() {
        assert_eq!(width_for(None), 80);
        assert_eq!(width_for(Some(300)), WIDTH_MAX);
        assert_eq!(width_for(Some(10)), WIDTH_MIN);
        assert_eq!(width_for(Some(72)), 71);
    }

    /// A line as wide as the terminal leaves the cursor in the pending-wrap state, and a
    /// terminal that wraps eagerly (or counts one glyph wider than we do) then breaks the
    /// line: the rest lands on a line of its own with no rail. So the card never draws a
    /// line as wide as the terminal, whatever its width and whatever the text.
    #[test]
    fn no_line_is_as_wide_as_the_terminal_and_every_body_line_has_its_rail() {
        // Words sized so that, at some terminal width, a body line fills the inner width
        // exactly (the case behind the blank, rail-less line in a 100-column card).
        let filler = |n: usize| "x".repeat(n);
        for cols in [40usize, 41, 60, 79, 80, 99, 100, 101, 120, 300] {
            let width = width_for(Some(cols));
            let inner = width - measure_text_width(RAIL);
            for first in 1..inner {
                let rest = inner - first - 1;
                if rest == 0 {
                    continue;
                }
                let text = format!(
                    "{} {} tail of the paragraph, so a project, or an API.",
                    filler(first),
                    filler(rest)
                );
                let out = strip(&render(&card(&text), width, FANCY));
                for l in out.lines() {
                    assert!(
                        measure_text_width(l) < cols.max(WIDTH_MIN + 1),
                        "{cols} columns, line of {}: {l}",
                        measure_text_width(l)
                    );
                }
                let body: Vec<&str> = out
                    .lines()
                    .skip(1)
                    .take_while(|l| !l.starts_with('╰'))
                    .collect();
                assert!(
                    body.iter().all(|l| l.starts_with('│')),
                    "{cols} columns, first {first}: {out}"
                );
            }
        }
    }
}
