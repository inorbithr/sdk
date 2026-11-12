//! Typed identities (ADR 0013 in inorbithr/core).
//!
//! Every record has an identity that cannot be confused with another kind's: a claim's
//! id is not an observation id, even though both are `UUIDv7` underneath. The kind is a
//! zero-sized marker, so the type checker rejects `fn f(c: ClaimId)` called with an
//! `ObservationId`. `UUIDv7` sorts by creation time, which keeps bitemporal rows in order.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

/// A kind of record that has an [`Id`]. Implemented only by the markers in this module:
/// the supertrait is crate-private, so no other crate can add a kind.
#[allow(private_bounds)] // the seal is the point
pub trait Kind: sealed::Sealed {
    /// The prefix shown in text form, e.g. `clm` for claims.
    const PREFIX: &'static str;
}

mod sealed {
    pub(crate) trait Sealed {}
}

/// An identity of a record of kind `K`.
pub struct Id<K: Kind> {
    raw: Uuid,
    kind: PhantomData<fn() -> K>,
}

impl<K: Kind> Id<K> {
    /// A new identity, ordered by creation time (`UUIDv7`).
    #[must_use]
    pub fn new() -> Self {
        Self::from_uuid(Uuid::now_v7())
    }

    /// The identity with this UUID, for records read back from the store.
    #[must_use]
    pub const fn from_uuid(raw: Uuid) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.raw
    }

    /// An identity derived from content: the same `parts` always give the same identity, so
    /// a record rebuilt from the same input is the same record and a store can tell it has it
    /// already. Each part is length-prefixed, so `["ab", "c"]` and `["a", "bc"]` differ, and
    /// the kind's prefix is mixed in, so a claim and an artefact built from the same parts do
    /// not collide. The identity is a version 8 UUID (RFC 9562, custom bytes) of the first 16
    /// bytes of a SHA-256; it does not sort by time like [`Id::new`].
    #[must_use]
    pub fn derived(parts: &[&[u8]]) -> Self {
        let mut h = Sha256::new();
        h.update(K::PREFIX.as_bytes());
        h.update([0]);
        for p in parts {
            h.update((p.len() as u64).to_be_bytes());
            h.update(p);
        }
        let digest = h.finalize();
        let mut raw = [0_u8; 16];
        raw.copy_from_slice(&digest[..16]);
        Self::from_uuid(uuid::Builder::from_custom_bytes(raw).into_uuid())
    }
}

impl<K: Kind> Default for Id<K> {
    fn default() -> Self {
        Self::new()
    }
}

// Manual impls: derives would require `K: Clone` etc., which markers need not be.
impl<K: Kind> Clone for Id<K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<K: Kind> Copy for Id<K> {}
impl<K: Kind> PartialEq for Id<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<K: Kind> Eq for Id<K> {}
impl<K: Kind> PartialOrd for Id<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K: Kind> Ord for Id<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}
impl<K: Kind> Hash for Id<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}
impl<K: Kind> fmt::Debug for Id<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", K::PREFIX, self.raw)
    }
}
impl<K: Kind> fmt::Display for Id<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", K::PREFIX, self.raw)
    }
}
impl<K: Kind> Serialize for Id<K> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(s)
    }
}
impl<'de, K: Kind> Deserialize<'de> for Id<K> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Uuid::deserialize(d).map(Self::from_uuid)
    }
}

macro_rules! kinds {
    ($($(#[$doc:meta])* $marker:ident, $alias:ident, $prefix:literal;)*) => {$(
        $(#[$doc])*
        #[derive(Debug)]
        pub enum $marker {}
        impl sealed::Sealed for $marker {}
        impl Kind for $marker {
            const PREFIX: &'static str = $prefix;
        }
        $(#[$doc])*
        pub type $alias = Id<$marker>;
    )*};
}

kinds! {
    /// Something in the world: a host, a service, a repository, a document.
    EntityKind, EntityId, "ent";
    /// A source of observations, with its class (ADR 0006).
    ObserverKind, ObserverId, "obs";
    /// A fact as an observer reported it.
    ObservationKind, ObservationId, "obv";
    /// A directly observed artefact: bytes at a content digest.
    ArtifactObservationKind, ArtifactObservationId, "art";
    /// A model's reading of an artefact, before any proof.
    ExtractionKind, ExtractionId, "ext";
    /// A person's answer, recorded as evidence.
    TestimonyKind, TestimonyId, "tst";
    /// A typed statement about the world.
    ClaimKind, ClaimId, "clm";
    /// A diff between snapshots or commits; what invalidates claims.
    ChangeKind, ChangeId, "chg";
    /// A premise a proof or hypothesis takes for granted.
    AssumptionKind, AssumptionId, "asm";
    /// A candidate explanation.
    HypothesisKind, HypothesisId, "hyp";
    /// A question with competing hypotheses.
    QuestionKind, QuestionId, "qst";
    /// One recorded reasoner call: model, prompt digest, context.
    ReasonerInstanceKind, ReasonerInstanceId, "rsn";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_of_one_kind_order_by_creation() {
        let a = ClaimId::new();
        let b = ClaimId::new();
        assert!(a < b, "UUIDv7 identities sort by creation time");
    }

    #[test]
    fn a_derived_identity_is_the_same_for_the_same_content_and_differs_otherwise() {
        let a = ClaimId::derived(&[b"site", b"fact"]);
        assert_eq!(a, ClaimId::derived(&[b"site", b"fact"]));
        assert_ne!(a, ClaimId::derived(&[b"site", b"fact2"]));
        assert_ne!(
            ClaimId::derived(&[b"ab", b"c"]),
            ClaimId::derived(&[b"a", b"bc"]),
            "parts are length-prefixed"
        );
        assert_ne!(
            a.uuid(),
            ChangeId::derived(&[b"site", b"fact"]).uuid(),
            "the kind is part of the content"
        );
        assert_eq!(a.uuid().get_version_num(), 8);
    }

    #[test]
    fn id_text_carries_its_kind() {
        let id = ClaimId::from_uuid(Uuid::nil());
        assert_eq!(id.to_string(), "clm_00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn id_round_trips_through_serde_as_a_plain_uuid() {
        let id = EvidenceLikeCheck::id();
        let text = serde_json::to_string(&id).unwrap();
        assert!(text.starts_with('"') && !text.contains('_'));
        let back: ObservationId = serde_json::from_str(&text).unwrap();
        assert_eq!(back, id);
    }

    struct EvidenceLikeCheck;
    impl EvidenceLikeCheck {
        fn id() -> ObservationId {
            ObservationId::new()
        }
    }
}
