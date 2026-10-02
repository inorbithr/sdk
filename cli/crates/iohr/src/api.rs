//! A small client for the InOrbit API, private to the command line.
//!
//! It follows `docs/design.md` sections 4 to 6: problem errors, retries of idempotent
//! calls only, `Retry-After` honoured up to 60 s, a stated user agent and request ids.
//! When the Rust SDK ships, the command line uses it instead (ADR 0009).

use std::fmt;
use std::io::Write as _;
use std::time::{Duration, Instant};

use iohr_auth::{AuthError, Credential};
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;

/// The API every command talks to unless `IOHR_BASE_URL` says otherwise.
pub const DEFAULT_BASE_URL: &str = "https://api.inorbit.hr";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RETRIES: u32 = 2;
const RETRY_AFTER_CAP: Duration = Duration::from_secs(60);
const BACKOFF_BASE: Duration = Duration::from_millis(500);
const BACKOFF_CAP: Duration = Duration::from_secs(8);
/// The largest answer read into memory (SR-19).
pub const MAX_BODY: usize = 16 * 1024 * 1024;

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    /// The base URL or the path cannot be used.
    #[error("{0}")]
    Config(String),
    /// No credential could be produced.
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// The API could not be reached.
    #[error("cannot reach {host}: {reason}")]
    Connection {
        /// The host that was called.
        host: String,
        /// What went wrong, from the HTTP stack.
        reason: String,
    },
    /// No answer within the deadline.
    #[error("{host} did not answer within {secs} s")]
    Timeout {
        /// The host that was called.
        host: String,
        /// The deadline in seconds.
        secs: u64,
    },
    /// The answer was larger than [`MAX_BODY`].
    #[error("the answer is larger than {} MiB; refusing to read it", MAX_BODY / 1024 / 1024)]
    TooLarge,
    /// The API answered with an error.
    #[error("{problem}")]
    Status {
        /// The error the API returned.
        problem: Problem,
    },
    /// The answer was not the JSON the command expected.
    #[error("the API answered something this version of iohr cannot read: {0}")]
    Decode(String),
}

impl ApiError {
    /// The HTTP status, when the API answered.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { problem } => Some(problem.status),
            _ => None,
        }
    }
}

/// An error answer from the API, in the platform's problem shape
/// (`{"code": ..., "error": ..., "details": [...]}`), or mapped from a gateway's
/// plain-text answer by its status.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Problem {
    /// The HTTP status.
    pub status: u16,
    /// The stable error code, such as `permission_denied`.
    pub code: String,
    /// The message the API gave.
    pub message: String,
    /// The request id the gateway assigned, for support.
    pub request_id: Option<String>,
}

impl Problem {
    /// Reads an error answer. Never fails: an unreadable body is described by its
    /// status alone.
    #[must_use]
    pub fn parse(status: u16, body: &[u8], request_id: Option<String>) -> Self {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            code: String,
            #[serde(default)]
            error: String,
        }
        let (code, message) = match serde_json::from_slice::<Wire>(body) {
            Ok(w) if !w.code.is_empty() || !w.error.is_empty() => (w.code, w.error),
            _ => (String::new(), String::new()),
        };
        let code = if code.is_empty() {
            by_status(status).to_owned()
        } else {
            code
        };
        let message = if message.is_empty() {
            gateway_message(status).to_owned()
        } else {
            truncate(&message, 300)
        };
        Self {
            status,
            code,
            message,
            request_id,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}, HTTP {})", self.message, self.code, self.status)?;
        if let Some(id) = &self.request_id {
            write!(f, ", request id {id}")?;
        }
        Ok(())
    }
}

fn by_status(status: u16) -> &'static str {
    match status {
        400 => "invalid_argument",
        401 => "unauthenticated",
        403 => "permission_denied",
        404 => "not_found",
        409 => "conflict",
        429 => "rate_limited",
        503 => "unavailable",
        504 => "deadline_exceeded",
        s if s >= 500 => "internal",
        _ => "unknown",
    }
}

