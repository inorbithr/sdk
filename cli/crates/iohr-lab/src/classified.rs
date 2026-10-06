//! Classified spans: a fact kept in the document for the readers cleared for it and cut
//! out, before anything is rendered, for everyone else. The same grammar and the same
//! cut as the platform's TypeScript (`ui/shared/lab/classified.ts`); the cases in
//! `tests/classified.json` hold both to it.
//!
//! ```text
//! inline:  [[classified:partner reason="the internal port"]]8443[[/classified]]
//! a block: ```classified level=internal reason="the routing table"
//!          ...the original lines...
//!          ```
//! ```
//!
//! The level is a rung of the access ladder, [`AccessLevel`]. A span is revealed to a
//! reader whose clearance is at or above its level; `admin` is an alias of `internal`.
//! [`cut`] replaces every span above the clearance by the withheld markers
//! (`[REDACTED: reason]` inline, a ```` ```redacted ```` block), so every renderer
//! downstream keeps working and never sees the original, and unwraps the spans at or
//! below it. A withheld span keeps its line count, so a finding's `file:line` and a
//! diff's lines still match the source.
//!
//! Never fail open: a malformed, nested, stray or unclosed marker is a problem, and
//! [`cut`] returns an error rather than guess where a span ends. A line ends at `\n`,
//! `\r\n` or a lone `\r`, as `CommonMark` reads it, so a renderer never sees a fence
//! this parse did not; the text keeps its own line endings. Offsets are in bytes;
//! lengths that the grammar limits (a reason, a needle) count UTF-16 code units, as
//! JavaScript does, so both implementations draw the line in the same place.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

/// The access ladder, lowest first. One comparison decides: a higher rung reads every
/// lower one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccessLevel {
    /// Everyone, signed in or not.
    Public,
    /// Readers shown a preview.
    Preview,
    /// Partners (the rooms).
    Partner,
    /// The team.
    Team,
    /// Internal (`admin` on a marker).
    Internal,
}

impl AccessLevel {
    /// Every rung, lowest first.
    pub const ALL: [Self; 5] = [
        Self::Public,
        Self::Preview,
        Self::Partner,
        Self::Team,
        Self::Internal,
    ];

    /// The rung's name as a marker writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Preview => "preview",
            Self::Partner => "partner",
            Self::Team => "team",
            Self::Internal => "internal",
        }
    }

    /// A rung by its name (`public` to `internal`), or `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == name)
    }

    /// A span's level as written on a marker, `admin` resolved to `internal`; `None`
    /// for `public` and anything that is not a rung.
    #[must_use]
    pub fn span_level(written: &str) -> Option<Self> {
        if written == "admin" {
            return Some(Self::Internal);
        }
        Self::from_name(written).filter(|l| *l != Self::Public)
    }
}

impl fmt::Display for AccessLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a reader at `clearance` may read a span at `level`.
#[must_use]
pub fn cleared(level: AccessLevel, clearance: AccessLevel) -> bool {
    level <= clearance
}

/// Inline or a fenced block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    /// `[[classified:LEVEL reason="..."]]...[[/classified]]`
    Inline,
    /// A ```` ```classified level=LEVEL reason="..." ```` fence.
    Block,
}

/// One classified span, by its byte offsets in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedSpan {
    /// Inline or a block.
    pub kind: SpanKind,
    /// Its level, never [`AccessLevel::Public`].
    pub level: AccessLevel,
    /// Public: shown on the bar.
    pub reason: String,
    /// The withheld text: an inline span's content, a block's body lines.
    pub original: String,
    /// Byte offset where the span starts, its marker or fence included.
    pub start: usize,
    /// Byte offset where it ends (a block ends before its closing newline).
    pub end: usize,
    /// The line it opens on, from 1.
    pub line: usize,
    /// The line it closes on, from 1.
    pub end_line: usize,
    /// A block's indentation, kept on what replaces it (a block inside a list item).
    pub indent: String,
}

/// A malformed marker: the line, from 1, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedProblem {
    /// The line, from 1.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

