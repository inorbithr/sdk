//! The middleware pipeline (`docs/config.md` section 7): every call goes through an
//! ordered list of named steps, each a function of the request and the rest of the
//! list. The built-ins, outermost first:
//!
//! | Name | Stage | What it does |
//! |---|---|---|
//! | `request_id` | per call | `x-request-id: iohr-<16 hex>`, the same on every attempt |
//! | `user_agent` | per call | `inorbithr-sdk-rust/<version> rust/<rustc> <os>/<arch>[ <suffix>]` |
//! | `idempotency_key` | per call | one key per call on operations that take `Idempotency-Key` |
//! | `call_tracing` | per call | one OpenTelemetry span per call (feature `otel`) |
//! | `deadline` | per call | the call's total timeout |
//! | `retry` | boundary | retries, with `Retry-After` and the retry budget |
//! | `auth` | per retry | `Authorization: Bearer`, one fresh token after a `401` |
//! | `rate_limit` | per retry | reads the rate-limit headers; waits in `wait` mode |
//! | `attempt_tracing` | per retry | one HTTP client span per attempt; `traceparent` |
//! | `logging` | per retry | allowlisted, redacted records; silent until `log` is set |
//! | `hooks` | per retry | the [`Hook`](crate::Hook)s |
//! | `timeout` | per retry | the attempt's timeout |
//!
//! A middleware of your own is a [`Middleware`]; [`Pipeline`] places it by name:
//!
//! ```
//! use inorbithr::middleware::{BoxFuture, Middleware, Next, Request, Response};
//! use inorbithr::{Client, Error};
//!
//! struct Tag;
//!
//! impl Middleware for Tag {
//!     fn name(&self) -> &'static str {
//!         "tag"
//!     }
//!
//!     fn handle<'a>(&'a self, mut req: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response, Error>> {
//!         req.headers_mut().insert("x-team", "payments");
//!         Box::pin(next.run(req))
//!     }
//! }
//!
//! # fn build() -> Result<(), Error> {
//! let client: Client = Client::builder()
//!     .token("api-token")
//!     .pipeline(|p| p.add_per_call(Tag).remove("rate_limit"))
//!     .build()?;
//! # Ok(())
//! # }
//! ```
//!
//! A middleware must not log secrets or bodies: the built-ins never do, and one of your
//! own is yours to keep to that rule.

pub(crate) mod builtins;
pub(crate) mod log;

use std::borrow::Cow;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use url::Url;

pub(crate) use self::log::Logger;
pub use self::log::{LogLevel, LogRecord};
use crate::client::Method;
use crate::error::{Error, Headers};
use crate::transport::Transport;

/// A boxed, sendable future, the return type of [`Middleware::handle`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One named step of the pipeline.
pub trait Middleware: Send + Sync + 'static {
    /// The name the pipeline knows it by; unique in a pipeline.
    fn name(&self) -> &'static str;

    /// Handles one request: change it, answer it without calling `next`, call
    /// `next.run(req)` once, or more than once. From the per-call stage, each call of
    /// `next` is a fresh attempt.
    fn handle<'a>(&'a self, req: Request, next: Next<'a>)
    -> BoxFuture<'a, Result<Response, Error>>;
}

/// Where a middleware sits: once per call, or on every attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Once per call, outside the retry loop.
    PerCall,
    /// On every attempt, inside the retry loop.
    PerRetry,
}

impl Stage {
    /// `per_call` or `per_retry`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PerCall => "per_call",
            Self::PerRetry => "per_retry",
        }
    }
}

/// Per-call options: given with [`Client::with_options`](crate::Client::with_options)
/// for the calls made through the returned client.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CallOptions {
    pub(crate) timeout: Option<Duration>,
    pub(crate) idempotency_key: Option<String>,
    pub(crate) traceparent: Option<String>,
}

impl CallOptions {
    /// No options: the client's settings apply.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the attempt timeout, and shortens the total timeout to it (it never
    /// extends it).
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The idempotency key to send, for an operation that takes one; without it the
    /// client makes one per call. On an operation that takes none, a key is an error.
    #[must_use]
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// A W3C `traceparent` to send as is when the client does not trace itself.
    #[must_use]
    pub fn traceparent(mut self, traceparent: impl Into<String>) -> Self {
        self.traceparent = Some(traceparent.into());
        self
    }
}

/// What one call shares between its attempts.
pub(crate) struct CallState {
    pub(crate) options: CallOptions,
    attempts: AtomicU32,
    pub(crate) refreshed: AtomicBool,
    deadline: Mutex<Option<Instant>>,
    pub(crate) request_id: OnceLock<String>,
    pub(crate) idempotency_key: OnceLock<String>,
    pub(crate) started: Instant,
    #[cfg(feature = "otel")]
    pub(crate) otel: Mutex<Option<opentelemetry::Context>>,
}

impl CallState {
    pub(crate) fn new(options: CallOptions) -> Self {
        Self {
            options,
            attempts: AtomicU32::new(0),
            refreshed: AtomicBool::new(false),
            deadline: Mutex::new(None),
            request_id: OnceLock::new(),
            idempotency_key: OnceLock::new(),
            started: Instant::now(),
            #[cfg(feature = "otel")]
            otel: Mutex::new(None),
        }
    }