fn gateway_message(status: u16) -> &'static str {
    match status {
        401 => "the token was refused: it is missing, expired or revoked",
        403 => "this credential may not call this route: its scopes or role do not allow it",
        404 => "no such route or resource",
        429 => "too many requests; try again shortly",
        s if s >= 500 => "the API failed to answer",
        _ => "the request was refused",
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// A successful answer.
#[derive(Debug)]
#[non_exhaustive]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The `content-type`, if any.
    pub content_type: Option<String>,
    /// The gateway's request id, if any.
    pub request_id: Option<String>,
    /// The body, at most [`MAX_BODY`] bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// The body as `T`.
    ///
    /// # Errors
    ///
    /// [`ApiError::Decode`] when the body is not that JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, ApiError> {
        serde_json::from_slice(&self.body).map_err(|e| ApiError::Decode(e.to_string()))
    }
}

/// Checks a base URL: HTTPS, or plain HTTP to this machine only (SR-07).
///
/// # Errors
///
/// [`ApiError::Config`] for any other URL.
pub fn base_url(raw: &str) -> Result<Url, ApiError> {
    let url = Url::parse(raw)
        .map_err(|e| ApiError::Config(format!("IOHR_BASE_URL is not a URL: {e}")))?;
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
            return Err(ApiError::Config(
                "the API URL must use https (plain http only to this machine)".into(),
            ));
        }
    }
    if url.path() != "/"
        || url.query().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ApiError::Config(
            "the API URL is an origin only, such as https://api.inorbit.hr".into(),
        ));
    }
    Ok(url)
}

/// The client.
pub struct Api<C> {
    http: reqwest::Client,
    base: Url,
    credential: C,
    verbose: bool,
}

impl<C> fmt::Debug for Api<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl<C: Credential> Api<C> {
    /// A client for `base` that authorises every call with `credential`. With
    /// `verbose`, each attempt is described on stderr: method, path, status, time and
    /// request id, never a header, a query value or a body (SR-13).
    ///
    /// # Errors
    ///
    /// [`ApiError::Config`] when the HTTP stack cannot start.
    pub fn new(base: Url, credential: C, verbose: bool) -> Result<Self, ApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .user_agent(user_agent())
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(ATTEMPT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(base.scheme() == "https")
            .build()
            .map_err(|e| ApiError::Config(format!("cannot start the HTTP client: {e}")))?;
        Ok(Self {
            http,
            base,
            credential,
            verbose,
        })
    }

    /// `GET path` and read the JSON answer.
    ///
    /// # Errors
    ///
    /// See [`send`](Self::send); also [`ApiError::Decode`].
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ApiError> {
        self.send(Method::GET, path, query, None).await?.json()
    }

    /// One call. Idempotent methods are retried up to 2 times after a connection
    /// failure, a timeout, or a 429, 503 or 504; other methods are sent once (SR-18).
    ///
    /// # Errors
    ///
    /// [`ApiError::Status`] for an error answer, and the other variants when the call
    /// cannot be made or read.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&serde_json::Value>,
    ) -> Result<Response, ApiError> {
        let url = self.url(path)?;
        let host = self.base.host_str().unwrap_or_default().to_owned();
        let idempotent = matches!(
            method,
            Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS
        );
        let request_id = request_id();
        let mut attempt = 0;
        loop {
            let bearer = self.credential.bearer().await?;
            let mut req = self
                .http
                .request(method.clone(), url.clone())
                .query(query)
                .bearer_auth(bearer.expose())
                .header("x-request-id", &request_id);
            if let Some(b) = body {
                req = req.json(b);
            }
            let started = Instant::now();
            let outcome = req.send().await;
            let retry = match &outcome {
                Err(e) => (e.is_connect() || e.is_timeout()).then_some(None),
                Ok(r) => {
                    matches!(r.status().as_u16(), 429 | 503 | 504).then(|| retry_after(r.headers()))
                }
            };
            if self.verbose {
                let status = outcome.as_ref().map_or_else(
                    |_| "no answer".to_owned(),
                    |r| r.status().as_u16().to_string(),
                );
                note(&format!(
                    "{method} {} -> {status} in {} ms (request id {request_id}, attempt {})",
                    url.path(),
                    started.elapsed().as_millis(),
                    attempt + 1
                ));
            }
            if let Some(after) = retry
                && idempotent
                && attempt < MAX_RETRIES
            {
                tokio::time::sleep(after.unwrap_or_else(|| backoff(attempt))).await;
                attempt += 1;
                continue;
            }
            let resp = outcome.map_err(|e| {
                if e.is_timeout() {
                    ApiError::Timeout {
                        host: host.clone(),
                        secs: ATTEMPT_TIMEOUT.as_secs(),
                    }
                } else {
                    ApiError::Connection {
                        host: host.clone(),
                        reason: reason(&e),
                    }
                }
            })?;
            return read(resp).await;
        }
    }

    fn url(&self, path: &str) -> Result<Url, ApiError> {
        let path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        if path.starts_with("//")
            || path.contains("://")
            || path.contains('?')
            || path.contains('#')
        {
            return Err(ApiError::Config(
                "give a path on the API, such as /v1/me; put query values in -f key=value".into(),
            ));
        }
        let url = self
            .base
            .join(&path)
            .map_err(|e| ApiError::Config(format!("not a valid path: {e}")))?;
        if url.origin() != self.base.origin() {
            return Err(ApiError::Config(
                "the path must stay on the API host".into(),
            ));
        }
        Ok(url)
    }
}

