//! PKCE (RFC 7636, S256 only), the `state` value, and a constant-time comparison.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

use crate::secret::Secret;

/// The system could not produce random bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the system has no randomness available: {0}")]
pub struct RandomError(String);

/// 32 random bytes as base64url: 43 characters, the RFC's recommended verifier, and the
/// shape of the `state` value.
///
/// # Errors
///
/// [`RandomError`] when the system has no randomness.
pub fn random_token() -> Result<Secret<String>, RandomError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| RandomError(e.to_string()))?;
    Ok(Secret::new(URL_SAFE_NO_PAD.encode(bytes)))
}

/// A PKCE verifier and its S256 challenge. `plain` is never offered.
#[derive(Debug)]
pub struct Pkce {
    /// The secret the token request proves possession with.
    pub verifier: Secret<String>,
    /// `BASE64URL(SHA256(verifier))`, sent with the authorization request.
    pub challenge: String,
}

impl Pkce {
    /// A fresh verifier and its challenge.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the system has no randomness.
    pub fn new() -> Result<Self, RandomError> {
        let verifier = random_token()?;
        let challenge = challenge(verifier.expose());
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

/// `BASE64URL(SHA256(verifier))`.
#[must_use]
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Compares two byte strings in time that depends only on their lengths.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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
