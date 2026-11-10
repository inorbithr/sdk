//! Questions, hypotheses and the next observation, without probabilities (ADR 0012).
//!
//! A question holds competing hypotheses. Each hypothesis states what it predicts and
//! what would falsify it. Evidence eliminates hypotheses; the survivors are what Atlas
//! does not yet know how to tell apart. "Underdetermined" is a property of a question
//! with more than one survivor, never of a claim.
//!
//! The next step is chosen by how many survivors it is guaranteed to eliminate (the
//! survivors minus the largest group that predicts the same outcome), then by risk and
//! cost. No likelihood is estimated: until calibration data exists, a number here would
//! be invented (RFC 0086, ranking without probabilities).

use std::cmp::Reverse;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::evidence::EvidenceRef;
use crate::ids::{AssumptionId, HypothesisId, QuestionId};
use crate::nonempty::NonEmpty;

/// An observable outcome a hypothesis predicts, e.g. "postgres lock waits under the
/// affected requests". Compared exactly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Outcome(pub String);

/// Whether a hypothesis survives the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HypothesisStatus {
    /// Not contradicted by any evidence yet.
    Surviving,
    /// Falsified by this evidence.
    Eliminated {
        /// The evidence that falsified it.
        by: EvidenceRef,
    },
}

/// A candidate explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hypothesis {
    /// Its identity.
    pub id: HypothesisId,
    /// What it says, in words.
    pub statement: String,
    /// What would be observed if it were true.
    pub predicts: Vec<Outcome>,
    /// What would show it false. At least one: a hypothesis nothing can falsify is not
    /// admitted.
    pub falsifiers: NonEmpty<Outcome>,
    /// What it takes for granted.
    pub assumptions: Vec<AssumptionId>,
    /// Whether it survives.
    pub status: HypothesisStatus,
}

/// A question and its hypotheses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Its identity.
    pub id: QuestionId,
    /// The question, e.g. "why is checkout slow?".
    pub text: String,
    /// The hypotheses, in the order they were proposed.
    pub hypotheses: Vec<Hypothesis>,
}

/// How far the evidence settles a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Determinacy {
    /// No hypotheses proposed yet.
    Open,
    /// Every hypothesis was eliminated: none of the proposed explanations is right.
    AllEliminated,
    /// Exactly one survives. Determined among those proposed, not proven true.
    OneSurvivor(HypothesisId),
    /// More than one survives.
    Underdetermined(Vec<HypothesisId>),
}

impl Question {
    /// The surviving hypotheses.
    pub fn survivors(&self) -> impl Iterator<Item = &Hypothesis> {
        self.hypotheses
            .iter()
            .filter(|h| h.status == HypothesisStatus::Surviving)
    }

    /// How far the evidence settles it.
    #[must_use]
    pub fn determinacy(&self) -> Determinacy {
        if self.hypotheses.is_empty() {
            return Determinacy::Open;
        }
        let survivors: Vec<HypothesisId> = self.survivors().map(|h| h.id).collect();
        match survivors.len() {
            0 => Determinacy::AllEliminated,
            1 => Determinacy::OneSurvivor(survivors[0]),
            _ => Determinacy::Underdetermined(survivors),
        }
    }

    /// Eliminate a hypothesis because `by` showed one of its falsifiers. Elimination is
    /// permanent for this question; if the evidence is later retracted, the question is
    /// re-opened as a new record, not edited.
    ///
    /// # Errors
    /// The hypothesis is not in this question.
    pub fn eliminate(&mut self, id: HypothesisId, by: EvidenceRef) -> Result<(), NotInQuestion> {
        let h = self
            .hypotheses
            .iter_mut()
            .find(|h| h.id == id)
            .ok_or(NotInQuestion(id))?;
        if h.status == HypothesisStatus::Surviving {
            h.status = HypothesisStatus::Eliminated { by };
        }
        Ok(())
    }
}

/// A hypothesis is not part of the question.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("hypothesis {0} is not in this question")]
pub struct NotInQuestion(pub HypothesisId);

/// An ordinal level for cost and risk. No numbers: they would claim a precision nobody
/// has measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Negligible.
    None,
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
}

/// Something Atlas could do next: query stored data, enable a collector, run an
/// experiment, ask a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discriminator {
    /// What it is, e.g. "observe postgres wait events".
    pub name: String,
    /// What it costs.
    pub cost: Level,
    /// What it risks.
    pub risk: Level,
    /// Whether the authority to do it is held now.
    pub authority: bool,
    /// The outcome it would show under each surviving hypothesis.
    pub predicted: BTreeMap<HypothesisId, Outcome>,
}

/// A ranked candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    /// The candidate's name.
    pub name: String,
    /// Survivors it eliminates whatever the outcome.
    pub guaranteed: usize,
}

