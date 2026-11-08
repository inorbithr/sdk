//! Approving an ask from the panel.
//!
//! Approving needs the proof of a device key (RFC 0113, ADR 0142, ADR 0215), and the keys live
//! on the phone, so today every approve from the panel hands over to the phone, whatever the
//! kind. Merge, roll, migration and auto-approve asks stay on the phone even when a signer
//! exists (the owner's rule): [`phone_only`] is the list.
//!
//! Where a signer plugs in: the Touch ID signing of `feat/mac-touchid-sign` (a `DeviceSigner`
//! and a fail-closed risk tier) is not in this tree. When it lands, [`attempt`] asks it for the
//! kinds that are not [`phone_only`] and returns [`Outcome::Signed`]; the panel already shows
//! whatever [`Outcome`] says and sends nothing itself.

/// What an approve key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Nothing was sent: the phone's key signs it. The line to show.
    Phone(String),
}

/// The ask kinds that are never approved from a terminal, whatever signer there is.
pub(super) fn phone_only(kind: &str) -> bool {
    let k = kind.to_ascii_lowercase().replace([' ', '-'], "_");
    matches!(
        k.as_str(),
        "pull_request" | "merge" | "roll" | "migration" | "auto_approve"
    )
}

/// The approve key on an ask of `kind`.
pub(super) fn attempt(kind: &str) -> Outcome {
    let why = if phone_only(kind) {
        "approve on your phone: this kind is never signed here"
    } else {
        "approve on your phone: its key signs it"
    };
    Outcome::Phone(why.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_roll_migration_and_auto_approve_are_phone_only_and_nothing_is_signed_here_yet() {
        for k in [
            "pull request",
            "pull_request",
            "roll",
            "migration",
            "auto_approve",
            "Merge",
        ] {
            assert!(phone_only(k), "{k}");
            assert!(matches!(attempt(k), Outcome::Phone(s) if s.contains("never signed here")));
        }
        assert!(!phone_only("decision"));
        assert_eq!(
            attempt("decision"),
            Outcome::Phone("approve on your phone: its key signs it".into())
        );
    }
}
