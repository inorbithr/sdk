//! Chunking and tokens: a cleaned page cut along its headings into pieces a search can return
//! and a model can read, each with a stable id, its heading path, its place in the page's text
//! and a token count.
//!
//! The page text is the page's blocks joined by a blank line. A chunk is an exact slice of it:
//! `start` and `end` are character offsets, so `text` can be checked against the page.
//!
//! Tokens: `tokens` counts words (the same words search indexes). `est_model_tokens` is an
//! estimate of what a language model would count, one token per four characters; it is
//! labelled an estimate wherever it is shown, because no model's tokenizer runs here.

use std::collections::HashSet;

use super::clean::{Block, BlockKind};
use crate::front;
use crate::memory::text;

/// A chunk grows to about this many words before the next block starts a new one.
pub const TARGET_WORDS: usize = 180;
/// A single block longer than this is cut at sentence ends.
pub const MAX_WORDS: usize = 260;
/// A last chunk shorter than this joins the one before it.
const MIN_WORDS: usize = 30;

/// One chunk of one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Stable: the page, the heading path and the chunk's number under that path.
    pub id: String,
    /// Its number in the page, from 0.
    pub ord: usize,
    /// The nearest heading, or empty.
    pub heading: String,
    /// The headings above it, outermost first, joined with ` > `.
    pub path: String,
    /// Character offsets in the page text.
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub tokens: usize,
    pub est_model_tokens: usize,
}

/// The page text and its chunks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Chunked {
    pub text: String,
    pub chunks: Vec<Chunk>,
}

/// What identifies a block across pages: its text, lower case, words only.
#[must_use]
pub fn block_key(text: &str) -> String {
    front::sha_hex(&text::normalize(text))
        .chars()
        .take(16)
        .collect()
}

/// A rough count of what a language model would tokenize `s` into.
#[must_use]
pub fn estimate_model_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// The number of words in `s`.
#[must_use]
pub fn tokens(s: &str) -> usize {
    text::words(s).len()
}

/// A unit the chunks are made of: a block, or a run of sentences of a long block.
struct Unit {
    /// Byte range in the page text.
    range: (usize, usize),
    words: usize,
    heading: bool,
    path: String,
    title: String,
}

/// Sentence pieces of `s` (byte ranges), none longer than [`MAX_WORDS`] words.
fn pieces(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut words = 0;
    let mut last_end = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() && (i == 0 || !s[..i].ends_with(char::is_whitespace)) {
            words += 1;
        }
        let end_of_sentence =
            matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|(_, n)| n.is_whitespace());
        let next = i + c.len_utf8();
        if end_of_sentence || words >= MAX_WORDS {
            last_end = next;
        }
        if words >= TARGET_WORDS && last_end > start && end_of_sentence {
            out.push((start, last_end));
            start = last_end;
            words = 0;
        } else if words >= MAX_WORDS {
            out.push((start, next));
            start = next;
            words = 0;
        }
    }
    if start < s.len() && !s[start..].trim().is_empty() {
        out.push((start, s.len()));
    }
    out
}

/// The page text (blocks joined by a blank line) and its units: one per heading or block, a
/// long block cut at sentences.
fn lay_out(kept: &[&Block]) -> (String, Vec<Unit>) {
    let mut page = String::new();
    let mut units: Vec<Unit> = Vec::new();
    let mut stack: Vec<(u8, String)> = Vec::new();
    for b in kept.iter().copied() {
        if !page.is_empty() {
            page.push_str("\n\n");
        }
        let heading = if let BlockKind::Heading(level) = b.kind {
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            stack.push((level, b.text.clone()));
            true
        } else {
            false
        };
        let path = stack
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" > ");
        let title = stack.last().map(|(_, t)| t.clone()).unwrap_or_default();
        let base = page.len();
        page.push_str(&b.text);
        let words = b.text.split_whitespace().count();
        if heading || words <= MAX_WORDS {
            units.push(Unit {
                range: (base, page.len()),
                words,
                heading,
                path,
                title,
            });
        } else {
            for (s, e) in pieces(&b.text) {
                units.push(Unit {
                    range: (base + s, base + e),
                    words: b.text[s..e].split_whitespace().count(),
                    heading: false,
                    path: path.clone(),
                    title: title.clone(),
                });
            }
        }
    }
    (page, units)
}

