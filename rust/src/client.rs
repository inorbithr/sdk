//! The client: configuration, one call with retries, and the raw path
//! (`docs/design.md` sections 2, 4 and 6).

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::auth::{
    ClientCredentials, DEFAULT_TOKEN_URL, DynProvider, StaticToken, TokenProvider, transport_reason,
};
use crate::error::{ApiError, ConfigError, Error, Headers, MAX_BODY, RawResponse};
use crate::hooks::{Attempt, Hook};
use crate::profile::{Profile, Public, env_prefix};
use crate::retry::{backoff, request_id, retry_after, retryable_status};
use crate::secret::Secret;
use crate::socket::{Ctx as SocketCtx, Hub};
use crate::stream::{EventStream, Guard, QUEUE, Streams, opened, read_sse};

/// The API every client calls unless told otherwise.
pub const DEFAULT_BASE_URL: &str = "https://api.inorbit.hr";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_RETRIES: u32 = 2;
const DEFAULT_STREAM_IDLE: Duration = Duration::from_secs(45);
/// A stream has no deadline of its own; the platform ends one after 24 hours.
const STREAM_DEADLINE: Duration = Duration::from_hours(48);

/// An HTTP method, as the operations use them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
}

impl Method {
    /// Whether the method is idempotent by definition, so a call may be retried
    /// without an idempotency key (`docs/design.md` section 6).
    #[must_use]
    pub fn is_idempotent(self) -> bool {
        matches!(self, Self::Get | Self::Put | Self::Delete | Self::Head)
    }

    fn as_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
            Self::Head => reqwest::Method::HEAD,
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        })
    }
}

/// One call to make: the method, the path with its parameters already bound, the
/// query, and a JSON body. A generated surface builds one per operation; the raw path
/// builds one by hand.
///
/// ```
/// use inorbithr::{Method, Operation};
///
/// let op = Operation::new(Method::Get, "/v1/radar/digests")
///     .named("radar.list_digests")
///     .query("limit", "5");
/// assert_eq!(op.path(), "/v1/radar/digests");
/// ```
#[derive(Debug, Clone)]
pub struct Operation<'a> {
    name: &'static str,
    method: Method,
    path: Cow<'a, str>,
    query: Vec<(Cow<'static, str>, String)>,
    body: Option<Vec<u8>>,
    idempotent: bool,
    scopes: &'static [&'static str],
    rpc: Option<&'static str>,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl<'a> Operation<'a> {
    /// A call of `method` to `path` (`/v1/me`), without a body. The path is sent as
    /// given: bind and percent-encode parameters before building the operation.
    pub fn new(method: Method, path: impl Into<Cow<'a, str>>) -> Self {
        Self {
            name: "request",
            method,
            path: path.into(),
            query: Vec::new(),
            body: None,
            idempotent: method.is_idempotent(),
            scopes: &[],
            rpc: None,
            fields: serde_json::Map::new(),
        }
    }

    /// The operation's name as the hooks see it (`accounts.get_me`).
    #[must_use]
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Adds one query parameter; a key may repeat.
    #[must_use]
    pub fn query(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }

    /// Adds one query parameter per item, under the same key.
    #[must_use]
    pub fn query_each(
        mut self,
        key: impl Into<Cow<'static, str>>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let key = key.into();
        for v in values {
            self.query.push((key.clone(), v.into()));
        }
        self
    }

    /// Adds a query parameter when `value` is `Some`.
    #[must_use]
    pub fn query_opt(
        self,
        key: impl Into<Cow<'static, str>>,
        value: Option<impl Into<String>>,
    ) -> Self {
        match value {
            Some(v) => self.query(key, v),
            None => self,
        }
    }

    /// Sends `body` as JSON.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] when `body` cannot be serialised.
    pub fn json<T: Serialize + ?Sized>(mut self, body: &T) -> Result<Self, Error> {
        self.body = Some(serde_json::to_vec(body).map_err(|e| ConfigError::Body {
            reason: e.to_string(),
        })?);
        Ok(self)
    }

    /// Marks a `POST` or `PATCH` safe to retry (or a `PUT`/`DELETE` unsafe).
    #[must_use]
    pub fn idempotent(mut self, yes: bool) -> Self {
        self.idempotent = yes;
        self
    }

    /// The scopes the operation needs, for the hooks and error messages.
    #[must_use]
    pub fn scopes(mut self, scopes: &'static [&'static str]) -> Self {
        self.scopes = scopes;
        self
    }

    /// The RPC a stream's call names on the `/v1/ws` socket
    /// (`iohr.events.v1.EventsService/StreamEvents`, the operation's `x-iohr-rpc`).
    #[must_use]
    pub fn rpc(mut self, name: &'static str) -> Self {
        self.rpc = Some(name);
        self
    }

    /// One field of the request message a stream's call carries on the socket: a path
    /// or query parameter by its wire name, `a.b` for a nested one. A value that does not
    /// serialise is left out.
    #[must_use]
    pub fn field(mut self, name: &str, value: impl Serialize) -> Self {
        let Ok(value) = serde_json::to_value(value) else {
            return self;
        };
        let mut parts = name.split('.').peekable();
        let mut node = &mut self.fields;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                node.insert(part.to_owned(), value);
                break;
            }
            let entry = node
                .entry(part.to_owned())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if !entry.is_object() {
                *entry = serde_json::Value::Object(serde_json::Map::new());
            }
            let serde_json::Value::Object(next) = entry else {
                break;
            };
            node = next;
        }
        self
    }

    /// [`field`](Self::field) when `value` is `Some`.
    #[must_use]
    pub fn field_opt(self, name: &str, value: Option<impl Serialize>) -> Self {
        match value {
            Some(v) => self.field(name, v),
            None => self,
        }
    }

    /// The operation's name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The method.
    #[must_use]
    pub fn method(&self) -> Method {
        self.method
    }

    /// The path, as it will be sent.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// A typed answer next to the raw one.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Response<T> {
    /// The body, decoded.
    pub value: T,
    /// The answer as it came: status, headers, body and request ids.
    pub raw: RawResponse,
}

