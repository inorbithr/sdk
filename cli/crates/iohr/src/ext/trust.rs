//! Who may publish an extension, and the check that they did (ADR 0012, SR-26).
//!
//! An artifact is trusted when Sigstore bundles attached to it prove one of:
//!
//! - **InOrbit's release workflow signed it.** A bundle whose certificate Fulcio
//!   issued to `https://github.com/inorbithr/<repo>/.github/workflows/release.yml` at a
//!   tag, for the GitHub Actions issuer, logged in Rekor, signing an in-toto statement
//!   whose subject is the artifact's digest; and a second such bundle with the same
//!   certificate identity carrying SLSA v1 provenance that names that repository,
//!   workflow and ref. The repository and ref are the certificate's (its identity and
//!   Fulcio's source repository claims); the provenance must agree with them, and is
//!   never what decides them. Both are checked offline against the Sigstore trusted
//!   root built into `iohr`. An extension's repository is `<name>` or
//!   `iohr-ext-<name>` ([`Signer::publishes`]).
//! - **For the `iohr` release archives**, exactly [`IOHR_RELEASE_IDENTITY`]: the release
//!   workflow of `inorbithr/sdk` on `main`, and no other repository, branch or tag.
//! - **A key the company configured signed it** (`ext.trusted_keys`), for a mirror that
//!   re-signs what it copies and for an internal channel: a bundle with that public
//!   key's signature over a statement naming the digest. Its transparency-log entry is
//!   checked when the bundle carries one.
//!
//! Anything else is refused, and there is no switch that skips the check.

use std::fmt;

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sigstore_verify::crypto::x509::FulcioCiClaims;
use sigstore_verify::types::bundle::VerificationMaterialContent;
use sigstore_verify::types::{Bundle, DerPublicKey, Sha256Hash, SignatureContent, Statement};
use sigstore_verify::{PublicKeyVerificationPolicy, SubjectAltName, VerificationPolicy, Verifier};

use super::oci::{Digest, hex};

/// The OIDC issuer of GitHub Actions' workload identity tokens.
pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";
/// The SLSA provenance predicate type.
pub const SLSA_PROVENANCE: &str = "https://slsa.dev/provenance/v1";

/// The workflow every InOrbit release is signed by, in its repository.
pub const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";
/// The repository the `iohr` release archives are built and signed in.
pub const IOHR_RELEASE_REPOSITORY: &str = "https://github.com/inorbithr/sdk";
/// The one ref the `iohr` release archives are signed at: the release workflow runs on a
/// push to `main` (the release PR's merge) and checks the tag out, so its certificate
/// names `main`, never the tag.
pub const IOHR_RELEASE_REF: &str = "refs/heads/main";
/// The one certificate identity (SAN) accepted for an `iohr` release archive.
/// `cli/install/install.sh` and `install.ps1` pass this, and [`IOHR_RELEASE_REF`], to
/// `gh attestation verify`; a test holds them to these constants.
pub const IOHR_RELEASE_IDENTITY: &str =
    "https://github.com/inorbithr/sdk/.github/workflows/release.yml@refs/heads/main";

/// The workflow identity extensions are published by, and the keys a company added.
#[derive(Debug, Clone)]
pub struct Policy {
    org: String,
    workflow: String,
    refs: Refs,
    keys: Vec<TrustedKey>,
}

/// The refs a policy accepts a workflow at.
#[derive(Debug, Clone)]
enum Refs {
    /// Any ref under this prefix that names one (`refs/tags/`), in any of the org's
    /// repositories.
    Under(String),
    /// Exactly one repository (`https://github.com/<org>/<repo>`) at exactly one ref.
    Exactly {
        repository: String,
        reference: String,
    },
}

impl Policy {
    /// InOrbit's release workflow, plus `keys`.
    #[must_use]
    pub fn inorbit(keys: Vec<TrustedKey>) -> Self {
        Self {
            org: "inorbithr".into(),
            workflow: RELEASE_WORKFLOW.into(),
            refs: Refs::Under("refs/tags/".into()),
            keys,
        }
    }

