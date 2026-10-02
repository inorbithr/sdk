use std::future::Future;

use crate::error::AuthError;
use crate::secret::Redacted;

/// The value of an `Authorization: Bearer` header for one request.
#[derive(Debug, Clone)]
pub struct Bearer(Redacted<String>);

impl Bearer {
    /// Wraps an access token.
    #[must_use]
    pub fn new(token: Redacted<String>) -> Self {
        Self(token)
    }

    /// The token, for the one place it is written into a header.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

/// Something that can authorise a request to the API.
///
/// Every command is written once against this trait. Implementations: [`StaticToken`]
/// for an API token, and a person's refreshing session (with `iohr login` in a browser
/// or with a device code). Tests use a [`StaticToken`] with a fixed value.
pub trait Credential: Send + Sync {
    /// A bearer token that is valid for the next request.
    ///
    /// # Errors
    ///
    /// [`AuthError`] when no valid token can be produced, for example when a session
    /// can no longer be refreshed.
    fn bearer(&self) -> impl Future<Output = Result<Bearer, AuthError>> + Send;
}

/// An API token used as it is: from `IOHR_TOKEN`, or kept for a profile.
#[derive(Debug, Clone)]
pub struct StaticToken {
    token: Redacted<String>,
}

impl StaticToken {
    /// Uses `token` for every request.
    #[must_use]
    pub fn new(token: Redacted<String>) -> Self {
        Self { token }
    }
}

impl Credential for StaticToken {
    fn bearer(&self) -> impl Future<Output = Result<Bearer, AuthError>> + Send {
        std::future::ready(Ok(Bearer::new(self.token.clone())))
    }
}