/// Every span in a text and every problem with its markers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classified {
    /// The spans, in the order they open.
    pub spans: Vec<ClassifiedSpan>,
    /// The problems; when there is one, nothing may be cut.
    pub problems: Vec<ClassifiedProblem>,
}

/// Returned by [`cut`] when the markers do not parse: nothing is cut on a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedError {
    /// Every problem found.
    pub problems: Vec<ClassifiedProblem>,
}

impl fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, p) in self.problems.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "line {}: {}", p.line, p.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ClassifiedError {}

/// JavaScript's `\s`: its white space and line terminators, `U+FEFF` in, `U+0085` out.
const WS: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

#[allow(clippy::expect_used)]
fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a pattern of this crate's own compiles")
}

/// Anything that looks like an inline marker, however it is spelled: each must parse.
static MARKER_LIKE: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"\[\[{WS}*/?{WS}*(?i-u:classified)")));
static OPEN: LazyLock<Regex> =
    LazyLock::new(|| re(r#"^\[\[classified:([a-z]+) reason="([^"\n\r]*)"\]\]"#));
const CLOSE: &str = "[[/classified]]";
/// A block's opening fence: up to three spaces, three or more backticks, `classified`.
static BLOCK_OPEN: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^( {{0,3}})(`{{3,}}){WS}*(?i-u:classified)(?-u:\b)([^\n]*)$"
    ))
});
static BLOCK_ATTRS: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r#"^{WS}+level=([a-z]+){WS}+reason="([^"\n\r]*)"{WS}*$"#
    ))
});
static TILDE_OPEN: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"^ {{0,3}}~{{3,}}{WS}*(?i-u:classified)(?-u:\b)")));
static BLOCK_CLOSE: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^ {{0,3}}(`{{3,}}){WS}*$")));

/// The longest reason, in UTF-16 code units.
pub const MAX_REASON: usize = 160;

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn is_js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// JavaScript's `trim()`.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

fn reason_problem(reason: &str) -> Option<String> {
    if js_trim(reason).is_empty() {
        return Some("a classified span needs a reason, the public text shown on the bar".into());
    }
    if js_len(reason) > MAX_REASON {
        return Some(format!("a reason is at most {MAX_REASON} characters"));
    }
    if reason.contains(['[', ']', '`']) {
        return Some("a reason may not contain [, ] or a backtick".into());
    }
    None
}

fn level_problem(written: &str) -> Option<String> {
    if AccessLevel::span_level(written).is_some() {
        return None;
    }
    Some(format!(
        "unknown level `{written}`: one of preview, partner, team, internal (admin is internal)"
    ))
}

enum Open {
    Inline {
        level: AccessLevel,
        reason: String,
        start: usize,
        body_start: usize,
        line: usize,
    },
    Block {
        level: AccessLevel,
        reason: String,
        start: usize,
        body_start: usize,
        line: usize,
        fence: usize,
        indent: String,
    },
}

impl Open {
    const fn line(&self) -> usize {
        match self {
            Self::Inline { line, .. } | Self::Block { line, .. } => *line,
        }
    }
}

fn problem(problems: &mut Vec<ClassifiedProblem>, line: usize, message: impl Into<String>) {
    problems.push(ClassifiedProblem {
        line,
        message: message.into(),
    });
}

/// Every classified span in `text`, and every problem with the markers. Markers are
/// read everywhere, code spans and other fences included: a marker that should stay
/// literal does not belong in a lab document.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn parse_classified(text: &str) -> Classified {
    let mut spans = Vec::new();
    let mut problems = Vec::new();
    let mut open: Option<Open> = None;
    let mut prev_ending = 0usize;
    for (i, (line_start, raw, ending)) in lines(text).into_iter().enumerate() {
        let line_no = i + 1;
        // Where the next line starts: a block's body, when this line opens one.
        let offset = line_start + raw.len() + ending;
        let before = prev_ending;
        prev_ending = ending;

        if let Some(Open::Block {
            level,
            reason,
            start,
            body_start,
            line,
            fence,
            indent,
        }) = &open
        {
            let closes = BLOCK_CLOSE
                .captures(raw)
                .and_then(|c| c.get(1))
                .is_some_and(|m| m.len() >= *fence);
            if closes {
                // The body ends before the line ending that precedes the closing fence.
                let body_end = (*body_start).max(line_start.saturating_sub(before));
                let original = text.get(*body_start..body_end).unwrap_or_default();
                if line_start <= *body_start || js_trim(original).is_empty() {
                    problem(&mut problems, *line, "a classified block is empty");
                } else {
                    spans.push(ClassifiedSpan {
                        kind: SpanKind::Block,
                        level: *level,
                        reason: reason.clone(),
                        original: original.to_owned(),
                        start: *start,
                        end: line_start + raw.len(),
                        line: *line,
                        end_line: line_no,
                        indent: indent.clone(),
                    });
                }
                open = None;
                continue;
            }
            if BLOCK_OPEN.is_match(raw) || TILDE_OPEN.is_match(raw) {
                problem(
                    &mut problems,
                    line_no,
                    "a classified block inside a classified block",
                );
            }
            if MARKER_LIKE.is_match(raw) {
                problem(
                    &mut problems,
                    line_no,
                    "a classified marker inside a classified block",
                );
            }
            continue;
        }

        if let Some(block) = BLOCK_OPEN.captures(raw) {
            if open.is_some() {
                problem(
                    &mut problems,
                    line_no,
                    "a classified block inside an inline classified span",
                );
                continue;
            }
            let rest = block.get(3).map_or("", |m| m.as_str());
            let attrs = BLOCK_ATTRS.captures(rest);
            let (written, reason) = attrs.as_ref().map_or(("", ""), |a| {
                (
                    a.get(1).map_or("", |m| m.as_str()),
                    a.get(2).map_or("", |m| m.as_str()),
                )
            });
            if attrs.is_some() {
                if let Some(p) = level_problem(written) {
                    problem(&mut problems, line_no, p);
                }
                if let Some(p) = reason_problem(reason) {
                    problem(&mut problems, line_no, p);
                }
            } else {
                problem(
                    &mut problems,
                    line_no,
                    "a classified block opens as ```classified level=LEVEL reason=\"REASON\"",
                );
                // Still open it, so its body is never read as public text by the rest
                // of this parse.
            }
            open = Some(Open::Block {
                level: AccessLevel::span_level(written).unwrap_or(AccessLevel::Internal),
                reason: reason.to_owned(),
                start: line_start,
                body_start: offset,
                line: line_no,
                fence: block.get(2).map_or(3, |m| m.len()),
                indent: block.get(1).map_or("", |m| m.as_str()).to_owned(),
            });
            continue;
        }
        if TILDE_OPEN.is_match(raw) {
            problem(
                &mut problems,
                line_no,
                "a classified block is fenced with backticks, not tildes",
            );
            continue;
        }

        let mut pos = 0usize;
        while let Some(m) = MARKER_LIKE.find_at(raw, pos) {
            let at = m.start();
            let tail = &raw[at..];
            if let Some(o) = OPEN.captures(tail) {
                let whole = o.get(0).map_or(0, |x| x.len());
                if open.is_some() {
                    problem(
                        &mut problems,
                        line_no,
                        "a classified span inside a classified span",
                    );
                } else {
                    let written = o.get(1).map_or("", |x| x.as_str());
                    let reason = o.get(2).map_or("", |x| x.as_str());
                    if let Some(p) = level_problem(written) {
                        problem(&mut problems, line_no, p);
                    }
                    if let Some(p) = reason_problem(reason) {
                        problem(&mut problems, line_no, p);
                    }
                    open = Some(Open::Inline {
                        level: AccessLevel::span_level(written).unwrap_or(AccessLevel::Internal),
                        reason: reason.to_owned(),
                        start: line_start + at,
                        body_start: line_start + at + whole,
                        line: line_no,
                    });
                }
                pos = at + whole;
                continue;
            }
            if tail.starts_with(CLOSE) {
                match &open {
                    None => problem(&mut problems, line_no, "[[/classified]] with no span open"),
                    Some(Open::Inline {
                        level,
                        reason,
                        start,
                        body_start,
                        line,
                    }) => {
                        let original = &text[*body_start..line_start + at];
                        if js_trim(original).is_empty() {
                            problem(&mut problems, *line, "a classified span is empty");
                        } else if crosses_blank_line(original) {
                            problem(
                                &mut problems,
                                *line,
                                "an inline classified span crosses a blank line: use a ```classified block",
                            );
                        } else {
                            spans.push(ClassifiedSpan {
                                kind: SpanKind::Inline,
                                level: *level,
                                reason: reason.clone(),
                                original: original.to_owned(),
                                start: *start,
                                end: line_start + at + CLOSE.len(),
                                line: *line,
                                end_line: line_no,
                                indent: String::new(),
                            });
                        }
                        open = None;
                    }
                    // A block is handled above; it never reaches the inline markers.
                    Some(Open::Block { .. }) => {}
                }
                pos = at + CLOSE.len();
                continue;
            }
            problem(
                &mut problems,
                line_no,
                "a malformed classified marker: [[classified:LEVEL reason=\"REASON\"]]text[[/classified]]",
            );
            pos = m.end();
        }
    }
    if let Some(o) = open {
        let message = match o {
            Open::Block { .. } => "a classified block is never closed",
            Open::Inline { .. } => "a classified span is never closed: [[/classified]]",
        };
        problem(&mut problems, o.line(), message);
    }
    Classified { spans, problems }
}

/// The lines of `text`: where each starts, its text without the ending, and the
/// ending's length (`\n` or a lone `\r` 1, `\r\n` 2, the last line 0).
fn lines(text: &str) -> Vec<(usize, &str, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push((start, &text[start..i], 1));
                i += 1;
                start = i;
            }
            b'\r' => {
                let ending = if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                out.push((start, &text[start..i], ending));
                i += ending;
                start = i;
            }
            _ => i += 1,
        }
    }
    out.push((start, &text[start..], 0));
    out
}