/// The units in groups: a heading starts a chunk, words past the target start the next one, a
/// short tail joins the group before it, and a section of only headings is dropped.
fn group(units: Vec<Unit>) -> Vec<Vec<Unit>> {
    let mut groups: Vec<Vec<Unit>> = Vec::new();
    for u in units {
        let start_new = match groups.last() {
            None => true,
            Some(g) => {
                let w: usize = g.iter().map(|x| x.words).sum();
                let same_path = g.last().is_some_and(|x| x.path == u.path);
                u.heading || !same_path || w + u.words > TARGET_WORDS
            }
        };
        if start_new {
            groups.push(vec![u]);
        } else if let Some(g) = groups.last_mut() {
            g.push(u);
        }
    }
    // A short tail joins the group before it when both sit under the same path.
    let mut merged: Vec<Vec<Unit>> = Vec::new();
    for g in groups {
        let w: usize = g.iter().map(|x| x.words).sum();
        let joins = w < MIN_WORDS
            && merged.last().is_some_and(|p| {
                p.last()
                    .zip(g.first())
                    .is_some_and(|(a, b)| a.path == b.path)
                    && !g.first().is_some_and(|x| x.heading)
            });
        if joins {
            if let Some(p) = merged.last_mut() {
                p.extend(g);
            }
        } else {
            merged.push(g);
        }
    }
    // A section that is only a heading adds nothing the next chunk's path does not say.
    let only_headings = |g: &Vec<Unit>| g.iter().all(|u| u.heading);
    let has_body = merged.iter().any(|g| !only_headings(g));
    let merged: Vec<Vec<Unit>> = if has_body {
        merged.into_iter().filter(|g| !only_headings(g)).collect()
    } else {
        merged
    };
    merged
}

/// The page text and chunks for `blocks`. Blocks whose key is in `boilerplate` (text that
/// repeats across the site) and repeats within the page are left out.
#[must_use]
pub fn chunk<S: std::hash::BuildHasher>(
    url: &str,
    blocks: &[Block],
    boilerplate: &HashSet<String, S>,
) -> Chunked {
    let mut seen = HashSet::new();
    let kept: Vec<&Block> = blocks
        .iter()
        .filter(|b| {
            let k = block_key(&b.text);
            !boilerplate.contains(&k) && seen.insert(k)
        })
        .collect();
    let (page, units) = lay_out(&kept);
    let merged = group(units);
    let mut per_path: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut chunks = Vec::new();
    for (ord, g) in merged.iter().enumerate() {
        let (Some(first), Some(last)) = (g.first(), g.last()) else {
            continue;
        };
        let (b0, b1) = (first.range.0, last.range.1);
        let slice = page[b0..b1].to_owned();
        let n = per_path.entry(first.path.clone()).or_insert(0);
        let id = format!(
            "ch_{}",
            front::sha_hex(&format!("{url}\u{0}{}\u{0}{n}", first.path))
                .chars()
                .take(16)
                .collect::<String>()
        );
        *n += 1;
        chunks.push(Chunk {
            id,
            ord,
            heading: last_heading(g).unwrap_or_else(|| first.title.clone()),
            path: first.path.clone(),
            start: page[..b0].chars().count(),
            end: page[..b1].chars().count(),
            tokens: tokens(&slice),
            est_model_tokens: estimate_model_tokens(&slice),
            text: slice,
        });
    }
    Chunked { text: page, chunks }
}