async fn read(mut resp: reqwest::Response) -> Result<Response, ApiError> {
    let status = resp.status();
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let request_id = header("x-request-id");
    let content_type = header("content-type");
    if resp.content_length().is_some_and(|n| n > MAX_BODY as u64) {
        return Err(ApiError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| ApiError::Connection {
        host: String::new(),
        reason: reason(&e),
    })? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(ApiError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    if status.is_success() {
        Ok(Response {
            status: status.as_u16(),
            content_type,
            request_id,
            body,
        })
    } else {
        Err(ApiError::Status {
            problem: Problem::parse(status.as_u16(), &body, request_id),
        })
    }
}

/// The error's own message and its causes, without the URL (which may carry query
/// values, SR-13).
fn reason(e: &reqwest::Error) -> String {
    let mut parts = Vec::new();
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = source {
        parts.push(s.to_string());
        source = s.source();
    }
    if parts.is_empty() {
        "the connection failed".to_owned()
    } else {
        parts.join(": ")
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs).min(RETRY_AFTER_CAP))
}

/// Full jitter: a random wait between 0 and min(8 s, 0.5 s × 2^attempt).
fn backoff(attempt: u32) -> Duration {
    let ceiling = BACKOFF_BASE
        .saturating_mul(1 << attempt.min(5))
        .min(BACKOFF_CAP);
    let r = getrandom::u64().unwrap_or(0);
    let millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(if millis == 0 { 0 } else { r % (millis + 1) })
}

fn request_id() -> String {
    let n = getrandom::u64().unwrap_or(0);
    format!("iohr-{n:016x}")
}

fn user_agent() -> String {
    format!(
        "iohr/{} ({}; {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// One line on stderr, for `--verbose`.
pub(crate) fn note(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "iohr: {line}");
}

/// Whether a status means the caller is not signed in.
#[must_use]
pub fn is_unauthenticated(status: Option<u16>) -> bool {
    status == Some(StatusCode::UNAUTHORIZED.as_u16())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Problem, backoff, base_url, truncate};

    #[test]
    fn reads_the_problem_shape() {
        let p = Problem::parse(
            403,
            br#"{"code":"permission_denied","error":"a key reads its account"}"#,
            Some("r1".into()),
        );
        assert_eq!(p.code, "permission_denied");
        assert_eq!(
            p.to_string(),
            "a key reads its account (permission_denied, HTTP 403), request id r1"
        );
    }

    #[test]
    fn maps_a_gateway_plain_text_answer_by_status() {
        for (status, code) in [
            (401, "unauthenticated"),
            (403, "permission_denied"),
            (429, "rate_limited"),
            (502, "internal"),
        ] {
            let p = Problem::parse(status, b"RBAC: access denied", None);
            assert_eq!(p.code, code);
            assert!(!p.message.is_empty());
        }
    }

    #[test]
    fn a_long_message_is_cut() {
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("ab", 3), "ab");
    }

    #[test]
    fn base_urls() {
        assert!(base_url("https://api.inorbit.hr").is_ok());
        assert!(base_url("http://127.0.0.1:8080").is_ok());
        assert!(base_url("http://[::1]:8080").is_ok());
        assert!(base_url("http://localhost:8080").is_ok());
        for bad in [
            "http://api.inorbit.hr",
            "ftp://x",
            "https://api.inorbit.hr/v1",
            "https://u:p@api.inorbit.hr",
            "nope",
        ] {
            assert!(base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn backoff_stays_under_its_ceiling() {
        for attempt in 0..10 {
            assert!(backoff(attempt) <= Duration::from_secs(8));
        }
        assert!(backoff(0) <= Duration::from_millis(500));
    }
}