/// How many line endings `s` holds (`\r\n` is one).
fn newlines(s: &str) -> usize {
    lines(s).len() - 1
}

/// Whether a line between two line endings of `s` is blank (spaces and tabs only).
fn crosses_blank_line(s: &str) -> bool {
    let all = lines(s);
    all.len() > 2
        && all[1..all.len() - 1]
            .iter()
            .any(|(_, l, _)| l.bytes().all(|b| b == b' ' || b == b'\t'))
}

/// The default for a withheld span: the `[REDACTED: reason]` marker inline, a
/// ```` ```redacted ```` block holding the reason for a block, with the span's line
/// count kept.
#[must_use]
pub fn withheld_markers(span: &ClassifiedSpan) -> String {
    match span.kind {
        SpanKind::Inline => format!(
            "[REDACTED: {}]{}",
            span.reason,
            "\n".repeat(newlines(&span.original))
        ),
        SpanKind::Block => {
            let ind = &span.indent;
            let mut lines = vec![format!("{ind}```redacted"), format!("{ind}{}", span.reason)];
            lines.extend(std::iter::repeat_n(String::new(), newlines(&span.original)));
            lines.push(format!("{ind}```"));
            lines.join("\n")
        }
    }
}

/// The default for a revealed span: its text unwrapped (a block's fence lines become
/// blank lines).
#[must_use]
pub fn revealed_plain(span: &ClassifiedSpan) -> String {
    match span.kind {
        SpanKind::Inline => span.original.clone(),
        SpanKind::Block => format!("\n{}\n", span.original),
    }
}