struct Inner {
    streams: Streams,
    idle: Duration,
    hub: Arc<Hub>,
    socket: Arc<SocketCtx>,
    http: reqwest::Client,
    base: Url,
    host: String,
    provider: Arc<dyn DynProvider>,
    max_retries: u32,
    timeout: Duration,
    hooks: Vec<Arc<dyn Hook>>,
}

/// The InOrbit API client for one profile, built once and shared.
///
/// `P` is the [`Profile`] the client calls as: [`Public`] by default, or a type a
/// generated surface defines, whose operations are implemented for that type only.
pub struct Client<P: Profile = Public> {
    inner: Arc<Inner>,
    _profile: PhantomData<fn() -> P>,
}

impl<P: Profile> Clone for Client<P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _profile: PhantomData,
        }
    }
}

impl<P: Profile> fmt::Debug for Client<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("profile", &P::NAME)
            .field("base_url", &self.inner.base.as_str())
            .finish_non_exhaustive()
    }
}

impl<P: Profile> Client<P> {
    /// Starts configuring a client.
    #[must_use]
    pub fn builder() -> ClientBuilder<P> {
        ClientBuilder::new()
    }

    /// A client from the environment: `INORBIT_<PROFILE>_TOKEN` (an API token), or
    /// `INORBIT_<PROFILE>_KEY_ID`, `INORBIT_<PROFILE>_KEY_SECRET` and
    /// `INORBIT_<PROFILE>_SCOPES` (an API key); `INORBIT_<PROFILE>_BASE_URL` and
    /// `INORBIT_<PROFILE>_TOKEN_URL` falling back to the bare `INORBIT_BASE_URL` and
    /// `INORBIT_TOKEN_URL`. The [`Public`] profile reads the bare names for everything.
    /// Credentials never fall back: a named profile's client calls its own account or
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] naming the missing variables, or an unusable URL.
    pub fn from_env() -> Result<Self, Error> {
        let prefix = env_prefix(P::NAME);
        let var = |name: &str| {
            std::env::var(format!("{prefix}{name}"))
                .ok()
                .filter(|v| !v.is_empty())
        };
        let shared = |name: &str| {
            var(name).or_else(|| {
                if P::NAME == Public::NAME {
                    None
                } else {
                    std::env::var(format!("INORBIT_{name}"))
                        .ok()
                        .filter(|v| !v.is_empty())
                }
            })
        };
        let mut b = Self::builder();
        if let Some(u) = shared("BASE_URL") {
            b = b.base_url(u);
        }
        if let Some(u) = shared("TOKEN_URL") {
            b = b.token_url(u);
        }
        if let Some(token) = var("TOKEN") {
            b = b.token(Secret::new(token));
        } else if let (Some(id), Some(secret)) = (var("KEY_ID"), var("KEY_SECRET")) {
            b = b.key(id, Secret::new(secret));
            match var("SCOPES") {
                Some(s) => b = b.scopes(s.split_whitespace()),
                None => {
                    return Err(ConfigError::Missing {
                        what: "scopes",
                        env: format!("{prefix}SCOPES (space-separated, such as \"identity:read account:read\")"),
                    }
                    .into());
                }
            }
        } else {
            return Err(ConfigError::Missing {
                what: "credentials",
                env: format!("{prefix}TOKEN, or {prefix}KEY_ID and {prefix}KEY_SECRET"),
            }
            .into());
        }
        b.build()
    }

    /// The API this client calls.
    #[must_use]
    pub fn base_url(&self) -> &str {
        self.inner.base.as_str()
    }

    /// One call, decoded as `T`.
    ///
    /// # Errors
    ///
    /// See [`send`](Self::send); also [`Error::Decode`] when the body is not `T`.
    pub async fn request<T: DeserializeOwned>(
        &self,
        op: Operation<'_>,
    ) -> Result<Response<T>, Error> {
        let raw = self.send(op).await?;
        let value = raw.json()?;
        Ok(Response { value, raw })
    }

    /// One call, as it came back. Idempotent calls are retried up to `max_retries`
    /// times after a connection failure, a timeout, or a `429`, `503` or `504`; a
    /// `401` is answered by one fresh token and one more attempt; anything else is
    /// returned as [`Error::Api`].
    ///
    /// # Errors
    ///
    /// [`Error::Api`] for an error answer, [`Error::Auth`] when no token could be
    /// produced, and the other variants when the call could not be made or read.
    pub async fn send(&self, op: Operation<'_>) -> Result<RawResponse, Error> {
        let inner = &self.inner;
        let url = inner.url(&op.path)?;
        let request_id = request_id();
        let mut retries = 0;
        let mut refreshed = false;
        let mut number = 1;
        loop {
            let attempt = Attempt {
                operation: op.name,
                method: op.method,
                path: op.path.to_string(),
                number,
                request_id: request_id.clone(),
            };
            let result = match inner.attempt(&op, &url, &attempt).await {
                Outcome::Done(result) => result,
                Outcome::Unauthorized(_) if !refreshed => {
                    inner.provider.invalidate().await;
                    refreshed = true;
                    number += 1;
                    continue;
                }
                Outcome::Unauthorized(raw) => Err(ApiError::parse(raw).into()),
                Outcome::Retry { wait, .. } if op.idempotent && retries < inner.max_retries => {
                    tokio::time::sleep(wait.unwrap_or_else(|| backoff(retries))).await;
                    retries += 1;
                    number += 1;
                    continue;
                }
                Outcome::Retry { result, .. } => {
                    result.and_then(|raw| Err(ApiError::parse(raw).into()))
                }
            };
            if let Err(e) = &result {
                inner.hooks.iter().for_each(|h| h.on_error(&attempt, e));
            }
            return result;
        }
    }
}

