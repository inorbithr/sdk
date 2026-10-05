//! The built-in middlewares (`docs/config.md` section 7.2), in pipeline order.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{BoxFuture, Middleware, Next, Request, Response};
use crate::auth::DynProvider;
use crate::error::{ApiError, AuthError, ConfigError, Error, Headers, RawResponse};
use crate::hooks::{Attempt, Hook};
use crate::middleware::{LogLevel, LogRecord, Logger};
use crate::ratelimit::{Latest, RateLimit, RateLimitMode};
use crate::retry::{backoff, request_id, retry_after, retryable_status};

/// What the built-ins share: the client's settings and state.
pub(crate) struct Shared {
    pub(crate) provider: Arc<dyn DynProvider>,
    /// For a token that cannot be replaced (a static token from `load`): what it is and
    /// how to get a new one, for the error a `401` ends with.
    pub(crate) static_token: Option<(String, String)>,
    pub(crate) hooks: Vec<Arc<dyn Hook>>,
    pub(crate) logger: Logger,
    pub(crate) latest: Latest,
    pub(crate) budget: Option<Budget>,
    pub(crate) user_agent: String,
    pub(crate) timeout: Duration,
    pub(crate) total_timeout: Duration,
    pub(crate) max_retries: u32,
    pub(crate) retry_base_delay: Duration,
    pub(crate) retry_max_delay: Duration,
    pub(crate) retry_after_max: Duration,
    pub(crate) rate_limit: RateLimitMode,
    pub(crate) tracing: bool,
    pub(crate) metrics: bool,
    #[cfg(feature = "otel")]
    pub(crate) otel: Option<crate::otel::Otel>,
}

/// The retry budget: a token bucket per client (`docs/config.md` section 7.4).
pub(crate) struct Budget {
    tokens: Mutex<u32>,
    capacity: u32,
}

impl Budget {
    pub(crate) fn new(capacity: u32) -> Self {
        Self {
            tokens: Mutex::new(capacity),
            capacity,
        }
    }

    pub(crate) fn take(&self, cost: u32) -> bool {
        let mut t = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *t >= cost {
            *t -= cost;
            true
        } else {
            false
        }
    }

    pub(crate) fn give(&self, n: u32) {
        let mut t = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *t = (*t + n).min(self.capacity);
    }
}

fn host(req: &Request) -> String {
    req.url.host_str().unwrap_or_default().to_owned()
}

fn attempt_of(req: &Request) -> Attempt {
    Attempt {
        operation: req.info.operation,
        method: req.info.method,
        path: req.info.path.clone(),
        number: req.info.attempt,
        request_id: req.info.request_id.clone().unwrap_or_default(),
        idempotency_key: req.info.idempotency_key.clone(),
        stage: req.info.stage(),
    }
}

/// An answer as the hooks see it.
pub(crate) fn raw_view(req: &Request, resp: &Response) -> RawResponse {
    let mut raw = RawResponse::part(
        resp.status,
        resp.body().map(<[u8]>::to_vec).unwrap_or_default(),
        req.info.request_id.clone().unwrap_or_default(),
    );
    raw.server_request_id = resp.headers.get("x-request-id").map(str::to_owned);
    raw.headers = resp.headers.clone();
    raw.attempts = req.info.attempt;
    raw.idempotency_key = req.info.idempotency_key.clone();
    raw.rate_limit = resp.rate_limit.clone();
    raw
}

pub(crate) fn error_kind(e: &Error) -> &'static str {
    match e {
        Error::Api(_) => "api",
        Error::Connection { .. } => "connection",
        Error::Timeout { .. } | Error::RateLimitWait { .. } => "timeout",
        Error::Auth(_) => "auth",
        Error::Config(_) => "config",
        Error::TooLarge => "too_large",
        Error::Decode { .. } => "decode",
    }
}

// 1. request_id

pub(crate) struct RequestIdMw;

impl Middleware for RequestIdMw {
    fn name(&self) -> &'static str {
        "request_id"
    }

    fn handle<'a>(
        &'a self,
        mut req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        let id = request_id();
        let _ = req.info.state.request_id.set(id.clone());
        req.headers.insert("x-request-id", id.clone());
        req.info.request_id = Some(id);
        next.run(req)
    }
}

// 2. user_agent

pub(crate) struct UserAgentMw(pub(crate) Arc<Shared>);

impl Middleware for UserAgentMw {
    fn name(&self) -> &'static str {
        "user_agent"
    }

    fn handle<'a>(
        &'a self,
        mut req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        req.headers.insert("user-agent", self.0.user_agent.clone());
        next.run(req)
    }
}

