//! Who may publish an extension, and the check that they did (ADR 0012, SR-26).
//!
//! An artifact is trusted when Sigstore bundles attached to it prove one of:
//!
//! - **InOrbit's release workflow signed it.** A bundle whose certificate Fulcio
//!   issued to `https://github.com/inorbithr/<repo>/.github/workflows/release.yml` at a
//!   tag, for the GitHub Actions issuer, logged in Rekor, signing an in-toto statement
//!   whose subject is the artifact's digest; and a second such bundle from the same
//!   repository carrying SLSA v1 provenance that names that repository, workflow and a
//!   tag. Both are checked offline against the Sigstore trusted root built into `iohr`.
//! - **A key the company configured signed it** (`ext.trusted_keys`), for a mirror that
//!   re-signs what it copies and for an internal channel: a bundle with that public
//!   key's signature over a statement naming the digest. Its transparency-log entry is
//!   checked when the bundle carries one.
//!
//! Anything else is refused, and there is no switch that skips the check.

use std::fmt;

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sigstore_verify::types::bundle::VerificationMaterialContent;
use sigstore_verify::types::{Bundle, DerPublicKey, Sha256Hash, SignatureContent, Statement};
use sigstore_verify::{PublicKeyVerificationPolicy, SubjectAltName, VerificationPolicy, Verifier};

use super::oci::{Digest, hex};

/// The OIDC issuer of GitHub Actions' workload identity tokens.
pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";
/// The SLSA provenance predicate type.
pub const SLSA_PROVENANCE: &str = "https://slsa.dev/provenance/v1";

/// The workflow identity extensions are published by, and the keys a company added.
#[derive(Debug, Clone)]
pub struct Policy {
    org: String,
    workflow: String,
    refs: String,
    keys: Vec<TrustedKey>,
}

impl Policy {
    /// InOrbit's release workflow, plus `keys`.
    #[must_use]
    pub fn inorbit(keys: Vec<TrustedKey>) -> Self {
        Self {
            org: "inorbithr".into(),
            workflow: ".github/workflows/release.yml".into(),
            refs: "refs/tags/".into(),
            keys,
        }
    }

    /// The identity this policy accepts, for messages and documentation.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "https://github.com/{}/<repository>/{}@{}*",
            self.org, self.workflow, self.refs
        )
    }

    /// The repository a certificate identity names, when it is this policy's workflow
    /// at an accepted ref.
    fn repository_of(&self, identity: &str) -> Option<String> {
        let rest = identity
            .strip_prefix("https://github.com/")?
            .strip_prefix(&self.org)?
            .strip_prefix('/')?;
        let (repo, rest) = rest.split_once('/')?;
        let reference = rest.strip_prefix(&self.workflow)?.strip_prefix('@')?;
        let ok = !repo.is_empty()
            && repo
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            && reference
                .strip_prefix(&self.refs)
                .is_some_and(|t| !t.is_empty());
        ok.then(|| format!("https://github.com/{}/{repo}", self.org))
    }
}

/// A public key a company trusts to sign extensions, from `ext.trusted_keys`.
#[derive(Clone)]
pub struct TrustedKey {
    der: DerPublicKey,
    fingerprint: String,
}

impl fmt::Debug for TrustedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.fingerprint)
    }
}

impl TrustedKey {
    /// Reads a PEM `PUBLIC KEY` (ECDSA P-256 or P-384, Ed25519, RSA).
    ///
    /// # Errors
    ///
    /// [`TrustError`] when it is not one.
    pub fn from_pem(pem: &str) -> Result<Self, TrustError> {
        let der = DerPublicKey::from_pem(pem)
            .map_err(|e| TrustError(format!("not a PEM public key: {e}")))?;
        sigstore_verify::crypto::KeyAlgorithm::from_spki(&der)
            .map_err(|e| TrustError(format!("a public key of a kind iohr cannot use: {e}")))?;
        let fingerprint = format!("key:sha256:{}", hex(&Sha256::digest(der.as_bytes())));
        Ok(Self { der, fingerprint })
    }

    /// `key:sha256:<hex of the DER public key>`, the name a lock file records.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

/// Who signed an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Signer {
    /// A release workflow, by its certificate identity.
    Workflow {
        /// The certificate's identity (SAN URI).
        identity: String,
        /// `https://github.com/<org>/<repo>`.
        repository: String,
    },
    /// A configured public key.
    Key {
        /// `key:sha256:...`.
        fingerprint: String,
    },
}

