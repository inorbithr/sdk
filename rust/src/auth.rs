//! Tokens and where they come from (`docs/design.md` section 3).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;
use url::Url;

use crate::error::{AuthError, Headers, MAX_BODY};
use crate::retry::{backoff, retry_after, retryable_status};
use crate::secret::Secret;

/// An access token and when it stops being valid.
#[derive(Debug, Clone)]
pub struct Token {
    access: Secret<String>,
    expires_at: Option<Instant>,
}

impl Token {
    /// A token that is valid for `lifetime`, or for ever when `None`.
    #[must_use]
    pub fn new(access: Secret<String>, lifetime: Option<Duration>) -> Self {
        Self {
            access,
            expires_at: lifetime.map(|l| Instant::now() + l),
        }
    }

    /// The bearer value, for the one place it is written into a header.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.access.expose()
    }

    /// When the token expires, if it does.
    #[must_use]
    pub fn expires_at(&self) -> Option<Instant> {
        self.expires_at
    }
}

/// Something that produces a valid token for the next request: the extension point
/// for apps that already hold one, such as the `iohr` command line's sessions.
///
/// Implement it with an `async fn`:
///
/// ```
/// use inorbithr::{AuthError, Secret, Token, TokenProvider};
///
/// struct FromVault;
///
/// impl TokenProvider for FromVault {
///     async fn token(&self) -> Result<Token, AuthError> {
///         let value = Secret::from("eyJ...");   // fetched from somewhere safe
///         Ok(Token::new(value, None))
///     }
/// }
/// ```
pub trait TokenProvider: Send + Sync {
    /// A token that is valid for the next request. Called once per attempt, so a
    /// provider caches and refreshes on its own.
    ///
    /// # Errors
    ///
    /// [`AuthError`] when no valid token can be produced.
    fn token(&self) -> impl Future<Output = Result<Token, AuthError>> + Send;

