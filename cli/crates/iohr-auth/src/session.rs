//! A person's session: a short access token refreshed with a rotating refresh token.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::{Mutex, OnceCell};
use zeroize::Zeroizing;

use crate::credential::{Bearer, Credential};
use crate::error::AuthError;
use crate::flow::{Authorization, Granted};
use crate::oidc::{Provider, TokenError};
use crate::secret::Redacted;
use crate::store::{EntryKey, Store, StoreError};

/// Refresh when less than this is left of the access token.
const REFRESH_MARGIN: time::Duration = time::Duration::seconds(60);

/// What the credential store keeps for a person: both tokens and the access token's
/// expiry, as one JSON value, so a new process can call without refreshing.
#[derive(Deserialize)]
struct Stored {
    refresh_token: Redacted<String>,
    access_token: Redacted<String>,
    expires_at: i64,
}

#[derive(Serialize)]
struct StoredRef<'a> {
    refresh_token: &'a str,
    access_token: &'a str,
    expires_at: i64,
}

#[derive(Clone)]
struct Current {
    access: Redacted<String>,
    refresh: Redacted<String>,
    expires_at: OffsetDateTime,
}

impl Current {
    fn encode(&self) -> Result<Redacted<String>, AuthError> {
        let mut buf = Zeroizing::new(Vec::new());
        serde_json::to_writer(
            &mut *buf,
            &StoredRef {
                refresh_token: self.refresh.expose(),
                access_token: self.access.expose(),
                expires_at: self.expires_at.unix_timestamp(),
            },
        )
        .map_err(|e| AuthError::Http(e.to_string()))?;
        let text = String::from_utf8(std::mem::take(&mut *buf))
            .map_err(|e| AuthError::Http(e.to_string()))?;
        Ok(Redacted::new(text))
    }

    fn decode(secret: &Redacted<String>) -> Option<Self> {
        let s: Stored = serde_json::from_str(secret.expose()).ok()?;
        Some(Self {
            access: s.access_token,
            refresh: s.refresh_token,
            expires_at: OffsetDateTime::from_unix_timestamp(s.expires_at).ok()?,
        })
    }

    fn fresh(&self) -> bool {
        self.expires_at - OffsetDateTime::now_utc() > REFRESH_MARGIN
    }
}

/// A signed-in person. Implements [`Credential`]: hands out the access token while it
/// has more than a minute left, and otherwise refreshes it, writing the rotated refresh
/// token back to the store before using the new access token.
///
/// Two `iohr` processes may refresh the same profile at once. Before refreshing, the
/// session re-reads the store and takes a newer token another process wrote; when a
/// refresh is refused, it re-reads once more and retries with a token rotated in the
/// meantime. Only a refusal with nothing newer in the store ends the session.
pub struct Session {
    profile: String,
    issuer: String,
    client_id: String,
    provider: OnceCell<Provider>,
    store: Arc<dyn Store>,
    key: EntryKey,
    current: Mutex<Current>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("profile", &self.profile)
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Stores a finished sign-in under `key` and returns its session. Only a granted
    /// sign-in can become a session.
    ///
    /// # Errors
    ///
    /// [`AuthError::Store`] when the secret cannot be kept.
    pub async fn create(
        granted: Authorization<Granted>,
        store: Arc<dyn Store>,
        key: EntryKey,
    ) -> Result<Self, AuthError> {
        let (provider, tokens) = granted.into_parts();
        let refresh = tokens
            .refresh
            .ok_or_else(|| AuthError::Http("the sign-in returned no refresh token".into()))?;
        let current = Current {
            access: tokens.access,
            refresh,
            expires_at: tokens.expires_at,
        };
        let session = Self {
            profile: key.profile().to_string(),
            issuer: provider.issuer().to_owned(),
            client_id: provider.client_id().to_owned(),
            provider: OnceCell::new_with(Some(provider)),
            store,
            key,
            current: Mutex::new(current.clone()),
        };
        session.save(&current).await?;
        Ok(session)
    }