// 3. idempotency_key

pub(crate) struct IdempotencyKeyMw;

/// A random UUID, version 4.
pub(crate) fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

impl Middleware for IdempotencyKeyMw {
    fn name(&self) -> &'static str {
        "idempotency_key"
    }

    fn handle<'a>(
        &'a self,
        mut req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        let given = req.info.state.options.idempotency_key.clone();
        if !req.info.takes_key {
            if given.is_some() {
                let op = req.info.operation;
                return Box::pin(async move {
                    Err(ConfigError::Call(format!(
                        "{op} takes no idempotency key: the API would ignore it, so the call is not safe to repeat; leave idempotency_key out"
                    ))
                    .into())
                });
            }
            return next.run(req);
        }
        let key = given.unwrap_or_else(uuid_v4);
        let _ = req.info.state.idempotency_key.set(key.clone());
        req.headers.insert("idempotency-key", key.clone());
        req.info.idempotency_key = Some(key);
        next.run(req)
    }
}

// 4. call_tracing

pub(crate) struct CallTracingMw(pub(crate) Arc<Shared>);

impl Middleware for CallTracingMw {
    fn name(&self) -> &'static str {
        "call_tracing"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        #[cfg(feature = "otel")]
        if let Some(otel) = self.0.otel.as_ref().filter(|_| self.0.tracing) {
            return Box::pin(otel.call(req, next));
        }
        next.run(req)
    }
}

// 5. deadline

pub(crate) struct DeadlineMw(pub(crate) Arc<Shared>);

impl Middleware for DeadlineMw {
    fn name(&self) -> &'static str {
        "deadline"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        let total = req
            .info
            .state
            .options
            .timeout
            .map_or(self.0.total_timeout, |t| t.min(self.0.total_timeout));
        let at = Instant::now() + total;
        req.info.state.set_deadline(at);
        let host = host(&req);
        Box::pin(async move {
            match tokio::time::timeout_at(at.into(), next.run(req)).await {
                Ok(r) => r,
                Err(_) => Err(Error::Timeout {
                    host,
                    secs: total.as_secs().max(1),
                }),
            }
        })
    }
}

// 6. retry

pub(crate) struct RetryMw(pub(crate) Arc<Shared>);

/// Why an attempt may be repeated, what a retry costs, and how long the server asked
/// to wait.
struct Retryable {
    reason: String,
    cost: u32,
    wait: Option<Duration>,
}

fn envelope_retry(resp: &Response) -> Option<Duration> {
    let body = resp.body()?;
    let v: Value = serde_json::from_slice(body).ok()?;
    v["details"].as_array()?.iter().find_map(|d| {
        (d["type"] == "retry")
            .then(|| d["after_seconds"].as_u64().map(Duration::from_secs))
            .flatten()
    })
}

fn classify(result: &Result<Response, Error>) -> Option<Retryable> {
    match result {
        Ok(resp) if retryable_status(resp.status) => {
            let header = retry_after(&resp.headers);
            let wait = header.or_else(|| envelope_retry(resp));
            let cost = if resp.status == 429 || (resp.status == 503 && wait.is_some()) {
                5
            } else {
                10
            };
            Some(Retryable {
                reason: resp.status.to_string(),
                cost,
                wait,
            })
        }
        Err(Error::Connection { .. }) => Some(Retryable {
            reason: "connection".into(),
            cost: 10,
            wait: None,
        }),
        Err(Error::Timeout { .. }) => Some(Retryable {
            reason: "timeout".into(),
            cost: 10,
            wait: None,
        }),
        _ => None,
    }
}

