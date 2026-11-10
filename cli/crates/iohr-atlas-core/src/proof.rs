//! Proof rules and verification (ADRs 0007, 0010).
//!
//! A proof rule is a deterministic function from a claim and an evidence view to an
//! outcome. Rules are named and versioned (`existence@1`); a verified claim records the
//! rule that verified it. The trait is sealed: only rules in this crate exist, so no
//! other crate can mint a verification. [`prove`] is the only function that writes a
//! claim's support.
//!
//! A [`Verification`] carries a binding digest over the rule, the claim's statement,
//! class, scope and the evidence evaluated. It is an integrity check, not a secret: it
//! catches a certificate copied to another claim or a claim edited after verification
//! (see [`crate::claim::Claim::integrity_holds`]). Who may write to the store is the
//! store's access control, and no model holds that access (ADR 0005).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::claim::{Claim, ClaimClass, Support};
use crate::digest::ContentDigest;
use crate::evidence::{EvidenceItem, EvidenceRef, independence_groups};
use crate::observation::Polarity;
use crate::observer::{AuthorityContext, Calibration, Coverage, Health, ObserverClass, Ratio};

/// A rule's name and version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RuleRef {
    /// Its name, e.g. `existence`.
    pub name: String,
    /// Its version. A new version re-evaluates what the old one verified.
    pub version: u32,
}

impl fmt::Display for RuleRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// The proof certificate: the record that a rule verified a claim, bound to exactly
/// what it verified. Change the claim, its evidence, the rule or its version, the
/// predicate's version, the snapshot or the scope, and it no longer holds (ADR 0007).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    rule: RuleRef,
    predicate: crate::vocabulary::PredicateRef,
    snapshot: crate::snapshot::SnapshotId,
    claim_hash: ContentDigest,
    evidence_root: ContentDigest,
    issued_at: crate::time::Timestamp,
    binding: ContentDigest,
}

impl Verification {
    /// The rule that verified it.
    #[must_use]
    pub const fn rule(&self) -> &RuleRef {
        &self.rule
    }
    /// The predicate, at the version it was verified under.
    #[must_use]
    pub const fn predicate(&self) -> &crate::vocabulary::PredicateRef {
        &self.predicate
    }
    /// The snapshot it holds in.
    #[must_use]
    pub const fn snapshot(&self) -> crate::snapshot::SnapshotId {
        self.snapshot
    }
    /// The digest of the claim's identity, statement, class and scope.
    #[must_use]
    pub const fn claim_hash(&self) -> ContentDigest {
        self.claim_hash
    }
    /// The digest of the sorted evidence it evaluated.
    #[must_use]
    pub const fn evidence_root(&self) -> ContentDigest {
        self.evidence_root
    }
    /// When it was issued.
    #[must_use]
    pub const fn issued_at(&self) -> crate::time::Timestamp {
        self.issued_at
    }
    /// The digest over all of the above: what a future seal would anchor.
    #[must_use]
    pub const fn binding(&self) -> ContentDigest {
        self.binding
    }

    /// Whether this certificate belongs to exactly this claim as it is now, and was not
    /// altered after it was issued.
    #[must_use]
    pub fn binds(&self, claim: &Claim) -> bool {
        self.predicate == claim.statement().predicate
            && self.snapshot == claim.scope().snapshot
            && self.claim_hash == claim_hash(claim)
            && self.evidence_root == evidence_root(claim.evidence())
            && self.binding == self.compute_binding()
    }

    pub(crate) fn mint(rule: RuleRef, claim: &Claim, evidence: &[EvidenceRef]) -> Self {
        let mut v = Self {
            rule,
            predicate: claim.statement().predicate.clone(),
            snapshot: claim.scope().snapshot,
            claim_hash: claim_hash(claim),
            evidence_root: evidence_root(evidence),
            issued_at: chrono::Utc::now(),
            binding: ContentDigest::of_bytes(b""),
        };
        v.binding = v.compute_binding();
        v
    }

    fn compute_binding(&self) -> ContentDigest {
        #[derive(Serialize)]
        struct Bound<'a> {
            rule: &'a RuleRef,
            predicate: &'a crate::vocabulary::PredicateRef,
            snapshot: crate::snapshot::SnapshotId,
            claim_hash: ContentDigest,
            evidence_root: ContentDigest,
            issued_at: crate::time::Timestamp,
        }
        let b = Bound {
            rule: &self.rule,
            predicate: &self.predicate,
            snapshot: self.snapshot,
            claim_hash: self.claim_hash,
            evidence_root: self.evidence_root,
            issued_at: self.issued_at,
        };
        canonical(&b)
    }
}

/// The digest of a claim's identity, statement, class and scope. The identity is in it
/// on purpose: two claims that say the same thing are still two claims, and a
/// certificate never moves between them.
fn claim_hash(claim: &Claim) -> ContentDigest {
    #[derive(Serialize)]
    struct C<'a> {
        id: crate::ids::ClaimId,
        statement: &'a crate::observation::Statement,
        class: ClaimClass,
        scope: &'a crate::claim::Scope,
    }
    canonical(&C {
        id: claim.id(),
        statement: claim.statement(),
        class: claim.class(),
        scope: claim.scope(),
    })
}

/// The digest of the sorted, de-duplicated evidence references.
fn evidence_root(evidence: &[EvidenceRef]) -> ContentDigest {
    let mut sorted = evidence.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    canonical(&sorted)
}

fn canonical<T: Serialize>(value: &T) -> ContentDigest {
    // These values contain only string-keyed maps, so encoding cannot fail; a library
    // never panics, so a failure would yield a digest no certificate can match.
    ContentDigest::of_canonical(value)
        .unwrap_or_else(|_| ContentDigest::of_bytes(b"atlas:unencodable"))
}