impl Signer {
    /// The signer's name as a lock file records it: the workflow identity without the
    /// tag, or the key's fingerprint. The same for every version of one extension.
    #[must_use]
    pub fn lock_name(&self) -> String {
        match self {
            Self::Workflow { identity, .. } => identity
                .split_once('@')
                .map_or_else(|| identity.clone(), |(w, _)| w.to_owned()),
            Self::Key { fingerprint } => fingerprint.clone(),
        }
    }
}

impl fmt::Display for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workflow { identity, .. } => f.write_str(identity),
            Self::Key { fingerprint } => f.write_str(fingerprint),
        }
    }
}

/// What the check established.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Verified {
    /// Who signed.
    pub signer: Signer,
    /// What the provenance says built it: repository and ref, when there was one.
    pub provenance: Option<String>,
}

/// Why an artifact is not trusted. Says what was checked and what failed; never skips.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TrustError(pub String);

/// One bundle's outcome.
#[derive(Debug)]
enum Finding {
    Release {
        identity: String,
        repository: String,
        provenance: Option<String>,
    },
    Key(String),
    Refused(String),
}

/// Checks the bundles found for an artifact. Each comes with the digest it was
/// attached to, which its statement's subject must name.
///
/// # Errors
///
/// [`TrustError`] unless the policy's conditions hold (see the module docs).
pub fn verify(policy: &Policy, bundles: &[(Digest, Vec<u8>)]) -> Result<Verified, TrustError> {
    let root = sigstore_verify::trust_root::TrustedRoot::from_json(
        sigstore_verify::trust_root::SIGSTORE_PRODUCTION_TRUSTED_ROOT,
    )
    .map_err(|e| {
        TrustError(format!(
            "the built-in Sigstore trusted root cannot be read: {e}"
        ))
    })?;
    let verifier = Verifier::new(&root).map_err(|e| {
        TrustError(format!(
            "the built-in Sigstore trusted root cannot be used: {e}"
        ))
    })?;
    let findings: Vec<Finding> = bundles
        .iter()
        .map(|(digest, bytes)| examine(&verifier, policy, digest, bytes))
        .collect();
    decide(policy, &findings)
}

fn decide(policy: &Policy, findings: &[Finding]) -> Result<Verified, TrustError> {
    // A release: a signature and a provenance from the same repository.
    for f in findings {
        if let Finding::Release {
            identity,
            repository,
            provenance: None,
        } = f
            && let Some(prov) = findings.iter().find_map(|g| match g {
                Finding::Release {
                    repository: r,
                    provenance: Some(p),
                    ..
                } if r == repository => Some(p.clone()),
                _ => None,
            })
        {
            return Ok(Verified {
                signer: Signer::Workflow {
                    identity: identity.clone(),
                    repository: repository.clone(),
                },
                provenance: Some(prov),
            });
        }
    }
    if let Some(fingerprint) = findings.iter().find_map(|f| match f {
        Finding::Key(k) => Some(k.clone()),
        _ => None,
    }) {
        let provenance = findings.iter().find_map(|f| match f {
            Finding::Release {
                provenance: Some(p),
                ..
            } => Some(p.clone()),
            _ => None,
        });
        return Ok(Verified {
            signer: Signer::Key { fingerprint },
            provenance,
        });
    }
    let mut why: Vec<String> = Vec::new();
    let has_release_sig = findings.iter().any(|f| {
        matches!(
            f,
            Finding::Release {
                provenance: None,
                ..
            }
        )
    });
    let has_release_prov = findings.iter().any(|f| {
        matches!(
            f,
            Finding::Release {
                provenance: Some(_),
                ..
            }
        )
    });
    if findings.is_empty() {
        why.push("the registry holds no Sigstore bundle for it".into());
    }
    if has_release_prov && !has_release_sig {
        why.push("its provenance checks, but there is no signature from the same workflow".into());
    }
    if has_release_sig && !has_release_prov {
        why.push(
            "its signature checks, but there is no SLSA provenance from the same workflow".into(),
        );
    }
    for f in findings {
        if let Finding::Refused(r) = f
            && !why.contains(r)
        {
            why.push(r.clone());
        }
    }
    Err(TrustError(format!(
        "not installed: no trusted signature. iohr accepts {} with SLSA provenance{}, and found: {}",
        policy.describe(),
        if policy.keys.is_empty() {
            String::new()
        } else {
            format!(
                ", or a signature by one of {} configured key(s)",
                policy.keys.len()
            )
        },
        why.join("; ")
    )))
}

