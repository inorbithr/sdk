//! Tokens and where they come from (`docs/design.md` section 3, `docs/config.md`
//! section 5): the [`TokenProvider`] extension point and the built-in sources, each
//! with the caching rules of section 5.3.

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use url::Url;

use crate::error::{AuthError, Headers, MAX_BODY};
use crate::middleware::{LogLevel, LogRecord, Logger};
use crate::retry::{
    BACKOFF_BASE, BACKOFF_CAP, RETRY_AFTER_CAP, backoff, retry_after, retryable_status,
};
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
///
/// Wrap a provider that fetches from a vault or a secrets manager in [`CachedToken`] to
/// give it the caching rules of the built-in sources.
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
/// After a failed refresh while the old token is still valid, the next try waits this.
const SOFT_EXPIRY_PAUSE: Duration = Duration::from_secs(5);
/// How often a token file's modification time and size are checked.
const FILE_CHECK: Duration = Duration::from_secs(60);
/// The `iohr` process's limit.
const CLI_TIMEOUT: Duration = Duration::from_secs(10);

/// A token in the cache: when it was fetched and how long it lives.
#[derive(Clone)]
struct Cached {
    access: Secret<String>,
    issued: Instant,
    lifetime: Option<Duration>,
}

impl Cached {
    fn left(&self, now: Instant) -> Option<Duration> {
        self.lifetime.map(|l| {
            l.checked_sub(now.saturating_duration_since(self.issued))
                .unwrap_or_default()
        })
    }

    /// More than a fifth of its lifetime left (or no expiry).
    fn fresh(&self, now: Instant) -> bool {
        match (self.left(now), self.lifetime) {
            (Some(left), Some(l)) => left.as_secs_f64() > l.as_secs_f64() * REFRESH_AT,
            _ => true,
        }
    }

    fn valid(&self, now: Instant) -> bool {
        self.left(now).is_none_or(|l| !l.is_zero())
    }

    fn token(&self, now: Instant) -> Token {
        Token::new(self.access.clone(), self.left(now))
    }

    fn from_token(t: &Token) -> Self {
        let now = Instant::now();
        Self {
            access: t.access.clone(),
            issued: now,
            lifetime: t.expires_at.map(|e| e.saturating_duration_since(now)),
        }
    }
}

#[derive(Default)]
struct CacheState {
    token: Option<Cached>,
    /// A refresh failed with the old token still valid: no new try before this.
    pause_until: Option<Instant>,
}

/// The caching rules of `docs/config.md` section 5.3: refresh ahead at 20 % of the
/// lifetime left, one refresh at a time (a caller that gives up does not cancel it for
/// the others), and a still-valid token kept when a refresh fails.
#[derive(Default)]
struct Cache {
    state: Mutex<CacheState>,
    gate: tokio::sync::Mutex<()>,
    logger: OnceLock<Logger>,
}

impl Cache {
    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fresh(&self) -> Option<Token> {
        let now = Instant::now();
        let s = self.lock();
        let c = s.token.as_ref()?;
        if c.fresh(now) || (s.pause_until.is_some_and(|p| p > now) && c.valid(now)) {
            Some(c.token(now))
        } else {
            None
        }
    }

    fn store(&self, c: Cached) {
        let mut s = self.lock();
        s.token = Some(c);
        s.pause_until = None;
    }

    fn invalidate(&self) {
        let mut s = self.lock();
        s.token = None;
        s.pause_until = None;
    }