impl<P: Profile> Client<P> {
    /// Opens a stream (`docs/design.md` section 7): server-sent events, or a call on
    /// the client's `/v1/ws` socket when it was built with [`Streams::Socket`]. The
    /// opening follows the rules of any `GET`: one fresh token after a `401`, retries
    /// after a connection failure, `429`, `503` or `504`.
    ///
    /// # Errors
    ///
    /// The error the opening ended with, as [`send`](Self::send) has them; errors after
    /// the stream opened arrive as its items.
    pub async fn stream<T: DeserializeOwned>(
        &self,
        op: Operation<'_>,
    ) -> Result<EventStream<T>, Error> {
        match (self.inner.streams, op.rpc) {
            (Streams::Socket, Some(rpc)) => {
                let body = serde_json::Value::Object(op.fields.clone());
                self.inner.hub.open(&self.inner.socket, rpc, body).await
            }
            _ => self.open_sse(&op).await,
        }
    }

    async fn open_sse<T: DeserializeOwned>(
        &self,
        op: &Operation<'_>,
    ) -> Result<EventStream<T>, Error> {
        let inner = &self.inner;
        let url = inner.url(&op.path)?;
        let request_id = request_id();
        let mut retries = 0;
        let mut refreshed = false;
        let mut number = 1;
        loop {
            let attempt = Attempt {
                operation: op.name,
                method: op.method,
                path: op.path.to_string(),
                number,
                request_id: request_id.clone(),
            };
            let outcome = match inner.open_attempt(op, &url, &attempt).await {
                Ok(resp) => {
                    let (tx, rx) = tokio::sync::mpsc::channel(QUEUE);
                    let task = tokio::spawn(read_sse(
                        resp,
                        tx,
                        inner.idle,
                        inner.host.clone(),
                        request_id.clone(),
                    ));
                    return Ok(EventStream::new(rx, Guard::Task(task.abort_handle())));
                }
                Err(outcome) => outcome,
            };
            let result = match outcome {
                Outcome::Unauthorized(_) if !refreshed => {
                    inner.provider.invalidate().await;
                    refreshed = true;
                    number += 1;
                    continue;
                }
                Outcome::Retry { wait, .. } if retries < inner.max_retries => {
                    tokio::time::sleep(wait.unwrap_or_else(|| backoff(retries))).await;
                    retries += 1;
                    number += 1;
                    continue;
                }
                Outcome::Unauthorized(raw) => Err(ApiError::parse(raw).into()),
                Outcome::Retry { result, .. } => {
                    result.and_then(|raw| Err(ApiError::parse(raw).into()))
                }
                Outcome::Done(result) => result.and_then(|raw| Err(ApiError::parse(raw).into())),
            };
            if let Err(e) = &result {
                inner.hooks.iter().for_each(|h| h.on_error(&attempt, e));
            }
            return result;
        }
    }
}

