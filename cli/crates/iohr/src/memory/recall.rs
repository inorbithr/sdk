//! Recall: the memories that say most about a topic. A word match (stems count), weighted by
//! how recently the memory was confirmed and whether it is pinned.
//!
//! The ranking idea follows the desktop app's memory crate (`word_hits` coverage, recency by
//! kind, a pin boost), kept small: no confidence or seen counters, because a file has none.

use std::collections::HashSet;

use super::store::{Entry, Kind};
use super::text;
use crate::front;

/// Days over which a fact or episode loses weight; a preference never does.
const HALF_LIFE_DAYS: f64 = 90.0;

/// One recalled memory with its score.
#[derive(Debug, Clone)]
pub struct Hit {
    pub entry: Entry,
    pub score: f64,
}

/// The score of `e` for `query`, `0.0` when it shares no word with it.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "word counts and days are small")]
pub fn score<S: std::hash::BuildHasher>(
    e: &Entry,
    query: &HashSet<String, S>,
    now_unix: i64,
) -> f64 {
    if query.is_empty() {
        return 0.0;
    }
    let hay = format!("{} {} {}", e.text, e.why, e.refs.join(" "));
    let hits = text::word_hits(query, &hay);
    if hits == 0 {
        return 0.0;
    }
    let cover = hits as f64 / query.len() as f64;
    let age_days = front::unix_of(&e.updated).map_or(HALF_LIFE_DAYS, |t| {
        ((now_unix - t) as f64 / 86_400.0).max(0.0)
    });
    let recency = if e.kind == Kind::Preference {
        1.0
    } else {
        1.0 / (1.0 + age_days / HALF_LIFE_DAYS).sqrt()
    };
    cover * (0.4 + 0.6 * recency) * if e.pinned { 1.5 } else { 1.0 }
}

/// The best `limit` memories for `query`, best first; ties by id so the order is stable.
#[must_use]
pub fn recall(entries: &[Entry], query: &str, limit: usize, now_unix: i64) -> Vec<Hit> {
    let words: HashSet<String> = text::content_words(query).into_iter().collect();
    let mut hits: Vec<Hit> = entries
        .iter()
        .map(|e| Hit {
            entry: e.clone(),
            score: score(e, &words, now_unix),
        })
        .filter(|h| h.score > 0.0)
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.entry.id.cmp(&b.entry.id))
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(text: &str, pinned: bool) -> Entry {
        let mut e = Entry::new(text, Kind::Fact, "", Vec::new(), pinned);
        e.updated = front::now();
        e
    }

    #[test]
    fn the_matching_memory_comes_first_and_the_rest_not_at_all() {
        let all = vec![
            e("Tests run with cargo nextest from the cli folder.", false),
            e("The release train rolls on fridays.", false),
        ];
        let now = front::unix_of(&front::now()).unwrap_or(0);
        let hits = recall(&all, "how do tests run", 5, now);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].entry.text.contains("nextest"));
    }

    #[test]
    fn pinned_wins_a_tie() {
        let all = vec![e("builds use mise.", false), e("builds use cargo.", true)];
        let now = front::unix_of(&front::now()).unwrap_or(0);
        let hits = recall(&all, "builds", 5, now);
        assert!(hits[0].entry.pinned);
    }

    #[test]
    fn a_topic_with_no_content_word_recalls_nothing() {
        let all = vec![e("anything", false)];
        assert!(recall(&all, "the of", 5, 0).is_empty());
    }
}