/// Rank candidates for the next step on `question`. A candidate that cannot change the
/// survivor set (it predicts the same outcome under every survivor, or does not cover
/// them all) is dropped: collecting it would teach nothing. Order: authority held first,
/// then most survivors guaranteed eliminated, then lower risk, then lower cost, then
/// name, so the result is deterministic.
#[must_use]
pub fn rank_discriminators(question: &Question, candidates: &[Discriminator]) -> Vec<Ranked> {
    let survivors: Vec<HypothesisId> = question.survivors().map(|h| h.id).collect();
    if survivors.len() < 2 {
        return Vec::new();
    }
    let mut scored: Vec<(&Discriminator, usize)> = candidates
        .iter()
        .filter_map(|c| {
            let mut groups: BTreeMap<&Outcome, usize> = BTreeMap::new();
            for h in &survivors {
                *groups.entry(c.predicted.get(h)?).or_default() += 1;
            }
            let largest = groups.values().copied().max().unwrap_or(0);
            let guaranteed = survivors.len() - largest;
            (guaranteed > 0).then_some((c, guaranteed))
        })
        .collect();
    scored.sort_by_key(|(c, g)| {
        (
            Reverse(c.authority),
            Reverse(*g),
            c.risk,
            c.cost,
            c.name.clone(),
        )
    });
    scored
        .into_iter()
        .map(|(c, guaranteed)| Ranked {
            name: c.name.clone(),
            guaranteed,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ObservationId;

    fn hyp(statement: &str, falsifier: &str) -> Hypothesis {
        Hypothesis {
            id: HypothesisId::new(),
            statement: statement.into(),
            predicts: vec![],
            falsifiers: NonEmpty::one(Outcome(falsifier.into())),
            assumptions: vec![],
            status: HypothesisStatus::Surviving,
        }
    }

    fn o(s: &str) -> Outcome {
        Outcome(s.into())
    }

    /// The owner's example: why is checkout slow?
    #[test]
    fn checkout_is_slow_eliminate_then_pick_the_cheapest_discriminator() {
        let lock = hyp(
            "H17 database lock contention",
            "no lock waits under affected requests",
        );
        let storage = hyp(
            "H18 storage latency",
            "storage latency normal in the interval",
        );
        let upstream = hyp("H21 upstream latency", "upstream duration normal");
        let mut q = Question {
            id: QuestionId::new(),
            text: "Why is checkout slow?".into(),
            hypotheses: vec![lock.clone(), storage.clone(), upstream.clone()],
        };
        assert!(matches!(q.determinacy(), Determinacy::Underdetermined(ref s) if s.len() == 3));

        // Existing evidence: the gateway's duration was normal. H21 is eliminated.
        q.eliminate(upstream.id, EvidenceRef::Observation(ObservationId::new()))
            .unwrap();
        assert!(matches!(q.determinacy(), Determinacy::Underdetermined(ref s) if s.len() == 2));

        let waits = Discriminator {
            name: "observe postgres wait events".into(),
            cost: Level::Low,
            risk: Level::None,
            authority: true,
            predicted: BTreeMap::from([
                (lock.id, o("lock waits")),
                (storage.id, o("no lock waits")),
            ]),
        };
        let block_io = Discriminator {
            name: "enable block I/O attribution".into(),
            cost: Level::Medium,
            risk: Level::Low,
            authority: true,
            predicted: BTreeMap::from([(lock.id, o("io normal")), (storage.id, o("io slow"))]),
        };
        let dns = Discriminator {
            name: "enable DNS collector".into(),
            cost: Level::None,
            risk: Level::None,
            authority: true,
            predicted: BTreeMap::from([(lock.id, o("normal")), (storage.id, o("normal"))]),
        };
        let ranked = rank_discriminators(&q, &[block_io, dns, waits]);
        let names: Vec<_> = ranked.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "observe postgres wait events",
                "enable block I/O attribution"
            ],
            "DNS teaches nothing and is dropped"
        );
    }

    #[test]
    fn a_candidate_without_authority_ranks_after_ones_with_it() {
        let a = hyp("A", "x");
        let b = hyp("B", "y");
        let q = Question {
            id: QuestionId::new(),
            text: "?".into(),
            hypotheses: vec![a.clone(), b.clone()],
        };
        let mk = |name: &str, authority| Discriminator {
            name: name.into(),
            cost: Level::None,
            risk: Level::None,
            authority,
            predicted: BTreeMap::from([(a.id, o("1")), (b.id, o("2"))]),
        };
        let ranked = rank_discriminators(&q, &[mk("needs a grant", false), mk("allowed", true)]);
        assert_eq!(ranked[0].name, "allowed");
    }

    #[test]
    fn determinacy_covers_open_one_and_none() {
        let mut q = Question {
            id: QuestionId::new(),
            text: "?".into(),
            hypotheses: vec![],
        };
        assert_eq!(q.determinacy(), Determinacy::Open);
        let h = hyp("only", "z");
        q.hypotheses.push(h.clone());
        assert_eq!(q.determinacy(), Determinacy::OneSurvivor(h.id));
        q.eliminate(h.id, EvidenceRef::Observation(ObservationId::new()))
            .unwrap();
        assert_eq!(q.determinacy(), Determinacy::AllEliminated);
    }
}