/// What one attempt came to.
enum Outcome {
    /// The call is over: a final answer, or an error nothing can be done about.
    Done(Result<RawResponse, Error>),
    /// The API refused the token; one refresh may fix it.
    Unauthorized(RawResponse),
    /// Worth another attempt after `wait`; `result` is the answer when none is left.
    Retry {
        wait: Option<Duration>,
        result: Result<RawResponse, Error>,
    },
}

impl Inner {
    /// One attempt to open a server-sent events stream: the response when it opened,
    /// else what the attempt came to.
    async fn open_attempt(
        &self,
        op: &Operation<'_>,
        url: &Url,
        attempt: &Attempt,
    ) -> Result<reqwest::Response, Outcome> {
        let token = match self.provider.token().await {
            Ok(t) => t,
            Err(e) => return Err(Outcome::Done(Err(Error::Auth(e)))),
        };
        let req = self
            .http
            .get(url.clone())
            .query(&op.query)
            .bearer_auth(token.expose())
            .header(header::ACCEPT, "text/event-stream")
            .header("x-request-id", &attempt.request_id)
            .timeout(STREAM_DEADLINE);
        self.hooks.iter().for_each(|h| h.on_request(attempt));
        let resp = match tokio::time::timeout(self.timeout, req.send()).await {
            Err(_) => {
                return Err(Outcome::Retry {
                    wait: None,
                    result: Err(Error::Timeout {
                        host: self.host.clone(),
                        secs: self.timeout.as_secs(),
                    }),
                });
            }
            Ok(Err(e)) => {
                let error = Error::Connection {
                    host: self.host.clone(),
                    reason: transport_reason(&e),
                };
                return Err(if e.is_connect() || e.is_request() {
                    Outcome::Retry {
                        wait: None,
                        result: Err(error),
                    }
                } else {
                    Outcome::Done(Err(error))
                });
            }
            Ok(Ok(r)) => r,
        };
        if resp.status().is_success() {
            let raw = opened(&resp, &attempt.request_id, attempt.number);
            self.hooks.iter().for_each(|h| h.on_response(attempt, &raw));
            return Ok(resp);
        }
        let raw = match read(resp, &attempt.request_id, attempt.number, &self.host).await {
            Ok(r) => r,
            Err(e) => return Err(Outcome::Done(Err(e))),
        };
        self.hooks.iter().for_each(|h| h.on_response(attempt, &raw));
        Err(match raw.status {
            401 => Outcome::Unauthorized(raw),
            s if retryable_status(s) => Outcome::Retry {
                wait: retry_after(&raw.headers),
                result: Ok(raw),
            },
            _ => Outcome::Done(Ok(raw)),
        })
    }