/// `text` as a reader at `clearance` may see it: every span above the clearance
/// withheld by [`withheld_markers`], every span at or below it revealed by
/// [`revealed_plain`].
///
/// # Errors
///
/// [`ClassifiedError`] with every problem when a marker does not parse.
pub fn cut(text: &str, clearance: AccessLevel) -> Result<String, ClassifiedError> {
    cut_with(text, clearance, withheld_markers, revealed_plain)
}

/// [`cut`] with renderers of your own: `withhold` for each span above the clearance,
/// `reveal` for each one at or below it.
///
/// # Errors
///
/// [`ClassifiedError`] with every problem when a marker does not parse.
pub fn cut_with(
    text: &str,
    clearance: AccessLevel,
    withhold: impl Fn(&ClassifiedSpan) -> String,
    reveal: impl Fn(&ClassifiedSpan) -> String,
) -> Result<String, ClassifiedError> {
    let Classified { spans, problems } = parse_classified(text);
    if !problems.is_empty() {
        return Err(ClassifiedError { problems });
    }
    if spans.is_empty() {
        return Ok(text.to_owned());
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for s in &spans {
        out.push_str(&text[at..s.start]);
        out.push_str(&if cleared(s.level, clearance) {
            reveal(s)
        } else {
            withhold(s)
        });
        at = s.end;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

/// The strings a leak would show, for a span: the original trimmed, and each of its
/// lines of `min` characters (UTF-16 units) or more. Shorter ones are left out, since
/// they would match ordinary text; [`cut`] is the guarantee, these are the test.
#[must_use]
pub fn needles(span: &ClassifiedSpan, min: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let whole = js_trim(&span.original);
    for t in std::iter::once(whole).chain(span.original.split('\n').map(js_trim)) {
        if js_len(t) >= min && seen.insert(t) {
            out.push(t.to_owned());
        }
    }
    out
}

/// A small stable hash of a reason (FNV-1a over UTF-16 units, as the site computes
/// it), so a bar's width depends on the reason and never on the original. `buckets`
/// of 0 is treated as 1.
#[must_use]
pub fn reason_bucket(reason: &str, buckets: u32) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for unit in reason.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(16_777_619);
    }
    h % buckets.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "zq-canary-7731";

    fn inline(level: &str, body: &str, reason: &str) -> String {
        format!("[[classified:{level} reason=\"{reason}\"]]{body}[[/classified]]")
    }

    fn block(level: &str, body: &str, reason: &str, fence: &str) -> String {
        format!("{fence}classified level={level} reason=\"{reason}\"\n{body}\n{fence}")
    }

    #[test]
    fn the_ladder_is_ordered_and_one_comparison_decides() {
        use AccessLevel::{Internal, Partner, Preview, Public, Team};
        assert_eq!(
            AccessLevel::ALL.map(AccessLevel::as_str),
            ["public", "preview", "partner", "team", "internal"]
        );
        assert!(!cleared(Partner, Public));
        assert!(!cleared(Partner, Preview));
        assert!(cleared(Partner, Partner));
        assert!(cleared(Partner, Internal));
        assert!(!cleared(Internal, Team));
        assert_eq!(AccessLevel::span_level("admin"), Some(Internal));
        assert_eq!(AccessLevel::span_level("public"), None);
    }

    #[test]
    fn cut_public_never_holds_an_original_for_every_shape() {
        let doc = [
            inline("preview", SECRET, "the internal port"),
            inline("partner", &format!("{SECRET}-b"), "r"),
            inline("team", &format!("x {SECRET}-c\ny"), "r"),
            block(
                "internal",
                &format!("{SECRET}\nsecond line"),
                "the routing table",
                "```",
            ),
            block("admin", &format!("{SECRET}-d"), "r", "```"),
            block(
                "partner",
                &format!("```sh\nrun {SECRET}-e\n```"),
                "the command",
                "````",
            ),
        ]
        .join("\n\n");
        let out = cut(&doc, AccessLevel::Public).unwrap();
        assert!(!out.contains(SECRET), "{out}");
        let spans = parse_classified(&doc).spans;
        assert_eq!(spans.len(), 6);
        for s in &spans {
            assert!(!out.contains(&s.original));
            for n in needles(s, 4) {
                assert!(!out.contains(&n), "{n} leaked");
            }
        }
        assert_eq!(out.split('\n').count(), doc.split('\n').count());
        let all = cut(&doc, AccessLevel::Internal).unwrap();
        assert_eq!(all.split('\n').count(), doc.split('\n').count());
        assert!(all.contains(&format!("{SECRET}-e")));
    }

    #[test]
    fn a_problem_cuts_nothing() {
        let text = format!("ok [[classified:partner reason=\"r\"]]{SECRET}\n\nmore");
        let err = cut(&text, AccessLevel::Public).unwrap_err();
        assert_eq!(err.problems.len(), 1);
        assert_eq!(err.problems[0].line, 1);
        assert!(!err.to_string().contains(SECRET));
    }

    #[test]
    fn a_crlf_document_is_cut_like_an_lf_one() {
        let lf = [
            format!("a {} b", inline("partner", SECRET, "the port")),
            block(
                "team",
                &format!("{SECRET}-x\nsecond {SECRET}-y"),
                "the table",
                "```",
            ),
            format!(
                "x {} y",
                inline("internal", &format!("{SECRET}-m\ncontinued"), "r")
            ),
            "end".to_owned(),
        ]
        .join("\n\n");
        for ending in ["\r\n", "\r"] {
            let other = lf.replace('\n', ending);
            let out = cut(&other, AccessLevel::Public).unwrap();
            assert!(!out.contains(SECRET), "{out:?}");
            assert_eq!(newlines(&out), newlines(&other));
            assert_eq!(parse_classified(&other).spans.len(), 3);
        }
        let crlf = lf.replace('\n', "\r\n");
        let parsed = parse_classified(&crlf);
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        assert_eq!(parsed.spans.len(), 3);
        let out = cut(&crlf, AccessLevel::Public).unwrap();
        assert!(!out.contains(SECRET), "{out:?}");
        assert_eq!(out.split('\n').count(), crlf.split('\n').count());
        assert_eq!(
            out.replace('\r', ""),
            cut(&lf, AccessLevel::Public).unwrap().replace('\r', "")
        );
        let shown = cut(&crlf, AccessLevel::Internal).unwrap();
        assert!(shown.contains(&format!("{SECRET}-x")));
    }

    #[test]
    fn a_stray_carriage_return_never_makes_an_ordinary_fence() {
        for text in [
            format!("```classified level=team reason=\"a\rb\"\n{SECRET}\n```"),
            format!("[[classified:team reason=\"a\rb\"]]{SECRET}[[/classified]]"),
            format!("```classified level=team reason=\"r\"\r\n{SECRET}\r\n"),
            format!("```classified level=team\r reason=\"r\"\n{SECRET}\n```"),
        ] {
            assert!(
                !parse_classified(&text).problems.is_empty(),
                "no problem in {text:?}"
            );
            assert!(cut(&text, AccessLevel::Public).is_err());
        }
        // A second `\r` ends an empty line, the body's first: still a block, and cut.
        let text = format!("```classified level=team reason=\"r\"\r\r\n{SECRET}\r\n```");
        assert!(!cut(&text, AccessLevel::Public).unwrap().contains(SECRET));
    }

    #[test]
    fn needles_and_buckets() {
        let span = &parse_classified(&inline("team", "  abcd\nef\nghijk ", "r")).spans[0];
        assert_eq!(needles(span, 4), ["abcd\nef\nghijk", "abcd", "ghijk"]);
        assert_eq!(reason_bucket("", 3), 2_166_136_261 % 3);
        // FNV-1a of "a" over 32 bits.
        assert_eq!(reason_bucket("a", 1_000_000), 3_826_002_220 % 1_000_000);
        assert_eq!(reason_bucket("x", 0), 0);
    }

    #[test]
    fn a_long_reason_counts_utf16_units() {
        let ok = "é".repeat(160);
        let long = "😀".repeat(81); // 162 units
        assert!(reason_problem(&ok).is_none());
        assert!(reason_problem(&long).is_some());
    }
}