impl Middleware for RetryMw {
    fn name(&self) -> &'static str {
        "retry"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        Box::pin(async move {
            let s = &self.0;
            let state = Arc::clone(&req.info.state);
            let mut retries = 0u32;
            let mut last_cost = 0u32;
            loop {
                let mut attempt = req.clone();
                attempt.info.attempt = state.next_attempt();
                let result = next.run(attempt).await;
                let Some(r) = classify(&result) else {
                    if let Some(b) = &s.budget
                        && result.as_ref().is_ok_and(|r| r.status < 400)
                    {
                        b.give(if retries > 0 { last_cost } else { 1 });
                    }
                    return result;
                };
                if !req.info.idempotent() || retries >= s.max_retries {
                    return result;
                }
                if r.wait.is_some_and(|w| w > s.retry_after_max) {
                    return result;
                }
                let wait = r
                    .wait
                    .unwrap_or_else(|| backoff(retries, s.retry_base_delay, s.retry_max_delay));
                if state.deadline().is_some_and(|d| Instant::now() + wait >= d) {
                    return result;
                }
                if let Some(b) = &s.budget
                    && !b.take(r.cost)
                {
                    return result;
                }
                last_cost = r.cost;
                retries += 1;
                let mut seen = req.clone();
                seen.info.attempt = state.attempts();
                let a = attempt_of(&seen);
                for h in &s.hooks {
                    h.on_retry(&a, &r.reason, wait);
                }
                s.logger.emit(
                    LogRecord::new(LogLevel::Warn, "retry")
                        .with("operation", a.operation)
                        .with("attempt", a.number)
                        .with("reason", r.reason.clone())
                        .with(
                            "delay_ms",
                            u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                        )
                        .with("request_id", a.request_id.clone()),
                );
                #[cfg(feature = "otel")]
                if let Some(otel) = s.otel.as_ref().filter(|_| s.metrics) {
                    otel.retried(a.operation, &r.reason);
                }
                drop(result);
                tokio::time::sleep(wait).await;
            }
        })
    }
}

// 7. auth

pub(crate) struct AuthMw(pub(crate) Arc<Shared>);

impl Middleware for AuthMw {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn handle<'a>(
        &'a self,
        mut req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        Box::pin(async move {
            let s = &self.0;
            let token = s.provider.token().await.map_err(Error::Auth)?;
            req.headers
                .insert("authorization", format!("Bearer {}", token.expose()));
            drop(token);
            let resp = next.run(req.clone()).await?;
            if resp.status != 401 {
                return Ok(resp);
            }
            if let Some((what, remedy)) = &s.static_token {
                return Err(AuthError::Refused {
                    what: what.clone(),
                    remedy: remedy.clone(),
                }
                .into());
            }
            if req
                .info
                .state
                .refreshed
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(resp);
            }
            drop(resp);
            s.provider.invalidate().await;
            let token = s.provider.token().await.map_err(Error::Auth)?;
            req.headers
                .insert("authorization", format!("Bearer {}", token.expose()));
            req.info.attempt = req.info.state.next_attempt();
            next.run(req).await
        })
    }
}

// 8. rate_limit

pub(crate) struct RateLimitMw(pub(crate) Arc<Shared>);

impl Middleware for RateLimitMw {
    fn name(&self) -> &'static str {
        "rate_limit"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        Box::pin(async move {
            let s = &self.0;
            if s.rate_limit == RateLimitMode::Off {
                return next.run(req).await;
            }
            if s.rate_limit == RateLimitMode::Wait
                && let Some(until) = s.latest.hold_until()
            {
                let jitter = Duration::from_millis(getrandom::u64().unwrap_or(0) % 101);
                let until = until + jitter;
                if let Some(d) = req.info.state.deadline()
                    && until >= d
                {
                    return Err(Error::RateLimitWait {
                        host: host(&req),
                        secs: s.total_timeout.as_secs().max(1),
                    });
                }
                s.logger.emit(
                    LogRecord::new(LogLevel::Warn, "rate_limit_wait")
                        .with("attempt", req.info.attempt)
                        .with("reason", "rate_limit")
                        .with(
                            "delay_ms",
                            u64::try_from(
                                until.saturating_duration_since(Instant::now()).as_millis(),
                            )
                            .unwrap_or(u64::MAX),
                        )
                        .with(
                            "request_id",
                            req.info.request_id.clone().unwrap_or_default(),
                        ),
                );
                tokio::time::sleep_until(until.into()).await;
            }
            let mut resp = next.run(req).await?;
            if let Some(snapshot) = RateLimit::from_headers(&resp.headers) {
                s.latest.set(snapshot.clone());
                resp.rate_limit = Some(snapshot);
            }
            Ok(resp)
        })
    }
}

// 9. attempt_tracing

pub(crate) struct AttemptTracingMw(pub(crate) Arc<Shared>);

impl Middleware for AttemptTracingMw {
    fn name(&self) -> &'static str {
        "attempt_tracing"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        #[cfg(feature = "otel")]
        if let Some(otel) = self
            .0
            .otel
            .as_ref()
            .filter(|_| self.0.tracing || self.0.metrics)
        {
            return Box::pin(otel.attempt(req, next, self.0.tracing, self.0.metrics));
        }
        next.run(req)
    }
}

// 10. logging

pub(crate) struct LoggingMw(pub(crate) Arc<Shared>);

