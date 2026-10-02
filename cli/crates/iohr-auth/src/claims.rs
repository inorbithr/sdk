use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use time::OffsetDateTime;

/// The audience every InOrbit API token carries.
pub(crate) const AUDIENCE: &str = "iohr-api";

/// What an InOrbit access token says about itself, read without verifying it.
///
/// The command line uses this only to name a profile's account and to warn about an
/// expired or foreign token before sending it. The gateway verifies every token; a
/// forged claim here changes nothing the API will do.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Claims {
    /// The subject: a person's id, or the key id for an API key or token.
    pub sub: String,
    /// The account the calls count against.
    #[serde(default)]
    pub org: Option<String>,
    /// The account's plan.
    #[serde(default)]
    pub plan: Option<String>,
    /// The scopes the token holds.
    #[serde(default, deserialize_with = "scopes")]
    pub scp: Vec<String>,
    /// Expiry, seconds since the Unix epoch.
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default, deserialize_with = "audiences")]
    aud: Vec<String>,
}

/// Why a value is not an InOrbit access token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClaimsError {
    /// Not three dot-separated base64url parts with a JSON payload.
    #[error("this is not an InOrbit access token: it should start with eyJ and contain two dots")]
    Malformed,
    /// A key's client id rather than a token.
    #[error(
        "this is an API key's client id, not a token: exchange the key for a token, or create a token in the console"
    )]
    KeyId,
    /// A token for another API.
    #[error("this token is not for the InOrbit API (audience {AUDIENCE})")]
    Audience,
    /// The token has expired.
    #[error("this token has expired: create a new one")]
    Expired,
}

impl Claims {
    /// Reads the claims of `token` and checks its audience and expiry.
    ///
    /// # Errors
    ///
    /// [`ClaimsError`] when the value is not a JWT, is a key's client id, is for
    /// another audience, or has expired at `now`.
    pub fn read(token: &str, now: OffsetDateTime) -> Result<Self, ClaimsError> {
        let token = token.trim();
        if token.starts_with("ak_") {
            return Err(ClaimsError::KeyId);
        }
        let mut parts = token.split('.');
        let (Some(_), Some(payload), Some(_), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ClaimsError::Malformed);
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| ClaimsError::Malformed)?;
        let claims: Self = serde_json::from_slice(&bytes).map_err(|_| ClaimsError::Malformed)?;
        if !claims.aud.iter().any(|a| a == AUDIENCE) {
            return Err(ClaimsError::Audience);
        }
        if claims.expires_at().is_some_and(|exp| exp <= now) {
            return Err(ClaimsError::Expired);
        }
        Ok(claims)
    }

    /// When the token expires, if it says.
    #[must_use]
    pub fn expires_at(&self) -> Option<OffsetDateTime> {
        self.exp
            .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
    }
}

fn scopes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Scopes {
        List(Vec<String>),
        Spaced(String),
    }
    Ok(match Option::<Scopes>::deserialize(d)? {
        Some(Scopes::List(v)) => v,
        Some(Scopes::Spaced(s)) => s.split_whitespace().map(str::to_owned).collect(),
        None => Vec::new(),
    })
}

fn audiences<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Aud {
        List(Vec<String>),
        One(String),
    }
    Ok(match Option::<Aud>::deserialize(d)? {
        Some(Aud::List(v)) => v,
        Some(Aud::One(s)) => vec![s],
        None => Vec::new(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use time::OffsetDateTime;

    use super::{Claims, ClaimsError};

    /// An unsigned token with the given claims, for tests.
    pub(crate) fn token(claims: &serde_json::Value) -> String {
        let enc = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!(
            "{}.{}.c2ln",
            enc(&serde_json::json!({"alg": "RS256"})),
            enc(claims)
        )
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap()
    }

    #[test]
    fn reads_an_api_token() {
        let t = token(&serde_json::json!({
            "sub": "ak_x", "aud": ["iohr-api"], "exp": 1_790_000_600,
            "scp": ["radar:read", "identity:read"], "org": "acc_1", "plan": "free"
        }));
        let c = Claims::read(&t, now()).unwrap();
        assert_eq!(c.org.as_deref(), Some("acc_1"));
        assert_eq!(c.scp, ["radar:read", "identity:read"]);
    }

    #[test]
    fn a_spaced_scope_string_and_a_single_audience_are_read_too() {
        let t = token(&serde_json::json!({"sub": "u", "aud": "iohr-api", "scp": "a:b c:d"}));
        assert_eq!(Claims::read(&t, now()).unwrap().scp, ["a:b", "c:d"]);
    }

    #[test]
    fn refuses_what_is_not_a_usable_token() {
        let expired = token(&serde_json::json!({"sub": "u", "aud": ["iohr-api"], "exp": 1}));
        let foreign = token(&serde_json::json!({"sub": "u", "aud": ["other"]}));
        assert_eq!(Claims::read(&expired, now()), Err(ClaimsError::Expired));
        assert_eq!(Claims::read(&foreign, now()), Err(ClaimsError::Audience));
        assert_eq!(Claims::read("ak_123", now()), Err(ClaimsError::KeyId));
        assert_eq!(Claims::read("hello", now()), Err(ClaimsError::Malformed));
        assert_eq!(Claims::read("a.b.c.d", now()), Err(ClaimsError::Malformed));
        assert_eq!(Claims::read("a.!!!.c", now()), Err(ClaimsError::Malformed));
    }
}
