//! The `login` source of the credential chain (RFC 0231, `docs/config.md` section 5.6):
//! a stored person's sign-in as a [`TokenProvider`].

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::OnceCell;

use super::{
    DEFAULT_CLIENT_ID, DEFAULT_ISSUER, EntryKey, LoginError, ProfileName, Session, Storage, Store,
    StoreError, store_for,
};
use crate::auth::{CachedToken, Token, TokenProvider};
use crate::config::LoginTarget;
use crate::error::AuthError;

/// A stored sign-in, as a credential: the access token of the person `iohr login` (or
/// [`Session::save`]) left in the store.
///
/// It owns no refresh logic. [`Session`] reads the store, refreshes when under a minute
/// is left, writes the rotated refresh token back before using the new access token, and
/// re-reads the store when another process rotated it first; this wraps that in the
/// SDK's usual token cache ([`CachedToken`]: one refresh at a time, a still-valid token
/// kept when a refresh fails). [`invalidate`](TokenProvider::invalidate), called after a
/// `401`, makes the next call take a newer token from the store or refresh at once.
///
/// `Debug` shows nothing of the session.
pub struct StoredSession {
    inner: CachedToken<Source>,
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredSession").finish_non_exhaustive()
    }
}

impl StoredSession {
    /// The credential for a signed-in `session`.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self::from_source(Source {
            session: OnceCell::new_with(Some(session)),
            open: None,
            stale: AtomicBool::new(false),
        })
    }

    /// The credential for the session kept in `store` under `key`, read on the first
    /// call, for the sign-in service `issuer` and client `client_id` of its profile. A
    /// missing entry is an error then, not now.
    #[must_use]
    pub fn open(store: Arc<dyn Store>, key: EntryKey, issuer: &str, client_id: &str) -> Self {
        Self::from_source(Source {
            session: OnceCell::new(),
            open: Some(Open {
                store,
                key,
                issuer: issuer.to_owned(),
                client_id: client_id.to_owned(),
            }),
            stale: AtomicBool::new(false),
        })
    }

    pub(crate) fn attach(
        &self,
        logger: &crate::middleware::log::Logger,
        exchanged: Option<crate::auth::Exchanged>,
    ) {
        self.inner.attach(logger, exchanged);
    }

    fn from_source(source: Source) -> Self {
        Self {
            inner: CachedToken::new(source),
        }
    }

    /// The credential the chain chose for `target`: its store, opened without a prompt.
    pub(crate) fn from_target(
        target: &LoginTarget,
        store: Option<Arc<dyn Store>>,
    ) -> Result<Self, String> {
        let (store, key) = open_store(target, store)?;
        Ok(Self::open(
            store,
            key,
            target.issuer.as_deref().unwrap_or(DEFAULT_ISSUER),
            target.client_id.as_deref().unwrap_or(DEFAULT_CLIENT_ID),
        ))
    }
}

impl TokenProvider for StoredSession {
    async fn token(&self) -> Result<Token, AuthError> {
        self.inner.token().await
    }

    async fn invalidate(&self) {
        self.inner.invalidate().await;
    }
}

struct Open {
    store: Arc<dyn Store>,
    key: EntryKey,
    issuer: String,
    client_id: String,
}

/// What [`CachedToken`] asks for tokens.
struct Source {
    session: OnceCell<Session>,
    open: Option<Open>,
    /// The token last handed out was refused.
    stale: AtomicBool,
}

impl TokenProvider for Source {
    async fn token(&self) -> Result<Token, AuthError> {
        let session = self
            .session
            .get_or_try_init(|| async {
                let Some(o) = &self.open else {
                    return Err(LoginError::Http("the session was not opened".into()));
                };
                Session::load(Arc::clone(&o.store), o.key.clone(), &o.issuer, &o.client_id).await
            })
            .await
            .map_err(|e| provider(&e))?;
        let rejected = self.stale.swap(false, Ordering::AcqRel);
        match session.obtain(rejected).await {
            Ok((access, expires_at)) => {
                let left = (expires_at - time::OffsetDateTime::now_utc()).max(time::Duration::ZERO);
                let left = Duration::try_from(left).unwrap_or_default();
                Ok(Token::new(access, Some(left)))
            }
            Err(e) => {
                // The refused token is still the one held: ask again next time.
                self.stale.store(rejected, Ordering::Release);
                Err(provider(&e))
            }
        }
    }

    async fn invalidate(&self) {
        self.stale.store(true, Ordering::Release);
    }
}

fn provider(e: &LoginError) -> AuthError {
    AuthError::Provider(e.to_string())
}

/// The store of `target` (`injected` when the caller gave one) and its entry's key.
fn open_store(
    target: &LoginTarget,
    injected: Option<Arc<dyn Store>>,
) -> Result<(Arc<dyn Store>, EntryKey), String> {
    let name: ProfileName = target
        .profile
        .parse()
        .map_err(|_| format!("{:?} is not a profile name", target.profile))?;
    let key = EntryKey::new(name, &target.account)
        .map_err(|e| format!("profile {} has no usable account: {e}", target.profile))?;
    let storage = if target.file_store {
        Storage::File
    } else {
        Storage::Keyring
    };
    let store = match injected {
        Some(store) => store,
        None => store_for(storage, std::path::Path::new(&target.dir), true)
            .map_err(|e| format!("the credential store cannot be opened: {e}"))?,
    };
    Ok((store, key))
}

/// Whether the store holds an entry for `target`, without a prompt: `Err` carries the
/// reason the `login` source is skipped.
pub(crate) fn probe(target: &LoginTarget, store: Option<Arc<dyn Store>>) -> Result<(), String> {
    let (store, key) = open_store(target, store)?;
    match store.get(&key) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!("no stored session for profile {}", target.profile)),
        Err(StoreError::WouldAsk) => Err(format!(
            "the credential store would ask for a password to read profile {}",
            target.profile
        )),
        Err(e) => Err(format!("the credential store cannot be read: {e}")),
    }
}
