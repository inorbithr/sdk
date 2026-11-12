//! Approving from this Mac (RFC 0113, ADR 0215, study 0167): a per-Mac P-256 key in the
//! Secure Enclave behind Touch ID, enrolled like a phone's device key (`apk_`) and best
//! countersigned by an enrolled phone key, then used to sign the same decision proof the phone
//! signs. Asks that are hard to take back stay phone-only ([`tier`]); Windows and Linux never
//! sign, they hand the ask to the phone.
//!
//! What a decision signs is [`crate::runner::approvals::decision_message`], the one builder
//! the runner's verifier already uses (core `approval_messages::decision`); nothing here
//! builds that text a second time. The enclave sits behind [`DeviceSigner`], so every step
//! but Touch ID itself is tested with an in-memory key.
//!
//! The key never leaves the enclave. What is on disk is an opaque blob only this Mac's enclave
//! can use, in a 0700 folder, and the key's id and public key. Nothing of an ask's text is
//! written anywhere.

use serde_json::{Value, json};

use crate::runner::approvals::{Proof, decision_message};

pub(crate) mod store;
pub(crate) mod tier;

#[cfg(target_os = "macos")]
mod secure_enclave;

/// The platform the service records a Mac key under (its allow-list: ios, android, macos).
pub(crate) const PLATFORM: &str = "macos";

/// A key that signs for a person. The Mac's lives in the Secure Enclave; a test's in memory.
pub(crate) trait DeviceSigner {
    /// Makes the key when this machine has none (behind the fingerprints enrolled now) and
    /// returns its public key; with a key it only returns it.
    ///
    /// # Errors
    /// The key cannot be made (no enclave, no Touch ID set up).
    fn ensure(&self) -> Result<String, String>;

    /// The public key: base64url of the 65-byte uncompressed P-256 point.
    ///
    /// # Errors
    /// The key cannot be read (its file is gone, the enclave is not there).
    fn public_key(&self) -> Result<String, String>;

    /// base64url of the DER ECDSA signature over `message` (SHA-256). `reason` is the line the
    /// Touch ID prompt shows.
    ///
    /// # Errors
    /// Touch ID was refused or cancelled, or the key was invalidated (fingerprints changed).
    fn sign(&self, message: &str, reason: &str) -> Result<String, String>;
}

/// Where an ask is decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    /// Sign here with the Mac's key.
    Mac,
    /// The phone: it says why in a few words that follow "this is" or "approve on your phone".
    Phone(String),
}

/// `ask` goes to the Mac only when this machine can sign (`can_sign_here`: a Mac) and the tier
/// lets it; everything else goes to the phone, with the reason.
pub(crate) fn route(ask: &Value, can_sign_here: bool) -> Route {
    match tier::classify(ask) {
        tier::Tier::Phone(why) => Route::Phone(format!("this is {why}")),
        tier::Tier::Mac if !can_sign_here => {
            Route::Phone("this machine has no key that can sign".to_owned())
        }
        tier::Tier::Mac => Route::Mac,
    }
}

/// Whether this build can hold a Secure Enclave key at all.
pub(crate) const fn can_sign_here() -> bool {
    cfg!(target_os = "macos")
}

/// What is decided: the fields a decision proof signs, besides the key and the time.
pub(crate) struct ToSign<'a> {
    pub(crate) approval_id: &'a str,
    pub(crate) payload_sha256: &'a str,
    pub(crate) decision: &'a str,
    pub(crate) note: &'a str,
}

/// Signs `d` as `key_id` at `signed_at` with `nonce`, and returns the proof the service takes:
/// `{key_id, signed_at, nonce, signature}`. `reason` is the Touch ID line.
///
/// # Errors
/// The signer refused (Touch ID cancelled or failed).
pub(crate) fn sign_decision(
    signer: &dyn DeviceSigner,
    key_id: &str,
    d: &ToSign<'_>,
    reason: &str,
    signed_at: i64,
    nonce: &str,
) -> Result<Value, String> {
    let message = decision_message(
        d.approval_id,
        d.payload_sha256,
        d.decision,
        d.note,
        signed_at,
        nonce,
        key_id,
    );
    let signature = signer.sign(&message, reason)?;
    Ok(json!({
        "key_id": key_id,
        "signed_at": signed_at,
        "nonce": nonce,
        "signature": signature,
    }))
}

/// A fresh nonce: base64url of 16 random bytes (a nonce is used once per key).
///
/// # Errors
/// The system has no randomness.
pub(crate) fn fresh_nonce() -> Result<String, String> {
    use base64::Engine as _;
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness for a nonce: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// The `EnrollKey` request for this Mac's key. With a `countersign` from an enrolled phone key
/// the service activates it at once; without one it waits its delay (24 hours), unless it is
/// the person's first key.
pub(crate) fn enroll_body(
    public_key: &str,
    label: &str,
    device_id: &str,
    countersign: Option<&Proof>,
) -> Value {
    let mut body = json!({
        "public_key": public_key,
        "platform": PLATFORM,
        "label": label,
        "device_id": device_id,
    });
    if let (Some(p), Some(map)) = (countersign, body.as_object_mut()) {
        map.insert(
            "countersign".to_owned(),
            json!({
                "key_id": p.key_id,
                "signed_at": p.signed_at,
                "nonce": p.nonce,
                "signature": p.signature,
            }),
        );
    }
    body
}

/// The line Touch ID shows: what is being approved, cut and clean, with the first characters of
/// the payload's hash so the person can match it to what the terminal printed. The ask's
/// title is other people's text; control characters are dropped and it is cut.
pub(crate) fn touch_reason(title: &str, decision: &str, payload_sha256: &str) -> String {
    let title: String = title.chars().filter(|c| !c.is_control()).collect();
    let title = title.trim();
    let title = if title.chars().count() > 60 {
        format!("{}\u{2026}", title.chars().take(59).collect::<String>())
    } else {
        title.to_owned()
    };
    let hash: String = payload_sha256.chars().take(8).collect();
    format!("{decision} \"{title}\" ({hash})")
}

/// The signer for this machine: the Secure Enclave key whose files are in `dir`.
///
/// # Errors
/// Not a Mac, or the helper that talks to the enclave cannot be made.
#[cfg(target_os = "macos")]
pub(crate) fn this_machine(dir: &std::path::Path) -> Result<Box<dyn DeviceSigner>, String> {
    secure_enclave::SecureEnclave::open(dir).map(|k| Box::new(k) as Box<dyn DeviceSigner>)
}

/// Windows and Linux never sign: the ask goes to the phone.
///
/// # Errors
/// Always: there is no key here.
#[cfg(not(target_os = "macos"))]
pub(crate) fn this_machine(_dir: &std::path::Path) -> Result<Box<dyn DeviceSigner>, String> {
    Err("only a Mac signs asks; approve on your phone".to_owned())
}

#[cfg(test)]
mod tests;
