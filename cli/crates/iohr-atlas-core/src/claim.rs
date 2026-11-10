//! Claims and their state on four independent axes (ADRs 0002, 0003, 0005).
//!
//! A claim's state is four fields, each answering one question:
//!
//! | Axis | Question | Set by |
//! |---|---|---|
//! | [`Support`] | how strongly is it proven? | proof rules only |
//! | [`Conflict`] | does credible evidence contradict it? | contradiction detection |
//! | [`Validity`] | does it still describe the current world? | changes to its scope |
//! | [`Observability`] | could we see it at all? | observer coverage and authority |
//!
//! So "verified and stale" and "supported and contradicted" are ordinary states. There is
//! no "unknown" claim (unknown is the absence of a claim) and no "underdetermined" claim
//! (that belongs to a [`crate::question::Question`]).
//!
//! Support can be raised or lowered only by [`crate::proof::prove`] and the derivation
//! rules in this crate: the setter is crate-private, so no other crate (and no model
//! integration) can mark a claim verified.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::derivation::Derivation;
use crate::evidence::EvidenceRef;
use crate::ids::{ChangeId, ClaimId};
use crate::nonempty::NonEmpty;
use crate::observation::Statement;
use crate::proof::Verification;
use crate::snapshot::{ComponentKey, SnapshotId};
use crate::time::Timestamp;

/// The kind of claim; proof rules are specific to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimClass {
    /// Something exists.
    Existence,
    /// Two names refer to one thing.
    Identity,
    /// One thing depends on another.
    Dependency,
    /// What a user experienced.
    UserOutcome,
    /// Something happened within a time bound.
    Temporal,
    /// Something did not happen, or is not there.
    Absence,
    /// One thing caused another.
    Causal,
    /// A security property.
    Security,
    /// A compliance control holds.
    ComplianceControl,
    /// An architectural invariant holds.
    ArchitecturalInvariant,
    /// A release has a property.
    ReleaseProperty,
}

/// What a claim holds in: a snapshot and the components it depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// The identity of the snapshot restricted to `components`.
    pub snapshot: SnapshotId,
    /// The components the claim depends on.
    pub components: BTreeSet<ComponentKey>,
}

/// How strongly a claim is proven. Ordered: none < observed < supported < verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum Support {
    /// No proof rule has accepted any evidence for it.
    None,
    /// Seen, but by evidence too weak to support it (a model's extraction alone).
    Observed,
    /// Evidence supports it, short of the strongest rule.
    Supported,
    /// A proof rule's strongest condition held, recorded with the rule and its version.
    Verified(Box<Verification>),
}

impl Support {
    /// The level as a number for comparison: 0 none, 1 observed, 2 supported, 3 verified.
    #[must_use]
    pub const fn level(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Observed => 1,
            Self::Supported => 2,
            Self::Verified(_) => 3,
        }
    }
}

/// Whether credible evidence contradicts a claim. Both sides are kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Conflict {
    /// No contradicting evidence.
    None,
    /// Contradicted by this evidence; the claim's own support is untouched.
    Contradicted {
        /// The contradicting evidence.
        by: NonEmpty<EvidenceRef>,
    },
}

/// Whether a claim still describes the current world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Validity {
    /// Nothing it depends on has changed.
    Current,
    /// Something it depends on changed. Not false: the last time it could be proven, it
    /// held (ADR 0003).
    Stale {
        /// When it went stale.
        since: Timestamp,
        /// The change that made it stale.
        change: ChangeId,
    },
    /// Past a validity window it declared (a certificate's expiry, a TTL).
    Expired {
        /// When it expired.
        at: Timestamp,
    },
}

/// Why something cannot be observed from where Atlas stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unobservable {
    /// No observer has the capability (no collector for it).
    Capability,
    /// An observer could, but lacks the authority.
    Authority,
    /// The environment prevents it (a third party's internals).
    Environment,
}

/// Whether a claim could be observed at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum Observability {
    /// Observers that can see it exist and have authority.
    Observable,
    /// Observable in principle, not observed yet.
    NotObserved,
    /// Cannot be observed from here, and why.
    NotObservable(Unobservable),
}