    /// A valid token: the cached one, or one `fetch` produces.
    async fn get<F>(self: &Arc<Self>, fetch: F) -> Result<Token, AuthError>
    where
        F: Future<Output = Result<Cached, AuthError>> + Send + 'static,
    {
        if let Some(t) = self.fresh() {
            return Ok(t);
        }
        let _gate = self.gate.lock().await;
        if let Some(t) = self.fresh() {
            return Ok(t);
        }
        let me = Arc::clone(self);
        // The refresh runs on its own task and stores its result itself, so a caller
        // that gives up does not cancel it for the others.
        let task = tokio::spawn(async move {
            let r = fetch.await;
            if let Ok(c) = &r {
                me.store(c.clone());
            }
            r
        });
        let result = task.await.unwrap_or_else(|e| {
            Err(AuthError::Provider(format!(
                "the token refresh stopped: {e}"
            )))
        });
        let now = Instant::now();
        match result {
            Ok(c) => Ok(c.token(now)),
            Err(e) => {
                let mut s = self.lock();
                match s.token.as_ref().filter(|c| c.valid(now)) {
                    Some(c) => {
                        let t = c.token(now);
                        s.pause_until = Some(now + SOFT_EXPIRY_PAUSE);
                        drop(s);
                        if let Some(log) = self.logger.get() {
                            log.emit(
                                LogRecord::new(LogLevel::Warn, "token_refresh_failed")
                                    .with("reason", e.to_string())
                                    .with("delay_ms", 5000),
                            );
                        }
                        Ok(t)
                    }
                    None => Err(e),
                }
            }
        }
    }
}

/// Where a key's secret is: in memory, or in a file read before every exchange.
#[derive(Clone)]
enum SecretSource {
    Value(Secret<String>),
    File(PathBuf),
}

/// What one exchange needs, shared with the task that runs it.
struct Exchange {
    http: reqwest::Client,
    token_url: Url,
    key_id: String,
    secret: SecretSource,
    scopes: Vec<String>,
    user_agent: String,
}

/// An API key exchanged for a 15-minute token with the client credentials grant.
///
/// The token is cached and refreshed when less than 20 % of its lifetime remains, or
/// once after a 401. One exchange runs at a time per provider: concurrent callers wait
/// for it (single flight). When a refresh fails and the cached token is still valid,
/// it is used and the next try waits 5 s. A secret file is read before every exchange,
/// so a rotated secret is used at the next refresh. The token endpoint is rate limited,
/// so a provider is built once and shared.
pub struct ClientCredentials {
    exchange: Arc<Exchange>,
    cache: Arc<Cache>,
}

impl fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("ClientCredentials");
        d.field("token_url", &self.exchange.token_url.as_str())
            .field("key_id", &self.exchange.key_id);
        match &self.exchange.secret {
            SecretSource::Value(s) => d.field("secret", s),
            SecretSource::File(p) => d.field("secret_file", p),
        };
        d.field("scopes", &self.exchange.scopes)
            .finish_non_exhaustive()
    }
}

fn token_http(https_only: bool) -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(https_only)
        .build()
        .map_err(|e| AuthError::Transport(e.to_string()))
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
        Self::with(key_id.into(), SecretSource::Value(secret), scopes)
    }

    /// A provider whose secret is in the file at `path`, read before every exchange so
    /// a rotated secret (a Kubernetes Secret, a Vault agent file) is used at the next
    /// refresh. Surrounding whitespace in the file is ignored.
    ///
    /// # Errors
    ///
    /// [`AuthError::Transport`] when the HTTP stack cannot start.
    pub fn from_secret_file(
        key_id: impl Into<String>,
        path: impl Into<PathBuf>,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AuthError> {
        Self::with(key_id.into(), SecretSource::File(path.into()), scopes)
    }

    fn with(
        key_id: String,
        secret: SecretSource,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AuthError> {
        Ok(Self {
            exchange: Arc::new(Exchange {
                http: token_http(true)?,
                token_url: Url::parse(DEFAULT_TOKEN_URL)
                    .map_err(|e| AuthError::Transport(e.to_string()))?,
                key_id,
                secret,
                scopes: scopes.into_iter().map(Into::into).collect(),
                user_agent: crate::client::user_agent(None),
            }),
            cache: Arc::new(Cache::default()),
        })
    }

    fn edit(mut self, f: impl FnOnce(&mut Exchange)) -> Self {
        let e = &self.exchange;
        let mut copy = Exchange {
            http: e.http.clone(),
            token_url: e.token_url.clone(),
            key_id: e.key_id.clone(),
            secret: e.secret.clone(),
            scopes: e.scopes.clone(),
            user_agent: e.user_agent.clone(),
        };
        f(&mut copy);
        self.exchange = Arc::new(copy);
        self
    }

    /// Exchange at `url` instead of the default (loopback may be plain HTTP).
    #[must_use]
    pub fn token_url(self, url: Url) -> Self {
        let plain = url.scheme() == "http";
        self.edit(|e| {
            if plain {
                // A plain-HTTP token endpoint is accepted on loopback only (SR-07); the
                // client builder checked that before handing the URL over.
                if let Ok(h) = token_http(false) {
                    e.http = h;
                }
            }
            e.token_url = url;
        })
    }

    /// Exchanges through `http` (the client's own, with its proxy and trust) and sends
    /// `user_agent`.
    pub(crate) fn transport(self, http: reqwest::Client, user_agent: String) -> Self {
        self.edit(|e| {
            e.http = http;
            e.user_agent = user_agent;
        })
    }

    pub(crate) fn attach(&self, logger: &Logger) {
        let _ = self.cache.logger.set(logger.clone());
    }

    /// The key id this provider exchanges.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.exchange.key_id
    }
}

