//! The runner's Ed25519 identity (RFC 0120, "The runner protocol"; core PR #879,
//! `docs/coordinator/README.md`, "Signatures"): the proof it holds its key when it
//! registers, and the signature on every turn report.
//!
//! The signed messages are UTF-8 lines a runner in any language can rebuild without a
//! JSON canonicaliser: the domain on the first line, then one line per field, every
//! string as the SHA-256 of its bytes in lowercase hex, integers in decimal, booleans as
//! `true` or `false`, a list as the SHA-256 of its elements' lines joined by `\n`.
//!
//! The private key is the 32-byte seed. It is kept in the credential store the profile
//! uses (the macOS Keychain on a Mac), never in a plain file the runner wrote.

use std::fmt;

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use sha2::{Digest as _, Sha256};

use super::claude::hex;
use super::service::{GateReport, TurnReport};

/// The first line of a registration proof.
pub(crate) const REGISTER_DOMAIN: &str = "iohr.coordinator.v1.RegisterRunner";
/// The first line of a report's signed message.
pub(crate) const REPORT_DOMAIN: &str = "iohr.coordinator.v1.ReportTurn";

/// The first line of a gate report's signed message.
pub(crate) const GATE_DOMAIN: &str = "iohr.coordinator.v1.SubmitGateReport";

/// The runner's key.
pub(crate) struct Signer {
    pair: Ed25519KeyPair,
}

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the key.
        f.debug_struct("Signer")
            .field("public_key", &self.public_key())
            .finish()
    }
}

impl Signer {
    /// The key made from a 32-byte seed.
    pub(crate) fn from_seed(seed: &[u8]) -> Result<Self, String> {
        Ed25519KeyPair::from_seed_unchecked(seed)
            .map(|pair| Self { pair })
            .map_err(|_| "the runner key is not a 32-byte Ed25519 seed".to_owned())
    }

    /// A new key and its seed (the secret to keep).
    pub(crate) fn generate() -> Result<(Self, [u8; 32]), String> {
        let mut seed = [0_u8; 32];
        getrandom::fill(&mut seed).map_err(|e| format!("no randomness for a key: {e}"))?;
        Ok((Self::from_seed(&seed)?, seed))
    }

    /// The 32 public bytes in standard base64 with padding (44 characters).
    pub(crate) fn public_key(&self) -> String {
        B64.encode(self.pair.public_key().as_ref())
    }

    /// The 64-byte signature over `message`.
    pub(crate) fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.pair.sign(message).as_ref().to_vec()
    }
}

/// What a runner signs to register: the domain, the challenge and its public key as
/// sent, one per line.
pub(crate) fn registration_message(challenge: &str, public_key: &str) -> String {
    [REGISTER_DOMAIN, challenge, public_key].join("\n")
}

