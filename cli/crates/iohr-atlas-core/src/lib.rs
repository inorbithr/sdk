//! `iohr-atlas-core`: Atlas core (RFC 0086, ADR 0023): the deterministic types under Atlas.
//!
//! Stored truth is claims and evidence. Everything else (the world graph, intent,
//! contradictions, staleness) is derived from them. Models propose; this crate decides.
//! What evidence *is* (typed ids, content digests, observed time, methods, observers,
//! observations, artefacts, extractions, testimony and snapshots) and what evidence
//! *proves* (claims, proof rules, certificates, derivation) live in one crate, so a
//! program that wants to check a proof needs nothing from InOrbit's platform.
//!
//! Invariants enforced here, by type or by test:
//!
//! 1. A claim's state is four independent axes: support, conflict, validity,
//!    observability ([`claim`], ADR 0002).
//! 2. Stale is not false: no change to validity, conflict or observability lowers
//!    support ([`claim::ClaimState`], ADR 0003).
//! 3. Today's knowledge never rewrites yesterday's: records are bitemporal and closed,
//!    never edited ([`time::Bitemporal`], ADR 0004).
//! 4. Only proof rules in this crate set support, and the rule trait is sealed
//!    ([`proof`], ADRs 0005, 0007).
//! 5. A model measures nothing: artefacts are observed by digest of their bytes, a
//!    model's reading is an extraction, and extraction alone never verifies
//!    ([`observation`], [`proof`], ADR 0006).
//! 6. Absence fails closed ([`proof::AbsenceRule`], ADR 0010).
//! 7. Corroboration counts independent failure-mode groups, not sources
//!    ([`evidence`], ADR 0009).
//! 8. Every derived claim carries its proof ([`derivation`], ADR 0011).
//! 9. Hypotheses are ranked by elimination, never by invented probabilities
//!    ([`question`], ADR 0012).
//! 10. A reasoner's output is a proposal; nothing turns it into truth ([`proposal`]).
//! 11. An id of one kind is not an id of another ([`ids`]).
//! 12. Equal content has an equal digest, whatever the build order ([`digest`], [`snapshot`]).
//! 13. Observations come only from instruments and deterministic readers; people testify
//!     and models interpret ([`observer::ObserverClass`], [`method::MethodCategory`]).
//! 14. Absence needs complete coverage: a drop, a parse failure or a gap makes coverage
//!     incomplete ([`observer::Coverage`]).
//!
//! The ones that are guarantees of the type system, not only of tests:
//!
//! ```compile_fail,E0624
//! // No crate outside iohr-atlas-core can set a claim's support.
//! let mut c: iohr_atlas_core::claim::Claim = todo!();
//! c.state_mut().set_support(iohr_atlas_core::claim::Support::Supported);
//! ```
//!
//! ```compile_fail,E0451
//! // A verification cannot be built outside the proof rules.
//! let v = iohr_atlas_core::proof::Verification { rule: todo!(), binding: todo!() };
//! ```
//!
//! ```compile_fail,E0277
//! // No crate outside iohr-atlas-core can implement a proof rule.
//! struct Mine;
//! impl iohr_atlas_core::proof::ProofRule for Mine {
//!     fn rule(&self) -> iohr_atlas_core::proof::RuleRef { todo!() }
//!     fn evaluate(&self, _: &iohr_atlas_core::claim::Claim, _: &iohr_atlas_core::proof::EvidenceView<'_>)
//!         -> iohr_atlas_core::proof::ProofOutcome { todo!() }
//! }
//! ```
//!
//! ```compile_fail,E0308
//! // An id of one kind is not an id of another.
//! fn takes_claim(_: iohr_atlas_core::ids::ClaimId) {}
//! takes_claim(iohr_atlas_core::ids::ObservationId::new());
//! ```

pub mod assertion;
pub mod claim;
pub mod derivation;
pub mod digest;
pub mod evidence;
pub mod ids;
pub mod method;
pub mod nonempty;
pub mod observation;
pub mod observer;
pub mod proof;
pub mod proposal;
pub mod question;
pub mod snapshot;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod time;
pub mod vocabulary;
