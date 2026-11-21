use super::{ClaimsError, ConfigError, RandomError, StoreError};

/// Why a sign-in, a refresh or a sign-out failed.
///
/// No variant carries a token, an authorization code or a device code: messages are
/// fixed sentences plus the sign-in service's own error code (SR-10, SR-11).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoginError {
    /// The profile has no secret in its store.
    #[error("profile {profile} has no credential on this machine: sign in again")]
    NotSignedIn {
        /// The profile's name.
        profile: String,
    },
    /// The session can no longer be refreshed: it expired, was revoked, or was signed
    /// out from the console.
    #[error("the session of profile {profile} has ended: sign in again")]
    SessionEnded {
        /// The profile's name.
        profile: String,
    },
    /// The token itself is not usable.
    #[error(transparent)]
    Claims(#[from] ClaimsError),
    /// The credential store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The config could not be read or written.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The sign-in service's configuration could not be read or failed a check.
    #[error("cannot use the sign-in service: {0}")]
    Discovery(String),
    /// A call to the sign-in service failed.
    #[error("the sign-in service: {0}")]
    Http(String),
    /// The sign-in service refused the request.
    #[error("the sign-in service refused: {error}{}", if description.is_empty() { String::new() } else { format!(" ({description})") })]
    OAuth {
        /// The OAuth error code.
        error: String,
        /// Its description.
        description: String,
    },
    /// The person declined.
    #[error("the sign-in was declined")]
    Denied,
    /// The sign-in was not finished in time.
    #[error("the sign-in was not finished in time; start it again")]
    TimedOut,
    /// The browser came back with a `state` this sign-in did not send.
    #[error("the browser came back from a different sign-in (state mismatch); start it again")]
    StateMismatch,
    /// An answer named another issuer.
    #[error(
        "an answer came from another sign-in service than the one configured (issuer mismatch)"
    )]
    IssuerMismatch,
    /// The loopback redirect failed.
    #[error("the browser sign-in failed: {0}")]
    Callback(String),
    /// The provider has no device authorization endpoint.
    #[error("this sign-in service does not offer device codes; use the browser sign-in")]
    NoDeviceGrant,
    /// The system could not produce random bytes.
    #[error("the system has no randomness available: {0}")]
    Random(String),
}

impl From<RandomError> for LoginError {
    fn from(e: RandomError) -> Self {
        Self::Random(e.to_string())
    }
}
