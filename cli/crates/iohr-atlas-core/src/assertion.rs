//! The words Atlas may use (ADR 0018).
//!
//! "Service recovered", "incident fixed" and "root cause eliminated" are three different
//! statements; tools that blur them claim more than they proved. Atlas speaks only in
//! this closed vocabulary, and each word is produced from state, never chosen freely.

use std::fmt;

use crate::claim::{Claim, Conflict, Support, Validity};

/// A word Atlas may use about a claim, a cause, a change, a fix or a recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Assertion {
    /// No proof rule accepted any evidence.
    Unproven,
    /// Seen, by evidence too weak to support it.
    Observed,
    /// Supported, short of verified.
    Supported,
    /// A proof rule's strongest condition held, in its scope and snapshot.
    Verified,
    /// Credible evidence contradicts it.
    Contradicted,
    /// Something it depends on changed since it was proven.
    Stale,
    /// Past its declared validity.
    Expired,
    /// A cause is supported, not yet verified.
    CauseSupported,
    /// A cause is verified for its scope.
    CauseVerified,
    /// A change is proposed, not applied.
    ChangeProposed,
    /// A change is applied, not yet verified.
    ChangeApplied,
    /// The failure was not seen after the change; not yet verified.
    FixObserved,
    /// The fix is verified for every part of its required scope.
    FixVerified,
    /// The fix is verified for part of its scope; the rest is named.
    FixVerifiedForScope,
    /// The symptom went away; nothing about the cause.
    RecoveryObserved,
    /// Recovery is verified; still nothing about the cause.
    RecoveryVerified,
}

impl fmt::Display for Assertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unproven => "UNPROVEN",
            Self::Observed => "OBSERVED",
            Self::Supported => "SUPPORTED",
            Self::Verified => "VERIFIED",
            Self::Contradicted => "CONTRADICTED",
            Self::Stale => "STALE",
            Self::Expired => "EXPIRED",
            Self::CauseSupported => "CAUSE SUPPORTED",
            Self::CauseVerified => "CAUSE VERIFIED",
            Self::ChangeProposed => "CHANGE PROPOSED",
            Self::ChangeApplied => "CHANGE APPLIED",
            Self::FixObserved => "FIX OBSERVED",
            Self::FixVerified => "FIX VERIFIED",
            Self::FixVerifiedForScope => "FIX VERIFIED FOR SCOPE",
            Self::RecoveryObserved => "RECOVERY OBSERVED",
            Self::RecoveryVerified => "RECOVERY VERIFIED",
        })
    }
}