/// Why a rule did not reach its strongest outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Shortfall {
    /// The rule does not apply to this claim's class or polarity.
    WrongKind,
    /// No evidence at all.
    NoEvidence,
    /// Only a model's extraction; it interprets, it does not measure.
    OnlyExtraction,
    /// Only a person's testimony.
    OnlyTestimony,
    /// Not enough independent groups of deterministic evidence.
    TooFewIndependentSources {
        /// Groups present.
        have: usize,
        /// Groups needed.
        need: usize,
    },
    /// The observer did not cover the whole scope and window.
    CoverageIncomplete,
    /// The observer was not healthy.
    ObserverUnhealthy,
    /// The observer's authority did not cover the scope.
    AuthorityInsufficient,
    /// No calibration on record for this predicate.
    CalibrationMissing,
    /// Calibrated recall below the rule's threshold.
    RecallBelowThreshold,
    /// Evidence of the thing claimed absent exists.
    ContraryEvidence,
    /// Evidence exists, but none of it is of a kind the predicate's contract accepts.
    NoProvingMethod,
}

/// The outcome of evaluating a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofOutcome {
    /// The rule's strongest condition held.
    Verified,
    /// Supported, with what fell short of verified.
    Supported(Vec<Shortfall>),
    /// Seen only by weak evidence.
    Observed(Vec<Shortfall>),
    /// Not even observed.
    Insufficient(Vec<Shortfall>),
}

/// What a rule may look at.
#[derive(Debug, Clone, Copy)]
pub struct EvidenceView<'a> {
    /// The evidence for the claim.
    pub items: &'a [EvidenceItem],
    /// Evidence found against it (for absence: any sighting of the absent thing).
    pub contrary: &'a [EvidenceItem],
    /// The observer's coverage over the claim's scope and window, for absence.
    pub coverage: Option<&'a Coverage>,
    /// The observer's health during that window.
    pub health: Health,
    /// The observer's authority.
    pub authority: Option<&'a AuthorityContext>,
    /// The observer's calibration for the claim's predicate.
    pub calibration: Option<&'a Calibration>,
}

mod sealed {
    pub trait Sealed {}
}

/// A deterministic proof rule. Sealed: implemented only in this crate.
pub trait ProofRule: sealed::Sealed {
    /// Its name and version.
    fn rule(&self) -> RuleRef;
    /// Evaluate, without side effects.
    fn evaluate(&self, claim: &Claim, view: &EvidenceView<'_>) -> ProofOutcome;
}

/// What a verification could not exclude. A certificate proves that Atlas applied a
/// proof contract correctly to this evidence; it does not prove the evidence honest
/// (ADR 0017). These are recorded next to it, always.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlindSpot {
    /// Every proof rests on its observers being honest; a compromised observer can
    /// produce evidence that satisfies any rule. Always present.
    ObserverIntegrityAssumed,
    /// All the evidence shares one failure domain: one compromise explains all of it.
    SingleTrustDomain,
    /// Some evidence was sampled: what was not sampled was not seen.
    SampledCoverage,
    /// Some evidence can be edited after the fact.
    MutableEvidence,
    /// A model's interpretation is part of the evidence.
    InterpretationInvolved,
    /// A person's testimony is part of the evidence.
    TestimonyInvolved,
}

/// The epistemic assessment of the evidence a rule evaluated, kept apart from the
/// certificate's integrity: what the evidence was, and what it cannot rule out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assessment {
    /// Independent failure-mode groups among the evidence.
    pub independent_groups: usize,
    /// What the evidence cannot rule out.
    pub blind_spots: std::collections::BTreeSet<BlindSpot>,
}

/// Assess the evidence in `view`.
#[must_use]
pub fn assess(view: &EvidenceView<'_>) -> Assessment {
    use crate::method::{CoverageModel, MethodCategory, Trust};
    let groups = independence_groups(view.items).len();
    let mut blind = std::collections::BTreeSet::from([BlindSpot::ObserverIntegrityAssumed]);
    if groups <= 1 {
        blind.insert(BlindSpot::SingleTrustDomain);
    }
    for i in view.items {
        if matches!(
            i.method.coverage,
            CoverageModel::Sampled { .. } | CoverageModel::BestEffort
        ) {
            blind.insert(BlindSpot::SampledCoverage);
        }
        if i.method.trust == Trust::Mutable {
            blind.insert(BlindSpot::MutableEvidence);
        }
        match i.method.category {
            MethodCategory::ModelInterpretation => {
                blind.insert(BlindSpot::InterpretationInvolved);
            }
            MethodCategory::HumanTestimony => {
                blind.insert(BlindSpot::TestimonyInvolved);
            }
            _ => {}
        }
    }
    Assessment {
        independent_groups: groups,
        blind_spots: blind,
    }
}

/// Evaluate `rule` and record the result on `claim`: its evidence becomes the view's
/// items, and its support becomes the outcome's level. The only writer of support for
/// observed claims.
pub fn prove(rule: &dyn ProofRule, claim: &mut Claim, view: &EvidenceView<'_>) -> ProofOutcome {
    let outcome = rule.evaluate(claim, view);
    let mut evidence: Vec<EvidenceRef> = view.items.iter().map(|i| i.reference).collect();
    evidence.sort_unstable();
    evidence.dedup();
    claim.evidence = evidence;
    claim.assessment = Some(assess(view));
    let support = match &outcome {
        ProofOutcome::Verified => Support::Verified(Box::new(Verification::mint(
            rule.rule(),
            claim,
            &claim.evidence,
        ))),
        ProofOutcome::Supported(_) => Support::Supported,
        ProofOutcome::Observed(_) => Support::Observed,
        ProofOutcome::Insufficient(_) => Support::None,
    };
    claim.state.set_support(support);
    outcome
}

