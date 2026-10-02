use crate::claims::ClaimsError;
use crate::config::ConfigError;
use crate::store::StoreError;

/// Why a credential could not be produced or kept.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// The profile has no secret in its store.
    #[error(
        "profile {profile} has no credential on this machine: sign in again with `iohr login --profile {profile}`"
    )]
    NotSignedIn {
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
}
