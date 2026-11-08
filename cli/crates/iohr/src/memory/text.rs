//! The text logic memory uses: the normal form for exact duplicates, word shingles for near
//! duplicates, content words for recall, and the check that refuses a secret.
//!
//! Ported as small pure functions from the desktop app's memory crate (`text.rs`: `normalize`,
//! `shingles`, `fnv`, `jaccard`, `content_words`, `word_hits`; `privacy.rs`: the generic secret
//! pattern). Only the generic parts: nothing here names anyone's private material. Text is
//! never logged.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

static NON_WORD: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}#/._ -]+").ok());
static WORD: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}'._/#-]*").ok());
static SECRET: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        sk-ant-[A-Za-z0-9_-]{8,} | sk-[A-Za-z0-9]{20,} | gh[pousr]_[A-Za-z0-9]{20,} | github_pat_[A-Za-z0-9_]{20,}
      | AKIA[0-9A-Z]{16} | xox[abpr]-[A-Za-z0-9-]{10,} | -----BEGIN[ A-Z]*PRIVATE\ KEY----- | eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}
      | (?i:(password|passwd|secret|api[_-]?key|token)\s*[:=]\s*\S{6,})",
    )
    .ok()
});

const FILLER: &[&str] = &[
    "please", "and", "also", "ok", "okay", "so", "but", "then", "now", "yes", "well", "hey",
];

const STOP: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "if", "then", "so", "to", "of", "in", "on", "at", "for",
    "with", "by", "from", "as", "is", "are", "was", "were", "be", "been", "it", "its", "this",
    "that", "these", "those", "i", "you", "we", "they", "he", "she", "me", "my", "your", "our",
    "us", "do", "does", "did", "not", "no", "can", "could", "would", "should", "will", "just",
    "also", "all", "any", "some", "what", "which", "who", "when", "where", "how", "why", "there",
    "here", "have", "has", "had", "get", "got", "let", "make", "please", "ok", "okay", "yes",
    "now", "like", "one", "more", "very", "much", "into", "out", "up", "about", "than", "too",
    "them", "their", "see", "need", "want",
];

/// Whether `text` holds something that looks like a secret (a key, a token, a private key
/// block, `password: ...`). Memory never keeps one: it is committed with the project.
#[must_use]
pub fn has_secret(text: &str) -> bool {
    SECRET.as_ref().is_some_and(|r| r.is_match(text))
}

/// The normal form used for exact duplicates: lower case, punctuation gone, fillers at the
/// start dropped, whitespace collapsed.
#[must_use]
pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let t = NON_WORD.as_ref().map_or_else(
        || lower.clone(),
        |r| r.replace_all(&lower, " ").into_owned(),
    );
    let mut words: Vec<&str> = t
        .split_whitespace()
        .map(|w| w.trim_matches(['.', '-', '_', '/']))
        .filter(|w| !w.is_empty())
        .collect();
    while words.first().is_some_and(|w| FILLER.contains(w)) {
        words.remove(0);
    }
    words.join(" ")
}

/// Every word of `s`, lower case, stop words included.
#[must_use]
pub fn words(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    WORD.as_ref().map_or_else(Vec::new, |r| {
        r.find_iter(&lower)
            .map(|m| m.as_str().trim_end_matches(['.', '\'']).to_owned())
            .filter(|w| !w.is_empty())
            .collect()
    })
}

/// The words that carry meaning: no stop words, at least two characters.
#[must_use]
pub fn content_words(s: &str) -> Vec<String> {
    words(s)
        .into_iter()
        .filter(|w| w.chars().count() >= 2 && !STOP.contains(&w.as_str()))
        .collect()
}

/// FNV-1a: stable across builds and machines.
#[must_use]
pub fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Word 3-shingles (hashed) of a normal form, sorted, for near-duplicate matching. Short
/// texts fall back to single words so they still compare.
#[must_use]
pub fn shingles(norm: &str) -> Vec<u32> {
    let w: Vec<&str> = norm.split(' ').filter(|w| !w.is_empty()).collect();
    let mut set: HashSet<u32> = HashSet::new();
    if w.len() < 4 {
        for x in &w {
            set.insert(fnv(x));
        }
    } else {
        for win in w.windows(3) {
            set.insert(fnv(&win.join(" ")));
        }
    }
    let mut v: Vec<u32> = set.into_iter().collect();
    v.sort_unstable();
    v
}

/// Jaccard similarity of two sorted shingle sets, 0.0 to 1.0.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "set sizes are tiny")]
pub fn jaccard(a: &[u32], b: &[u32]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut inter) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
        }
    }
    inter as f64 / (a.len() + b.len() - inter) as f64
}

/// How many of the query's words `mem` has: a word counts when `mem` has it, or a word that
/// starts with it minus its last letter ("restart" finds "restarts").
#[must_use]
pub fn word_hits<S: std::hash::BuildHasher>(query: &HashSet<String, S>, mem: &str) -> usize {
    let mw = content_words(mem);
    query
        .iter()
        .filter(|w| {
            let stem = w.char_indices().last().map_or("", |(i, _)| &w[..i]);
            mw.iter()
                .any(|x| x == *w || (w.len() >= 5 && x.starts_with(stem)))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_form_ignores_case_punctuation_and_leading_fillers() {
        assert_eq!(
            normalize("Please, use SQLx for queries!"),
            normalize("use sqlx for queries")
        );
        assert_ne!(normalize("use sqlx"), normalize("use diesel"));
    }

    #[test]
    fn near_duplicates_score_high_and_different_things_low() {
        let a = shingles(&normalize(
            "run cargo nextest from the cli directory before a push",
        ));
        let b = shingles(&normalize(
            "run cargo nextest from the cli directory before pushing",
        ));
        let c = shingles(&normalize("the release train rolls on fridays only"));
        assert!(jaccard(&a, &b) > 0.4, "{}", jaccard(&a, &b));
        assert!(jaccard(&a, &c) < 0.1);
        assert!((jaccard(&a, &a) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn secrets_are_found_and_ordinary_text_is_not() {
        assert!(has_secret(
            "the key is ghp_abcdefghijklmnopqrstuvwxyz012345"
        ));
        assert!(has_secret("password: hunter2hunter2"));
        assert!(!has_secret("tokens expire after an hour; see src/auth.rs"));
    }

    #[test]
    fn word_hits_match_a_stem() {
        let q: HashSet<String> = content_words("restart policy").into_iter().collect();
        assert_eq!(word_hits(&q, "the runner restarts on exit"), 1);
        assert_eq!(word_hits(&q, "restart policy is always"), 2);
    }
}