fn examine(verifier: &Verifier, policy: &Policy, digest: &Digest, bytes: &[u8]) -> Finding {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Finding::Refused("a bundle that is not text".into());
    };
    let bundle = match Bundle::from_json(text) {
        Ok(b) => b,
        Err(e) => return Finding::Refused(format!("a bundle that cannot be read ({e})")),
    };
    let Ok(subject) = Sha256Hash::from_hex(digest.hex()) else {
        return Finding::Refused("a digest that is not SHA-256".into());
    };
    let statement = match &bundle.content {
        SignatureContent::DsseEnvelope(env)
            if env.payload_type == "application/vnd.in-toto+json" =>
        {
            match serde_json::from_slice::<Statement>(env.payload.as_bytes()) {
                Ok(s) => s,
                Err(_) => {
                    return Finding::Refused("a bundle whose statement cannot be read".into());
                }
            }
        }
        _ => {
            return Finding::Refused(
                "a bundle that is not a signed in-toto statement (DSSE)".into(),
            );
        }
    };
    if let VerificationMaterialContent::PublicKey(_) = bundle.verification_material.content {
        return examine_keyed(verifier, policy, subject, &bundle, &statement);
    }
    let sig_policy = VerificationPolicy::any_identity().require_issuer(GITHUB_ISSUER);
    let result = match verifier.verify(subject, &bundle, &sig_policy) {
        Ok(r) => r,
        Err(e) => return Finding::Refused(format!("a Sigstore bundle that does not verify ({e})")),
    };
    // `any_identity` checks chain, SCT, log and signature; the signer is checked here.
    if !(result.certificate_verified() && result.tlog_verified()) {
        return Finding::Refused("a bundle without a verified certificate and log entry".into());
    }
    let identity = match result.identity() {
        Some(SubjectAltName::Uri(u)) => u.clone(),
        Some(other) => {
            return Finding::Refused(format!(
                "a signature by {}, not a release workflow",
                super::manifest::printable(other.as_str())
            ));
        }
        None => return Finding::Refused("a certificate that names no signer".into()),
    };
    let Some(repository) = policy.repository_of(&identity) else {
        return Finding::Refused(format!(
            "a signature by {}, which is not {}",
            super::manifest::printable(&identity),
            policy.describe()
        ));
    };
    if statement.predicate_type != SLSA_PROVENANCE {
        return Finding::Release {
            identity,
            repository,
            provenance: None,
        };
    }
    match provenance(policy, &repository, &statement.predicate) {
        Ok(p) => Finding::Release {
            identity,
            repository,
            provenance: Some(p),
        },
        Err(why) => Finding::Refused(why),
    }
}

