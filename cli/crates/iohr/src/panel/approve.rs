//! Approving an ask from the panel.
//!
//! Approving needs the proof of a device key (RFC 0113, ADR 0142, ADR 0215). The panel is a
//! raw-mode screen and never asks Touch ID itself: every approve from it hands over, to the
//! phone, or for an ask a Mac may sign to `iohr asks approve <id>` in a shell (`mac_sign`, the
//! Secure Enclave key behind Touch ID). Which asks a Mac may sign is the one list in
//! [`crate::mac_sign::tier`]; a merge, a roll, a migration and an auto-approve rule are never
//! signed on a Mac ([`phone_only`] reads that list, there is no second copy).

/// What an approve key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Nothing was sent: the phone's key signs it. The line to show.
    Phone(String),
}

/// The ask kinds that are never approved from a terminal, whatever signer there is: every kind
/// a Mac may not sign (fail closed, so an empty or new kind is on the list).
pub(super) fn phone_only(kind: &str) -> bool {
    crate::mac_sign::tier::by_kind(kind).is_some()
}

/// The approve key on an ask of `kind`.
pub(super) fn attempt(kind: &str) -> Outcome {
    let why = if phone_only(kind) {
        "approve on your phone: this kind is never signed here"
    } else {
        "approve on your phone, or iohr asks approve"
    };
    Outcome::Phone(why.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_roll_migration_and_auto_approve_are_phone_only_and_the_panel_signs_nothing() {
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
            Outcome::Phone("approve on your phone, or iohr asks approve".into())
        );
    }
}