    async fn attempt(&self, op: &Operation<'_>, url: &Url, attempt: &Attempt) -> Outcome {
        let token = match self.provider.token().await {
            Ok(t) => t,
            Err(e) => return Outcome::Done(Err(Error::Auth(e))),
        };
        let mut req = self
            .http
            .request(op.method.as_reqwest(), url.clone())
            .query(&op.query)
            .bearer_auth(token.expose())
            .header("x-request-id", &attempt.request_id);
        if let Some(b) = &op.body {
            req = req
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )
                .body(b.clone());
        }
        self.hooks.iter().for_each(|h| h.on_request(attempt));
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let error = if e.is_timeout() {
                    Error::Timeout {
                        host: self.host.clone(),
                        secs: self.timeout.as_secs(),
                    }
                } else {
                    Error::Connection {
                        host: self.host.clone(),
                        reason: transport_reason(&e),
                    }
                };
                return if e.is_connect() || e.is_timeout() || e.is_request() {
                    Outcome::Retry {
                        wait: None,
                        result: Err(error),
                    }
                } else {
                    Outcome::Done(Err(error))
                };
            }
        };
        let raw = match read(resp, &attempt.request_id, attempt.number, &self.host).await {
            Ok(r) => r,
            Err(e) => return Outcome::Done(Err(e)),
        };
        self.hooks.iter().for_each(|h| h.on_response(attempt, &raw));
        match raw.status {
            401 => Outcome::Unauthorized(raw),
            s if retryable_status(s) => Outcome::Retry {
                wait: retry_after(&raw.headers),
                result: Ok(raw),
            },
            200..=299 => Outcome::Done(Ok(raw)),
            _ => Outcome::Done(Err(ApiError::parse(raw).into())),
        }
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.contains('?')
            || path.contains('#')
        {
            return Err(ConfigError::InvalidUrl {
                what: "path",
                reason: format!("{path:?} is not a path on the API, such as /v1/me; put query values in the operation's query"),
            }
            .into());
        }
        let url = self.base.join(path).map_err(|e| ConfigError::InvalidUrl {
            what: "path",
            reason: e.to_string(),
        })?;
        if url.origin() != self.base.origin() {
            return Err(ConfigError::InvalidUrl {
                what: "path",
                reason: "the path must stay on the API host".into(),
            }
            .into());
        }
        Ok(url)
    }
}