fn h(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

fn list<I: IntoIterator<Item = String>>(lines: I) -> String {
    h(&lines.into_iter().collect::<Vec<_>>().join("\n"))
}

/// The report's signed message: 27 lines, field by field in the proto's order.
pub(crate) fn report_message(r: &TurnReport) -> String {
    [
        REPORT_DOMAIN.to_owned(),
        h(&r.turn_id),
        h(&r.lease_id),
        r.fencing_token.to_string(),
        h(&r.runner_id),
        h(&r.pack_sha256),
        h(&r.outcome),
        h(&r.say),
        h(&r.state),
        list(
            r.actions
                .iter()
                .map(|a| list([h(&a.kind), h(&a.payload_json)])),
        ),
        list(r.evidence.iter().map(|e| h(e))),
        list(r.left.iter().map(|e| h(e))),
        h(&r.provider),
        h(&r.provider_session_id),
        r.resumed.to_string(),
        h(&r.provider_version),
        r.num_turns.to_string(),
        r.duration_ms.to_string(),
        // The IEEE 754 bits, so no decimal printing can differ.
        r.total_cost_usd
            .map(|c| format!("{:016x}", c.to_bits()))
            .unwrap_or_default(),
        r.usage.input.to_string(),
        r.usage.output.to_string(),
        r.usage.cache_read.to_string(),
        r.usage.cache_write.to_string(),
        list(r.denied_tools.iter().map(|t| h(t))),
        h(&r.detail),
        r.attempt.to_string(),
        h(&r.resets_at),
    ]
    .join("\n")
}

/// The gate report's signed message: 10 lines, field by field in the proto's order
/// (`account_id` and the signature are not signed).
pub(crate) fn gate_report_message(r: &GateReport) -> String {
    [
        GATE_DOMAIN.to_owned(),
        h(&r.job_id),
        h(&r.lease_id),
        r.fencing_token.to_string(),
        h(&r.runner_id),
        h(&r.command),
        r.exit_code.to_string(),
        h(&r.head),
        h(&r.main_commit),
        h(&r.output_sha256),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;
    use crate::runner::service::{Action, Usage};

    fn signer() -> Signer {
        Signer::from_seed(&[7_u8; 32]).unwrap()
    }

    /// The report of the service's test vector (core PR #879, `service/signing.rs`).
    fn vector() -> TurnReport {
        TurnReport {
            turn_id: "trn_01J00000000000000000000000".into(),
            lease_id: "lse_01J00000000000000000000000".into(),
            fencing_token: 3,
            runner_id: "rnr_01J00000000000000000000000".into(),
            pack_sha256: "a".repeat(64),
            outcome: "replied".into(),
            say: "Done.\nTwo lines.".into(),
            state: "state".into(),
            actions: vec![Action {
                kind: "create_job".into(),
                payload_json: r#"{"kind":"docs"}"#.into(),
            }],
            evidence: vec!["cargo test: exit 0".into()],
            left: vec![],
            provider: "claude".into(),
            provider_session_id: "4f6c1f0e-0000-4000-8000-000000000000".into(),
            resumed: false,
            provider_version: "2.1.0".into(),
            num_turns: 4,
            duration_ms: 12_345,
            total_cost_usd: Some(0.125),
            usage: Usage {
                input: 10,
                output: 20,
                cache_read: 30,
                cache_write: 40,
            },
            denied_tools: vec![],
            detail: String::new(),
            attempt: 1,
            resets_at: String::new(),
        }
    }

    /// The service's vector, from the README: the key from the seed of 32 bytes of 7,
    /// the message's SHA-256 and the signature (Ed25519 is deterministic).
    #[test]
    fn the_report_test_vector() {
        let s = signer();
        let msg = report_message(&vector());
        assert_eq!(msg.lines().count(), 27);
        assert!(msg.starts_with("iohr.coordinator.v1.ReportTurn\n"));
        assert_eq!(
            s.public_key(),
            "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw="
        );
        assert_eq!(
            h(&msg),
            "f0115c82c5c01d91b97ea7a0d232261837e0057e676c1d739f93a68bcdb86a9d"
        );
        let sig = s.sign(msg.as_bytes());
        assert_eq!(sig.len(), 64);
        assert_eq!(
            B64.encode(sig),
            "RXnFtoZOwUXtJMWSbFRHWXmqVTwzH33QgayOA68ejIJyTg3F5QeaV0sGFuHS1ujBshAIpMx0kJvqVgJ2/qJZAA=="
        );
    }

    /// The service's gate report vector (core `service/signing.rs`, README "Signatures"):
    /// the same key, the message's digest and the signature, Ed25519 being deterministic.
    #[test]
    fn the_gate_report_test_vector() {
        let r = GateReport {
            job_id: "job_01J00000000000000000000000".into(),
            lease_id: "lse_01J00000000000000000000000".into(),
            fencing_token: 3,
            runner_id: "rnr_01J00000000000000000000000".into(),
            command: "mise run ci:changed".into(),
            exit_code: 0,
            head: "1".repeat(40),
            main_commit: "2".repeat(40),
            output_sha256: "b".repeat(64),
        };
        let msg = gate_report_message(&r);
        assert_eq!(msg.lines().count(), 10);
        assert!(msg.starts_with("iohr.coordinator.v1.SubmitGateReport\n"));
        assert_eq!(
            h(&msg),
            "33a926ca4766776446fc155e78a88a6943a96d016bbe21b45ee80525c7103c51"
        );
        assert_eq!(
            B64.encode(signer().sign(msg.as_bytes())),
            "jATJlhhTDANbtQbqxqRWkjW2fAm1ZOWPPJay9ErQWE51CjR2f7TqqPi+A1DDVRx2bMQ0WMNmLSkxbV2xNZvqAA=="
        );
        assert_ne!(GATE_DOMAIN, REPORT_DOMAIN);
    }

    #[test]
    fn the_registration_message_is_the_domain_the_challenge_and_the_key() {
        let key = signer().public_key();
        assert_eq!(key.len(), 44);
        assert_eq!(
            registration_message(&"c".repeat(64), &key),
            format!(
                "iohr.coordinator.v1.RegisterRunner\n{}\n{key}",
                "c".repeat(64)
            )
        );
    }

    #[test]
    fn a_new_key_is_a_32_byte_seed_that_gives_the_same_key_again() {
        let (a, seed) = Signer::generate().unwrap();
        assert_eq!(
            Signer::from_seed(&seed).unwrap().public_key(),
            a.public_key()
        );
        assert!(Signer::from_seed(&[1, 2, 3]).is_err());
    }
}
