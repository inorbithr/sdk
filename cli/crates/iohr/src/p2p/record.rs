//! The report record, `p2p/report/v0` (RFC 0230 section 2): one domain or one handle, a
//! reason from a fixed list, signed by the person's device key. No free text anywhere, and
//! never a person, an email address or a phone number.
//!
//! The signed message reuses the approvals layout (`runner::approvals::decision_message`):
//! one field per line, the domain tag first. Signing goes through `mac_sign::DeviceSigner`
//! (the Secure Enclave key on a Mac, a software P-256 key elsewhere) and verification
//! through `runner::approvals::DeviceKeys`, so this module adds no crypto of its own.
//!
//! Errors never repeat the subject: a subject is a report about someone, and errors end up
//! in logs.

use serde::{Deserialize, Serialize};

use crate::mac_sign::{DeviceSigner, fresh_nonce};
use crate::runner::approvals::DeviceKeys;
use crate::runner::signing::h;

/// The first line of every signed report.
pub(crate) const REPORT_TAG: &str = "iohr.p2p.v1.Report";

/// The wire version this code writes and reads.
pub(crate) const VERSION: u8 = 1;

/// The largest record accepted, in bytes. A real one is under 400.
pub(crate) const MAX_RECORD: usize = 1024;

/// What a report is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// A domain name, ASCII (an internationalised name in its `xn--` form).
    Domain,
    /// An account on a network: `<network>/@<name>`.
    Handle,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Handle => "handle",
        }
    }
}

/// Why it is reported. A fixed list: no free text travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    FakeShop,
    Phishing,
    Impersonation,
    InvestmentFraud,
    FakeSupport,
    Malware,
}

impl Reason {
    pub(crate) const ALL: [Self; 6] = [
        Self::FakeShop,
        Self::Phishing,
        Self::Impersonation,
        Self::InvestmentFraud,
        Self::FakeSupport,
        Self::Malware,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FakeShop => "fake_shop",
            Self::Phishing => "phishing",
            Self::Impersonation => "impersonation",
            Self::InvestmentFraud => "investment_fraud",
            Self::FakeSupport => "fake_support",
            Self::Malware => "malware",
        }
    }

    /// The reason named `s`.
    ///
    /// # Errors
    /// `s` is not one of the list; the error names the list, not `s`.
    pub(crate) fn parse(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == s.trim())
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|r| r.as_str()).collect();
                format!("the reason is one of: {}", names.join(", "))
            })
    }
}

/// The networks a handle may be on.
pub(crate) const NETWORKS: [&str; 6] = [
    "telegram",
    "instagram",
    "x",
    "tiktok",
    "facebook",
    "discord",
];

/// `raw` as it is signed: lowercase, checked against the rules for `kind`.
///
/// # Errors
/// `raw` breaks a rule; the error says which rule, never the subject.
pub(crate) fn normalise(kind: Kind, raw: &str) -> Result<String, String> {
    match kind {
        Kind::Domain => domain(raw),
        Kind::Handle => handle(raw),
    }
}

fn domain(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let s = s.strip_suffix('.').unwrap_or(s);
    if !s.is_ascii() {
        return Err("the domain must be ASCII: give an international name in its xn-- form".into());
    }
    let s = s.to_ascii_lowercase();
    if s.is_empty() || s.len() > 253 {
        return Err("the domain must be 1 to 253 characters".into());
    }
    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return Err("the domain needs at least two labels (example.com)".into());
    }
    for l in &labels {
        if l.is_empty() || l.len() > 63 {
            return Err("each label of the domain must be 1 to 63 characters".into());
        }
        if !l
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("a domain holds only letters, digits, '-' and '.'".into());
        }
        if l.starts_with('-') || l.ends_with('-') {
            return Err("a label of the domain cannot start or end with '-'".into());
        }
    }
    if labels
        .last()
        .is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("an IP address is not a domain".into());
    }
    Ok(s)
}

fn handle(raw: &str) -> Result<String, String> {
    let s = raw.trim().to_ascii_lowercase();
    let Some((network, name)) = s.split_once("/@") else {
        return Err("a handle is <network>/@<name>, for example telegram/@shop_help".into());
    };
    if !NETWORKS.contains(&network) {
        return Err(format!("the network is one of: {}", NETWORKS.join(", ")));
    }
    if name.is_empty() || name.len() > 64 {
        return Err("the name of a handle must be 1 to 64 characters".into());
    }
    if !name.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-'
    }) {
        return Err("a handle's name holds only letters, digits, '_', '.' and '-'".into());
    }
    Ok(format!("{network}/@{name}"))
}