fn examine_keyed(
    verifier: &Verifier,
    policy: &Policy,
    subject: Sha256Hash,
    bundle: &Bundle,
    statement: &Statement,
) -> Finding {
    if policy.keys.is_empty() {
        return Finding::Refused(
            "a signature by a public key, and no key is configured in ext.trusted_keys".into(),
        );
    }
    let mut last = String::new();
    for key in &policy.keys {
        let outcome = if bundle.verification_material.tlog_entries.is_empty() {
            // A private channel may have no transparency log: then the check is the
            // DSSE signature by the configured key over a statement naming the digest.
            dsse_signed_by(key, bundle, statement, &subject)
        } else {
            verifier
                .verify_with_key(
                    subject,
                    bundle,
                    &key.der,
                    &PublicKeyVerificationPolicy::default(),
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        match outcome {
            Ok(()) => return Finding::Key(key.fingerprint.clone()),
            Err(e) => last = e,
        }
    }
    Finding::Refused(format!(
        "a signature by a public key that no key in ext.trusted_keys verifies ({last})"
    ))
}

fn dsse_signed_by(
    key: &TrustedKey,
    bundle: &Bundle,
    statement: &Statement,
    subject: &Sha256Hash,
) -> Result<(), String> {
    let SignatureContent::DsseEnvelope(env) = &bundle.content else {
        return Err("not a DSSE envelope".into());
    };
    if !statement.matches_sha256(subject) {
        return Err("the statement does not name this artifact".into());
    }
    let vk =
        sigstore_verify::crypto::VerificationKey::from_spki(&key.der).map_err(|e| e.to_string())?;
    vk.verify(env.pae(), &env.signature.sig)
        .map_err(|_| "the signature does not match".to_owned())
}

/// Checks SLSA v1 provenance from GitHub Actions: built by the same repository's
/// release workflow at an accepted ref. Returns `repository@ref`.
fn provenance(policy: &Policy, repository: &str, predicate: &Value) -> Result<String, String> {
    let wf = predicate
        .pointer("/buildDefinition/externalParameters/workflow")
        .ok_or("provenance that names no workflow")?;
    let field = |k: &str| wf.get(k).and_then(Value::as_str).unwrap_or_default();
    if field("repository") != repository {
        return Err(format!(
            "provenance from {}, not {repository}",
            super::manifest::printable(field("repository"))
        ));
    }
    if field("path") != policy.workflow {
        return Err(format!(
            "provenance from workflow {}, not {}",
            super::manifest::printable(field("path")),
            policy.workflow
        ));
    }
    let reference = field("ref");
    if !reference.starts_with(&policy.refs) {
        return Err(format!(
            "provenance from {}, not a release ({}*)",
            super::manifest::printable(reference),
            policy.refs
        ));
    }
    Ok(format!("{repository}@{reference}"))
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use sigstore_verify::crypto::KeyPair;

    use super::{Policy, Signer, TrustedKey, verify};
    use crate::ext::oci::Digest;

    /// GitHub's SLSA provenance for `gh_2.102.0_linux_amd64.tar.gz`, from
    /// `gh attestation download` (public, logged in Rekor).
    const GH_PROVENANCE: &str = include_str!("testdata/gh-2.102.0-provenance.sigstore.json");
    const GH_DIGEST: &str =
        "sha256:bb766f710eef8ede859c18578c72c327597cd4c8a85b06001b1f3843c6019386";

    fn gh_policy() -> Policy {
        Policy {
            org: "cli".into(),
            workflow: ".github/workflows/deployment.yml".into(),
            refs: "refs/heads/".into(),
            keys: Vec::new(),
        }
    }

    pub(crate) fn sign_with(key: &KeyPair, digest: &Digest) -> Vec<u8> {
        let statement = serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [{"name": "", "digest": {"sha256": digest.hex()}}],
            "predicateType": "https://sigstore.dev/cosign/sign/v1",
            "predicate": {}
        });
        let payload = statement.to_string();
        let pae = sigstore_verify::types::pae("application/vnd.in-toto+json", payload.as_bytes());
        let sig = key.sign(&pae).unwrap();
        serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "verificationMaterial": {"publicKey": {"hint": ""}},
            "dsseEnvelope": {
                "payload": STANDARD.encode(payload),
                "payloadType": "application/vnd.in-toto+json",
                "signatures": [{"sig": STANDARD.encode(sig.as_bytes()), "keyid": ""}]
            }
        })
        .to_string()
        .into_bytes()
    }

    pub(crate) fn pem(key: &KeyPair) -> String {
        key.public_key_der().unwrap().to_pem()
    }

    #[test]
    fn real_provenance_verifies_and_is_matched_to_its_repository() {
        let d = Digest::parse(GH_DIGEST).unwrap();
        let e = verify(&gh_policy(), &[(d, GH_PROVENANCE.as_bytes().to_vec())]).unwrap_err();
        // Cryptographically sound, but a provenance alone is not a release.
        assert!(
            e.0.contains("provenance checks, but there is no signature"),
            "{e}"
        );
    }

    #[test]
    fn a_tampered_digest_is_refused() {
        let other = Digest::of(b"another artifact");
        let e = verify(&gh_policy(), &[(other, GH_PROVENANCE.as_bytes().to_vec())]).unwrap_err();
        assert!(e.0.contains("does not verify"), "{e}");
    }

    #[test]
    fn the_wrong_signer_is_refused() {
        let d = Digest::parse(GH_DIGEST).unwrap();
        let e = verify(
            &Policy::inorbit(Vec::new()),
            &[(d, GH_PROVENANCE.as_bytes().to_vec())],
        )
        .unwrap_err();
        assert!(
            e.0.contains("which is not https://github.com/inorbithr/"),
            "{e}"
        );
    }

    #[test]
    fn a_tampered_bundle_is_refused() {
        let d = Digest::parse(GH_DIGEST).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(GH_PROVENANCE).unwrap();
        let sig = v["dsseEnvelope"]["signatures"][0]["sig"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut raw = STANDARD.decode(sig).unwrap();
        let last = raw.len() - 2;
        raw[last] ^= 1;
        v["dsseEnvelope"]["signatures"][0]["sig"] = STANDARD.encode(raw).into();
        let e = verify(&gh_policy(), &[(d, v.to_string().into_bytes())]).unwrap_err();
        assert!(e.0.contains("does not verify"), "{e}");
    }

    #[test]
    fn a_configured_key_is_trusted_and_another_is_not() {
        let d = Digest::of(b"artifact");
        let key = KeyPair::generate_ecdsa_p256().unwrap();
        let bundle = sign_with(&key, &d);
        let trusted = TrustedKey::from_pem(&pem(&key)).unwrap();
        let fp = trusted.fingerprint().to_owned();
        let ok = verify(
            &Policy::inorbit(vec![trusted]),
            &[(d.clone(), bundle.clone())],
        )
        .unwrap();
        assert_eq!(ok.signer, Signer::Key { fingerprint: fp });

        let stranger =
            TrustedKey::from_pem(&pem(&KeyPair::generate_ecdsa_p256().unwrap())).unwrap();
        let e = verify(
            &Policy::inorbit(vec![stranger]),
            &[(d.clone(), bundle.clone())],
        )
        .unwrap_err();
        assert!(e.0.contains("no key in ext.trusted_keys verifies"), "{e}");
        let e = verify(&Policy::inorbit(Vec::new()), &[(d, bundle.clone())]).unwrap_err();
        assert!(e.0.contains("no key is configured"), "{e}");

        let other = Digest::of(b"tampered");
        let trusted = TrustedKey::from_pem(&pem(&key)).unwrap();
        assert!(verify(&Policy::inorbit(vec![trusted]), &[(other, bundle)]).is_err());
    }

    #[test]
    fn nothing_attached_is_refused() {
        let e = verify(&Policy::inorbit(Vec::new()), &[]).unwrap_err();
        assert!(e.0.contains("no Sigstore bundle"), "{e}");
        let e = verify(
            &Policy::inorbit(Vec::new()),
            &[(Digest::of(b"x"), b"not json".to_vec())],
        )
        .unwrap_err();
        assert!(e.0.contains("cannot be read"), "{e}");
    }

    #[test]
    fn workflow_identities() {
        let p = Policy::inorbit(Vec::new());
        assert_eq!(
            p.repository_of(
                "https://github.com/inorbithr/agent/.github/workflows/release.yml@refs/tags/v0.1.0"
            )
            .as_deref(),
            Some("https://github.com/inorbithr/agent")
        );
        for bad in [
            "https://github.com/inorbithr/agent/.github/workflows/release.yml@refs/heads/main",
            "https://github.com/inorbithr/agent/.github/workflows/ci.yml@refs/tags/v1",
            "https://github.com/evil/agent/.github/workflows/release.yml@refs/tags/v1",
            "https://github.com/inorbithrx/agent/.github/workflows/release.yml@refs/tags/v1",
            "https://github.com/inorbithr/agent/.github/workflows/release.yml@refs/tags/",
            "https://gitlab.com/inorbithr/agent/.github/workflows/release.yml@refs/tags/v1",
        ] {
            assert!(p.repository_of(bad).is_none(), "{bad}");
        }
        let s = Signer::Workflow {
            identity:
                "https://github.com/inorbithr/agent/.github/workflows/release.yml@refs/tags/v1"
                    .into(),
            repository: String::new(),
        };
        assert_eq!(
            s.lock_name(),
            "https://github.com/inorbithr/agent/.github/workflows/release.yml"
        );
    }
}
