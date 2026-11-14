//! A person's session for the command line: the SDK's [`inorbithr::login::Session`]
//! (refresh, rotation, sign-out) used as a [`Credential`].

use std::sync::Arc;

use inorbithr::login::{self, Authorization, EntryKey, Granted, SignOut, Store};

use crate::credential::{Bearer, Credential};
use crate::error::AuthError;

/// A signed-in person. Implements [`Credential`]; the work is the SDK's.
#[derive(Debug)]
pub struct Session(login::Session);

impl Session {
    /// Stores a finished sign-in under `key` and returns its session.
    ///
    /// # Errors
    ///
    /// [`AuthError::Store`] when the secret cannot be kept.
    pub async fn create(
        granted: Authorization<Granted>,
        store: Arc<dyn Store>,
        key: EntryKey,
    ) -> Result<Self, AuthError> {
        Ok(Self(login::Session::save(granted, store, key).await?))
    }

    /// The session kept in `store` under `key`.
    ///
    /// # Errors
    ///
    /// [`AuthError::NotSignedIn`] when there is no readable session there.
    pub async fn load(
        store: Arc<dyn Store>,
        key: EntryKey,
        issuer: &str,
        client_id: &str,
    ) -> Result<Self, AuthError> {
        Ok(Self(
            login::Session::load(store, key, issuer, client_id).await?,
        ))
    }

    /// Revokes the session at the sign-in service when it can, then deletes the store
    /// entry. A failed revoke still deletes locally; the answer says so.
    ///
    /// # Errors
    ///
    /// [`AuthError::Store`] when the entry cannot be deleted.
    pub async fn sign_out(self) -> Result<SignOut, AuthError> {
        Ok(self.0.sign_out().await?)
    }
}

impl Credential for Session {
    async fn bearer(&self) -> Result<Bearer, AuthError> {
        Ok(Bearer::new(self.0.access_token().await?))
    }
}