    /// The next attempt's number, 1 for the first.
    pub(crate) fn next_attempt(&self) -> u32 {
        self.attempts.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// How many attempts were made.
    pub(crate) fn attempts(&self) -> u32 {
        self.attempts.load(Ordering::SeqCst)
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn set_deadline(&self, at: Instant) {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(at);
    }
}

/// What a middleware may know about the call a request belongs to. Read only.
#[derive(Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "what the call is, flag by flag"
)]
pub struct CallInfo {
    pub(crate) operation: &'static str,
    pub(crate) method: Method,
    pub(crate) path: String,
    #[cfg_attr(not(feature = "otel"), allow(dead_code))]
    pub(crate) template: Option<&'static str>,
    pub(crate) idempotent: bool,
    pub(crate) takes_key: bool,
    pub(crate) stream: bool,
    pub(crate) upgrade: bool,
    pub(crate) profile: &'static str,
    pub(crate) request_id: Option<String>,
    pub(crate) idempotency_key: Option<String>,
    pub(crate) attempt: u32,
    pub(crate) state: Arc<CallState>,
}

impl fmt::Debug for CallInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallInfo")
            .field("operation", &self.operation)
            .field("idempotent", &self.idempotent())
            .field("request_id", &self.request_id)
            .field("attempt", &self.attempt)
            .field("stream", &self.stream)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl CallInfo {
    /// The operation's name (`accounts.get_me`), or `request` for a raw call.
    #[must_use]
    pub fn operation(&self) -> &'static str {
        self.operation
    }

    /// Whether `retry` may repeat the call: its method is idempotent, it is marked so,
    /// or it carries an idempotency key.
    #[must_use]
    pub fn idempotent(&self) -> bool {
        self.idempotent || self.takes_key
    }

    /// The call's idempotency key, once `idempotency_key` has set it.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// The call's request id, once `request_id` has set it.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// The attempt, 1 for the first; 0 in the per-call stage.
    #[must_use]
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// When the call's total timeout passes, once `deadline` has set it.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.state.deadline()
    }

    /// Whether the answer is a stream, whose body a middleware must not read.
    #[must_use]
    pub fn stream(&self) -> bool {
        self.stream
    }

    /// The profile the client calls as.
    #[must_use]
    pub fn profile(&self) -> &'static str {
        self.profile
    }

    /// The stage the request is in.
    #[must_use]
    pub fn stage(&self) -> Stage {
        if self.attempt == 0 {
            Stage::PerCall
        } else {
            Stage::PerRetry
        }
    }
}

/// A request on its way through the pipeline: method, URL, headers, the body as
/// bytes, and [`CallInfo`].
#[derive(Clone)]
pub struct Request {
    pub(crate) method: Method,
    pub(crate) url: Url,
    pub(crate) headers: Headers,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) info: CallInfo,
}

/// The method and path only: headers carry credentials, URLs query values, bodies data
/// (SR-13).
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("path", &self.url.path())
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Request {
    /// The method.
    #[must_use]
    pub fn method(&self) -> Method {
        self.method
    }

    /// The URL, query included.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The URL, to change.
    pub fn url_mut(&mut self) -> &mut Url {
        &mut self.url
    }

    /// The headers.
    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// The headers, to change.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// The body, when there is one.
    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }

    /// The body, to change or remove.
    pub fn body_mut(&mut self) -> &mut Option<Vec<u8>> {
        &mut self.body
    }

    /// What the request belongs to.
    #[must_use]
    pub fn info(&self) -> &CallInfo {
        &self.info
    }
}

/// The body of a [`Response`].
pub(crate) enum Body {
    Bytes(Vec<u8>),
    Stream(reqwest::Response),
    Socket(crate::socket::Ws),
}

/// An answer on its way back: status, headers, and the body, or a handle to a stream's
/// body, which a middleware must not read.
pub struct Response {
    pub(crate) status: u16,
    pub(crate) headers: Headers,
    pub(crate) body: Body,
    pub(crate) rate_limit: Option<crate::ratelimit::RateLimit>,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("stream", &self.is_stream())
            .finish_non_exhaustive()
    }
}

impl Response {
    /// An answer made by a middleware without calling the API.
    #[must_use]
    pub fn new(status: u16, headers: Headers, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body: Body::Bytes(body),
            rate_limit: None,
        }
    }

    /// The HTTP status.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The headers.
    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// The headers, to change.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// The body; `None` for a stream.
    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        match &self.body {
            Body::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Whether the body is a stream.
    #[must_use]
    pub fn is_stream(&self) -> bool {
        !matches!(self.body, Body::Bytes(_))
    }
}

/// The rest of the pipeline after a middleware. Copy it to call it more than once.
#[derive(Clone, Copy)]
pub struct Next<'a> {
    rest: &'a [Entry],
    transport: &'a Transport,
}

impl fmt::Debug for Next<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.rest.iter().map(|e| &e.name))
            .finish()
    }
}