/// A claim's state on its four axes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimState {
    support: Support,
    conflict: Conflict,
    validity: Validity,
    observability: Observability,
}

impl Default for ClaimState {
    /// A new claim: unproven, uncontested, current, not observed yet.
    fn default() -> Self {
        Self {
            support: Support::None,
            conflict: Conflict::None,
            validity: Validity::Current,
            observability: Observability::NotObserved,
        }
    }
}

impl ClaimState {
    /// How strongly it is proven.
    #[must_use]
    pub const fn support(&self) -> &Support {
        &self.support
    }
    /// Whether it is contradicted.
    #[must_use]
    pub const fn conflict(&self) -> &Conflict {
        &self.conflict
    }
    /// Whether it still describes the current world.
    #[must_use]
    pub const fn validity(&self) -> &Validity {
        &self.validity
    }
    /// Whether it can be observed.
    #[must_use]
    pub const fn observability(&self) -> Observability {
        self.observability
    }

    /// Only proof and derivation rules in this crate change support.
    pub(crate) fn set_support(&mut self, support: Support) {
        self.support = support;
    }

    /// Something the claim depends on changed. Support is kept: stale is not false
    /// (ADR 0003). An expired claim stays expired.
    pub fn mark_stale(&mut self, change: ChangeId, since: Timestamp) {
        if matches!(self.validity, Validity::Current) {
            self.validity = Validity::Stale { since, change };
        }
    }

    /// The claim's declared validity window ended. Support is kept.
    pub fn expire(&mut self, at: Timestamp) {
        self.validity = Validity::Expired { at };
    }

    /// Credible evidence contradicts the claim. Support is kept, and the contradicting
    /// evidence accumulates: contradictions are never averaged away (ADR 0002).
    pub fn contradict(&mut self, by: &NonEmpty<EvidenceRef>) {
        let merged = match &self.conflict {
            Conflict::None => by.as_slice().to_vec(),
            Conflict::Contradicted { by: old } => {
                let mut all: BTreeSet<EvidenceRef> = old.as_slice().iter().copied().collect();
                all.extend(by.as_slice().iter().copied());
                all.into_iter().collect()
            }
        };
        // `merged` holds at least the new evidence, so it is never empty.
        if let Ok(by) = NonEmpty::new(merged) {
            self.conflict = Conflict::Contradicted { by };
        }
    }

    /// Record what observers can see.
    pub fn set_observability(&mut self, observability: Observability) {
        self.observability = observability;
    }
}

/// A typed statement about the world, scoped to a snapshot, with its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    id: ClaimId,
    statement: Statement,
    class: ClaimClass,
    scope: Scope,
    pub(crate) evidence: Vec<EvidenceRef>,
    pub(crate) state: ClaimState,
    pub(crate) derivation: Option<Derivation>,
    pub(crate) assessment: Option<crate::proof::Assessment>,
}

impl Claim {
    /// A new, unproven claim.
    #[must_use]
    pub fn new(statement: Statement, class: ClaimClass, scope: Scope) -> Self {
        Self {
            id: ClaimId::new(),
            statement,
            class,
            scope,
            evidence: Vec::new(),
            state: ClaimState::default(),
            derivation: None,
            assessment: None,
        }
    }

    /// The same claim under this identity: for a claim whose identity comes from its content
    /// ([`ClaimId::derived`]), so rebuilding it from the same input gives the same claim.
    #[must_use]
    pub const fn with_id(mut self, id: ClaimId) -> Self {
        self.id = id;
        self
    }