impl Middleware for LoggingMw {
    fn name(&self) -> &'static str {
        "logging"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        let log = &self.0.logger;
        if !log.enabled(LogLevel::Debug) {
            return next.run(req);
        }
        Box::pin(async move {
            let base = |event| {
                LogRecord::new(LogLevel::Debug, event)
                    .with("operation", req.info.operation)
                    .with("method", req.method.to_string())
                    .with("path", req.url.path().to_owned())
                    .with("attempt", req.info.attempt)
                    .with(
                        "request_id",
                        req.info.request_id.clone().unwrap_or_default(),
                    )
            };
            let mut rec = base("request");
            if log.headers {
                rec = rec.with("headers", log.headers(&req.headers, false));
            }
            log.emit(rec);
            let started = Instant::now();
            let request = base("response");
            let result = next.run(req).await;
            let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            match &result {
                Ok(resp) => {
                    let mut rec = request
                        .with("status", resp.status)
                        .with("duration_ms", duration)
                        .with(
                            "server_request_id",
                            resp.headers
                                .get("x-request-id")
                                .unwrap_or_default()
                                .to_owned(),
                        );
                    if log.headers {
                        rec = rec.with("headers", log.headers(&resp.headers, true));
                    }
                    log.emit(rec);
                }
                Err(e) => log.emit(
                    request
                        .with("duration_ms", duration)
                        .with("error_kind", error_kind(e)),
                ),
            }
            result
        })
    }
}

// 11. hooks

pub(crate) struct HooksMw(pub(crate) Arc<Shared>);

impl Middleware for HooksMw {
    fn name(&self) -> &'static str {
        "hooks"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        if self.0.hooks.is_empty() {
            return next.run(req);
        }
        Box::pin(async move {
            let hooks = &self.0.hooks;
            let attempt = attempt_of(&req);
            hooks.iter().for_each(|h| h.on_request(&attempt));
            let view = req.clone();
            let result = next.run(req).await;
            if let Ok(resp) = &result {
                let raw = raw_view(&view, resp);
                hooks.iter().for_each(|h| h.on_response(&attempt, &raw));
            }
            result
        })
    }
}

// 12. timeout

pub(crate) struct TimeoutMw(pub(crate) Arc<Shared>);

impl Middleware for TimeoutMw {
    fn name(&self) -> &'static str {
        "timeout"
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        let t = req.info.state.options.timeout.unwrap_or(self.0.timeout);
        let host = host(&req);
        Box::pin(async move {
            match tokio::time::timeout(t, next.run(req)).await {
                Ok(r) => r,
                Err(_) => Err(Error::Timeout {
                    host,
                    secs: t.as_secs().max(1),
                }),
            }
        })
    }
}

/// The built-ins, in order.
pub(crate) fn defaults(shared: &Arc<Shared>) -> Vec<Arc<dyn Middleware>> {
    vec![
        Arc::new(RequestIdMw),
        Arc::new(UserAgentMw(Arc::clone(shared))),
        Arc::new(IdempotencyKeyMw),
        Arc::new(CallTracingMw(Arc::clone(shared))),
        Arc::new(DeadlineMw(Arc::clone(shared))),
        Arc::new(RetryMw(Arc::clone(shared))),
        Arc::new(AuthMw(Arc::clone(shared))),
        Arc::new(RateLimitMw(Arc::clone(shared))),
        Arc::new(AttemptTracingMw(Arc::clone(shared))),
        Arc::new(LoggingMw(Arc::clone(shared))),
        Arc::new(HooksMw(Arc::clone(shared))),
        Arc::new(TimeoutMw(Arc::clone(shared))),
    ]
}

/// The answer a call ends with: the body read, an error status as [`ApiError`].
pub(crate) fn to_raw(resp: Response, state: &super::CallState) -> Result<RawResponse, Error> {
    let status = resp.status;
    let headers: Headers = resp.headers;
    let body = match resp.body {
        super::Body::Bytes(b) => b,
        _ => Vec::new(),
    };
    let mut raw = RawResponse::part(
        status,
        body,
        state.request_id.get().cloned().unwrap_or_default(),
    );
    raw.server_request_id = headers.get("x-request-id").map(str::to_owned);
    raw.idempotency_replayed = headers
        .get("idempotency-replayed")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
    raw.headers = headers;
    raw.attempts = state.attempts().max(1);
    raw.idempotency_key = state.idempotency_key.get().cloned();
    raw.rate_limit = resp.rate_limit;
    if (200..300).contains(&status) {
        Ok(raw)
    } else {
        Err(ApiError::parse(raw).into())
    }
}
