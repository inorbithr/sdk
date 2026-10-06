//! The checks every lab document passes (RFC 0035): its file name, its front matter,
//! its status log and, when it is public, the redaction rules. The InOrbit site's build
//! runs the same checks in TypeScript; the cases in `spec/lab/conformance.json`, synced
//! from the platform, hold both to the same findings, by rule id and line.
//!
//! Classified spans (`[[classified:LEVEL reason="..."]]...[[/classified]]` and the
//! ```` ```classified ```` block) are parsed on every document, and the redaction rules
//! read a public document as an uncleared reader sees it, after [`cut`]: withheld text
//! is exempt, its reason is not. [`cut`] is the same function the site renders with.
//!
//! The generic redaction rules are data, `spec/lab/rules.json`, embedded at build
//! time; a lab adds its own domains, words and patterns in its config
//! (`docs/lab/redaction.json`).

mod check;
mod classified;
mod redaction;

pub use check::{Finding, Kind, check_document, check_folder};
pub use classified::{
    AccessLevel, Classified, ClassifiedError, ClassifiedProblem, ClassifiedSpan, MAX_REASON,
    SpanKind, cleared, cut, cut_with, needles, parse_classified, reason_bucket, revealed_plain,
    withheld_markers,
};
pub use redaction::{Config, Hit, Pattern, Redaction, parse_rules};

/// The generic redaction rules, as synced from the platform.
pub const GENERIC_RULES: &str = include_str!("../../../../spec/lab/rules.json");

/// Why the rules or a config could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A rules file or a config is not the JSON it should be.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// A rule's pattern does not compile.
    #[error("rule `{id}`: {source}")]
    Pattern {
        /// The rule.
        id: String,
        /// Why it does not compile.
        source: regex::Error,
    },
}