    /// The session kept in `store` under `key`, for the sign-in service `issuer` and
    /// client `client_id` recorded in its profile.
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
        let profile = key.profile().to_string();
        let current = read(&store, &key)
            .await?
            .and_then(|s| Current::decode(&s))
            .ok_or_else(|| AuthError::NotSignedIn {
                profile: profile.clone(),
            })?;
        Ok(Self {
            profile,
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
            provider: OnceCell::new(),
            store,
            key,
            current: Mutex::new(current),
        })
    }

    /// Revokes the refresh token at the sign-in service, which ends the session
    /// everywhere. The caller removes the store entry afterwards.
    ///
    /// # Errors
    ///
    /// [`AuthError::Http`] or [`AuthError::Discovery`] when the service cannot be
    /// reached.
    pub async fn revoke(&self) -> Result<(), AuthError> {
        let refresh = self.current.lock().await.refresh.clone();
        self.provider().await?.revoke(&refresh).await
    }

    async fn provider(&self) -> Result<&Provider, AuthError> {
        self.provider
            .get_or_try_init(|| Provider::discover(&self.issuer, &self.client_id))
            .await
    }

    async fn save(&self, current: &Current) -> Result<(), AuthError> {
        let secret = current.encode()?;
        let (store, key) = (Arc::clone(&self.store), self.key.clone());
        blocking(move || store.set(&key, &secret)).await
    }

    async fn refresh_with(&self, refresh: &Redacted<String>) -> Result<Current, TokenOutcome> {
        let provider = self.provider().await.map_err(TokenOutcome::Failed)?;
        let wire = provider
            .token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.expose()),
                ("client_id", provider.client_id()),
            ])
            .await
            .map_err(|e| match e {
                TokenError::OAuth { error, .. } if error == "invalid_grant" => {
                    TokenOutcome::Refused
                }
                other => TokenOutcome::Failed(other.into()),
            })?;
        let tokens = provider
            .tokens(wire, false, false)
            .map_err(TokenOutcome::Failed)?;
        Ok(Current {
            access: tokens.access,
            refresh: tokens.refresh.unwrap_or_else(|| refresh.clone()),
            expires_at: tokens.expires_at,
        })
    }
}

enum TokenOutcome {
    Refused,
    Failed(AuthError),
}

impl Credential for Session {
    async fn bearer(&self) -> Result<Bearer, AuthError> {
        let mut current = self.current.lock().await;
        if current.fresh() {
            return Ok(Bearer::new(current.access.clone()));
        }
        // Another process may have refreshed already.
        if let Some(stored) = read(&self.store, &self.key)
            .await?
            .and_then(|s| Current::decode(&s))
        {
            *current = stored;
            if current.fresh() {
                return Ok(Bearer::new(current.access.clone()));
            }
        }
        let used = current.refresh.clone();
        let next = match self.refresh_with(&used).await {
            Ok(next) => next,
            Err(TokenOutcome::Failed(e)) => return Err(e),
            Err(TokenOutcome::Refused) => {
                // Rotated by another process between our read and our refresh?
                let newer = read(&self.store, &self.key)
                    .await?
                    .and_then(|s| Current::decode(&s))
                    .filter(|s| s.refresh != used);
                let Some(newer) = newer else {
                    return Err(AuthError::SessionEnded {
                        profile: self.profile.clone(),
                    });
                };
                if newer.fresh() {
                    newer
                } else {
                    self.refresh_with(&newer.refresh)
                        .await
                        .map_err(|e| match e {
                            TokenOutcome::Refused => AuthError::SessionEnded {
                                profile: self.profile.clone(),
                            },
                            TokenOutcome::Failed(e) => e,
                        })?
                }
            }
        };
        self.save(&next).await?;
        *current = next;
        Ok(Bearer::new(current.access.clone()))
    }
}

async fn read(
    store: &Arc<dyn Store>,
    key: &EntryKey,
) -> Result<Option<Redacted<String>>, AuthError> {
    let (store, key) = (Arc::clone(store), key.clone());
    blocking(move || store.get(&key)).await
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| {
            AuthError::Store(StoreError::Failed(format!(
                "the credential store call stopped: {e}"
            )))
        })?
        .map_err(AuthError::from)
}