    /// The API refused the last token with a 401; the next [`token`](Self::token)
    /// call should produce a fresh one. The default does nothing.
    fn invalidate(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// The object-safe shape the client stores, so `Client` is not generic over the
/// provider. Every [`TokenProvider`] is one.
pub(crate) trait DynProvider: Send + Sync {
    fn token<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<Token, AuthError>> + Send + 'a>>;
    fn invalidate<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

impl<T: TokenProvider> DynProvider for T {
    fn token<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<Token, AuthError>> + Send + 'a>> {
        Box::pin(TokenProvider::token(self))
    }

    fn invalidate<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(TokenProvider::invalidate(self))
    }
}

/// An API token (RFC 0016) used as it is, for every request.
#[derive(Debug, Clone)]
pub struct StaticToken(Secret<String>);

impl StaticToken {
    /// Uses `token` for every request.
    #[must_use]
    pub fn new(token: Secret<String>) -> Self {
        Self(token)
    }
}

impl TokenProvider for StaticToken {
    fn token(&self) -> impl Future<Output = Result<Token, AuthError>> + Send {
        std::future::ready(Ok(Token::new(self.0.clone(), None)))
    }
}

/// The token endpoint every client uses unless told otherwise.
pub const DEFAULT_TOKEN_URL: &str = "https://auth.inorbit.hr/oauth2/token";
/// The audience every token is minted for.
pub const AUDIENCE: &str = "iohr-api";
/// The share of a token's lifetime left when it is refreshed early.
const REFRESH_AT: f64 = 0.2;
const TOKEN_RETRIES: u32 = 2;
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

struct Cached {
    access: Secret<String>,
    issued: Instant,
    lifetime: Duration,
}

impl Cached {
    fn fresh(&self, now: Instant) -> bool {
        let left = self
            .lifetime
            .checked_sub(now.saturating_duration_since(self.issued))
            .unwrap_or_default();
        left.as_secs_f64() > self.lifetime.as_secs_f64() * REFRESH_AT
    }
}

/// An API key exchanged for a 15-minute token with the client credentials grant.
///
/// The token is cached and refreshed when less than 20 % of its lifetime remains, or
/// once after a 401. One exchange runs at a time per provider: concurrent callers wait
/// for it (single flight). The token endpoint is rate limited, so a provider is built
/// once and shared.
pub struct ClientCredentials {
    http: reqwest::Client,
    token_url: Url,
    key_id: String,
    secret: Secret<String>,
    scopes: Vec<String>,
    cache: Mutex<Option<Cached>>,
}

impl fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientCredentials")
            .field("token_url", &self.token_url.as_str())
            .field("key_id", &self.key_id)
            .field("secret", &self.secret)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

impl ClientCredentials {
    /// A provider for `key_id` and `secret`, asking for `scopes` at
    /// [`DEFAULT_TOKEN_URL`].
    ///
    /// # Errors
    ///
    /// [`AuthError::Transport`] when the HTTP stack cannot start.
    pub fn new(
        key_id: impl Into<String>,
        secret: Secret<String>,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .user_agent(crate::client::user_agent(None))
            .connect_timeout(Duration::from_secs(10))
            .timeout(TOKEN_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .build()
            .map_err(|e| AuthError::Transport(e.to_string()))?;
        Ok(Self {
            http,
            token_url: Url::parse(DEFAULT_TOKEN_URL)
                .map_err(|e| AuthError::Transport(e.to_string()))?,
            key_id: key_id.into(),
            secret,
            scopes: scopes.into_iter().map(Into::into).collect(),
            cache: Mutex::new(None),
        })
    }

    /// Exchange at `url` instead of the default (loopback may be plain HTTP).
    #[must_use]
    pub fn token_url(mut self, url: Url) -> Self {
        if url.scheme() == "http" {
            // A plain-HTTP token endpoint is accepted on loopback only (SR-07); the
            // client builder checked that before handing the URL over.
            self.http = reqwest::Client::builder()
                .user_agent(crate::client::user_agent(None))
                .connect_timeout(Duration::from_secs(10))
                .timeout(TOKEN_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| self.http.clone());
        }
        self.token_url = url;
        self
    }

    /// The key id this provider exchanges.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    async fn exchange(&self) -> Result<Cached, AuthError> {
        #[derive(Deserialize)]
        struct Answer {
            access_token: Secret<String>,
            #[serde(default)]
            expires_in: Option<u64>,
        }
        #[derive(Deserialize, Default)]
        struct Refusal {
            #[serde(default)]
            error: String,
            #[serde(default)]
            error_description: String,
        }
        let form = [
            ("grant_type", "client_credentials"),
            ("audience", AUDIENCE),
            ("scope", &self.scopes.join(" ")),
        ];
        let mut attempt = 0;
        loop {
            let outcome = self
                .http
                .post(self.token_url.clone())
                .basic_auth(&self.key_id, Some(self.secret.expose()))
                .form(&form)
                .send()
                .await;
            let retry = match &outcome {
                Err(e) => (e.is_connect() || e.is_timeout()).then_some(None),
                Ok(r) => retryable_status(r.status().as_u16()).then(|| {
                    let headers = Headers::new(r.headers().iter().map(|(k, v)| {
                        (
                            k.as_str().to_owned(),
                            v.to_str().unwrap_or_default().to_owned(),
                        )
                    }));
                    retry_after(&headers)
                }),
            };
            if let Some(after) = retry
                && attempt < TOKEN_RETRIES
            {
                tokio::time::sleep(after.unwrap_or_else(|| backoff(attempt))).await;
                attempt += 1;
                continue;
            }
            let resp = outcome.map_err(|e| AuthError::Transport(transport_reason(&e)))?;
            let status = resp.status().as_u16();
            if resp.content_length().is_some_and(|n| n > MAX_BODY as u64) {
                return Err(AuthError::Transport("the answer is too large".into()));
            }
            let body = resp
                .bytes()
                .await
                .map_err(|e| AuthError::Transport(transport_reason(&e)))?;
            if !(200..300).contains(&status) {
                let refusal = serde_json::from_slice::<Refusal>(&body).unwrap_or_default();
                return Err(AuthError::Exchange {
                    key_id: self.key_id.clone(),
                    error: if refusal.error.is_empty() {
                        format!("HTTP {status}")
                    } else {
                        refusal.error
                    },
                    description: refusal.error_description,
                    status,
                });
            }
            let answer: Answer = serde_json::from_slice(&body).map_err(|e| {
                AuthError::Transport(format!("the token answer could not be read: {e}"))
            })?;
            let lifetime = Duration::from_secs(answer.expires_in.unwrap_or(900));
            return Ok(Cached {
                access: answer.access_token,
                issued: Instant::now(),
                lifetime,
            });
        }
    }
}

impl TokenProvider for ClientCredentials {
    async fn token(&self) -> Result<Token, AuthError> {
        // Holding the lock across the exchange is the single flight: a second caller
        // waits here and then finds the fresh token.
        let mut cache = self.cache.lock().await;
        let now = Instant::now();
        if let Some(c) = cache.as_ref()
            && c.fresh(now)
        {
            return Ok(Token::new(
                c.access.clone(),
                Some(
                    c.lifetime
                        .saturating_sub(now.saturating_duration_since(c.issued)),
                ),
            ));
        }
        let fresh = self.exchange().await?;
        let token = Token::new(fresh.access.clone(), Some(fresh.lifetime));
        *cache = Some(fresh);
        Ok(token)
    }

    async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }
}

/// The error's own message and its causes, without the URL (which may carry query
/// values, SR-13).
pub(crate) fn transport_reason(e: &reqwest::Error) -> String {
    let mut parts = Vec::new();
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = source {
        parts.push(s.to_string());
        source = s.source();
    }
    if parts.is_empty() {
        if e.is_timeout() {
            "timed out".to_owned()
        } else {
            "the connection failed".to_owned()
        }
    } else {
        parts.join(": ")
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Cached, ClientCredentials, StaticToken, TokenProvider};
    use crate::secret::Secret;

    #[test]
    fn a_token_is_fresh_until_a_fifth_of_its_life_is_left() {
        let now = Instant::now();
        let c = Cached {
            access: Secret::from("t"),
            issued: now,
            lifetime: Duration::from_secs(100),
        };
        assert!(c.fresh(now + Duration::from_secs(79)));
        assert!(!c.fresh(now + Duration::from_secs(81)));
        assert!(!c.fresh(now + Duration::from_secs(500)));
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let p = ClientCredentials::new("ak_1", Secret::from("s3cr3t"), ["identity:read"]).unwrap();
        let shown = format!("{p:?}");
        assert!(
            shown.contains("ak_1") && !shown.contains("s3cr3t"),
            "{shown}"
        );
        assert!(!format!("{:?}", StaticToken::new(Secret::from("tok"))).contains("tok"));
    }

    #[tokio::test]
    async fn a_static_token_never_expires() {
        let t = StaticToken::new(Secret::from("abc")).token().await.unwrap();
        assert_eq!(t.expose(), "abc");
        assert!(t.expires_at().is_none());
    }
}