/// Classify evidence into deterministic, extraction-only and testimony-only.
fn sources(items: &[EvidenceItem]) -> (Vec<&EvidenceItem>, bool, bool) {
    let deterministic: Vec<&EvidenceItem> = items
        .iter()
        .filter(|i| i.class.is_deterministic())
        .collect();
    let extraction = items
        .iter()
        .any(|i| i.class == ObserverClass::ModelExtractor);
    let testimony = items
        .iter()
        .any(|i| i.class == ObserverClass::HumanTestimony);
    (deterministic, extraction, testimony)
}

/// The independent groups that contain at least one deterministic item.
fn deterministic_groups(items: &[EvidenceItem]) -> usize {
    independence_groups(items)
        .iter()
        .filter(|g| g.iter().any(|&i| items[i].class.is_deterministic()))
        .count()
}

fn weak_outcome(extraction: bool, testimony: bool) -> ProofOutcome {
    match (testimony, extraction) {
        (true, _) => ProofOutcome::Supported(vec![Shortfall::OnlyTestimony]),
        (false, true) => ProofOutcome::Observed(vec![Shortfall::OnlyExtraction]),
        (false, false) => ProofOutcome::Insufficient(vec![Shortfall::NoEvidence]),
    }
}

/// `existence@1`: something exists. One deterministic observation verifies it.
/// Testimony alone supports it; extraction alone only observes it.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExistenceRule;
impl sealed::Sealed for ExistenceRule {}
impl ProofRule for ExistenceRule {
    fn rule(&self) -> RuleRef {
        RuleRef {
            name: "existence".into(),
            version: 1,
        }
    }
    fn evaluate(&self, claim: &Claim, view: &EvidenceView<'_>) -> ProofOutcome {
        if claim.class() != ClaimClass::Existence || claim.statement().polarity != Polarity::Holds {
            return ProofOutcome::Insufficient(vec![Shortfall::WrongKind]);
        }
        let (deterministic, extraction, testimony) = sources(view.items);
        if deterministic.is_empty() {
            weak_outcome(extraction, testimony)
        } else {
            ProofOutcome::Verified
        }
    }
}

/// `dependency@1`: one thing depends on another. Verified by deterministic evidence in at
/// least two independent groups (configuration and runtime, say); supported by one;
/// extraction or testimony alone never verifies it.
#[derive(Debug, Clone, Copy, Default)]
pub struct DependencyRule;
impl sealed::Sealed for DependencyRule {}
impl ProofRule for DependencyRule {
    fn rule(&self) -> RuleRef {
        RuleRef {
            name: "dependency".into(),
            version: 1,
        }
    }
    fn evaluate(&self, claim: &Claim, view: &EvidenceView<'_>) -> ProofOutcome {
        if claim.class() != ClaimClass::Dependency || claim.statement().polarity != Polarity::Holds
        {
            return ProofOutcome::Insufficient(vec![Shortfall::WrongKind]);
        }
        let (deterministic, extraction, testimony) = sources(view.items);
        if deterministic.is_empty() {
            return weak_outcome(extraction, testimony);
        }
        let groups = deterministic_groups(view.items);
        if groups >= 2 {
            ProofOutcome::Verified
        } else {
            ProofOutcome::Supported(vec![Shortfall::TooFewIndependentSources {
                have: groups,
                need: 2,
            }])
        }
    }
}

/// `absence@1`: something is absent over a scope and window. Fails closed: verified only
/// when a deterministic observer covered the whole scope and window, stayed healthy, had
/// sufficient authority, is calibrated for the predicate with recall at or above the
/// threshold, and nothing contrary was seen. Anything less is insufficient; extraction
/// and testimony never count, because absence cannot be read from text.
#[derive(Debug, Clone, Copy)]
pub struct AbsenceRule {
    /// The minimum calibrated recall.
    pub recall_threshold: Ratio,
}
impl sealed::Sealed for AbsenceRule {}
impl ProofRule for AbsenceRule {
    fn rule(&self) -> RuleRef {
        RuleRef {
            name: "absence".into(),
            version: 1,
        }
    }
    fn evaluate(&self, claim: &Claim, view: &EvidenceView<'_>) -> ProofOutcome {
        if claim.statement().polarity != Polarity::Absent {
            return ProofOutcome::Insufficient(vec![Shortfall::WrongKind]);
        }
        let mut short = Vec::new();
        if !view.contrary.is_empty() {
            short.push(Shortfall::ContraryEvidence);
        }
        let (deterministic, extraction, testimony) = sources(view.items);
        if deterministic.is_empty() {
            short.push(match (extraction, testimony) {
                (true, _) => Shortfall::OnlyExtraction,
                (false, true) => Shortfall::OnlyTestimony,
                (false, false) => Shortfall::NoEvidence,
            });
        }
        if !view.coverage.is_some_and(Coverage::is_complete) {
            short.push(Shortfall::CoverageIncomplete);
        }
        if view.health != Health::Healthy {
            short.push(Shortfall::ObserverUnhealthy);
        }
        if !view.authority.is_some_and(|a| a.sufficient) {
            short.push(Shortfall::AuthorityInsufficient);
        }
        match view.calibration {
            None => short.push(Shortfall::CalibrationMissing),
            Some(c) if c.predicate != claim.statement().predicate => {
                short.push(Shortfall::CalibrationMissing);
            }
            Some(c) if !c.recall.at_least(&self.recall_threshold) => {
                short.push(Shortfall::RecallBelowThreshold);
            }
            Some(_) => {}
        }
        if short.is_empty() {
            ProofOutcome::Verified
        } else {
            ProofOutcome::Insufficient(short)
        }
    }
}

