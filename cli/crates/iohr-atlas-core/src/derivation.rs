//! Proof-carrying derivation (ADR 0011).
//!
//! A derived claim carries how it was derived: the rule at its version and the claims it
//! was derived from. Following inputs down to claims proven directly from evidence gives
//! the proof tree, which answers "why do you believe this?" down to observations. A
//! derived claim is never stronger than its weakest input, never more current than its
//! least current input, and is contradicted when an input is.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::claim::{Claim, ClaimClass, Conflict, Scope, Support, Validity};
use crate::evidence::EvidenceRef;
use crate::ids::ClaimId;
use crate::nonempty::NonEmpty;
use crate::observation::{Polarity, Statement};
use crate::proof::{RuleRef, Verification};
use crate::vocabulary::Value;

/// How a claim was derived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivation {
    rule: RuleRef,
    inputs: NonEmpty<ClaimId>,
}

impl Derivation {
    /// The rule.
    #[must_use]
    pub const fn rule(&self) -> &RuleRef {
        &self.rule
    }
    /// The claims it was derived from.
    #[must_use]
    pub fn inputs(&self) -> &[ClaimId] {
        self.inputs.as_slice()
    }
}

/// Finds claims by identity.
pub trait ClaimLookup {
    /// The claim with this identity, if known.
    fn claim(&self, id: ClaimId) -> Option<&Claim>;
}

impl ClaimLookup for BTreeMap<ClaimId, Claim> {
    fn claim(&self, id: ClaimId) -> Option<&Claim> {
        self.get(&id)
    }
}

/// Why a claim is believed, down to evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProofTree {
    /// Derived by `rule` from `inputs`.
    Derived {
        /// The derived claim.
        claim: ClaimId,
        /// The rule and version.
        rule: RuleRef,
        /// Why each input is believed.
        inputs: Vec<ProofTree>,
    },
    /// Proven directly from evidence.
    Base {
        /// The claim.
        claim: ClaimId,
        /// The evidence its proof evaluated.
        evidence: Vec<EvidenceRef>,
    },
}

impl ProofTree {
    /// Every leaf rests on evidence. A tree with an evidence-less leaf cannot be shown
    /// to a person as a reason (RFC 0086's acceptance criterion).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        match self {
            Self::Derived { inputs, .. } => inputs.iter().all(Self::is_complete),
            Self::Base { evidence, .. } => !evidence.is_empty(),
        }
    }

    /// The longest chain from this claim to evidence.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Derived { inputs, .. } => 1 + inputs.iter().map(Self::depth).max().unwrap_or(0),
            Self::Base { .. } => 0,
        }
    }
}

/// Why a proof tree could not be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TreeError {
    /// An input claim is not known.
    #[error("claim {0} is not known")]
    Missing(ClaimId),
    /// A claim is derived from itself, directly or through others.
    #[error("derivation cycle through {0:?}")]
    Cycle(Vec<ClaimId>),
}

/// The proof tree of `id`.
///
/// # Errors
/// An input is missing, or derivations form a cycle.
pub fn proof_tree(lookup: &impl ClaimLookup, id: ClaimId) -> Result<ProofTree, TreeError> {
    fn walk(
        lookup: &impl ClaimLookup,
        id: ClaimId,
        path: &mut Vec<ClaimId>,
    ) -> Result<ProofTree, TreeError> {
        if path.contains(&id) {
            let mut cycle = path.clone();
            cycle.push(id);
            return Err(TreeError::Cycle(cycle));
        }
        let claim = lookup.claim(id).ok_or(TreeError::Missing(id))?;
        match claim.derivation() {
            None => Ok(ProofTree::Base {
                claim: id,
                evidence: claim.evidence().to_vec(),
            }),
            Some(d) => {
                path.push(id);
                let inputs = d
                    .inputs()
                    .iter()
                    .map(|&i| walk(lookup, i, path))
                    .collect::<Result<Vec<_>, _>>();
                path.pop();
                Ok(ProofTree::Derived {
                    claim: id,
                    rule: d.rule().clone(),
                    inputs: inputs?,
                })
            }
        }
    }
    walk(lookup, id, &mut Vec::new())
}