    /// Its identity.
    #[must_use]
    pub const fn id(&self) -> ClaimId {
        self.id
    }
    /// What it says.
    #[must_use]
    pub const fn statement(&self) -> &Statement {
        &self.statement
    }
    /// Its class.
    #[must_use]
    pub const fn class(&self) -> ClaimClass {
        self.class
    }
    /// Its scope.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }
    /// The evidence the last proof evaluated, sorted.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceRef] {
        &self.evidence
    }
    /// Its state.
    #[must_use]
    pub const fn state(&self) -> &ClaimState {
        &self.state
    }
    /// Its state, for the transitions any caller may make (stale, expire, contradict,
    /// observability). Support stays crate-private.
    pub const fn state_mut(&mut self) -> &mut ClaimState {
        &mut self.state
    }
    /// What the evidence behind its support cannot exclude, kept apart from the
    /// certificate's integrity (ADR 0017).
    #[must_use]
    pub const fn assessment(&self) -> Option<&crate::proof::Assessment> {
        self.assessment.as_ref()
    }
    /// How it was derived, if it is a derived claim.
    #[must_use]
    pub const fn derivation(&self) -> Option<&Derivation> {
        self.derivation.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn with_scope_for_test(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_statement_for_test(mut self, statement: Statement) -> Self {
        self.statement = statement;
        self
    }

    /// Check a claim read back from storage: a verified claim's certificate must bind
    /// to exactly this statement, class, scope and evidence. A certificate copied from
    /// another claim, or a claim edited after it was verified, fails.
    #[must_use]
    pub fn integrity_holds(&self) -> bool {
        match &self.state.support {
            Support::Verified(v) => v.binds(self),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{EntityId, ObservationId};
    use crate::observation::Polarity;
    use crate::vocabulary::{PredicateRef, Value};
    use chrono::Utc;
    use proptest::prelude::*;

    pub(crate) fn claim() -> Claim {
        Claim::new(
            Statement {
                subject: EntityId::new(),
                predicate: PredicateRef {
                    name: "exists".to_owned().try_into().unwrap(),
                    version: 1,
                },
                value: Value::Bool(true),
                polarity: Polarity::Holds,
            },
            ClaimClass::Existence,
            Scope {
                snapshot: crate::digest::ContentDigest::of_bytes(b"s1"),
                components: BTreeSet::new(),
            },
        )
    }

    #[test]
    fn a_new_claim_is_unproven_uncontested_current_and_not_observed() {
        let c = claim();
        assert_eq!(c.state().support(), &Support::None);
        assert_eq!(c.state().conflict(), &Conflict::None);
        assert_eq!(c.state().validity(), &Validity::Current);
        assert_eq!(c.state().observability(), Observability::NotObserved);
    }

    #[test]
    fn contradictions_accumulate_and_never_touch_support() {
        let mut s = ClaimState::default();
        s.set_support(Support::Supported);
        let a = EvidenceRef::Observation(ObservationId::new());
        let b = EvidenceRef::Observation(ObservationId::new());
        s.contradict(&NonEmpty::one(a));
        s.contradict(&NonEmpty::one(b));
        assert_eq!(s.support(), &Support::Supported);
        match s.conflict() {
            Conflict::Contradicted { by } => assert_eq!(by.len(), 2),
            Conflict::None => panic!("expected a contradiction"),
        }
    }

    #[test]
    fn expired_stays_expired_when_a_change_arrives() {
        let mut s = ClaimState::default();
        let t = Utc::now();
        s.expire(t);
        s.mark_stale(ChangeId::new(), t);
        assert_eq!(s.validity(), &Validity::Expired { at: t });
    }

    proptest! {
        // ADR 0003: no sequence of staleness, expiry, contradiction or observability
        // changes ever lowers support.
        #[test]
        fn support_survives_every_non_proof_transition(level in 0u8..3, ops in proptest::collection::vec(0u8..4, 0..30)) {
            let mut s = ClaimState::default();
            let support = match level { 0 => Support::None, 1 => Support::Observed, _ => Support::Supported };
            s.set_support(support.clone());
            for op in ops {
                match op {
                    0 => s.mark_stale(ChangeId::new(), Utc::now()),
                    1 => s.expire(Utc::now()),
                    2 => s.contradict(&NonEmpty::one(EvidenceRef::Observation(ObservationId::new()))),
                    _ => s.set_observability(Observability::NotObservable(Unobservable::Authority)),
                }
                prop_assert_eq!(s.support(), &support);
            }
        }
    }
}