async fn read(
    mut resp: reqwest::Response,
    request_id: &str,
    attempts: u32,
    host: &str,
) -> Result<RawResponse, Error> {
    let status = resp.status().as_u16();
    let headers = Headers::new(resp.headers().iter().map(|(k, v)| {
        (
            k.as_str().to_owned(),
            v.to_str().unwrap_or_default().to_owned(),
        )
    }));
    if resp.content_length().is_some_and(|n| n > MAX_BODY as u64) {
        return Err(Error::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Connection {
        host: host.to_owned(),
        reason: transport_reason(&e),
    })? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(Error::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let server_request_id = headers.get("x-request-id").map(str::to_owned);
    Ok(RawResponse {
        status,
        headers,
        body,
        request_id: request_id.to_owned(),
        server_request_id,
        attempts,
    })
}

/// `inorbithr-sdk-rust/<version> rust/<rustc> <os>/<arch>`, plus the caller's suffix.
pub(crate) fn user_agent(suffix: Option<&str>) -> String {
    let mut ua = format!(
        "inorbithr-sdk-rust/{} rust/{} {}/{}",
        env!("CARGO_PKG_VERSION"),
        env!("INORBITHR_RUSTC_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some(s) = suffix.map(str::trim).filter(|s| !s.is_empty()) {
        ua.push(' ');
        ua.push_str(s);
    }
    ua
}

/// Checks a base or token URL: HTTPS, or plain HTTP to this machine only (SR-07), an
/// origin only for a base URL.
fn check_url(what: &'static str, raw: &str, origin_only: bool) -> Result<Url, ConfigError> {
    let url = Url::parse(raw).map_err(|e| ConfigError::InvalidUrl {
        what,
        reason: e.to_string(),
    })?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => {
            return Err(ConfigError::InvalidUrl {
                what,
                reason: "it must use https (plain http only to this machine)".into(),
            });
        }
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(ConfigError::InvalidUrl {
            what,
            reason: "it must not carry credentials or a fragment".into(),
        });
    }
    if origin_only && (url.path() != "/" || url.query().is_some()) {
        return Err(ConfigError::InvalidUrl {
            what,
            reason: "it is an origin only, such as https://api.inorbit.hr".into(),
        });
    }
    Ok(url)
}

/// Configures a [`Client`]. Every option has the default `docs/design.md` names.
pub struct ClientBuilder<P: Profile = Public> {
    base_url: String,
    token_url: String,
    key: Option<(String, Secret<String>)>,
    token: Option<Secret<String>>,
    scopes: Vec<String>,
    provider: Option<Arc<dyn DynProvider>>,
    timeout: Duration,
    max_retries: u32,
    user_agent_suffix: Option<String>,
    hooks: Vec<Arc<dyn Hook>>,
    streams: Streams,
    stream_idle_timeout: Duration,
    _profile: PhantomData<fn() -> P>,
}

impl<P: Profile> fmt::Debug for ClientBuilder<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("profile", &P::NAME)
            .field("base_url", &self.base_url)
            .field("token_url", &self.token_url)
            .field("key_id", &self.key.as_ref().map(|(id, _)| id))
            .field("scopes", &self.scopes)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .field("streams", &self.streams)
            .finish_non_exhaustive()
    }
}

