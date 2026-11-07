use std::cell::RefCell;

use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use serde_json::json;

use super::*;
use crate::runner::signing::RunKey;

/// An in-memory P-256 key standing in for the enclave: it signs what it is given and keeps
/// what it saw, so a test can say what Touch ID would have been shown.
struct TestSigner {
    key: RunKey,
    seen: RefCell<Vec<(String, String)>>,
    refuse: bool,
}

impl TestSigner {
    fn new() -> Self {
        let (key, _doc) = RunKey::generate().expect("a key");
        Self {
            key,
            seen: RefCell::default(),
            refuse: false,
        }
    }
}

impl DeviceSigner for TestSigner {
    fn ensure(&self) -> Result<String, String> {
        self.public_key()
    }

    fn public_key(&self) -> Result<String, String> {
        Ok(self.key.public_key())
    }

    fn sign(&self, message: &str, reason: &str) -> Result<String, String> {
        if self.refuse {
            return Err("signing was refused: Touch ID cancelled".to_owned());
        }
        self.seen
            .borrow_mut()
            .push((message.to_owned(), reason.to_owned()));
        self.key.sign(message.as_bytes())
    }
}

fn ask(kind: &str, title: &str) -> serde_json::Value {
    json!({"kind": kind, "title": title, "payload": {}})
}

#[test]
fn the_message_is_core_s_decision_vector() {
    // The vector in core `approval_messages` tests: trimmed note hashed, id and nonce trimmed.
    let m = decision_message("apr_1", "ab", "approve", " ok ", 9, " n ", " key_1 ");
    assert_eq!(
        m,
        format!(
            "iohr-approval-decision-v1\napr_1\nab\napprove\n{}\n9\nn\nkey_1",
            crate::runner::signing::h("ok")
        )
    );
}

#[test]
fn a_signed_decision_verifies_against_the_public_key_over_that_message() {
    let signer = TestSigner::new();
    let d = Decision {
        approval_id: "apr_01M6NZ452CGYK04BSGM9BNJD4Q",
        payload_sha256: "c0ffee",
        decision: "approve",
        note: "",
    };
    let proof = sign_decision(&signer, "apk_1", &d, "approve \"Plan\" (c0ffee)", 1_800_000_000, "bm9uY2U")
        .expect("signs");
    assert_eq!(proof["key_id"], "apk_1");
    assert_eq!(proof["signed_at"], 1_800_000_000);
    assert_eq!(proof["nonce"], "bm9uY2U");
    let message = decision_message(d.approval_id, "c0ffee", "approve", "", 1_800_000_000, "bm9uY2U", "apk_1");
    let public = B64URL.decode(signer.public_key().expect("public")).expect("b64");
    let sig = B64URL
        .decode(proof["signature"].as_str().expect("signature"))
        .expect("b64");
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public)
        .verify(message.as_bytes(), &sig)
        .expect("the service would accept it");
    // The signer was given exactly that message and the Touch ID line.
    let seen = signer.seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, message);
    assert_eq!(seen[0].1, "approve \"Plan\" (c0ffee)");
    // And the proof does not verify against another message (another ask, another time).
    let other = decision_message(d.approval_id, "c0ffee", "approve", "", 1_800_000_001, "bm9uY2U", "apk_1");
    let public = B64URL.decode(signer.public_key().expect("public")).expect("b64");
    assert!(
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public)
            .verify(other.as_bytes(), &sig)
            .is_err()
    );
}

#[test]
fn a_refused_touch_id_makes_no_proof() {
    let mut signer = TestSigner::new();
    signer.refuse = true;
    let d = Decision { approval_id: "apr_1", payload_sha256: "ab", decision: "approve", note: "" };
    let e = sign_decision(&signer, "apk_1", &d, "r", 1, "n").expect_err("refused");
    assert!(e.contains("refused"), "{e}");
}

#[test]
fn nonces_are_sixteen_random_bytes_and_never_repeat() {
    let a = fresh_nonce().expect("nonce");
    let b = fresh_nonce().expect("nonce");
    assert_ne!(a, b);
    assert_eq!(B64URL.decode(&a).expect("base64url").len(), 16);
}