impl Exchange {
    fn secret(&self) -> Result<Secret<String>, AuthError> {
        match &self.secret {
            SecretSource::Value(s) => Ok(s.clone()),
            SecretSource::File(p) => read_secret(p),
        }
    }

    async fn run(self: Arc<Self>) -> Result<Cached, AuthError> {
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
        let secret = self.secret()?;
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
                .basic_auth(&self.key_id, Some(secret.expose()))
                .header(reqwest::header::USER_AGENT, &self.user_agent)
                .timeout(TOKEN_TIMEOUT)
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
                    retry_after(&headers).map(|d| d.min(RETRY_AFTER_CAP))
                }),
            };
            if let Some(after) = retry
                && attempt < TOKEN_RETRIES
            {
                tokio::time::sleep(
                    after.unwrap_or_else(|| backoff(attempt, BACKOFF_BASE, BACKOFF_CAP)),
                )
                .await;
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
            return Ok(Cached {
                access: answer.access_token,
                issued: Instant::now(),
                lifetime: Some(Duration::from_secs(answer.expires_in.unwrap_or(900))),
            });
        }
    }
}

impl TokenProvider for ClientCredentials {
    async fn token(&self) -> Result<Token, AuthError> {
        let exchange = Arc::clone(&self.exchange);
        self.cache.get(exchange.run()).await
    }

    async fn invalidate(&self) {
        self.cache.invalidate();
    }
}

/// A secret read from a file, surrounding whitespace removed.
fn read_secret(path: &Path) -> Result<Secret<String>, AuthError> {
    let text = std::fs::read_to_string(path).map_err(|e| AuthError::File {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    Ok(Secret::new(text.trim().to_owned()))
}

/// The expiry a JWT's `exp` claim says (read, never verified): the time left.
fn jwt_lifetime(token: &str) -> Option<Duration> {
    use base64::Engine as _;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let exp = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?["exp"].as_u64()?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Duration::from_secs(exp.saturating_sub(now)))
}

#[derive(Default)]
struct FileState {
    token: Option<Cached>,
    stamp: Option<(Option<SystemTime>, u64)>,
    checked: Option<Instant>,
    stale: bool,
}

/// A bearer token in a file (an API token mounted from a secret): read at first use,
/// read again when the file's modification time or size changes (checked at most once a
/// minute) and right after the API refuses it. A JWT's `exp` claim is its expiry. A file
/// that goes away keeps the cached token until it is refused.
pub struct TokenFile {
    path: PathBuf,
    state: Mutex<FileState>,
}

impl fmt::Debug for TokenFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenFile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl TokenFile {
    /// A provider reading the token in `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(FileState::default()),
        }
    }

    fn stamp(&self) -> Option<(Option<SystemTime>, u64)> {
        let m = std::fs::metadata(&self.path).ok()?;
        Some((m.modified().ok(), m.len()))
    }

    fn read(&self, s: &mut FileState) -> Result<(), AuthError> {
        let stamp = self.stamp();
        let secret = read_secret(&self.path)?;
        let lifetime = jwt_lifetime(secret.expose());
        s.token = Some(Cached {
            access: secret,
            issued: Instant::now(),
            lifetime,
        });
        s.stamp = stamp;
        s.checked = Some(Instant::now());
        s.stale = false;
        Ok(())
    }
}