impl<'a> Next<'a> {
    /// Sends `req` through the rest of the pipeline and the transport.
    #[must_use = "the future does nothing until awaited"]
    pub fn run(self, req: Request) -> BoxFuture<'a, Result<Response, Error>> {
        match self.rest.split_first() {
            Some((first, rest)) => first.mw.handle(
                req,
                Next {
                    rest,
                    transport: self.transport,
                },
            ),
            None => Box::pin(self.transport.send(req)),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Entry {
    name: Cow<'static, str>,
    mw: Arc<dyn Middleware>,
}

/// The ordered, named middlewares of a client, edited by name at construction with
/// [`ClientBuilder::pipeline`](crate::ClientBuilder::pipeline) (`docs/config.md`
/// section 7.3). Names are unique; `retry`, `auth` and `timeout` can be replaced but
/// not removed. A mistake is reported when the client is built.
pub struct Pipeline {
    entries: Vec<Entry>,
    errors: Vec<String>,
}

impl fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.names()).finish()
    }
}

/// The built-ins that can be replaced but not removed.
const KEPT: [&str; 3] = ["retry", "auth", "timeout"];

impl Pipeline {
    pub(crate) fn new(builtins: Vec<Arc<dyn Middleware>>) -> Self {
        Self {
            entries: builtins
                .into_iter()
                .map(|mw| Entry {
                    name: Cow::Borrowed(mw.name()),
                    mw,
                })
                .collect(),
            errors: Vec::new(),
        }
    }

    /// The middlewares by name, outermost first.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.to_string()).collect()
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    fn insert_at(&mut self, at: usize, m: impl Middleware) {
        let name = m.name();
        if self.find(name).is_some() {
            self.errors.push(format!(
                "a middleware named {name:?} is already in the pipeline"
            ));
            return;
        }
        self.entries.insert(
            at.min(self.entries.len()),
            Entry {
                name: Cow::Borrowed(name),
                mw: Arc::new(m),
            },
        );
    }

    fn missing(&mut self, name: &str) {
        self.errors.push(format!(
            "there is no middleware named {name:?} in the pipeline ({})",
            self.names().join(", ")
        ));
    }

    /// Adds `m` at the per-call slot: just before `retry`, after earlier additions.
    pub fn add_per_call(&mut self, m: impl Middleware) -> &mut Self {
        match self.find("retry") {
            Some(i) => self.insert_at(i, m),
            None => self.missing("retry"),
        }
        self
    }

    /// Adds `m` at the per-retry slot: just before `timeout`, after earlier additions.
    pub fn add_per_retry(&mut self, m: impl Middleware) -> &mut Self {
        match self.find("timeout") {
            Some(i) => self.insert_at(i, m),
            None => self.missing("timeout"),
        }
        self
    }

    /// Inserts `m` just before the middleware named `name`.
    pub fn insert_before(&mut self, name: &str, m: impl Middleware) -> &mut Self {
        match self.find(name) {
            Some(i) => self.insert_at(i, m),
            None => self.missing(name),
        }
        self
    }

    /// Inserts `m` just after the middleware named `name`.
    pub fn insert_after(&mut self, name: &str, m: impl Middleware) -> &mut Self {
        match self.find(name) {
            Some(i) => self.insert_at(i + 1, m),
            None => self.missing(name),
        }
        self
    }

    /// Swaps the middleware named `name` for `m`, which takes its place and its name.
    pub fn replace(&mut self, name: &str, m: impl Middleware) -> &mut Self {
        match self.find(name) {
            Some(i) => self.entries[i].mw = Arc::new(m),
            None => self.missing(name),
        }
        self
    }

    /// Drops the middleware named `name`. `retry`, `auth` and `timeout` cannot be
    /// removed (set `max_retries` to 0 instead of removing `retry`).
    pub fn remove(&mut self, name: &str) -> &mut Self {
        if KEPT.contains(&name) {
            self.errors.push(format!(
                "{name} can be replaced but not removed; without it the client would {}",
                match name {
                    "retry" => "retry nothing (set max_retries to 0 instead)",
                    "auth" => "send no credentials",
                    _ => "wait for ever",
                }
            ));
            return self;
        }
        match self.find(name) {
            Some(i) => {
                self.entries.remove(i);
            }
            None => self.missing(name),
        }
        self
    }

    #[must_use]
    pub(crate) fn has(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    pub(crate) fn finish(self) -> Result<Vec<Entry>, crate::error::ConfigError> {
        if self.errors.is_empty() {
            Ok(self.entries)
        } else {
            Err(crate::error::ConfigError::Pipeline(self.errors.join("; ")))
        }
    }
}

/// A client's pipeline and transport, ready to run requests.
pub(crate) struct Engine {
    entries: Vec<Entry>,
    pub(crate) transport: Transport,
}

impl Engine {
    pub(crate) fn new(entries: Vec<Entry>, transport: Transport) -> Self {
        Self { entries, transport }
    }

    pub(crate) async fn run(&self, req: Request) -> Result<Response, Error> {
        Next {
            rest: &self.entries,
            transport: &self.transport,
        }
        .run(req)
        .await
    }
}