impl<P: Profile> Default for ClientBuilder<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Profile> ClientBuilder<P> {
    /// The defaults: the public API, 30 s per attempt, 2 retries, no credentials yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            token_url: DEFAULT_TOKEN_URL.to_owned(),
            key: None,
            token: None,
            scopes: Vec::new(),
            provider: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            user_agent_suffix: None,
            hooks: Vec::new(),
            streams: Streams::Sse,
            stream_idle_timeout: DEFAULT_STREAM_IDLE,
            _profile: PhantomData,
        }
    }

    /// The API to call, an origin such as `https://api.inorbit.hr`.
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Where an API key is exchanged for a token.
    #[must_use]
    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    /// An API key: its id (`ak_...`) and the secret shown once. Needs [`scopes`](Self::scopes).
    #[must_use]
    pub fn key(mut self, id: impl Into<String>, secret: impl Into<Secret<String>>) -> Self {
        self.key = Some((id.into(), secret.into()));
        self
    }

    /// The scopes to ask for with a key, a subset of the key's.
    #[must_use]
    pub fn scopes(mut self, scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// An API token (made in the console or with `iohr token create`), used as it is.
    #[must_use]
    pub fn token(mut self, token: impl Into<Secret<String>>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// A token provider of your own, in place of a key or a token.
    #[must_use]
    pub fn token_provider(mut self, provider: impl TokenProvider + 'static) -> Self {
        self.provider = Some(Arc::new(provider));
        self
    }

    /// The timeout of one attempt (30 s by default).
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// How many times an idempotent call is retried (2 by default; 0 disables).
    #[must_use]
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Appended to the SDK's user agent, for example your app's name and version.
    #[must_use]
    pub fn user_agent_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.user_agent_suffix = Some(suffix.into());
        self
    }

    /// How streams open: server-sent events (the default), or one `/v1/ws` socket for
    /// every stream of the client (`docs/design.md` section 7).
    #[must_use]
    pub fn streams(mut self, streams: Streams) -> Self {
        self.streams = streams;
        self
    }

    /// How long a stream may be silent, not even a keep-alive, before it fails (or, on
    /// the socket, reconnects); 45 s by default.
    #[must_use]
    pub fn stream_idle_timeout(mut self, timeout: Duration) -> Self {
        self.stream_idle_timeout = timeout;
        self
    }

    /// Observes every attempt.
    #[must_use]
    pub fn hook(mut self, hook: impl Hook + 'static) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] when there are no credentials, a key without scopes, or a URL
    /// that cannot be used; [`Error::Auth`] when the HTTP stack cannot start.
    pub fn build(self) -> Result<Client<P>, Error> {
        let base = check_url("base_url", &self.base_url, true)?;
        let token_url = check_url("token_url", &self.token_url, false)?;
        let prefix = env_prefix(P::NAME);
        let provider: Arc<dyn DynProvider> = match (self.provider, self.token, self.key) {
            (Some(p), _, _) => p,
            (None, Some(token), _) => Arc::new(StaticToken::new(token)),
            (None, None, Some((id, secret))) => {
                if self.scopes.is_empty() {
                    return Err(ConfigError::Missing {
                        what: "scopes",
                        env: format!("{prefix}SCOPES, or call scopes() on the builder"),
                    }
                    .into());
                }
                Arc::new(ClientCredentials::new(id, secret, self.scopes)?.token_url(token_url))
            }
            (None, None, None) => {
                return Err(ConfigError::Missing {
                    what: "credentials",
                    env: format!("{prefix}TOKEN, or {prefix}KEY_ID and {prefix}KEY_SECRET"),
                }
                .into());
            }
        };
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let agent = user_agent(self.user_agent_suffix.as_deref());
        let http = reqwest::Client::builder()
            .user_agent(agent.clone())
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(base.scheme() == "https")
            .build()
            .map_err(|e| ConfigError::Http(e.to_string()))?;
        let host = base.host_str().unwrap_or_default().to_owned();
        let socket = Arc::new(SocketCtx {
            base: base.clone(),
            host: host.clone(),
            provider: Arc::clone(&provider),
            max_retries: self.max_retries,
            idle: self.stream_idle_timeout,
            timeout: self.timeout,
            user_agent: agent,
        });
        Ok(Client {
            inner: Arc::new(Inner {
                streams: self.streams,
                idle: self.stream_idle_timeout,
                hub: Arc::new(Hub::default()),
                socket,
                http,
                base,
                host,
                provider,
                max_retries: self.max_retries,
                timeout: self.timeout,
                hooks: self.hooks,
            }),
            _profile: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Method, Operation, check_url, user_agent};

    #[test]
    fn urls_are_https_or_loopback() {
        assert!(check_url("base_url", "https://api.inorbit.hr", true).is_ok());
        assert!(check_url("base_url", "http://127.0.0.1:8080", true).is_ok());
        assert!(check_url("base_url", "http://[::1]:8080", true).is_ok());
        assert!(check_url("token_url", "http://localhost:8080/oauth2/token", false).is_ok());
        for bad in [
            "http://api.inorbit.hr",
            "ftp://x",
            "https://api.inorbit.hr/v1",
            "https://u:p@api.inorbit.hr",
            "nope",
        ] {
            assert!(check_url("base_url", bad, true).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_user_agent_names_the_sdk_and_the_runtime() {
        let ua = user_agent(Some("myapp/1.0"));
        assert!(ua.starts_with("inorbithr-sdk-rust/"), "{ua}");
        assert!(ua.contains(" rust/") && ua.ends_with(" myapp/1.0"), "{ua}");
    }

    #[test]
    fn operations_know_what_is_safe_to_retry() {
        assert!(Operation::new(Method::Get, "/v1/me").idempotent);
        assert!(!Operation::new(Method::Post, "/v1/x").idempotent);
        assert!(
            Operation::new(Method::Post, "/v1/x")
                .idempotent(true)
                .idempotent
        );
    }
}