/// A nonce as `mac_sign::fresh_nonce` makes it: base64url of 16 bytes, 22 characters.
fn nonce_ok(s: &str) -> bool {
    s.len() == 22
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A key id as the service hands it out (`apk_` and a ULID): 1 to 64 of letters, digits
/// and `_`.
fn key_id_ok(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A base64url DER ECDSA signature: at most 72 bytes, so at most 96 characters.
fn signature_ok(s: &str) -> bool {
    (1..=96).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// One signed report, exactly as it travels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub(crate) v: u8,
    pub(crate) kind: Kind,
    pub(crate) subject: String,
    pub(crate) reason: Reason,
    /// Unix seconds.
    pub(crate) signed_at: i64,
    pub(crate) nonce: String,
    pub(crate) key_id: String,
    /// base64url of the DER ECDSA P-256 SHA-256 signature over [`Report::message`].
    pub(crate) signature: String,
}

/// The message a report's key signs, one field per line.
pub(crate) fn message(
    kind: Kind,
    subject: &str,
    reason: Reason,
    signed_at: i64,
    nonce: &str,
    key_id: &str,
) -> String {
    [
        REPORT_TAG,
        &VERSION.to_string(),
        kind.as_str(),
        subject,
        reason.as_str(),
        &signed_at.to_string(),
        nonce.trim(),
        key_id.trim(),
    ]
    .join("\n")
}

impl Report {
    /// Normalises `subject`, signs it with `signer` as `key_id` and returns the record.
    /// `signer` shows the person what they sign (Touch ID on a Mac).
    ///
    /// # Errors
    /// The subject breaks a rule, the key id is malformed, or signing was refused.
    pub(crate) fn sign(
        signer: &dyn DeviceSigner,
        key_id: &str,
        kind: Kind,
        subject: &str,
        reason: Reason,
        now: i64,
    ) -> Result<Self, String> {
        let subject = normalise(kind, subject)?;
        let key_id = key_id.trim();
        if !key_id_ok(key_id) {
            return Err("the key id is malformed".into());
        }
        let nonce = fresh_nonce()?;
        let m = message(kind, &subject, reason, now, &nonce, key_id);
        let shown = format!("report {subject} as {}", reason.as_str());
        let signature = signer.sign(&m, &shown)?;
        Ok(Self {
            v: VERSION,
            kind,
            subject,
            reason,
            signed_at: now,
            nonce,
            key_id: key_id.to_owned(),
            signature,
        })
    }

    /// A record from the wire: at most [`MAX_RECORD`] bytes, no unknown fields, every
    /// field in its canonical form. The signature is not checked here ([`Report::verifies`]).
    ///
    /// # Errors
    /// The record is too large, not JSON, or a field is malformed; the error never quotes
    /// the record.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_RECORD {
            return Err(format!("a report is at most {MAX_RECORD} bytes"));
        }
        let r: Self = serde_json::from_slice(bytes)
            .map_err(|_| "the report is not a valid record".to_owned())?;
        r.check()?;
        Ok(r)
    }

    /// Whether every field is in its canonical form (what [`Report::sign`] writes).
    ///
    /// # Errors
    /// The first field that is not.
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.v != VERSION {
            return Err("the report's version is not supported".into());
        }
        if normalise(self.kind, &self.subject).as_deref() != Ok(self.subject.as_str()) {
            return Err("the report's subject is not in its normal form".into());
        }
        if self.signed_at <= 0 {
            return Err("the report's time is not valid".into());
        }
        if !nonce_ok(&self.nonce) {
            return Err("the report's nonce is malformed".into());
        }
        if !key_id_ok(&self.key_id) {
            return Err("the report's key id is malformed".into());
        }
        if !signature_ok(&self.signature) {
            return Err("the report's signature is malformed".into());
        }
        Ok(())
    }

    /// The signed message.
    pub(crate) fn message(&self) -> String {
        message(
            self.kind,
            &self.subject,
            self.reason,
            self.signed_at,
            &self.nonce,
            &self.key_id,
        )
    }

    /// The report's id: SHA-256 hex of the signed message. The same report has the same id
    /// wherever it travels, so copies collapse.
    pub(crate) fn id(&self) -> String {
        h(&self.message())
    }

    /// Whether the signature is `key_id`'s key's, over every field.
    pub(crate) fn verifies(&self, keys: &DeviceKeys) -> bool {
        self.check().is_ok() && keys.verifies(&self.key_id, &self.message(), &self.signature)
    }
}