impl TokenProvider for TokenFile {
    async fn token(&self) -> Result<Token, AuthError> {
        let mut s = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let due = s
            .checked
            .is_none_or(|c| now.duration_since(c) >= FILE_CHECK);
        if s.token.is_none() || s.stale {
            self.read(&mut s)?;
        } else if due {
            s.checked = Some(now);
            let stamp = self.stamp();
            if stamp.is_some() && stamp != s.stamp {
                // A file that cannot be read now keeps the cached token.
                let _ = self.read(&mut s);
            }
        }
        let c = s.token.as_ref().ok_or_else(|| AuthError::File {
            path: self.path.display().to_string(),
            reason: "it holds no token".into(),
        })?;
        Ok(c.token(Instant::now()))
    }

    async fn invalidate(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stale = true;
    }
}

/// The developer's `iohr` login: a token from `iohr auth token --profile <name>
/// --format json` (`docs/config.md` section 5.4), run without a shell, with standard
/// input closed and a 10 s limit, again in each refresh window.
pub struct CliToken {
    program: PathBuf,
    profile: String,
    cache: Arc<Cache>,
}

impl fmt::Debug for CliToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CliToken")
            .field("program", &self.program)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl CliToken {
    /// Tokens for `profile` from the command line at `program` (`iohr` on `PATH` when
    /// it is just a name).
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, profile: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            profile: profile.into(),
            cache: Arc::new(Cache::default()),
        }
    }

    pub(crate) fn attach(&self, logger: &Logger) {
        let _ = self.cache.logger.set(logger.clone());
    }
}

async fn run_cli(program: PathBuf, profile: String) -> Result<Cached, AuthError> {
    #[derive(Deserialize)]
    struct Answer {
        access_token: Secret<String>,
        #[serde(default)]
        expires_at: Option<String>,
    }
    let cli = |message: String| AuthError::Cli { message };
    let child = tokio::process::Command::new(&program)
        .args(["auth", "token", "--profile", &profile, "--format", "json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| cli(format!("cannot run {}: {e}", program.display())))?;
    let out = tokio::time::timeout(CLI_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| cli(format!("{} did not answer within 10 s", program.display())))?
        .map_err(|e| cli(e.to_string()))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let first: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        return Err(cli(if first.is_empty() {
            format!("it exited with {}", out.status)
        } else {
            first
        }));
    }
    let answer: Answer = serde_json::from_slice(&out.stdout)
        .map_err(|_| cli("its answer is not the JSON `iohr auth token` prints".into()))?;
    let lifetime = match answer.expires_at {
        None => None,
        Some(at) => {
            let at =
                time::OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)
                    .map_err(|_| cli("its expires_at is not an RFC 3339 time".into()))?;
            Some(Duration::try_from(at - time::OffsetDateTime::now_utc()).unwrap_or_default())
        }
    };
    Ok(Cached {
        access: answer.access_token,
        issued: Instant::now(),
        lifetime,
    })
}

impl TokenProvider for CliToken {
    async fn token(&self) -> Result<Token, AuthError> {
        self.cache
            .get(run_cli(self.program.clone(), self.profile.clone()))
            .await
    }

    async fn invalidate(&self) {
        self.cache.invalidate();
    }
}

/// Any provider with the caching rules of the built-in sources: refresh at 20 % of the
/// lifetime left, one refresh at a time, and the still-valid token kept when a refresh
/// fails. For a provider that fetches from a vault or a secrets manager.
pub struct CachedToken<P> {
    inner: Arc<P>,
    cache: Arc<Cache>,
}

