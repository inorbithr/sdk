//! The solver: the thing under test.

use crate::dataset::Case;

/// The answers the suite knows, in the order the rules list them.
pub const LABELS: [&str; 5] = ["billing", "bug", "feature_request", "account", "other"];

/// The thing under test: it turns a case's input into an answer. The one here is a STUB (a fixed
/// list of keyword rules), labelled as one everywhere it shows up (`stub: true` in the report, a
/// `stub-` name). It exists so the suite runs offline, is free and gives the same answer every
/// time. To grade a real model, write a type with these three methods that calls your provider,
/// sign in the way your provider's subscription allows and keep the credential in your secret
/// store, never in this folder. A solver that fails is counted as an error, never a pass.
pub trait Solver {
    fn name(&self) -> String;
    fn stub(&self) -> bool;
    fn solve(&self, case: &Case) -> Result<String, String>;
}

/// First match wins, in this order.
const RULES: [(&str, &[&str]); 4] = [
    (
        "billing",
        &["charged", "refund", "invoice", "payment", "card"],
    ),
    ("bug", &["crash", "error", "blank screen"]),
    (
        "feature_request",
        &["would be great", "add an option", "feature"],
    ),
    ("account", &["password", "account"]),
];

/// Keyword rules. Not a model: nothing is learned, nothing is called.
pub struct StubSolver;

impl Solver for StubSolver {
    fn name(&self) -> String {
        "stub-keyword-rules-v1".to_string()
    }

    fn stub(&self) -> bool {
        true
    }

    fn solve(&self, case: &Case) -> Result<String, String> {
        let text = case.input.to_lowercase();
        for (label, words) in RULES {
            if words.iter().any(|w| text.contains(w)) {
                return Ok(label.to_string());
            }
        }
        Ok("other".to_string())
    }
}