/// A predicate's proof contract: which kinds of evidence can prove a claim with this
/// predicate, which can only support it, and how many independent proving sources
/// verification needs (ADR 0016). "Knowing" is defined per predicate, never globally.
///
/// Examples: `calls` is proven by static resolution (a compiler or language server),
/// in any language; `connects_to` is proven by runtime observation or an application
/// trace and supported by configuration or source; prose, text matches, model
/// interpretation and testimony never prove anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofContract {
    predicate: crate::vocabulary::PredicateRef,
    version: u32,
    proves: std::collections::BTreeSet<Requirement>,
    supports: std::collections::BTreeSet<crate::method::MethodCategory>,
    independent_for_verify: usize,
}

/// One way a piece of evidence can prove a claim: a category, seen with at least this
/// coverage. "Runtime observation with complete coverage" is a requirement; "logs" alone
/// is not precise enough when one connector samples and another keeps everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Requirement {
    /// The category.
    pub category: crate::method::MethodCategory,
    /// The least coverage accepted.
    pub min_coverage: crate::method::CoverageModel,
}

impl Requirement {
    /// Any coverage.
    #[must_use]
    pub const fn any(category: crate::method::MethodCategory) -> Self {
        Self {
            category,
            min_coverage: crate::method::CoverageModel::BestEffort,
        }
    }

    /// Complete coverage only.
    #[must_use]
    pub const fn complete(category: crate::method::MethodCategory) -> Self {
        Self {
            category,
            min_coverage: crate::method::CoverageModel::Complete,
        }
    }

    /// Whether a method meets this requirement.
    #[must_use]
    pub fn met_by(&self, method: &crate::method::EvidenceMethod) -> bool {
        method.category == self.category && method.coverage >= self.min_coverage
    }
}

/// Why a proof contract was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    /// No category can prove it: nothing could ever verify such a claim.
    #[error("a contract needs at least one proving category")]
    NothingProves,
    /// A category that may never prove (prose, text search, interpretation, testimony)
    /// was listed as proving.
    #[error("{0:?} can never prove a claim")]
    WeakCategoryProves(crate::method::MethodCategory),
    /// Verification needs at least one independent proving source.
    #[error("verification needs at least one independent proving source")]
    ZeroSources,
}

impl ProofContract {
    /// A contract for `predicate` at `version`.
    ///
    /// # Errors
    /// Nothing proves, a weak category is listed as proving, or `independent_for_verify`
    /// is zero.
    pub fn new(
        predicate: crate::vocabulary::PredicateRef,
        version: u32,
        proves: std::collections::BTreeSet<Requirement>,
        supports: std::collections::BTreeSet<crate::method::MethodCategory>,
        independent_for_verify: usize,
    ) -> Result<Self, ContractError> {
        if proves.is_empty() {
            return Err(ContractError::NothingProves);
        }
        if let Some(weak) = proves.iter().find(|r| !r.category.may_prove()) {
            return Err(ContractError::WeakCategoryProves(weak.category));
        }
        if independent_for_verify == 0 {
            return Err(ContractError::ZeroSources);
        }
        Ok(Self {
            predicate,
            version,
            proves,
            supports,
            independent_for_verify,
        })
    }

    /// The predicate it governs.
    #[must_use]
    pub const fn predicate(&self) -> &crate::vocabulary::PredicateRef {
        &self.predicate
    }

    /// The rule a claim verified under this contract records: `contract.<predicate>@<v>`.
    #[must_use]
    pub fn rule_ref(&self) -> RuleRef {
        RuleRef {
            name: format!("contract.{}", self.predicate.name.as_str()),
            version: self.version,
        }
    }
}