/// Why two claims could not be chained.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DeriveError {
    /// Not two positive dependency claims on the same predicate.
    #[error("both inputs must be positive dependency claims with the same predicate")]
    WrongKind,
    /// The first claim's object is not the second claim's subject.
    #[error("the first claim's object is not the second claim's subject")]
    NotChained,
}

/// `dependency_transitivity@1`: from "A depends on B" and "B depends on C", derive "A
/// depends on C". The derived claim is verified only when both inputs are verified,
/// current and uncontested; otherwise it takes the weaker input's level, capped at
/// supported. It inherits staleness, expiry and contradiction from its inputs.
///
/// # Errors
/// The inputs are not chainable dependency claims.
pub fn derive_transitive(ab: &Claim, bc: &Claim, scope: Scope) -> Result<Claim, DeriveError> {
    for c in [ab, bc] {
        if c.class() != ClaimClass::Dependency || c.statement().polarity != Polarity::Holds {
            return Err(DeriveError::WrongKind);
        }
    }
    if ab.statement().predicate != bc.statement().predicate {
        return Err(DeriveError::WrongKind);
    }
    if ab.statement().value != Value::Entity(bc.statement().subject) {
        return Err(DeriveError::NotChained);
    }
    let rule = RuleRef {
        name: "dependency_transitivity".into(),
        version: 1,
    };
    let statement = Statement {
        subject: ab.statement().subject,
        predicate: ab.statement().predicate.clone(),
        value: bc.statement().value.clone(),
        polarity: Polarity::Holds,
    };
    let mut claim = Claim::new(statement, ClaimClass::Dependency, scope);
    let inputs = [ab, bc];
    claim.evidence = {
        let mut e: Vec<EvidenceRef> = inputs.iter().map(|c| EvidenceRef::Claim(c.id())).collect();
        e.sort_unstable();
        e
    };
    claim.derivation = Some(Derivation {
        rule: rule.clone(),
        inputs: NonEmpty::one(ab.id()).push(bc.id()),
    });

    let all_clean = inputs.iter().all(|c| {
        matches!(c.state().support(), Support::Verified(_))
            && matches!(c.state().validity(), Validity::Current)
            && matches!(c.state().conflict(), Conflict::None)
    });
    let weakest = inputs
        .iter()
        .map(|c| c.state().support().level())
        .min()
        .unwrap_or(0);
    let support = if all_clean {
        Support::Verified(Box::new(Verification::mint(rule, &claim, &claim.evidence)))
    } else {
        match weakest {
            0 => Support::None,
            1 => Support::Observed,
            _ => Support::Supported,
        }
    };
    claim.state.set_support(support);

    for c in inputs {
        match c.state().validity() {
            Validity::Current => {}
            Validity::Stale { since, change } => claim.state.mark_stale(*change, *since),
            Validity::Expired { at } => claim.state.expire(*at),
        }
        if matches!(c.state().conflict(), Conflict::Contradicted { .. }) {
            claim
                .state
                .contradict(&NonEmpty::one(EvidenceRef::Claim(c.id())));
        }
    }
    Ok(claim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::ContentDigest;
    use crate::evidence::EvidenceItem;
    use crate::ids::{ChangeId, EntityId, ObservationId};
    use crate::observer::{Health, ObserverClass};
    use crate::proof::{DependencyRule, EvidenceView, prove};
    use crate::vocabulary::PredicateRef;
    use std::collections::BTreeSet;

    fn scope() -> Scope {
        Scope {
            snapshot: ContentDigest::of_bytes(b"s1"),
            components: BTreeSet::new(),
        }
    }

    fn dep(a: EntityId, b: EntityId) -> Claim {
        Claim::new(
            Statement {
                subject: a,
                predicate: PredicateRef {
                    name: "depends_on".to_owned().try_into().unwrap(),
                    version: 1,
                },
                value: Value::Entity(b),
                polarity: Polarity::Holds,
            },
            ClaimClass::Dependency,
            scope(),
        )
    }

    fn det(class: ObserverClass) -> EvidenceItem {
        crate::testkit::item(class)
    }

    fn proven(a: EntityId, b: EntityId, independent_sources: usize) -> Claim {
        let mut c = dep(a, b);
        let items: Vec<_> = (0..independent_sources)
            .map(|_| det(ObserverClass::DirectSensor))
            .collect();
        let view = EvidenceView {
            items: &items,
            contrary: &[],
            coverage: None,
            health: Health::Healthy,
            authority: None,
            calibration: None,
        };
        prove(&DependencyRule, &mut c, &view);
        c
    }

    #[test]
    fn a_depends_on_d_with_its_full_proof_tree() {
        let [a, b, c, d] = [
            EntityId::new(),
            EntityId::new(),
            EntityId::new(),
            EntityId::new(),
        ];
        let ab = proven(a, b, 2); // e.g. an Envoy route and a runtime flow
        let bc = proven(b, c, 2); // an SDK span and a runtime flow
        let cd = proven(c, d, 2); // a Kubernetes manifest and a runtime flow
        let ac = derive_transitive(&ab, &bc, scope()).unwrap();
        let ad = derive_transitive(&ac, &cd, scope()).unwrap();
        assert!(matches!(ad.state().support(), Support::Verified(_)));
        assert_eq!(ad.statement().value, Value::Entity(d));

        let store: BTreeMap<_, _> = [ab, bc, cd, ac, ad.clone()]
            .into_iter()
            .map(|c| (c.id(), c))
            .collect();
        let tree = proof_tree(&store, ad.id()).unwrap();
        assert!(tree.is_complete(), "every leaf rests on evidence");
        assert_eq!(tree.depth(), 2);
        assert!(ad.integrity_holds());
    }

    #[test]
    fn a_derived_claim_is_never_stronger_than_its_weakest_input() {
        let [a, b, c] = [EntityId::new(), EntityId::new(), EntityId::new()];
        let ab = proven(a, b, 2);
        let bc = proven(b, c, 1); // supported only
        let ac = derive_transitive(&ab, &bc, scope()).unwrap();
        assert_eq!(ac.state().support(), &Support::Supported);
    }

    #[test]
    fn staleness_and_contradiction_flow_from_inputs_but_never_lower_support_by_themselves() {
        let [a, b, c] = [EntityId::new(), EntityId::new(), EntityId::new()];
        let ab = proven(a, b, 2);
        let mut bc = proven(b, c, 2);
        let change = ChangeId::new();
        bc.state_mut().mark_stale(change, chrono::Utc::now());
        let ac = derive_transitive(&ab, &bc, scope()).unwrap();
        assert!(matches!(ac.state().validity(), Validity::Stale { change: c, .. } if *c == change));
        // A stale input means the derivation can no longer be verified now: supported,
        // not unknown.
        assert_eq!(ac.state().support(), &Support::Supported);

        let mut bc2 = proven(b, c, 2);
        bc2.state_mut()
            .contradict(&NonEmpty::one(EvidenceRef::Observation(
                ObservationId::new(),
            )));
        let ac2 = derive_transitive(&ab, &bc2, scope()).unwrap();
        assert!(matches!(
            ac2.state().conflict(),
            Conflict::Contradicted { .. }
        ));
    }

    #[test]
    fn unchained_or_wrong_kind_inputs_are_refused() {
        let [a, b, c, x] = [
            EntityId::new(),
            EntityId::new(),
            EntityId::new(),
            EntityId::new(),
        ];
        assert_eq!(
            derive_transitive(&proven(a, b, 2), &proven(x, c, 2), scope()).unwrap_err(),
            DeriveError::NotChained
        );
    }

    #[test]
    fn missing_inputs_and_cycles_are_reported_not_looped() {
        let [a, b, c] = [EntityId::new(), EntityId::new(), EntityId::new()];
        let ab = proven(a, b, 2);
        let bc = proven(b, c, 2);
        let ac = derive_transitive(&ab, &bc, scope()).unwrap();
        let only_derived: BTreeMap<_, _> = [(ac.id(), ac.clone())].into_iter().collect();
        assert!(matches!(
            proof_tree(&only_derived, ac.id()),
            Err(TreeError::Missing(_))
        ));

        // Forge a cycle: a derived claim listed as its own input.
        let mut looped = ac.clone();
        looped.derivation = Some(Derivation {
            rule: RuleRef {
                name: "dependency_transitivity".into(),
                version: 1,
            },
            inputs: NonEmpty::one(looped.id()),
        });
        let store: BTreeMap<_, _> = [(looped.id(), looped.clone())].into_iter().collect();
        assert!(matches!(
            proof_tree(&store, looped.id()),
            Err(TreeError::Cycle(_))
        ));
    }
}