impl<P> fmt::Debug for CachedToken<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedToken").finish_non_exhaustive()
    }
}

impl<P: TokenProvider + 'static> CachedToken<P> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(inner: P) -> Self {
        Self {
            inner: Arc::new(inner),
            cache: Arc::new(Cache::default()),
        }
    }
}

impl<P: TokenProvider + 'static> TokenProvider for CachedToken<P> {
    async fn token(&self) -> Result<Token, AuthError> {
        let inner = Arc::clone(&self.inner);
        self.cache
            .get(async move { inner.token().await.map(|t| Cached::from_token(&t)) })
            .await
    }

    async fn invalidate(&self) {
        self.cache.invalidate();
        self.inner.invalidate().await;
    }
}

/// Providers tried in order: the first that produces a token is used, until it fails.
pub struct ChainedCredential {
    providers: Vec<Arc<dyn DynProvider>>,
    current: Mutex<usize>,
}

impl fmt::Debug for ChainedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainedCredential")
            .field("providers", &self.providers.len())
            .finish_non_exhaustive()
    }
}

impl Default for ChainedCredential {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainedCredential {
    /// An empty chain.
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            current: Mutex::new(0),
        }
    }

    /// Adds `provider` at the end.
    #[must_use]
    pub fn with(mut self, provider: impl TokenProvider + 'static) -> Self {
        self.providers.push(Arc::new(provider));
        self
    }

    fn current(&self) -> usize {
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl TokenProvider for ChainedCredential {
    async fn token(&self) -> Result<Token, AuthError> {
        let start = self.current();
        let mut failures = Vec::new();
        for (i, p) in self.providers.iter().enumerate().skip(start) {
            match p.token().await {
                Ok(t) => {
                    *self
                        .current
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = i;
                    return Ok(t);
                }
                Err(e) => failures.push(format!("{}: {e}", i + 1)),
            }
        }
        Err(AuthError::Provider(if failures.is_empty() {
            "the chain has no provider".into()
        } else {
            format!("every provider failed: {}", failures.join("; "))
        }))
    }

    async fn invalidate(&self) {
        if let Some(p) = self.providers.get(self.current()) {
            p.invalidate().await;
        }
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

    use super::{Cached, ClientCredentials, StaticToken, TokenFile, TokenProvider, jwt_lifetime};
    use crate::secret::Secret;

    #[test]
    fn a_token_is_fresh_until_a_fifth_of_its_life_is_left() {
        let now = Instant::now();
        let c = Cached {
            access: Secret::from("t"),
            issued: now,
            lifetime: Some(Duration::from_secs(100)),
        };
        assert!(c.fresh(now + Duration::from_secs(79)));
        assert!(!c.fresh(now + Duration::from_secs(81)));
        assert!(c.valid(now + Duration::from_secs(81)));
        assert!(!c.fresh(now + Duration::from_secs(500)));
        assert!(!c.valid(now + Duration::from_secs(500)));
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

    #[test]
    fn a_jwt_exp_is_read() {
        use base64::Engine as _;
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600;
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{{\"exp\":{exp}}}"));
        let left = jwt_lifetime(&format!("x.{payload}.y")).unwrap();
        assert!(left > Duration::from_secs(590) && left <= Duration::from_secs(600));
        assert_eq!(jwt_lifetime("opaque"), None);
    }

    #[tokio::test]
    async fn a_token_file_is_read_again_after_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t");
        std::fs::write(&path, "one\n").unwrap();
        let p = TokenFile::new(&path);
        assert_eq!(p.token().await.unwrap().expose(), "one");
        std::fs::write(&path, "two").unwrap();
        assert_eq!(p.token().await.unwrap().expose(), "one");
        p.invalidate().await;
        assert_eq!(p.token().await.unwrap().expose(), "two");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(p.token().await.unwrap().expose(), "two");
        p.invalidate().await;
        assert!(p.token().await.is_err());
    }
}