/// Evaluates a positive claim against its predicate's [`ProofContract`].
#[derive(Debug, Clone, Copy)]
pub struct ContractRule<'c> {
    /// The contract.
    pub contract: &'c ProofContract,
}
impl sealed::Sealed for ContractRule<'_> {}
impl ProofRule for ContractRule<'_> {
    fn rule(&self) -> RuleRef {
        self.contract.rule_ref()
    }
    fn evaluate(&self, claim: &Claim, view: &EvidenceView<'_>) -> ProofOutcome {
        let c = self.contract;
        if claim.statement().predicate != c.predicate
            || claim.statement().polarity != Polarity::Holds
        {
            return ProofOutcome::Insufficient(vec![Shortfall::WrongKind]);
        }
        if view.items.is_empty() {
            return ProofOutcome::Insufficient(vec![Shortfall::NoEvidence]);
        }
        let proving_groups = independence_groups(view.items)
            .iter()
            .filter(|g| {
                g.iter()
                    .any(|&i| c.proves.iter().any(|r| r.met_by(&view.items[i].method)))
            })
            .count();
        if proving_groups >= c.independent_for_verify {
            return ProofOutcome::Verified;
        }
        let supports = view.items.iter().any(|i| {
            c.proves.iter().any(|r| r.category == i.method.category)
                || c.supports.contains(&i.method.category)
        });
        let short = vec![Shortfall::TooFewIndependentSources {
            have: proving_groups,
            need: c.independent_for_verify,
        }];
        if supports {
            ProofOutcome::Supported(short)
        } else {
            ProofOutcome::Observed(vec![Shortfall::NoProvingMethod])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::{Scope, Validity};
    use crate::ids::{EntityId, ObservationId};
    use crate::observation::Statement;
    use crate::time::Interval;
    use crate::vocabulary::{PredicateRef, Value};
    use chrono::{TimeZone as _, Utc};
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    fn pred(name: &str) -> PredicateRef {
        PredicateRef {
            name: name.to_owned().try_into().unwrap(),
            version: 1,
        }
    }

    fn claim(class: ClaimClass, predicate: &str, polarity: Polarity) -> Claim {
        Claim::new(
            Statement {
                subject: EntityId::new(),
                predicate: pred(predicate),
                value: Value::Bool(true),
                polarity,
            },
            class,
            Scope {
                snapshot: ContentDigest::of_bytes(b"s1"),
                components: BTreeSet::new(),
            },
        )
    }

    use crate::testkit::item;

    fn view(items: &[EvidenceItem]) -> EvidenceView<'_> {
        EvidenceView {
            items,
            contrary: &[],
            coverage: None,
            health: Health::Healthy,
            authority: None,
            calibration: None,
        }
    }

    #[test]
    fn existence_one_measurement_verifies_and_records_the_rule() {
        let mut c = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        let items = [item(ObserverClass::DirectSensor)];
        assert_eq!(
            prove(&ExistenceRule, &mut c, &view(&items)),
            ProofOutcome::Verified
        );
        match c.state().support() {
            Support::Verified(v) => assert_eq!(v.rule().to_string(), "existence@1"),
            other => panic!("expected verified, got {other:?}"),
        }
        assert!(c.integrity_holds());
    }

    #[test]
    fn extraction_alone_only_observes_and_testimony_alone_only_supports() {
        let mut c = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut c,
            &view(&[item(ObserverClass::ModelExtractor)]),
        );
        assert_eq!(c.state().support(), &Support::Observed);
        prove(
            &ExistenceRule,
            &mut c,
            &view(&[item(ObserverClass::HumanTestimony)]),
        );
        assert_eq!(c.state().support(), &Support::Supported);
    }

    #[test]
    fn dependency_needs_two_independent_deterministic_groups() {
        let mut c = claim(ClaimClass::Dependency, "depends_on", Polarity::Holds);
        let one = [item(ObserverClass::DeterministicExtractor)];
        assert!(matches!(
            prove(&DependencyRule, &mut c, &view(&one)),
            ProofOutcome::Supported(_)
        ));
        let two = [
            item(ObserverClass::DeterministicExtractor),
            item(ObserverClass::DirectSensor),
        ];
        assert_eq!(
            prove(&DependencyRule, &mut c, &view(&two)),
            ProofOutcome::Verified
        );
        // The same two, sharing a kernel: one group, back to supported.
        let mode = crate::evidence::FailureMode::Kernel {
            host: "h".into(),
            boot: "b".into(),
        };
        let shared = [
            item(ObserverClass::DeterministicExtractor).with_failure_mode(mode.clone()),
            item(ObserverClass::DirectSensor).with_failure_mode(mode),
        ];
        assert!(matches!(
            prove(&DependencyRule, &mut c, &view(&shared)),
            ProofOutcome::Supported(_)
        ));
    }

    #[test]
    fn a_hundred_extractions_never_verify_a_dependency() {
        let mut c = claim(ClaimClass::Dependency, "depends_on", Polarity::Holds);
        let many: Vec<_> = (0..100)
            .map(|_| item(ObserverClass::ModelExtractor))
            .collect();
        assert!(matches!(
            prove(&DependencyRule, &mut c, &view(&many)),
            ProofOutcome::Observed(_)
        ));
    }

    fn at(s: i64) -> crate::time::Timestamp {
        Utc.timestamp_opt(1_800_000_000 + s, 0).unwrap()
    }

    /// The conditions absence@1 requires. The rule must verify only when none fails.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Cond {
        CompleteCoverage,
        HealthyObserver,
        SufficientAuthority,
        Calibrated,
        RecallAboveThreshold,
        DeterministicEvidence,
        NothingContrary,
    }

    const ALL: [Cond; 7] = [
        Cond::CompleteCoverage,
        Cond::HealthyObserver,
        Cond::SufficientAuthority,
        Cond::Calibrated,
        Cond::RecallAboveThreshold,
        Cond::DeterministicEvidence,
        Cond::NothingContrary,
    ];

    fn absence(failing: &BTreeSet<Cond>) -> ProofOutcome {
        let holds = |c: Cond| !failing.contains(&c);
        let mut c = claim(ClaimClass::Absence, "net.egress", Polarity::Absent);
        let window = Interval::closed(at(0), at(100)).unwrap();
        let coverage = Coverage {
            expected: window,
            actual: Some(window),
            received: 5,
            dropped: u64::from(!holds(Cond::CompleteCoverage)),
            parse_failures: 0,
        };
        let authority = AuthorityContext {
            principal: "agent".into(),
            capabilities: vec![],
            sufficient: holds(Cond::SufficientAuthority),
        };
        let recall = if holds(Cond::RecallAboveThreshold) {
            (999, 1000)
        } else {
            (900, 1000)
        };
        let calibration = Calibration {
            predicate: pred("net.egress"),
            recall: Ratio::new(recall.0, recall.1).unwrap(),
            precision: Ratio::new(1, 1).unwrap(),
            measured_in: ContentDigest::of_bytes(b"s1"),
            measured_at: at(0),
        };
        let class = if holds(Cond::DeterministicEvidence) {
            ObserverClass::DirectSensor
        } else {
            ObserverClass::ModelExtractor
        };
        let items = [item(class)];
        let contrary = [item(ObserverClass::DirectSensor)];
        let v = EvidenceView {
            items: &items,
            contrary: if holds(Cond::NothingContrary) {
                &[]
            } else {
                &contrary
            },
            coverage: Some(&coverage),
            health: if holds(Cond::HealthyObserver) {
                Health::Healthy
            } else {
                Health::Degraded
            },
            authority: Some(&authority),
            calibration: if holds(Cond::Calibrated) {
                Some(&calibration)
            } else {
                None
            },
        };
        let rule = AbsenceRule {
            recall_threshold: Ratio::new(995, 1000).unwrap(),
        };
        prove(&rule, &mut c, &v)
    }

    #[test]
    fn absence_verifies_only_when_everything_holds() {
        assert_eq!(absence(&BTreeSet::new()), ProofOutcome::Verified);
    }

    #[test]
    fn absence_fails_closed_for_each_single_missing_condition() {
        for c in ALL {
            let out = absence(&BTreeSet::from([c]));
            assert!(
                matches!(out, ProofOutcome::Insufficient(_)),
                "{c:?} missing gave {out:?}"
            );
        }
    }

    proptest! {
        // ADR 0010: absence fails closed. Any combination of missing conditions leaves
        // the claim unproven, never supported or observed.
        #[test]
        fn absence_fails_closed(failing in proptest::collection::btree_set(proptest::sample::select(ALL.to_vec()), 0..=7)) {
            let out = absence(&failing);
            if failing.is_empty() {
                prop_assert_eq!(out, ProofOutcome::Verified);
            } else {
                prop_assert!(matches!(out, ProofOutcome::Insufficient(_)), "got {:?}", out);
            }
        }

        // No mix of extractions and testimony ever verifies any rule's claim.
        #[test]
        fn interpretation_and_testimony_never_verify(n_ext in 0usize..20, n_tst in 0usize..20) {
            let items: Vec<_> = (0..n_ext).map(|_| item(ObserverClass::ModelExtractor))
                .chain((0..n_tst).map(|_| item(ObserverClass::HumanTestimony))).collect();
            let mut e = claim(ClaimClass::Existence, "exists", Polarity::Holds);
            let mut d = claim(ClaimClass::Dependency, "depends_on", Polarity::Holds);
            prop_assert_ne!(prove(&ExistenceRule, &mut e, &view(&items)), ProofOutcome::Verified);
            prop_assert_ne!(prove(&DependencyRule, &mut d, &view(&items)), ProofOutcome::Verified);
        }
    }

    mod contracts {
        use super::*;
        use crate::method::MethodCategory as M;
        use crate::testkit::{item_with, method};

        fn contract(predicate: &str, proves: &[M], supports: &[M], need: usize) -> ProofContract {
            ProofContract::new(
                pred(predicate),
                1,
                proves.iter().map(|&c| Requirement::any(c)).collect(),
                supports.iter().copied().collect(),
                need,
            )
            .unwrap()
        }

        fn calls() -> ProofContract {
            contract("calls", &[M::StaticResolution], &[M::SourceConstruction], 1)
        }

        fn connects_to() -> ProofContract {
            contract(
                "connects_to",
                &[M::RuntimeObservation, M::ApplicationTrace],
                &[M::Configuration, M::SourceConstruction],
                1,
            )
        }

        fn judge(
            contract: &ProofContract,
            predicate: &str,
            items: &[EvidenceItem],
        ) -> (ProofOutcome, Claim) {
            let mut c = claim(ClaimClass::Dependency, predicate, Polarity::Holds);
            let out = prove(&ContractRule { contract }, &mut c, &view(items));
            (out, c)
        }

        #[test]
        fn calls_is_proven_by_static_resolution_in_any_language() {
            let k = calls();
            for tool in [
                "rust_analyzer.resolve",
                "go_types.resolve",
                "typescript.checker",
                "jdt.resolve",
            ] {
                let (out, c) = judge(
                    &k,
                    "calls",
                    &[item_with(
                        ObserverClass::DeterministicExtractor,
                        method(tool, M::StaticResolution),
                    )],
                );
                assert_eq!(out, ProofOutcome::Verified, "{tool}");
                match c.state().support() {
                    Support::Verified(v) => assert_eq!(v.rule().to_string(), "contract.calls@1"),
                    other => panic!("{other:?}"),
                }
            }
        }

        #[test]
        fn text_search_or_a_model_never_proves_a_call() {
            let k = calls();
            let grep = item_with(
                ObserverClass::DeterministicExtractor,
                method("grep.match", M::TextSearch),
            );
            let model = item_with(
                ObserverClass::ModelExtractor,
                method("model.read", M::ModelInterpretation),
            );
            let (out, _) = judge(&k, "calls", &[grep, model]);
            assert!(matches!(out, ProofOutcome::Observed(_)), "got {out:?}");
        }

        #[test]
        fn connects_to_moves_from_supported_to_verified_when_runtime_corroborates() {
            let k = connects_to();
            let env = item_with(
                ObserverClass::DeterministicExtractor,
                method("k8s.manifest.env", M::Configuration),
            );
            let client = item_with(
                ObserverClass::DeterministicExtractor,
                method("rust_analyzer.construct", M::SourceConstruction),
            );
            let model = item_with(
                ObserverClass::ModelExtractor,
                method("model.read", M::ModelInterpretation),
            );
            let (before, _) = judge(
                &k,
                "connects_to",
                &[env.clone(), client.clone(), model.clone()],
            );
            assert!(
                matches!(before, ProofOutcome::Supported(_)),
                "source and config support, they do not prove: {before:?}"
            );
            let socket = item_with(
                ObserverClass::DirectSensor,
                method("ebpf.socket", M::RuntimeObservation),
            );
            let (after, c) = judge(&k, "connects_to", &[env, client, model, socket]);
            assert_eq!(after, ProofOutcome::Verified);
            assert!(c.integrity_holds());
        }

        #[test]
        fn a_new_connector_needs_no_core_change() {
            // An enterprise's own APM, unknown to the core, declares a trace method.
            let apm = item_with(
                ObserverClass::ExternalSystem,
                method("acme_apm.span", M::ApplicationTrace),
            );
            let (out, _) = judge(&connects_to(), "connects_to", &[apm]);
            assert_eq!(out, ProofOutcome::Verified);
            let grafana = item_with(
                ObserverClass::ExternalSystem,
                method("grafana.query", M::Metrics),
            );
            let (out, _) = judge(&connects_to(), "connects_to", &[grafana]);
            assert!(
                matches!(out, ProofOutcome::Observed(_)),
                "metrics are not in this contract: {out:?}"
            );
        }

        #[test]
        fn a_contract_cannot_let_weak_evidence_prove() {
            for weak in [
                M::Documentation,
                M::TextSearch,
                M::ModelInterpretation,
                M::HumanTestimony,
            ] {
                let r = ProofContract::new(
                    pred("calls"),
                    1,
                    [Requirement::any(weak)].into(),
                    BTreeSet::new(),
                    1,
                );
                assert_eq!(r, Err(ContractError::WeakCategoryProves(weak)));
            }
            assert_eq!(
                ProofContract::new(pred("calls"), 1, BTreeSet::new(), BTreeSet::new(), 1),
                Err(ContractError::NothingProves)
            );
            assert_eq!(
                ProofContract::new(
                    pred("calls"),
                    1,
                    [Requirement::any(M::StaticResolution)].into(),
                    BTreeSet::new(),
                    0
                ),
                Err(ContractError::ZeroSources)
            );
        }

        #[test]
        fn independence_is_required_when_the_contract_asks_for_it() {
            let k = contract(
                "connects_to",
                &[M::RuntimeObservation, M::ApplicationTrace],
                &[],
                2,
            );
            let kernel = crate::evidence::FailureMode::Kernel {
                host: "h".into(),
                boot: "b".into(),
            };
            let ebpf = item_with(
                ObserverClass::DirectSensor,
                method("ebpf.socket", M::RuntimeObservation),
            )
            .with_failure_mode(kernel.clone());
            let sdk = item_with(
                ObserverClass::DirectSensor,
                method("sdk.span", M::ApplicationTrace),
            )
            .with_failure_mode(kernel);
            let (out, _) = judge(&k, "connects_to", &[ebpf.clone(), sdk]);
            assert!(
                matches!(out, ProofOutcome::Supported(_)),
                "same host and boot is one source: {out:?}"
            );
            let db = item_with(
                ObserverClass::ExternalSystem,
                method("postgres.pg_stat_activity", M::RuntimeObservation),
            );
            let (out, _) = judge(&k, "connects_to", &[ebpf, db]);
            assert_eq!(out, ProofOutcome::Verified);
        }

        #[test]
        fn sampled_logs_support_but_only_a_complete_audit_log_proves() {
            use crate::method::CoverageModel;
            use crate::testkit::method_with;
            let k = ProofContract::new(
                pred("accessed_by"),
                1,
                [Requirement::complete(M::Logs)].into(),
                BTreeSet::new(),
                1,
            )
            .unwrap();
            let sampled = item_with(
                ObserverClass::ExternalSystem,
                method_with(
                    "loki.logs.search",
                    M::Logs,
                    CoverageModel::Sampled { per_mille: 100 },
                ),
            );
            let (out, _) = judge(&k, "accessed_by", &[sampled]);
            assert!(
                matches!(out, ProofOutcome::Supported(_)),
                "10% sampled logs support, they do not prove: {out:?}"
            );
            let audit = item_with(
                ObserverClass::ExternalSystem,
                method_with("cloudtrail.audit", M::Logs, CoverageModel::Complete),
            );
            let (out, _) = judge(&k, "accessed_by", &[audit]);
            assert_eq!(out, ProofOutcome::Verified);
        }

        #[test]
        fn a_contract_governs_only_its_own_predicate() {
            let (out, _) = judge(
                &calls(),
                "connects_to",
                &[item_with(
                    ObserverClass::DeterministicExtractor,
                    method("rust_analyzer.resolve", M::StaticResolution),
                )],
            );
            assert_eq!(out, ProofOutcome::Insufficient(vec![Shortfall::WrongKind]));
        }

        proptest! {
            // Whatever a valid contract says, evidence made only of prose, text matches,
            // interpretation and testimony never verifies.
            #[test]
            fn weak_evidence_never_verifies_under_any_contract(
                proves in proptest::collection::btree_set(proptest::sample::select(vec![M::StaticResolution, M::RuntimeObservation, M::ApplicationTrace, M::Metrics, M::Logs, M::Configuration, M::SourceConstruction, M::ExternalRecord]), 1..5),
                supports in proptest::collection::btree_set(proptest::sample::select(vec![M::Documentation, M::TextSearch, M::ModelInterpretation, M::HumanTestimony, M::Configuration]), 0..5),
                need in 1usize..3,
                n in 1usize..12,
            ) {
                let k = ProofContract::new(pred("connects_to"), 1, proves.into_iter().map(Requirement::any).collect(), supports, need).unwrap();
                let items: Vec<_> = (0..n).map(|i| match i % 4 {
                    0 => item_with(ObserverClass::DeterministicExtractor, method("readme.read", M::Documentation)),
                    1 => item_with(ObserverClass::DeterministicExtractor, method("grep.match", M::TextSearch)),
                    2 => item_with(ObserverClass::ModelExtractor, method("model.read", M::ModelInterpretation)),
                    _ => item_with(ObserverClass::HumanTestimony, method("person.answer", M::HumanTestimony)),
                }).collect();
                let (out, _) = judge(&k, "connects_to", &items);
                prop_assert_ne!(out, ProofOutcome::Verified);
            }
        }
    }

    #[test]
    fn a_certificate_copied_to_another_claim_or_a_claim_edited_after_proof_fails_integrity() {
        let mut a = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        let mut b = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut a,
            &view(&[item(ObserverClass::DirectSensor)]),
        );
        let stolen = a.state().support().clone();
        // Same rule, same evidence: only the binding to the claim itself can tell them
        // apart, so this is what proves the certificate is not transferable.
        b.evidence = a.evidence.clone();
        b.state.set_support(stolen);
        assert!(
            !b.integrity_holds(),
            "a certificate must not transfer to another claim"
        );
        let mut edited = a.clone();
        edited
            .evidence
            .push(EvidenceRef::Observation(ObservationId::new()));
        assert!(
            !edited.integrity_holds(),
            "editing the evidence after proof must break the binding"
        );
        assert!(a.integrity_holds());
    }

    #[test]
    fn a_certificate_never_moves_even_to_an_identical_twin() {
        // Same statement, class, scope and evidence: only the identity differs, so only
        // the identity in the claim hash can refuse the transfer.
        let mut a = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut a,
            &view(&[item(ObserverClass::DirectSensor)]),
        );
        let mut twin = Claim::new(a.statement().clone(), a.class(), a.scope().clone());
        twin.evidence = a.evidence.clone();
        twin.state.set_support(a.state().support().clone());
        assert_ne!(twin.id(), a.id());
        assert!(
            !twin.integrity_holds(),
            "Proof(A, E) is not Proof(B, E), even when A and B say the same thing"
        );
        assert!(a.integrity_holds());
    }

    #[test]
    fn a_certificate_breaks_when_any_bound_field_changes() {
        use crate::claim::Scope;
        let mut a = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut a,
            &view(&[item(ObserverClass::DirectSensor)]),
        );
        assert!(a.integrity_holds());

        // Another snapshot.
        let moved = a.clone().with_scope_for_test(Scope {
            snapshot: ContentDigest::of_bytes(b"s2"),
            components: BTreeSet::new(),
        });
        assert!(!moved.integrity_holds(), "snapshot");
        // The predicate at another version.
        let mut st = a.statement().clone();
        st.predicate.version = 2;
        assert!(
            !a.clone().with_statement_for_test(st).integrity_holds(),
            "predicate version"
        );
        // Evidence removed.
        let mut fewer = a.clone();
        fewer.evidence.clear();
        assert!(!fewer.integrity_holds(), "evidence");
    }

    #[test]
    fn tampering_with_the_stored_certificate_is_detected() {
        let mut a = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut a,
            &view(&[item(ObserverClass::DirectSensor)]),
        );
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"version\":1"));
        for (from, to) in [
            (
                "\"name\":\"existence\",\"version\":1",
                "\"name\":\"existence\",\"version\":9",
            ),
            ("\"name\":\"existence\"", "\"name\":\"dependency\""),
        ] {
            assert!(json.contains(from), "{from}");
            let forged: Claim = serde_json::from_str(&json.replacen(from, to, 1)).unwrap();
            assert!(
                !forged.integrity_holds(),
                "editing {from} must break the certificate"
            );
        }
        let back: Claim = serde_json::from_str(&json).unwrap();
        assert!(
            back.integrity_holds(),
            "an untouched round trip still holds"
        );
    }

    #[test]
    fn a_verified_claim_still_names_what_it_cannot_exclude() {
        let mut c = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut c,
            &view(&[item(ObserverClass::DirectSensor)]),
        );
        assert!(matches!(c.state().support(), Support::Verified(_)));
        assert!(c.integrity_holds(), "the certificate is intact");
        let a = c.assessment().unwrap();
        assert!(
            a.blind_spots.contains(&BlindSpot::ObserverIntegrityAssumed),
            "integrity is not truth"
        );
        assert!(
            a.blind_spots.contains(&BlindSpot::SingleTrustDomain),
            "one observer, one domain"
        );
    }

    #[test]
    fn two_independent_domains_remove_the_single_domain_blind_spot_but_never_the_integrity_one() {
        let mut c = claim(ClaimClass::Dependency, "depends_on", Polarity::Holds);
        prove(
            &DependencyRule,
            &mut c,
            &view(&[
                item(ObserverClass::DirectSensor),
                item(ObserverClass::DeterministicExtractor),
            ]),
        );
        let a = c.assessment().unwrap();
        assert_eq!(a.independent_groups, 2);
        assert!(!a.blind_spots.contains(&BlindSpot::SingleTrustDomain));
        assert!(a.blind_spots.contains(&BlindSpot::ObserverIntegrityAssumed));
    }

    #[test]
    fn stale_keeps_the_verification() {
        let mut c = claim(ClaimClass::Existence, "exists", Polarity::Holds);
        prove(
            &ExistenceRule,
            &mut c,
            &view(&[item(ObserverClass::DirectSensor)]),
        );
        c.state_mut()
            .mark_stale(crate::ids::ChangeId::new(), at(10));
        assert!(matches!(c.state().support(), Support::Verified(_)));
        assert!(matches!(c.state().validity(), Validity::Stale { .. }));
        assert!(c.integrity_holds());
    }
}
