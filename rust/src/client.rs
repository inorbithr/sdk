//! The client: configuration, one call through the pipeline, and the raw path
//! (`docs/design.md` sections 2, 4 and 6; `docs/config.md`).

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use url::Url;

use crate::auth::{
    CliToken, ClientCredentials, DEFAULT_TOKEN_URL, DynProvider, StaticToken, TokenFile,
    TokenProvider,
};
use crate::config::{LoadOptions, ResolvedConfig, Settings, show_duration};
use crate::error::{ConfigError, Error, Headers, RawResponse};
use crate::hooks::{Attempt, Hook};
use crate::middleware::builtins::{self, Budget, Shared, error_kind, to_raw};
use crate::middleware::{
    Body, CallInfo, CallOptions, CallState, Engine, LogLevel, LogRecord, Logger, Pipeline, Request,
};
use crate::profile::{Profile, Public, env_prefix};
use crate::ratelimit::{Latest, RateLimit, RateLimitMode};
use crate::secret::Secret;
use crate::socket::{Ctx as SocketCtx, Hub};
use crate::stream::{EventStream, Guard, QUEUE, Streams, read_sse};
use crate::transport::Transport;

/// The API every client calls unless told otherwise.
pub const DEFAULT_BASE_URL: &str = "https://api.inorbit.hr";
/// The retry budget's capacity (`docs/config.md` section 7.4).
const RETRY_BUDGET: u32 = 500;

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

    pub(crate) fn as_reqwest(self) -> reqwest::Method {
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
    idempotency_key: bool,
    template: Option<&'static str>,
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
            idempotency_key: false,
            template: None,
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

    /// Marks an operation that takes an `Idempotency-Key` (`docs/config.md` section
    /// 7.5): the client sends one key per call, the same on every attempt, and retries
    /// the call like a read.
    #[must_use]
    pub fn idempotency_key(mut self, yes: bool) -> Self {
        self.idempotency_key = yes;
        self
    }

    /// The path template (`/v1/radar/digests/{digest_id}`), for span names
    /// (`docs/config.md` section 7.10).
    #[must_use]
    pub fn template(mut self, template: &'static str) -> Self {
        self.template = Some(template);
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
    engine: Arc<Engine>,
    shared: Arc<Shared>,
    base: Url,
    host: String,
    config: ResolvedConfig,
    hooks_on: bool,
    profile: &'static str,
}

/// When the client goes, its socket task ends once its streams have.
impl Drop for Inner {
    fn drop(&mut self) {
        self.hub.close();
    }
}

/// The InOrbit API client for one profile, built once and shared.
///
/// `P` is the [`Profile`] the client calls as: [`Public`] by default, or a type a
/// generated surface defines, whose operations are implemented for that type only.
pub struct Client<P: Profile = Public> {
    inner: Arc<Inner>,
    call: Option<Arc<CallOptions>>,
    _profile: PhantomData<fn() -> P>,
}

impl<P: Profile> Clone for Client<P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            call: self.call.clone(),
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

    /// A client from code, the environment, the config file the `iohr` command line
    /// shares, the developer's `iohr` login, and defaults, each setting from the first
    /// source that sets it (`docs/config.md`). The usual way to build a client in an
    /// application; [`builder`](Self::builder)`()...load()` adds options in code, which
    /// always win.
    ///
    /// ```no_run
    /// use inorbithr::Client;
    ///
    /// # fn run() -> Result<(), inorbithr::Error> {
    /// let client: Client = Client::load()?;
    /// println!("{}", client.config().describe());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Config`] with [`ConfigError::Invalid`] listing every problem found,
    /// among them no credentials in any source.
    pub fn load() -> Result<Self, Error> {
        Self::builder().load()
    }

    /// A client from the environment: `INORBIT_<PROFILE>_TOKEN` (an API token), or
    /// `INORBIT_<PROFILE>_KEY_ID`, `INORBIT_<PROFILE>_KEY_SECRET` and
    /// `INORBIT_<PROFILE>_SCOPES` (an API key); `INORBIT_<PROFILE>_BASE_URL` and
    /// `INORBIT_<PROFILE>_TOKEN_URL` falling back to the bare `INORBIT_BASE_URL` and
    /// `INORBIT_TOKEN_URL`. The [`Public`] profile reads the bare names for everything.
    /// Credentials never fall back: a named profile's client calls its own account or
    /// nothing.
    ///
    /// Superseded by [`load`](Self::load), which reads every setting and the config
    /// file as well; this keeps its exact behaviour.
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

    /// The configuration the client was built with: [`ResolvedConfig::describe`]
    /// says where every setting came from.
    #[must_use]
    pub fn config(&self) -> &ResolvedConfig {
        &self.inner.config
    }

    /// The latest rate-limit snapshot any call of this client saw.
    #[must_use]
    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.inner.shared.latest.get()
    }

    /// This client with per-call options for the calls made through the copy it
    /// returns: a timeout, an idempotency key, a `traceparent`. The copy shares
    /// everything else (connections, tokens, the socket).
    ///
    /// ```no_run
    /// use inorbithr::middleware::CallOptions;
    /// use inorbithr::public::{CreateEndpointRequest, Surface as _};
    ///
    /// # async fn run(client: inorbithr::Client) -> Result<(), inorbithr::Error> {
    /// let body = CreateEndpointRequest::default();
    /// let created = client
    ///     .with_options(CallOptions::new().idempotency_key("order-42"))
    ///     .events()
    ///     .create_endpoint(&body)
    ///     .await?;
    /// assert_eq!(created.raw.idempotency_key.as_deref(), Some("order-42"));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_options(&self, options: CallOptions) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            call: Some(Arc::new(options)),
            _profile: PhantomData,
        }
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

    /// One call, as it came back, through the client's pipeline. Idempotent calls
    /// (and writes with an idempotency key) are retried up to `max_retries` times after
    /// a connection failure, a timeout, or a `429`, `503` or `504`; a `401` is answered
    /// by one fresh token and one more attempt; anything else is returned as
    /// [`Error::Api`].
    ///
    /// # Errors
    ///
    /// [`Error::Api`] for an error answer, [`Error::Auth`] when no token could be
    /// produced, and the other variants when the call could not be made or read.
    pub async fn send(&self, op: Operation<'_>) -> Result<RawResponse, Error> {
        let (req, state) = self.prepare(&op, false)?;
        let result = self.inner.engine.run(req).await;
        let result = result.and_then(|resp| to_raw(resp, &state));
        self.finish(&op, &state, &result);
        result
    }

    /// The request a call starts as, and the state its attempts share.
    fn prepare(
        &self,
        op: &Operation<'_>,
        stream: bool,
    ) -> Result<(Request, Arc<CallState>), Error> {
        let url = self.url_for(op)?;
        let options = self.call.as_deref().cloned().unwrap_or_default();
        let mut headers = Headers::default();
        headers.insert(
            "accept",
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
        if op.body.is_some() {
            headers.insert("content-type", "application/json");
        }
        if let Some(tp) = &options.traceparent {
            headers.insert("traceparent", tp.clone());
        }
        let state = Arc::new(CallState::new(options));
        let req = Request {
            method: op.method,
            url,
            headers,
            body: op.body.clone(),
            info: CallInfo {
                operation: op.name,
                method: op.method,
                path: op.path.to_string(),
                template: op.template,
                idempotent: op.idempotent,
                takes_key: op.idempotency_key,
                stream,
                upgrade: false,
                profile: self.inner.profile,
                request_id: None,
                idempotency_key: None,
                attempt: 0,
                state: Arc::clone(&state),
            },
        };
        Ok((req, state))
    }

    fn url_for(&self, op: &Operation<'_>) -> Result<Url, Error> {
        let mut url = url_on(&self.inner.base, &op.path)?;
        if !op.query.is_empty() {
            let mut q = url.query_pairs_mut();
            for (k, v) in &op.query {
                q.append_pair(k, v);
            }
        }
        Ok(url)
    }

    /// After a call: the hooks' `on_error`, the call's log record, its metrics.
    fn finish<T>(&self, op: &Operation<'_>, state: &CallState, result: &Result<T, Error>)
    where
        T: CallOutcome,
    {
        let inner = &self.inner;
        let request_id = state.request_id.get().cloned().unwrap_or_default();
        let duration = u64::try_from(state.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let log = &inner.shared.logger;
        match result {
            Ok(r) => log.emit(
                LogRecord::new(LogLevel::Info, "call")
                    .with("operation", op.name)
                    .with("status", r.status())
                    .with("attempts", state.attempts())
                    .with("duration_ms", duration)
                    .with("request_id", request_id)
                    .with("server_request_id", r.server_request_id()),
            ),
            Err(e) => {
                if inner.hooks_on {
                    let attempt = Attempt {
                        operation: op.name,
                        method: op.method,
                        path: op.path.to_string(),
                        number: state.attempts().max(1),
                        request_id: request_id.clone(),
                        idempotency_key: state.idempotency_key.get().cloned(),
                        stage: crate::middleware::Stage::PerCall,
                    };
                    inner
                        .shared
                        .hooks
                        .iter()
                        .for_each(|h| h.on_error(&attempt, e));
                }
                let (code, status) = match e {
                    Error::Api(api) => (api.code.to_string(), Some(api.status)),
                    _ => (String::new(), None),
                };
                let server = match e {
                    Error::Api(api) => api.raw.server_request_id.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                log.emit(
                    LogRecord::new(LogLevel::Info, "call")
                        .with("operation", op.name)
                        .with("error_kind", error_kind(e))
                        .with("error_code", code.clone())
                        .with("attempts", state.attempts())
                        .with("duration_ms", duration)
                        .with("request_id", request_id.clone())
                        .with("server_request_id", server),
                );
                log.emit(
                    LogRecord::new(LogLevel::Error, "call_failed")
                        .with("operation", op.name)
                        .with("error_kind", error_kind(e))
                        .with("error_code", code)
                        .with("status", status)
                        .with("request_id", request_id),
                );
            }
        }
        #[cfg(feature = "otel")]
        if let Some(otel) = inner.shared.otel.as_ref().filter(|_| inner.shared.metrics) {
            otel.call_done(op.name, state.started.elapsed(), result.as_ref().err());
        }
    }
}

/// What a call's log record reads from its success.
trait CallOutcome {
    fn status(&self) -> u16;
    fn server_request_id(&self) -> String;
}

impl CallOutcome for RawResponse {
    fn status(&self) -> u16 {
        self.status
    }

    fn server_request_id(&self) -> String {
        self.server_request_id.clone().unwrap_or_default()
    }
}

impl<T> CallOutcome for EventStream<T> {
    fn status(&self) -> u16 {
        200
    }

    fn server_request_id(&self) -> String {
        String::new()
    }
}

impl<P: Profile> Client<P> {
    /// Opens a stream (`docs/design.md` section 7): server-sent events, or a call on
    /// the client's `/v1/ws` socket when it was built with [`Streams::Socket`]. The
    /// opening goes through the pipeline and follows the rules of any `GET`: one fresh
    /// token after a `401`, retries after a connection failure, `429`, `503` or `504`.
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
                let body = Value::Object(op.fields.clone());
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
        let (req, state) = self.prepare(op, true)?;
        let result = match inner.engine.run(req).await {
            Ok(resp) if matches!(resp.body, Body::Stream(_)) => {
                let Body::Stream(body) = resp.body else {
                    unreachable!("matched above")
                };
                let (tx, rx) = tokio::sync::mpsc::channel(QUEUE);
                let task = tokio::spawn(read_sse(
                    body,
                    tx,
                    inner.idle,
                    inner.host.clone(),
                    state.request_id.get().cloned().unwrap_or_default(),
                ));
                Ok(EventStream::new(rx, Guard::Task(task.abort_handle())))
            }
            Ok(resp) => match to_raw(resp, &state) {
                Ok(raw) => Err(crate::error::ApiError::parse(raw).into()),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        self.finish(op, &state, &result);
        result
    }
}

/// `path` on `base`: a path on the API, never another host.
fn url_on(base: &Url, path: &str) -> Result<Url, Error> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains('?') || path.contains('#')
    {
        return Err(ConfigError::InvalidUrl {
            what: "path",
            reason: format!("{path:?} is not a path on the API, such as /v1/me; put query values in the operation's query"),
        }
        .into());
    }
    let url = base.join(path).map_err(|e| ConfigError::InvalidUrl {
        what: "path",
        reason: e.to_string(),
    })?;
    if url.origin() != base.origin() {
        return Err(ConfigError::InvalidUrl {
            what: "path",
            reason: "the path must stay on the API host".into(),
        }
        .into());
    }
    Ok(url)
}

/// The request a socket upgrade starts as.
pub(crate) fn upgrade_request(base: &Url, profile: &'static str) -> Request {
    let mut url = base.clone();
    url.set_path("/v1/ws");
    Request {
        method: Method::Get,
        url,
        headers: Headers::default(),
        body: None,
        info: CallInfo {
            operation: "socket",
            method: Method::Get,
            path: "/v1/ws".into(),
            template: Some("/v1/ws"),
            idempotent: true,
            takes_key: false,
            stream: true,
            upgrade: true,
            profile,
            request_id: None,
            idempotency_key: None,
            attempt: 0,
            state: Arc::new(CallState::new(CallOptions::default())),
        },
    }
}

/// `inorbithr-sdk-rust/<version> rust/<rustc> <os>/<arch>`, plus the caller's suffix,
/// in the vocabulary every SDK uses (`docs/config.md` section 7.6).
pub(crate) fn user_agent(suffix: Option<&str>) -> String {
    let os = match std::env::consts::OS {
        o @ ("linux" | "macos" | "windows" | "freebsd" | "android" | "ios") => o,
        _ => "other",
    };
    let arch = match std::env::consts::ARCH {
        a @ ("x86_64" | "aarch64" | "x86" | "arm" | "riscv64") => a,
        _ => "other",
    };
    let mut ua = format!(
        "inorbithr-sdk-rust/{} rust/{} {os}/{arch}",
        env!("CARGO_PKG_VERSION"),
        env!("INORBITHR_RUSTC_VERSION"),
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

type PipelineEdit = Box<dyn for<'p> FnOnce(&'p mut Pipeline) -> &'p mut Pipeline>;

/// The credential a client ends up with, before the transport exists.
enum Credential {
    Provider(Arc<dyn DynProvider>),
    Static(Secret<String>, Option<(String, String)>),
    Key {
        id: String,
        secret: Result<Secret<String>, PathBuf>,
        scopes: Vec<String>,
    },
    File(PathBuf),
    Cli {
        program: PathBuf,
        profile: String,
    },
}

/// Configures a [`Client`]: [`build`](Self::build) reads only what is set here;
/// [`load`](Self::load) also reads the environment, the config file and the `iohr`
/// login, with what is set here winning (`docs/config.md`).
pub struct ClientBuilder<P: Profile = Public> {
    base_url: Option<String>,
    token_url: Option<String>,
    key: Option<(String, Secret<String>)>,
    token: Option<Secret<String>>,
    scopes: Option<Vec<String>>,
    provider: Option<Arc<dyn DynProvider>>,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    total_timeout: Option<Duration>,
    max_retries: Option<u32>,
    retry_base_delay: Option<Duration>,
    retry_max_delay: Option<Duration>,
    retry_after_max: Option<Duration>,
    retry_budget: Option<bool>,
    retry_budget_capacity: Option<u32>,
    user_agent_suffix: Option<String>,
    hooks: Vec<Arc<dyn Hook>>,
    streams: Option<Streams>,
    stream_idle_timeout: Option<Duration>,
    profile: Option<String>,
    config_file: Option<String>,
    credential_sources: Option<Vec<String>>,
    cli_path: Option<String>,
    proxy: Option<Secret<String>>,
    no_proxy: Option<Vec<String>>,
    ca_bundle: Option<String>,
    system_trust: Option<bool>,
    client_cert: Option<String>,
    client_key: Option<String>,
    client_key_password: Option<Secret<String>>,
    pinned_keys: Option<Vec<String>>,
    log: Option<LogLevel>,
    log_headers: Option<bool>,
    log_allow_headers: Option<Vec<String>>,
    tracing: Option<bool>,
    metrics: Option<bool>,
    rate_limit: Option<RateLimitMode>,
    logger: Option<crate::middleware::log::Sink>,
    redact: Option<crate::middleware::log::Redact>,
    http_client: Option<reqwest::Client>,
    pipeline: Vec<PipelineEdit>,
    load_options: Option<LoadOptions>,
    #[cfg(feature = "otel")]
    tracer: Option<opentelemetry::global::BoxedTracer>,
    #[cfg(feature = "otel")]
    meter: Option<opentelemetry::metrics::Meter>,
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
    /// The defaults: the public API, 30 s per attempt, 120 s per call, 2 retries, no
    /// credentials yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base_url: None,
            token_url: None,
            key: None,
            token: None,
            scopes: None,
            provider: None,
            timeout: None,
            connect_timeout: None,
            total_timeout: None,
            max_retries: None,
            retry_base_delay: None,
            retry_max_delay: None,
            retry_after_max: None,
            retry_budget: None,
            retry_budget_capacity: None,
            user_agent_suffix: None,
            hooks: Vec::new(),
            streams: None,
            stream_idle_timeout: None,
            profile: None,
            config_file: None,
            credential_sources: None,
            cli_path: None,
            proxy: None,
            no_proxy: None,
            ca_bundle: None,
            system_trust: None,
            client_cert: None,
            client_key: None,
            client_key_password: None,
            pinned_keys: None,
            log: None,
            log_headers: None,
            log_allow_headers: None,
            tracing: None,
            metrics: None,
            rate_limit: None,
            logger: None,
            redact: None,
            http_client: None,
            pipeline: Vec::new(),
            load_options: None,
            #[cfg(feature = "otel")]
            tracer: None,
            #[cfg(feature = "otel")]
            meter: None,
            _profile: PhantomData,
        }
    }

    /// The API to call, an origin such as `https://api.inorbit.hr`.
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Where an API key is exchanged for a token.
    #[must_use]
    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = Some(url.into());
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
        self.scopes = Some(scopes.into_iter().map(Into::into).collect());
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

    /// The timeout of one attempt: sending, then the answer's headers and whole body
    /// (30 s by default).
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// DNS, TCP and the TLS handshake, per new connection (10 s by default).
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// One call, every attempt and every wait included (120 s by default).
    #[must_use]
    pub fn total_timeout(mut self, timeout: Duration) -> Self {
        self.total_timeout = Some(timeout);
        self
    }

    /// How many times a retryable call is retried (2 by default; 0 disables).
    #[must_use]
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = Some(retries);
        self
    }

    /// The exponential backoff's base (500 ms) and cap (8 s), with full jitter.
    #[must_use]
    pub fn retry_delays(mut self, base: Duration, max: Duration) -> Self {
        self.retry_base_delay = Some(base);
        self.retry_max_delay = Some(max);
        self
    }

    /// The longest `Retry-After` the client waits (60 s); a longer one ends the call.
    #[must_use]
    pub fn retry_after_max(mut self, max: Duration) -> Self {
        self.retry_after_max = Some(max);
        self
    }

    /// Whether retries draw from the client's retry budget (on by default).
    #[must_use]
    pub fn retry_budget(mut self, on: bool) -> Self {
        self.retry_budget = Some(on);
        self
    }

    /// The retry budget's capacity (500), for tests.
    #[must_use]
    pub fn retry_budget_capacity(mut self, capacity: u32) -> Self {
        self.retry_budget_capacity = Some(capacity);
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
        self.streams = Some(streams);
        self
    }

    /// How long a stream may be silent, not even a keep-alive, before it fails (or, on
    /// the socket, reconnects); 45 s by default.
    #[must_use]
    pub fn stream_idle_timeout(mut self, timeout: Duration) -> Self {
        self.stream_idle_timeout = Some(timeout);
        self
    }

    /// Observes every attempt.
    #[must_use]
    pub fn hook(mut self, hook: impl Hook + 'static) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// The config file's profile to use (`load` only; the public client only).
    #[must_use]
    pub fn profile(mut self, name: impl Into<String>) -> Self {
        self.profile = Some(name.into());
        self
    }

    /// The config file to read, or `"off"` for none (`load` only).
    #[must_use]
    pub fn config_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_file = Some(path.into().to_string_lossy().into_owned());
        self
    }

    /// The credential sources `load` may use, of `env`, `workload`, `file` and `cli`;
    /// code is always allowed.
    #[must_use]
    pub fn credential_sources(
        mut self,
        sources: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.credential_sources = Some(sources.into_iter().map(Into::into).collect());
        self
    }

    /// The `iohr` command line the login source runs (`iohr` on `PATH` by default).
    #[must_use]
    pub fn cli_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.cli_path = Some(path.into().to_string_lossy().into_owned());
        self
    }

    /// The proxy every request goes through (`http://` or `https://`, user-info sent as
    /// Basic `Proxy-Authorization`), or `"off"` for none, the standard variables
    /// included. A proxy set here applies to loopback too.
    #[must_use]
    pub fn proxy(mut self, url: impl Into<Secret<String>>) -> Self {
        self.proxy = Some(url.into());
        self
    }

    /// Hosts that skip the proxy: names (with their subdomains), `host:port`, IP
    /// addresses, CIDR ranges, or `*` (`docs/config.md` section 6.2).
    #[must_use]
    pub fn no_proxy(mut self, entries: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.no_proxy = Some(entries.into_iter().map(Into::into).collect());
        self
    }

    /// PEM certificates added to the system's trust store (a TLS-inspecting proxy's root).
    #[must_use]
    pub fn ca_bundle(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_bundle = Some(path.into().to_string_lossy().into_owned());
        self
    }

    /// `false` trusts only the [`ca_bundle`](Self::ca_bundle) (a private gateway).
    #[must_use]
    pub fn system_trust(mut self, on: bool) -> Self {
        self.system_trust = Some(on);
        self
    }

    /// A client certificate chain and its key (PEM files) for mTLS.
    #[must_use]
    pub fn client_cert(mut self, cert: impl Into<PathBuf>, key: impl Into<PathBuf>) -> Self {
        self.client_cert = Some(cert.into().to_string_lossy().into_owned());
        self.client_key = Some(key.into().to_string_lossy().into_owned());
        self
    }

    /// The password of an encrypted client key (feature `encrypted-key`).
    #[must_use]
    pub fn client_key_password(mut self, password: impl Into<Secret<String>>) -> Self {
        self.client_key_password = Some(password.into());
        self
    }

    /// Pins the server's public key: base64 SHA-256 hashes of the SPKI, at least two
    /// (the current key and a backup).
    #[must_use]
    pub fn pinned_keys(mut self, pins: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.pinned_keys = Some(pins.into_iter().map(Into::into).collect());
        self
    }

    /// The log level (`Off` by default; `docs/config.md` section 7.9).
    #[must_use]
    pub fn log(mut self, level: LogLevel) -> Self {
        self.log = Some(level);
        self
    }

    /// Logs allowlisted header values at `debug`.
    #[must_use]
    pub fn log_headers(mut self, on: bool) -> Self {
        self.log_headers = Some(on);
        self
    }

    /// Header names added to the logging allowlist; the never-logged ones stay out.
    #[must_use]
    pub fn log_allow_headers(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.log_allow_headers = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// Where records go instead of `tracing` events.
    #[must_use]
    pub fn logger(mut self, sink: impl Fn(&LogRecord) + Send + Sync + 'static) -> Self {
        self.logger = Some(Arc::new(sink));
        self
    }

    /// The last step before the sink: change a record, or drop it with `None`.
    #[must_use]
    pub fn redact(
        mut self,
        f: impl Fn(LogRecord) -> Option<LogRecord> + Send + Sync + 'static,
    ) -> Self {
        self.redact = Some(Arc::new(f));
        self
    }

    /// OpenTelemetry spans (on by default with the `otel` feature).
    #[must_use]
    pub fn tracing(mut self, on: bool) -> Self {
        self.tracing = Some(on);
        self
    }

    /// OpenTelemetry metrics (as `tracing` by default).
    #[must_use]
    pub fn metrics(mut self, on: bool) -> Self {
        self.metrics = Some(on);
        self
    }

    /// What the client does with rate-limit headers (`Observe` by default).
    #[must_use]
    pub fn rate_limit(mut self, mode: RateLimitMode) -> Self {
        self.rate_limit = Some(mode);
        self
    }

    /// Your own HTTP client: its proxy, trust, certificates and pool settings apply,
    /// and setting those here as well is an error. The pipeline still runs in full; the
    /// token exchange uses this client too. Redirects to another origin are refused.
    #[must_use]
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    /// Edits the pipeline by name (`docs/config.md` section 7.3):
    /// `.pipeline(|p| p.add_per_retry(Probe).remove("rate_limit"))`.
    #[must_use]
    pub fn pipeline(
        mut self,
        edit: impl for<'p> FnOnce(&'p mut Pipeline) -> &'p mut Pipeline + 'static,
    ) -> Self {
        self.pipeline.push(Box::new(edit));
        self
    }

    /// What `load` reads instead of the process: the environment, the OS, the home and
    /// working directories (for tests).
    #[must_use]
    pub fn load_options(mut self, options: LoadOptions) -> Self {
        self.load_options = Some(options);
        self
    }

    /// The tracer provider spans come from (the global one by default).
    #[cfg(feature = "otel")]
    #[must_use]
    pub fn tracer_provider<T>(mut self, provider: &T) -> Self
    where
        T: opentelemetry::trace::TracerProvider,
        T::Tracer: Send + Sync + 'static,
        <T::Tracer as opentelemetry::trace::Tracer>::Span: Send + Sync + 'static,
    {
        self.tracer = Some(opentelemetry::global::BoxedTracer::new(Box::new(
            provider.tracer("inorbithr"),
        )));
        self
    }

    /// The meter provider metrics go to (the global one by default).
    #[cfg(feature = "otel")]
    #[must_use]
    pub fn meter_provider(mut self, provider: &dyn opentelemetry::metrics::MeterProvider) -> Self {
        self.meter = Some(provider.meter("inorbithr"));
        self
    }

    /// The options set in code, by catalogue name, for resolution.
    fn code(&self) -> Map<String, Value> {
        let mut c = Map::new();
        let d = |v: Duration| {
            json!(show_duration(
                u64::try_from(v.as_millis()).unwrap_or(u64::MAX)
            ))
        };
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                c.insert(k.to_owned(), v);
            }
        };
        put("profile", self.profile.clone().map(Value::from));
        put("config_file", self.config_file.clone().map(Value::from));
        put("base_url", self.base_url.clone().map(Value::from));
        put("token_url", self.token_url.clone().map(Value::from));
        put(
            "token_provider",
            self.provider.as_ref().map(|_| json!("custom")),
        );
        put("token", self.token.as_ref().map(|_| json!("<redacted>")));
        put("key_id", self.key.as_ref().map(|(id, _)| json!(id)));
        put("scopes", self.scopes.clone().map(Value::from));
        put(
            "credential_sources",
            self.credential_sources.clone().map(Value::from),
        );
        put("cli_path", self.cli_path.clone().map(Value::from));
        put("connect_timeout", self.connect_timeout.map(d));
        put("timeout", self.timeout.map(d));
        put("total_timeout", self.total_timeout.map(d));
        put("stream_idle_timeout", self.stream_idle_timeout.map(d));
        put("max_retries", self.max_retries.map(Value::from));
        put("retry_base_delay", self.retry_base_delay.map(d));
        put("retry_max_delay", self.retry_max_delay.map(d));
        put("retry_after_max", self.retry_after_max.map(d));
        put("retry_budget", self.retry_budget.map(Value::from));
        put(
            "streams",
            self.streams.map(|s| {
                json!(if s == Streams::Socket {
                    "socket"
                } else {
                    "sse"
                })
            }),
        );
        put("proxy", self.proxy.as_ref().map(|p| json!(p.expose())));
        put("no_proxy", self.no_proxy.clone().map(Value::from));
        put("ca_bundle", self.ca_bundle.clone().map(Value::from));
        put("system_trust", self.system_trust.map(Value::from));
        put("client_cert", self.client_cert.clone().map(Value::from));
        put("client_key", self.client_key.clone().map(Value::from));
        put(
            "client_key_password",
            self.client_key_password.as_ref().map(|p| json!(p.expose())),
        );
        put("pinned_keys", self.pinned_keys.clone().map(Value::from));
        put("log", self.log.map(|l| json!(l.as_str())));
        put("log_headers", self.log_headers.map(Value::from));
        put(
            "log_allow_headers",
            self.log_allow_headers.clone().map(Value::from),
        );
        put("tracing", self.tracing.map(Value::from));
        put("metrics", self.metrics.map(Value::from));
        put(
            "rate_limit",
            self.rate_limit.map(|m| {
                json!(match m {
                    RateLimitMode::Wait => "wait",
                    RateLimitMode::Off => "off",
                    _ => "observe",
                })
            }),
        );
        put(
            "user_agent_suffix",
            self.user_agent_suffix.clone().map(Value::from),
        );
        put(
            "http_client",
            self.http_client.as_ref().map(|_| json!("custom")),
        );
        c
    }

    /// Builds the client from what was set here only: no environment, no file. Use
    /// [`load`](Self::load) to read those too.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] when there are no credentials, a key without scopes, or a URL
    /// or a setting that cannot be used; [`Error::Auth`] when the HTTP stack cannot
    /// start.
    pub fn build(self) -> Result<Client<P>, Error> {
        let prefix = env_prefix(P::NAME);
        if self.provider.is_none() && self.token.is_none() {
            match &self.key {
                Some(_) if self.scopes.as_ref().is_none_or(Vec::is_empty) => {
                    return Err(ConfigError::Missing {
                        what: "scopes",
                        env: format!("{prefix}SCOPES, or call scopes() on the builder"),
                    }
                    .into());
                }
                Some(_) => {}
                None => {
                    return Err(ConfigError::Missing {
                        what: "credentials",
                        env: format!("{prefix}TOKEN, or {prefix}KEY_ID and {prefix}KEY_SECRET"),
                    }
                    .into());
                }
            }
        }
        check_url(
            "base_url",
            self.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL),
            true,
        )?;
        check_url(
            "token_url",
            self.token_url.as_deref().unwrap_or(DEFAULT_TOKEN_URL),
            false,
        )?;
        // Code only, as before `load` existed; the standard proxy variables are still
        // honoured, as the HTTP stack honoured them.
        let env: Vec<(String, String)> = ["https_proxy", "HTTPS_PROXY", "no_proxy", "NO_PROXY"]
            .into_iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_owned(), v)))
            .collect();
        let options = LoadOptions::new().env(env);
        let mut code = self.code();
        code.insert("config_file".into(), json!("off"));
        code.remove("profile");
        self.finish(&options, code)
    }

    /// Builds the client from code, the environment, the config file, the `iohr`
    /// login and defaults, each setting from the first source that sets it
    /// (`docs/config.md`). What is set on this builder always wins.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] with [`ConfigError::Invalid`] listing every problem found.
    pub fn load(mut self) -> Result<Client<P>, Error> {
        let options = self.load_options.take().unwrap_or_default();
        let code = self.code();
        self.finish(&options, code)
    }

    fn finish(
        mut self,
        options: &LoadOptions,
        code: Map<String, Value>,
    ) -> Result<Client<P>, Error> {
        let typed = (P::NAME != Public::NAME).then_some(P::NAME);
        let resolved = crate::config::run(options, code, typed)?;
        let mut config = ResolvedConfig::new(resolved.describe);
        let settings = Settings::from_resolved(&config, &resolved.secrets)?;
        let credential = self.credential(&config, &settings, &resolved.secrets)?;
        let base = check_url("base_url", &settings.base_url, true)?;
        let token_url = check_url("token_url", &settings.token_url, false)?;
        let https_only = base.scheme() == "https" && token_url.scheme() == "https";
        let caller = self.http_client.take();
        let transport = Transport::new(&settings, caller, https_only)?;
        let agent = user_agent(settings.user_agent_suffix.as_deref());
        let logger = Logger {
            level: settings.log,
            headers: settings.log_headers,
            allow: settings.log_allow_headers.clone(),
            sink: self.logger.take(),
            redact: self.redact.take(),
            profile: config.profile().unwrap_or(P::NAME).to_owned(),
        };
        let (provider, static_token): (Arc<dyn DynProvider>, _) = match credential {
            Credential::Provider(p) => (p, None),
            Credential::Static(t, label) => (Arc::new(StaticToken::new(t)), label),
            Credential::Key { id, secret, scopes } => {
                let p = match secret {
                    Ok(s) => ClientCredentials::new(id, s, scopes),
                    Err(path) => ClientCredentials::from_secret_file(id, path, scopes),
                }?
                .token_url(token_url.clone())
                .transport(transport.http.clone(), agent.clone());
                p.attach(&logger);
                (Arc::new(p), None)
            }
            Credential::File(path) => (Arc::new(TokenFile::new(path)), None),
            Credential::Cli { program, profile } => {
                let p = CliToken::new(program, profile);
                p.attach(&logger);
                (Arc::new(p), None)
            }
        };
        let capacity = self.retry_budget_capacity.unwrap_or(RETRY_BUDGET);
        let shared = Arc::new(Shared {
            provider,
            static_token,
            hooks: std::mem::take(&mut self.hooks),
            logger,
            latest: Latest::default(),
            budget: settings.retry_budget.then(|| Budget::new(capacity)),
            user_agent: agent,
            timeout: settings.timeout,
            total_timeout: settings.total_timeout,
            max_retries: settings.max_retries,
            retry_base_delay: settings.retry_base_delay,
            retry_max_delay: settings.retry_max_delay,
            retry_after_max: settings.retry_after_max,
            rate_limit: settings.rate_limit,
            tracing: settings.tracing,
            metrics: settings.metrics,
            #[cfg(feature = "otel")]
            otel: Some(crate::otel::Otel::new(
                self.tracer.take(),
                self.meter.take(),
            )),
        });
        let mut pipeline = Pipeline::new(builtins::defaults(&shared));
        for edit in std::mem::take(&mut self.pipeline) {
            edit(&mut pipeline);
        }
        let names = pipeline.names();
        let hooks_on = pipeline.has("hooks");
        let entries = pipeline.finish()?;
        config.set_pipeline(names);
        let engine = Arc::new(Engine::new(entries, transport));
        let host = base.host_str().unwrap_or_default().to_owned();
        let socket = Arc::new(SocketCtx {
            base: base.clone(),
            host: host.clone(),
            engine: Arc::clone(&engine),
            shared: Arc::clone(&shared),
            profile: P::NAME,
            max_retries: settings.max_retries,
            idle: settings.stream_idle_timeout,
        });
        Ok(Client {
            inner: Arc::new(Inner {
                streams: settings.streams,
                idle: settings.stream_idle_timeout,
                hub: Arc::new(Hub::default()),
                socket,
                engine,
                shared,
                base,
                host,
                config,
                hooks_on,
                profile: P::NAME,
            }),
            call: None,
            _profile: PhantomData,
        })
    }

    /// The credential resolution chose, made concrete.
    fn credential(
        &mut self,
        config: &ResolvedConfig,
        settings: &Settings,
        secrets: &std::collections::BTreeMap<String, Secret<String>>,
    ) -> Result<Credential, Error> {
        let d = config.describe();
        let source = d["credential"]["source"].as_str().unwrap_or_default();
        let kind = d["credential"]["kind"].as_str().unwrap_or_default();
        let s = config.settings();
        let text = |k: &str| {
            s.get(k)
                .and_then(|e| e["value"].as_str())
                .map(str::to_owned)
        };
        let scopes = || -> Vec<String> {
            s.get("scopes")
                .and_then(|e| e["value"].as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let _ = settings;
        let missing = |what: &'static str| -> Error {
            ConfigError::Missing {
                what,
                env: "a credential in code, the environment, the config file or `iohr login`"
                    .into(),
            }
            .into()
        };
        Ok(match (source, kind) {
            ("code", _) => {
                if let Some(p) = self.provider.take() {
                    Credential::Provider(p)
                } else if let Some(t) = self.token.take() {
                    Credential::Static(t, None)
                } else if let Some((id, secret)) = self.key.take() {
                    let scopes = scopes();
                    if scopes.is_empty() {
                        return Err(ConfigError::Missing {
                            what: "scopes",
                            env: format!(
                                "{}SCOPES, or call scopes() on the builder",
                                env_prefix(P::NAME)
                            ),
                        }
                        .into());
                    }
                    Credential::Key {
                        id,
                        secret: Ok(secret),
                        scopes,
                    }
                } else {
                    return Err(missing("credentials"));
                }
            }
            (_, "static_token") => {
                let var = d["settings"]["token"]["source"]
                    .as_str()
                    .unwrap_or("env INORBIT_TOKEN")
                    .trim_start_matches("env ")
                    .to_owned();
                Credential::Static(
                    secrets
                        .get("token")
                        .cloned()
                        .ok_or_else(|| missing("token"))?,
                    Some((
                        format!("token from {var}"),
                        "make a new API token in the console or with `iohr token create`".into(),
                    )),
                )
            }
            (_, "token_file") => Credential::File(PathBuf::from(
                text("token_file").ok_or_else(|| missing("token_file"))?,
            )),
            (_, "client_credentials") => Credential::Key {
                id: text("key_id").ok_or_else(|| missing("key_id"))?,
                secret: match secrets.get("key_secret") {
                    Some(s) => Ok(s.clone()),
                    None => Err(PathBuf::from(
                        text("key_secret_file").ok_or_else(|| missing("key_secret_file"))?,
                    )),
                },
                scopes: scopes(),
            },
            (_, "cli") => Credential::Cli {
                program: PathBuf::from(text("cli_path").unwrap_or_else(|| "iohr".into())),
                profile: config.profile().unwrap_or_default().to_owned(),
            },
            _ => return Err(missing("credentials")),
        })
    }
}

/// A credential found the way `load` finds one (`docs/config.md` section 5.1), as a
/// [`TokenProvider`] for a chain of your own or another client.
pub struct DefaultCredential(Arc<dyn DynProvider>);

impl fmt::Debug for DefaultCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultCredential").finish_non_exhaustive()
    }
}

impl DefaultCredential {
    /// The credential the public client's `load` would use.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] when no source has credentials, as `load` reports it.
    pub fn load() -> Result<Self, Error> {
        let client: Client = Client::load()?;
        Ok(Self(Arc::clone(&client.inner.shared.provider)))
    }
}

impl TokenProvider for DefaultCredential {
    fn token(
        &self,
    ) -> impl std::future::Future<Output = Result<crate::Token, crate::AuthError>> + Send {
        self.0.token()
    }

    fn invalidate(&self) -> impl std::future::Future<Output = ()> + Send {
        self.0.invalidate()
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
