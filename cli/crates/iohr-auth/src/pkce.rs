//! PKCE (RFC 7636, S256 only), the `state` value, and a constant-time comparison.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

use crate::error::AuthError;
use crate::secret::Redacted;

/// 32 random bytes as base64url: 43 characters, the RFC's recommended verifier.
pub(crate) fn random_token() -> Result<Redacted<String>, AuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| AuthError::Random(e.to_string()))?;
    Ok(Redacted::new(URL_SAFE_NO_PAD.encode(bytes)))
}

/// A PKCE verifier and its S256 challenge. `plain` is never offered.
#[derive(Debug)]
pub(crate) struct Pkce {
    pub(crate) verifier: Redacted<String>,
    pub(crate) challenge: String,
}

impl Pkce {
    pub(crate) fn new() -> Result<Self, AuthError> {
        let verifier = random_token()?;
        let challenge = challenge(verifier.expose());
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

/// `BASE64URL(SHA256(verifier))`.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Compares two byte strings in time that depends only on their lengths.
#[must_use]
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::{Pkce, challenge, constant_time_eq, random_token};

    /// RFC 7636, appendix B.
    #[test]
    fn s256_matches_the_rfc_example() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifiers_are_long_unreserved_and_fresh() {
        let a = Pkce::new().unwrap();
        let b = Pkce::new().unwrap();
        let v = a.verifier.expose();
        assert_eq!(v.len(), 43);
        assert!(
            v.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(random_token().unwrap(), random_token().unwrap());
    }

    #[test]
    fn constant_time_eq_is_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abx"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