fn last_heading(g: &[Unit]) -> Option<String> {
    g.first().map(|u| u.title.clone()).filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(kind: BlockKind, t: &str) -> Block {
        Block {
            kind,
            text: t.to_owned(),
        }
    }

    fn words(n: usize, w: &str) -> String {
        vec![w; n].join(" ")
    }

    fn doc() -> Vec<Block> {
        vec![
            b(BlockKind::Heading(1), "Picker"),
            b(BlockKind::Text, &format!("{}.", words(40, "alpha"))),
            b(BlockKind::Heading(2), "Specs"),
            b(BlockKind::Text, &format!("{}.", words(50, "beta"))),
            b(BlockKind::Heading(3), "Battery"),
            b(BlockKind::Text, &format!("{}.", words(45, "gamma"))),
            b(BlockKind::Heading(2), "Support"),
            b(BlockKind::Text, &format!("{}.", words(60, "delta"))),
        ]
    }

    #[test]
    fn chunks_follow_the_headings_and_carry_the_path() {
        let c = chunk("https://a/x", &doc(), &HashSet::new());
        let paths: Vec<&str> = c.chunks.iter().map(|k| k.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "Picker",
                "Picker > Specs",
                "Picker > Specs > Battery",
                "Picker > Support"
            ]
        );
        assert_eq!(c.chunks[2].heading, "Battery");
        assert!(c.chunks[1].text.starts_with("Specs\n\nbeta"));
    }

    #[test]
    fn a_chunk_is_an_exact_slice_of_the_page_text() {
        let c = chunk("https://a/x", &doc(), &HashSet::new());
        for k in &c.chunks {
            let slice: String = c.text.chars().skip(k.start).take(k.end - k.start).collect();
            assert_eq!(slice, k.text);
            assert_eq!(k.tokens, tokens(&k.text));
            assert_eq!(k.est_model_tokens, k.text.chars().count().div_ceil(4));
        }
        let ords: Vec<usize> = c.chunks.iter().map(|k| k.ord).collect();
        assert_eq!(ords, [0, 1, 2, 3]);
    }

    #[test]
    fn ids_are_stable_and_do_not_move_when_another_section_changes() {
        let a = chunk("https://a/x", &doc(), &HashSet::new());
        let again = chunk("https://a/x", &doc(), &HashSet::new());
        assert_eq!(a, again);
        let mut edited = doc();
        edited[1] = b(BlockKind::Text, &format!("{}.", words(70, "omega")));
        let e = chunk("https://a/x", &edited, &HashSet::new());
        assert_ne!(a.chunks[0].text, e.chunks[0].text);
        assert_eq!(
            a.chunks[0].id, e.chunks[0].id,
            "same page, same path, same number"
        );
        assert_eq!(a.chunks[3].id, e.chunks[3].id);
        let other = chunk("https://a/y", &doc(), &HashSet::new());
        assert_ne!(a.chunks[0].id, other.chunks[0].id);
    }

    #[test]
    fn a_long_section_is_cut_and_a_long_paragraph_is_cut_at_sentences() {
        let long = (0..40)
            .map(|i| format!("{} {i}.", words(24, "word")))
            .collect::<Vec<_>>()
            .join(" ");
        let blocks = vec![b(BlockKind::Heading(1), "Long"), b(BlockKind::Text, &long)];
        let c = chunk("https://a/l", &blocks, &HashSet::new());
        assert!(c.chunks.len() >= 4, "{}", c.chunks.len());
        for k in &c.chunks {
            assert!(k.tokens <= MAX_WORDS + 5, "{}", k.tokens);
        }
        assert!(c.chunks.iter().skip(1).all(|k| k.path == "Long"));
        let ids: HashSet<&str> = c.chunks.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids.len(), c.chunks.len(), "numbers under one path differ");
    }

    #[test]
    fn site_wide_and_repeated_blocks_are_left_out() {
        let boiler: HashSet<String> = [block_key("All rights reserved")].into_iter().collect();
        let blocks = vec![
            b(BlockKind::Heading(1), "Page"),
            b(BlockKind::Text, &format!("{}.", words(40, "real"))),
            b(BlockKind::Text, "All rights reserved"),
            b(BlockKind::Text, &format!("{}.", words(40, "real"))),
        ];
        let c = chunk("https://a/p", &blocks, &boiler);
        assert!(!c.text.contains("rights"));
        assert_eq!(c.text.matches("real").count(), 40, "the repeat is dropped");
    }

    #[test]
    fn a_page_of_one_heading_still_has_a_chunk_and_an_empty_page_has_none() {
        let c = chunk(
            "https://a/h",
            &[b(BlockKind::Heading(1), "Only")],
            &HashSet::new(),
        );
        assert_eq!(c.chunks.len(), 1);
        assert_eq!(
            chunk("https://a/e", &[], &HashSet::new()).chunks,
            Vec::new()
        );
    }

    #[test]
    fn a_heading_alone_over_a_sub_heading_is_not_a_chunk() {
        let blocks = vec![
            b(BlockKind::Heading(1), "Top"),
            b(BlockKind::Heading(2), "Sub"),
            b(BlockKind::Text, &format!("{}.", words(40, "body"))),
        ];
        let c = chunk("https://a/s", &blocks, &HashSet::new());
        assert_eq!(c.chunks.len(), 1);
        assert_eq!(c.chunks[0].path, "Top > Sub");
    }
}