#[test]
fn a_merge_goes_to_the_phone_on_a_mac_and_a_review_goes_to_the_mac() {
    assert_eq!(
        route(&ask("pull_request", "Fix"), true),
        Route::Phone("this is a merge".to_owned())
    );
    assert_eq!(
        route(&ask("roll", "Roll"), true),
        Route::Phone("this is a roll".to_owned())
    );
    assert_eq!(route(&ask("review", "Read the plan"), true), Route::Mac);
}

#[test]
fn a_machine_that_cannot_sign_hands_every_ask_to_the_phone() {
    // What Windows and Linux are: `can_sign_here` is false, whatever the ask is.
    for kind in ["review", "document", "decision", "run", "pull_request"] {
        assert!(matches!(route(&ask(kind, "Plan"), false), Route::Phone(_)), "{kind}");
    }
    assert_eq!(
        route(&ask("review", "Plan"), false),
        Route::Phone("this machine has no key that can sign".to_owned())
    );
}

#[test]
fn this_build_signs_only_on_a_mac() {
    assert_eq!(can_sign_here(), cfg!(target_os = "macos"));
    #[cfg(not(target_os = "macos"))]
    assert!(
        this_machine(std::path::Path::new("."))
            .err()
            .is_some_and(|e| e.contains("phone"))
    );
}

#[test]
fn the_enrol_request_names_the_mac_and_carries_the_phone_s_countersignature() {
    let key = TestSigner::new().public_key().expect("public");
    let alone = enroll_body(&key, "Nevio's MacBook", "host-1", None);
    assert_eq!(
        alone,
        json!({"public_key": key, "platform": "macos", "label": "Nevio's MacBook", "device_id": "host-1"})
    );
    let p = Proof {
        key_id: "apk_phone".to_owned(),
        signed_at: 1_800_000_000,
        nonce: "bm9uY2U".to_owned(),
        signature: "c2ln".to_owned(),
    };
    let with = enroll_body(&key, "Nevio's MacBook", "host-1", Some(&p));
    assert_eq!(
        with["countersign"],
        json!({"key_id": "apk_phone", "signed_at": 1_800_000_000, "nonce": "bm9uY2U", "signature": "c2ln"})
    );
    assert_eq!(with["platform"], "macos");
}

#[test]
fn the_countersign_message_is_the_one_core_checks() {
    let p = Proof {
        key_id: " apk_phone ".to_owned(),
        signed_at: 5,
        nonce: " n ".to_owned(),
        signature: String::new(),
    };
    assert_eq!(
        countersign_message(" PUB ", &p),
        "iohr-approval-key-v1\nPUB\n5\nn\napk_phone"
    );
}

#[test]
fn the_touch_id_line_is_clean_short_and_carries_the_hash_start() {
    let r = touch_reason("Plan\u{1b}[2J for\nthe week", "approve", "0123456789abcdef");
    assert_eq!(r, "approve \"Plan[2J forthe week\" (01234567)");
    let long = touch_reason(&"x".repeat(200), "approve", "ab");
    assert!(long.chars().count() < 90, "{long}");
    assert!(long.contains('\u{2026}'));
}

#[test]
fn the_key_record_is_kept_private_and_removed_with_the_key() {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = store::dir(tmp.path());
    assert_eq!(store::load(&dir).expect("none"), None);
    let key = store::Enrolled {
        key_id: "apk_1".to_owned(),
        public_key: "PUB".to_owned(),
        label: "mac".to_owned(),
    };
    store::save(&dir, &key).expect("saved");
    assert_eq!(store::load(&dir).expect("read"), Some(key));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.join("key.json")).expect("meta").permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    std::fs::write(store::blob(&dir), b"blob").expect("blob");
    store::remove(&dir).expect("removed");
    assert_eq!(store::load(&dir).expect("none"), None);
    assert!(!store::blob(&dir).exists());
    store::remove(&dir).expect("removing twice is fine");
}
