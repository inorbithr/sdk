//! The scorers: pure functions from a case and an answer to 0 or 1.

use crate::dataset::Case;
use crate::solver::LABELS;

/// A scorer takes the case and the solver's answer and returns 0 or 1. It is a pure function: the
/// same case and answer give the same score, with no clock, no randomness and no network, so a
/// change in the score is a change in the solver. (A model that grades another model's answer is
/// a solver of its own: label it a stub in tests and give it a bar of its own.)
pub type Scorer = fn(&Case, &str) -> u8;

pub const SCORERS: [(&str, Scorer); 2] = [("exact", exact), ("format", format)];

/// The answer is the expected label, ignoring case and surrounding space.
pub fn exact(case: &Case, answer: &str) -> u8 {
    u8::from(answer.trim().to_lowercase() == case.expected.trim().to_lowercase())
}

/// The answer is one bare label from the list and nothing else (no sentence, no full stop). A
/// model that is right but chatty breaks the program that reads its answer, so it fails.
pub fn format(_case: &Case, answer: &str) -> u8 {
    u8::from(LABELS.contains(&answer))
}

pub fn scorer_names() -> Vec<&'static str> {
    SCORERS.iter().map(|(n, _)| *n).collect()
}
