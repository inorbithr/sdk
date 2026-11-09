//! Embeddings: a labelled stub.
//!
//! No local embedding model runs in `iohr` today, and the platform's `llm_embed` is a remote
//! call, which a crawl never makes (nothing leaves the machine until `iohr sync`). So the index
//! stores no vectors and search is lexical (BM25). This is the one place a local model plugs in
//! later: [`embed`] returns what it produced, and the index records the status string with the
//! site so every surface can say `stub` instead of implying vectors exist.

/// What the index records and every surface shows.
pub const STATUS: &str = "stub: no local embedding model; search is lexical (BM25)";

/// A vector per input text, in order, or `None` when no local model is available (always, for
/// now: this is a stub).
#[must_use]
pub fn embed(_texts: &[&str]) -> Option<Vec<Vec<f32>>> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_stub_says_it_is_a_stub_and_makes_no_vectors() {
        assert!(super::STATUS.starts_with("stub"));
        assert!(super::embed(&["a"]).is_none());
    }
}