    /// The policy for the `iohr` release archives themselves (`iohr upgrade`, the
    /// runner's upgrade): exactly [`IOHR_RELEASE_IDENTITY`], with Fulcio's source
    /// repository and ref claims naming [`IOHR_RELEASE_REPOSITORY`] and
    /// [`IOHR_RELEASE_REF`]. No company keys: only the workflow is accepted.
    #[must_use]
    pub fn iohr_release() -> Self {
        Self {
            org: "inorbithr".into(),
            workflow: RELEASE_WORKFLOW.into(),
            refs: Refs::Exactly {
                repository: IOHR_RELEASE_REPOSITORY.into(),
                reference: IOHR_RELEASE_REF.into(),
            },
            keys: Vec::new(),
        }
    }

    /// The identity this policy accepts, for messages and documentation.
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.refs {
            Refs::Under(prefix) => format!(
                "https://github.com/{}/<repository>/{}@{prefix}*",
                self.org, self.workflow
            ),
            Refs::Exactly {
                repository,
                reference,
            } => format!("{repository}/{}@{reference}", self.workflow),
        }
    }

    /// The repository and ref a certificate identity names, when it is this policy's
    /// workflow at an accepted ref.
    fn repository_of(&self, identity: &str) -> Option<(String, String)> {
        let rest = identity
            .strip_prefix("https://github.com/")?
            .strip_prefix(&self.org)?
            .strip_prefix('/')?;
        let (repo, rest) = rest.split_once('/')?;
        let reference = rest.strip_prefix(&self.workflow)?.strip_prefix('@')?;
        let repository = format!("https://github.com/{}/{repo}", self.org);
        let ok = !repo.is_empty()
            && repo
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            && self.accepts_ref(&repository, reference);
        ok.then(|| (repository, reference.to_owned()))
    }

    fn accepts_ref(&self, repository: &str, reference: &str) -> bool {
        match &self.refs {
            Refs::Under(prefix) => reference
                .strip_prefix(prefix)
                .is_some_and(|t| !t.is_empty()),
            Refs::Exactly {
                repository: r,
                reference: want,
            } => repository == r && reference == want,
        }
    }

    /// Holds Fulcio's claims about the build to the certificate's identity: the build
    /// signer is that identity, and the source repository and ref are the identity's
    /// repository and an accepted ref. A certificate that carries none of them (one
    /// issued before Fulcio had them) is judged by its identity alone, except under
    /// [`Refs::Exactly`], which requires them. Returns the ref the artifact was built
    /// from.
    fn check_claims(
        &self,
        identity: &str,
        repository: &str,
        reference: &str,
        claims: Option<&FulcioCiClaims>,
    ) -> Result<String, String> {
        let exact = matches!(self.refs, Refs::Exactly { .. });
        let shown = |s: &str| super::manifest::printable(s);
        let empty = FulcioCiClaims::default();
        let claims = claims.unwrap_or(&empty);
        if let Some(signer) = &claims.build_signer_uri
            && signer != identity
        {
            return Err(format!(
                "a certificate for {} whose build signer is {}",
                shown(identity),
                shown(signer)
            ));
        }
        match &claims.source_repository_uri {
            Some(r) if r != repository => {
                return Err(format!(
                    "a certificate for {} built from {}",
                    shown(identity),
                    shown(r)
                ));
            }
            None if exact => {
                return Err(format!(
                    "a certificate for {} that names no source repository",
                    shown(identity)
                ));
            }
            _ => {}
        }
        match &claims.source_repository_ref {
            Some(r) if !self.accepts_ref(repository, r) => Err(format!(
                "a certificate for {} built from {}, which is not {}",
                shown(identity),
                shown(r),
                self.describe()
            )),
            Some(r) => Ok(r.clone()),
            None if exact => Err(format!(
                "a certificate for {} that names no source ref",
                shown(identity)
            )),
            None => Ok(reference.to_owned()),
        }
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

/// The InOrbit repositories that publish extensions not named after them, and which.
/// Any other repository publishes only the extension it is named after.
const PUBLISHED_FROM: &[(&str, &[&str])] = &[(
    "https://github.com/inorbithr/dataplane",
    &["agent", "capture"],
)];

impl Signer {
    /// Whether this signer may publish the extension `name`. A release workflow
    /// publishes only from the repository named after the extension (`<name>` or
    /// `iohr-ext-<name>`), or one [`PUBLISHED_FROM`] lists for it; a key a company
    /// configured in `ext.trusted_keys` may sign any of its extensions.
    #[must_use]
    pub fn publishes(&self, name: &str) -> bool {
        match self {
            Self::Workflow { repository, .. } => {
                let base = repository.rsplit('/').next().unwrap_or_default();
                base == name
                    || base.strip_prefix("iohr-ext-") == Some(name)
                    || PUBLISHED_FROM
                        .iter()
                        .any(|(r, names)| r == repository && names.contains(&name))
            }
            Self::Key { .. } => true,
        }
    }

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
        /// The ref the certificate says the artifact was built from.
        reference: String,
        provenance: bool,
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

/// Checks the GitHub attestations of an `iohr` release archive: signed with exactly
/// [`IOHR_RELEASE_IDENTITY`], whose certificate names [`IOHR_RELEASE_REPOSITORY`] at
/// [`IOHR_RELEASE_REF`], with SLSA provenance that agrees, and bound to `digest`.
/// Anything else is refused.
///
/// # Errors
///
/// [`TrustError`] unless [`verify`] accepts it under [`Policy::iohr_release`].
pub fn verify_iohr_release(digest: &Digest, bundles: &[Vec<u8>]) -> Result<Verified, TrustError> {
    let pairs: Vec<(Digest, Vec<u8>)> = bundles
        .iter()
        .map(|b| (digest.clone(), b.clone()))
        .collect();
    let verified = verify(&Policy::iohr_release(), &pairs)?;
    let want = format!("{IOHR_RELEASE_REPOSITORY}@{IOHR_RELEASE_REF}");
    let exact = matches!(
        &verified.signer,
        Signer::Workflow { identity, repository }
            if identity == IOHR_RELEASE_IDENTITY && repository == IOHR_RELEASE_REPOSITORY
    );
    if exact && verified.provenance.as_deref() == Some(want.as_str()) {
        Ok(verified)
    } else {
        Err(TrustError(format!(
            "not installed: the attestation names {}, not {want}",
            super::manifest::printable(&format!(
                "{} ({})",
                verified.signer,
                verified.provenance.as_deref().unwrap_or("no provenance")
            ))
        )))
    }
}

fn decide(policy: &Policy, findings: &[Finding]) -> Result<Verified, TrustError> {
    // A release: a signature and a provenance under one certificate identity, built
    // from one repository at one ref.
    for f in findings {
        if let Finding::Release {
            identity,
            repository,
            reference,
            provenance: false,
        } = f
            && findings.iter().any(|g| {
                matches!(g, Finding::Release {
                    identity: i,
                    repository: r,
                    reference: x,
                    provenance: true,
                } if i == identity && r == repository && x == reference)
            })
        {
            return Ok(Verified {
                signer: Signer::Workflow {
                    identity: identity.clone(),
                    repository: repository.clone(),
                },
                provenance: Some(format!("{repository}@{reference}")),
            });
        }
    }
    if let Some(fingerprint) = findings.iter().find_map(|f| match f {
        Finding::Key(k) => Some(k.clone()),
        _ => None,
    }) {
        let provenance = findings.iter().find_map(|f| match f {
            Finding::Release {
                repository,
                reference,
                provenance: true,
                ..
            } => Some(format!("{repository}@{reference}")),
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
                provenance: false,
                ..
            }
        )
    });
    let has_release_prov = findings.iter().any(|f| {
        matches!(
            f,
            Finding::Release {
                provenance: true,
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
    judge(
        policy,
        &identity,
        result.certificate().map(|c| &c.ci_claims),
        &statement,
    )
}

/// Decides a bundle whose signature, certificate chain and log entry have verified,
/// from what its certificate says (the identity and Fulcio's claims) and its statement.
/// The repository and ref are the certificate's; the provenance must agree with them.
fn judge(
    policy: &Policy,
    identity: &str,
    claims: Option<&FulcioCiClaims>,
    statement: &Statement,
) -> Finding {
    let Some((repository, named)) = policy.repository_of(identity) else {
        return Finding::Refused(format!(
            "a signature by {}, which is not {}",
            super::manifest::printable(identity),
            policy.describe()
        ));
    };
    let reference = match policy.check_claims(identity, &repository, &named, claims) {
        Ok(r) => r,
        Err(why) => return Finding::Refused(why),
    };
    let is_provenance = statement.predicate_type == SLSA_PROVENANCE;
    if is_provenance
        && let Err(why) = provenance(policy, &repository, &reference, &statement.predicate)
    {
        return Finding::Refused(why);
    }
    Finding::Release {
        identity: identity.to_owned(),
        repository,
        reference,
        provenance: is_provenance,
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
                .verify(
                    subject,
                    bundle,
                    &PublicKeyVerificationPolicy::new(key.der.clone()),
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

/// Checks that SLSA v1 provenance from GitHub Actions agrees with the certificate: the
/// same repository, the policy's workflow and the certificate's ref. It never widens
/// what the certificate established.
fn provenance(
    policy: &Policy,
    repository: &str,
    reference: &str,
    predicate: &Value,
) -> Result<(), String> {
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
    if field("ref") != reference {
        return Err(format!(
            "provenance from {}, while its certificate names {}",
            super::manifest::printable(field("ref")),
            super::manifest::printable(reference)
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use sigstore_verify::crypto::KeyPair;

    use super::{
        Finding, FulcioCiClaims, IOHR_RELEASE_IDENTITY, IOHR_RELEASE_REF, IOHR_RELEASE_REPOSITORY,
        Policy, RELEASE_WORKFLOW, SLSA_PROVENANCE, Signer, Statement, TrustedKey, decide, judge,
        verify,
    };
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
            refs: super::Refs::Under("refs/heads/".into()),
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

    /// The two GitHub attestations (provenance and SBOM) of the real
    /// `iohr-0.1.0-alpha.14-aarch64-apple-darwin.tar.gz`, from the attestations API.
    const IOHR_A14: [&str; 2] = [
        include_str!("testdata/iohr-alpha14-aarch64-apple-darwin.0.sigstore.json"),
        include_str!("testdata/iohr-alpha14-aarch64-apple-darwin.1.sigstore.json"),
    ];
    const IOHR_A14_DIGEST: &str =
        "sha256:aa46a9a72b20f8da8025060bf3e182eedde541b362c4839aa0955e33e6cbe364";

    fn a14_bundles() -> Vec<Vec<u8>> {
        IOHR_A14.iter().map(|b| b.as_bytes().to_vec()).collect()
    }

    #[test]
    fn a_real_iohr_release_archive_verifies() {
        let d = Digest::parse(IOHR_A14_DIGEST).unwrap();
        let v = super::verify_iohr_release(&d, &a14_bundles()).unwrap();
        assert_eq!(
            v.provenance.as_deref(),
            Some("https://github.com/inorbithr/sdk@refs/heads/main")
        );
    }

    #[test]
    fn an_iohr_archive_with_another_digest_or_no_bundle_is_refused() {
        let other = Digest::of(b"a tarball someone swapped in");
        let e = super::verify_iohr_release(&other, &a14_bundles()).unwrap_err();
        assert!(e.0.contains("does not verify"), "{e}");
        let d = Digest::parse(IOHR_A14_DIGEST).unwrap();
        let e = super::verify_iohr_release(&d, &[]).unwrap_err();
        assert!(e.0.contains("holds no Sigstore bundle"), "{e}");
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
            .map(|(r, _)| r)
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

    /// What Fulcio writes into a GitHub Actions certificate for `identity`, built from
    /// `repository` at `reference`.
    fn claims(identity: &str, repository: &str, reference: &str) -> FulcioCiClaims {
        let mut c = FulcioCiClaims::default();
        c.build_signer_uri = Some(identity.into());
        c.source_repository_uri = Some(repository.into());
        c.source_repository_ref = Some(reference.into());
        c
    }

    /// An in-toto statement of `predicate_type`; SLSA provenance names `repository`,
    /// the release workflow and `reference`.
    fn statement(predicate_type: &str, repository: &str, reference: &str) -> Statement {
        serde_json::from_value(serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [{"name": "a", "digest": {"sha256": IOHR_A14_DIGEST.trim_start_matches("sha256:")}}],
            "predicateType": predicate_type,
            "predicate": {"buildDefinition": {"externalParameters": {"workflow": {
                "repository": repository, "path": RELEASE_WORKFLOW, "ref": reference
            }}}}
        }))
        .unwrap()
    }

    fn identity(repository: &str, reference: &str) -> String {
        format!("{repository}/{RELEASE_WORKFLOW}@{reference}")
    }

    fn accepted(f: &Finding) -> bool {
        matches!(f, Finding::Release { .. })
    }

    #[test]
    fn the_release_identity_is_the_sdk_release_workflow_on_main() {
        assert_eq!(
            IOHR_RELEASE_IDENTITY,
            identity(IOHR_RELEASE_REPOSITORY, IOHR_RELEASE_REF)
        );
        assert_eq!(IOHR_RELEASE_REF, "refs/heads/main");
        assert_eq!(
            Policy::iohr_release().describe(),
            IOHR_RELEASE_IDENTITY,
            "the refusal message names the one accepted identity"
        );
    }

    #[test]
    fn the_iohr_release_policy_accepts_only_main_of_the_sdk() {
        let p = Policy::iohr_release();
        let sdk = IOHR_RELEASE_REPOSITORY;
        let main = IOHR_RELEASE_REF;
        for st in [
            statement(SLSA_PROVENANCE, sdk, main),
            statement("https://cyclonedx.org/bom", sdk, main),
        ] {
            let f = judge(
                &p,
                IOHR_RELEASE_IDENTITY,
                Some(&claims(IOHR_RELEASE_IDENTITY, sdk, main)),
                &st,
            );
            assert!(accepted(&f), "{f:?}");
        }
        // Another branch, a tag, a look-alike ref, another repository or workflow, each
        // with claims that agree with its own identity.
        let other = "https://github.com/inorbithr/agent";
        for (repository, reference, workflow) in [
            (sdk, "refs/heads/feature", RELEASE_WORKFLOW),
            (sdk, "refs/heads/main-next", RELEASE_WORKFLOW),
            (sdk, "refs/heads/main/x", RELEASE_WORKFLOW),
            (sdk, "refs/tags/iohr/v1.0.0", RELEASE_WORKFLOW),
            (sdk, "refs/pull/1/merge", RELEASE_WORKFLOW),
            (other, main, RELEASE_WORKFLOW),
            (sdk, main, ".github/workflows/ci.yml"),
        ] {
            let id = format!("{repository}/{workflow}@{reference}");
            let f = judge(
                &p,
                &id,
                Some(&claims(&id, repository, reference)),
                &statement(SLSA_PROVENANCE, repository, reference),
            );
            assert!(!accepted(&f), "{id}: {f:?}");
        }
    }

    #[test]
    fn a_branch_certificate_with_provenance_naming_main_is_refused() {
        let sdk = IOHR_RELEASE_REPOSITORY;
        let branch = "refs/heads/feature";
        let id = identity(sdk, branch);
        let f = judge(
            &Policy::iohr_release(),
            &id,
            Some(&claims(&id, sdk, branch)),
            &statement(SLSA_PROVENANCE, sdk, IOHR_RELEASE_REF),
        );
        assert!(!accepted(&f), "{f:?}");
        // Under a policy that accepts every branch, the provenance must still name the
        // certificate's own ref.
        let p = Policy {
            refs: super::Refs::Under("refs/heads/".into()),
            ..Policy::iohr_release()
        };
        let f = judge(
            &p,
            &id,
            Some(&claims(&id, sdk, branch)),
            &statement(SLSA_PROVENANCE, sdk, IOHR_RELEASE_REF),
        );
        let Finding::Refused(why) = f else {
            panic!("{f:?}")
        };
        assert!(
            why.contains("while its certificate names refs/heads/feature"),
            "{why}"
        );
    }

    #[test]
    fn certificate_claims_must_agree_with_the_release_identity() {
        let p = Policy::iohr_release();
        let sdk = IOHR_RELEASE_REPOSITORY;
        let main = IOHR_RELEASE_REF;
        let st = statement(SLSA_PROVENANCE, sdk, main);
        let mut cases = vec![
            claims(IOHR_RELEASE_IDENTITY, sdk, "refs/heads/feature"),
            claims(
                IOHR_RELEASE_IDENTITY,
                "https://github.com/inorbithr/agent",
                main,
            ),
            claims(&identity(sdk, "refs/heads/feature"), sdk, main),
        ];
        let mut no_ref = claims(IOHR_RELEASE_IDENTITY, sdk, main);
        no_ref.source_repository_ref = None;
        let mut no_repo = claims(IOHR_RELEASE_IDENTITY, sdk, main);
        no_repo.source_repository_uri = None;
        cases.extend([no_ref, no_repo, FulcioCiClaims::default()]);
        for c in &cases {
            let f = judge(&p, IOHR_RELEASE_IDENTITY, Some(c), &st);
            assert!(!accepted(&f), "{c:?}: {f:?}");
        }
        let f = judge(&p, IOHR_RELEASE_IDENTITY, None, &st);
        assert!(!accepted(&f), "{f:?}");
    }

    #[test]
    fn an_extension_release_is_judged_by_its_certificate_ref() {
        let p = Policy::inorbit(Vec::new());
        let repo = "https://github.com/inorbithr/agent";
        let tag = "refs/tags/v1.0.0";
        let id = identity(repo, tag);
        let f = judge(
            &p,
            &id,
            Some(&claims(&id, repo, tag)),
            &statement(SLSA_PROVENANCE, repo, tag),
        );
        assert!(
            matches!(&f, Finding::Release { reference, provenance: true, .. } if reference == tag),
            "{f:?}"
        );
        // A tag certificate whose claims name a branch, and one whose provenance names
        // another tag.
        for (c, st) in [
            (
                claims(&id, repo, "refs/heads/main"),
                statement(SLSA_PROVENANCE, repo, tag),
            ),
            (
                claims(&id, repo, tag),
                statement(SLSA_PROVENANCE, repo, "refs/tags/v2.0.0"),
            ),
        ] {
            let f = judge(&p, &id, Some(&c), &st);
            assert!(!accepted(&f), "{f:?}");
        }
        // A certificate from before Fulcio's source claims is judged by its identity.
        let f = judge(&p, &id, None, &statement(SLSA_PROVENANCE, repo, tag));
        assert!(accepted(&f), "{f:?}");
    }

    #[test]
    fn a_signature_and_provenance_pair_only_under_one_identity() {
        let p = Policy::inorbit(Vec::new());
        let repo = "https://github.com/inorbithr/agent";
        let release = |tag: &str, provenance: bool| Finding::Release {
            identity: identity(repo, tag),
            repository: repo.into(),
            reference: tag.into(),
            provenance,
        };
        let v = decide(
            &p,
            &[
                release("refs/tags/v1", false),
                release("refs/tags/v1", true),
            ],
        )
        .unwrap();
        assert_eq!(
            v.provenance.as_deref(),
            Some("https://github.com/inorbithr/agent@refs/tags/v1")
        );
        let e = decide(
            &p,
            &[
                release("refs/tags/v1", false),
                release("refs/tags/v2", true),
            ],
        )
        .unwrap_err();
        assert!(e.0.contains("no trusted signature"), "{e}");
    }

    #[test]
    fn an_extension_is_published_by_the_repository_named_after_it() {
        let wf = |repo: &str| Signer::Workflow {
            identity: identity(repo, "refs/tags/v1"),
            repository: repo.into(),
        };
        assert!(wf("https://github.com/inorbithr/todo").publishes("todo"));
        assert!(wf("https://github.com/inorbithr/iohr-ext-todo").publishes("todo"));
        assert!(wf("https://github.com/inorbithr/dataplane").publishes("agent"));
        assert!(wf("https://github.com/inorbithr/dataplane").publishes("capture"));
        assert!(!wf("https://github.com/inorbithr/dataplane").publishes("todo"));
        assert!(!wf("https://github.com/inorbithr/iohr-ext-todo").publishes("agent"));
        assert!(!wf("https://github.com/inorbithr/todo-extra").publishes("todo"));
        assert!(!wf("https://github.com/inorbithr/sdk").publishes("agent"));
        let key = Signer::Key {
            fingerprint: "key:sha256:00".into(),
        };
        assert!(key.publishes("anything"));
    }

    #[test]
    fn the_installers_pin_the_release_identity() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install");
        for (script, identity, source_ref) in [
            ("install.sh", "identity=", "source_ref="),
            ("install.ps1", "$identity = ", "$sourceRef = "),
        ] {
            let text = std::fs::read_to_string(dir.join(script)).unwrap();
            let value = |key: &str| {
                text.lines()
                    .find_map(|l| l.trim().strip_prefix(key))
                    .map(|v| v.trim_matches(['"', '\''].as_slice()).to_owned())
                    .unwrap_or_else(|| panic!("{script} sets no {key}"))
            };
            assert_eq!(value(identity), IOHR_RELEASE_IDENTITY, "{script}");
            assert_eq!(value(source_ref), IOHR_RELEASE_REF, "{script}");
            for flag in ["--cert-identity", "--source-ref"] {
                assert!(text.contains(flag), "{script} does not pass {flag}");
            }
        }
    }
}