/// Everything Atlas may say about a claim, in order: its support, then every
/// qualification that applies. A verified claim that went stale reads "VERIFIED, STALE",
/// never "VERIFIED" alone.
#[must_use]
pub fn of_claim(claim: &Claim) -> Vec<Assertion> {
    let state = claim.state();
    let mut out = vec![match state.support() {
        Support::None => Assertion::Unproven,
        Support::Observed => Assertion::Observed,
        Support::Supported => Assertion::Supported,
        Support::Verified(_) => Assertion::Verified,
    }];
    if matches!(state.conflict(), Conflict::Contradicted { .. }) {
        out.push(Assertion::Contradicted);
    }
    match state.validity() {
        Validity::Current => {}
        Validity::Stale { .. } => out.push(Assertion::Stale),
        Validity::Expired { .. } => out.push(Assertion::Expired),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::{ClaimClass, Scope};
    use crate::digest::ContentDigest;
    use crate::evidence::EvidenceRef;
    use crate::ids::{ChangeId, EntityId, ObservationId};
    use crate::nonempty::NonEmpty;
    use crate::observation::{Polarity, Statement};
    use crate::observer::{Health, ObserverClass};
    use crate::proof::{EvidenceView, ExistenceRule, prove};
    use crate::testkit::item;
    use crate::vocabulary::{PredicateRef, Value};

    fn verified() -> Claim {
        let mut c = Claim::new(
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
                snapshot: ContentDigest::of_bytes(b"s"),
                components: std::collections::BTreeSet::new(),
            },
        );
        let items = [item(ObserverClass::DirectSensor)];
        let v = EvidenceView {
            items: &items,
            contrary: &[],
            coverage: None,
            health: Health::Healthy,
            authority: None,
            calibration: None,
        };
        prove(&ExistenceRule, &mut c, &v);
        c
    }

    #[test]
    fn a_stale_or_contradicted_claim_is_never_described_as_plainly_verified() {
        let mut c = verified();
        assert_eq!(of_claim(&c), vec![Assertion::Verified]);
        c.state_mut()
            .mark_stale(ChangeId::new(), chrono::Utc::now());
        assert_eq!(of_claim(&c), vec![Assertion::Verified, Assertion::Stale]);
        c.state_mut()
            .contradict(&NonEmpty::one(EvidenceRef::Observation(
                ObservationId::new(),
            )));
        assert_eq!(
            of_claim(&c),
            vec![
                Assertion::Verified,
                Assertion::Contradicted,
                Assertion::Stale
            ]
        );
    }

    #[test]
    fn recovery_fix_and_cause_are_distinct_words() {
        let words = [
            Assertion::RecoveryObserved,
            Assertion::RecoveryVerified,
            Assertion::FixObserved,
            Assertion::FixVerified,
            Assertion::FixVerifiedForScope,
            Assertion::CauseSupported,
            Assertion::CauseVerified,
        ];
        let text: std::collections::BTreeSet<String> =
            words.iter().map(ToString::to_string).collect();
        assert_eq!(text.len(), words.len());
        assert!(
            text.iter()
                .filter(|t| t.starts_with("RECOVERY"))
                .all(|t| !t.contains("FIX") && !t.contains("CAUSE"))
        );
    }
}

/// ADR 0018 (claim 0018.1, `docs/adrs/proofs.json`): Atlas speaks only the
/// ADR's words. The words are read from the ADR's own text, so the code and
/// the decision cannot drift apart; the exhaustive match makes a new variant
/// a compile error here until it is listed.
#[cfg(test)]
mod adr_0018 {
    use super::Assertion::{self, *};

    const ALL: [Assertion; 16] = [
        Unproven,
        Observed,
        Supported,
        Verified,
        Contradicted,
        Stale,
        Expired,
        CauseSupported,
        CauseVerified,
        ChangeProposed,
        ChangeApplied,
        FixObserved,
        FixVerified,
        FixVerifiedForScope,
        RecoveryObserved,
        RecoveryVerified,
    ];

    // Fails to compile when a variant is added and not listed in ALL.
    const fn listed(a: Assertion) -> bool {
        match a {
            Unproven | Observed | Supported | Verified | Contradicted | Stale | Expired
            | CauseSupported | CauseVerified | ChangeProposed | ChangeApplied | FixObserved
            | FixVerified | FixVerifiedForScope | RecoveryObserved | RecoveryVerified => true,
        }
    }

    /// ADR 0018's list, in the ADR's words (the decision lives in InOrbit's Decisions; the
    /// words are pinned here so the published crate cannot drift from them).
    const ADR_0018_WORDS: &str = "UNPROVEN, OBSERVED, SUPPORTED, VERIFIED, CONTRADICTED, STALE, EXPIRED, \
         CAUSE SUPPORTED, CAUSE VERIFIED, CHANGE PROPOSED, CHANGE APPLIED, FIX OBSERVED, \
         FIX VERIFIED, FIX VERIFIED FOR SCOPE, RECOVERY OBSERVED, RECOVERY VERIFIED";

    #[test]
    fn the_words_are_exactly_the_adrs() {
        let mut adr: Vec<String> = ADR_0018_WORDS
            .split(',')
            .map(|w| w.trim().to_owned())
            .collect();
        let mut code: Vec<String> = ALL
            .iter()
            .inspect(|a| assert!(listed(**a)))
            .map(ToString::to_string)
            .collect();
        adr.sort();
        code.sort();
        assert_eq!(code, adr, "Atlas's words differ from ADR 0018's");
    }
}
