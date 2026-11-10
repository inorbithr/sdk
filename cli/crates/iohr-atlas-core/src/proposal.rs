//! What a reasoner may hand the core (ADR 0005).
//!
//! A model never writes stored truth. It returns a [`Proposal`]: hypotheses with
//! predictions and falsifiers, citing the claims they rest on, plus requests for
//! evidence. The core admits or rejects it with [`admit`]. Nothing in this module (or
//! anywhere in this crate) turns a proposal into a claim, evidence or support: an
//! admitted hypothesis can only enter a [`crate::question::Question`], where evidence
//! decides its fate.

use serde::{Deserialize, Serialize};

use crate::digest::ContentDigest;
use crate::ids::{ClaimId, HypothesisId, ReasonerInstanceId};
use crate::nonempty::NonEmpty;
use crate::question::{Hypothesis, HypothesisStatus, Outcome};
use crate::time::Timestamp;

/// One recorded reasoner call: what model, with what prompt and context. Reasoning
/// quality depends on the model and its context together, so both are recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasonerInstance {
    /// Its identity.
    pub id: ReasonerInstanceId,
    /// Provider and model, e.g. `local/gpt-oss-120b`.
    pub model: String,
    /// The model's version.
    pub version: String,
    /// The digest of the system prompt.
    pub prompt: ContentDigest,
    /// The tools it could call.
    pub tools: Vec<String>,
    /// The claims it was shown.
    pub context: Vec<ClaimId>,
    /// When it ran.
    pub at: Timestamp,
}

/// A hypothesis as a model proposed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisDraft {
    /// What it says.
    pub statement: String,
    /// What it predicts.
    pub predicts: Vec<Outcome>,
    /// What would falsify it.
    pub falsifiers: Vec<Outcome>,
    /// The claims it rests on.
    pub cites: Vec<ClaimId>,
}

/// A request for more evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRequest {
    /// What to observe or ask.
    pub what: String,
    /// Which drafts it would discriminate between, by index.
    pub discriminates: Vec<usize>,
}

/// A reasoner's whole answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// The call that produced it.
    pub instance: ReasonerInstance,
    /// The hypotheses.
    pub hypotheses: Vec<HypothesisDraft>,
    /// The evidence it asks for.
    pub requests: Vec<EvidenceRequest>,
}

/// Why a draft was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    /// It cites nothing: an assertion with no basis.
    #[error("draft {0} cites no claim")]
    Uncited(usize),
    /// It cites a claim the core does not know: a fabricated basis.
    #[error("draft {0} cites unknown claim {1}")]
    UnknownCitation(usize, ClaimId),
    /// Nothing could falsify it.
    #[error("draft {0} has no falsifier")]
    Unfalsifiable(usize),
    /// A request names a draft that does not exist.
    #[error("a request names draft {0}, which does not exist")]
    BadRequest(usize),
}

/// A proposal the core accepted: its hypotheses, ready to join a question as survivors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedProposal {
    /// The call that produced it.
    pub instance: ReasonerInstanceId,
    /// The admitted hypotheses.
    pub hypotheses: Vec<Hypothesis>,
    /// The admitted evidence requests.
    pub requests: Vec<EvidenceRequest>,
}

/// Admit a proposal: every draft must cite at least one claim the core knows and name at
/// least one falsifier; every request must point at a real draft. All problems are
/// reported at once.
///
/// # Errors
/// Any draft or request breaks a rule.
pub fn admit(
    proposal: Proposal,
    known: impl Fn(ClaimId) -> bool,
) -> Result<AdmittedProposal, Vec<Rejection>> {
    let mut rejections = Vec::new();
    for (i, d) in proposal.hypotheses.iter().enumerate() {
        if d.cites.is_empty() {
            rejections.push(Rejection::Uncited(i));
        }
        for &c in &d.cites {
            if !known(c) {
                rejections.push(Rejection::UnknownCitation(i, c));
            }
        }
        if d.falsifiers.is_empty() {
            rejections.push(Rejection::Unfalsifiable(i));
        }
    }
    for r in &proposal.requests {
        for &i in &r.discriminates {
            if i >= proposal.hypotheses.len() {
                rejections.push(Rejection::BadRequest(i));
            }
        }
    }
    if !rejections.is_empty() {
        return Err(rejections);
    }
    let hypotheses = proposal
        .hypotheses
        .into_iter()
        .filter_map(|d| {
            Some(Hypothesis {
                id: HypothesisId::new(),
                statement: d.statement,
                predicts: d.predicts,
                falsifiers: NonEmpty::new(d.falsifiers).ok()?,
                assumptions: vec![],
                status: HypothesisStatus::Surviving,
            })
        })
        .collect();
    Ok(AdmittedProposal {
        instance: proposal.instance.id,
        hypotheses,
        requests: proposal.requests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn instance() -> ReasonerInstance {
        ReasonerInstance {
            id: ReasonerInstanceId::new(),
            model: "local/gpt-oss-120b".into(),
            version: "test".into(),
            prompt: ContentDigest::of_bytes(b"prompt"),
            tools: vec![],
            context: vec![],
            at: chrono::Utc::now(),
        }
    }

    #[test]
    fn uncited_unfalsifiable_and_fabricated_drafts_are_all_reported() {
        let real = ClaimId::new();
        let fake = ClaimId::new();
        let known: BTreeSet<_> = [real].into();
        let p = Proposal {
            instance: instance(),
            hypotheses: vec![
                HypothesisDraft {
                    statement: "no basis".into(),
                    predicts: vec![],
                    falsifiers: vec![Outcome("x".into())],
                    cites: vec![],
                },
                HypothesisDraft {
                    statement: "made up".into(),
                    predicts: vec![],
                    falsifiers: vec![Outcome("x".into())],
                    cites: vec![fake],
                },
                HypothesisDraft {
                    statement: "unfalsifiable".into(),
                    predicts: vec![],
                    falsifiers: vec![],
                    cites: vec![real],
                },
            ],
            requests: vec![EvidenceRequest {
                what: "?".into(),
                discriminates: vec![7],
            }],
        };
        let errs = admit(p, |c| known.contains(&c)).unwrap_err();
        assert_eq!(
            errs,
            vec![
                Rejection::Uncited(0),
                Rejection::UnknownCitation(1, fake),
                Rejection::Unfalsifiable(2),
                Rejection::BadRequest(7)
            ]
        );
    }

    #[test]
    fn an_admitted_draft_becomes_a_surviving_hypothesis_never_a_claim() {
        let real = ClaimId::new();
        let p = Proposal {
            instance: instance(),
            hypotheses: vec![HypothesisDraft {
                statement: "lock contention".into(),
                predicts: vec![Outcome("lock waits".into())],
                falsifiers: vec![Outcome("no lock waits".into())],
                cites: vec![real],
            }],
            requests: vec![],
        };
        let admitted = admit(p, |c| c == real).unwrap();
        assert_eq!(admitted.hypotheses.len(), 1);
        assert_eq!(admitted.hypotheses[0].status, HypothesisStatus::Surviving);
    }
}
