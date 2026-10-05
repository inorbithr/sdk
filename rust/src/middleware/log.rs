//! Log records (`docs/config.md` section 7.9): off by default, metadata only, headers
//! from an allowlist, and a `redact` step last before the sink.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::error::Headers;

/// How much the client logs. Ordered: `Off` < `Error` < `Warn` < `Info` < `Debug`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum LogLevel {
    /// Nothing (the default).
    #[default]
    Off,
    /// Calls that failed.
    Error,
    /// Retries, failed token refreshes, rate-limit waits.
    Warn,
    /// One record per call.
    Info,
    /// Every attempt's request and response.
    Debug,
}

impl LogLevel {
    /// `off`, `error`, `warn`, `info` or `debug`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

impl FromStr for LogLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "off" => Ok(Self::Off),
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => Err(()),
        }
    }
}

/// One log record: its level, its event (`request`, `response`, `call`, `retry`,
/// `token_refresh_failed`, `rate_limit_wait`, `call_failed`) and its fields, with the
/// names every language uses. Never a body, a query value or a secret.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LogRecord {
    /// The level.
    pub level: LogLevel,
    /// The event.
    pub event: &'static str,
    /// The fields, `event` included.
    pub fields: Map<String, Value>,
}

impl LogRecord {
    pub(crate) fn new(level: LogLevel, event: &'static str) -> Self {
        let mut fields = Map::new();
        fields.insert("event".into(), Value::from(event));
        Self {
            level,
            event,
            fields,
        }
    }

    pub(crate) fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.to_owned(), value.into());
        self
    }

    /// A field as text, for sinks that want one.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<String> {
        self.fields.get(key).map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }
}

/// Where records go: a callback, or `tracing` events with target `inorbithr`.
pub(crate) type Sink = Arc<dyn Fn(&LogRecord) + Send + Sync>;
/// The last step before the sink: change a record, or drop it.
pub(crate) type Redact = Arc<dyn Fn(LogRecord) -> Option<LogRecord> + Send + Sync>;

/// Headers never logged, whatever the settings.
const NEVER: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];
const REQUEST_ALLOW: [&str; 7] = [
    "accept",
    "content-type",
    "content-length",
    "user-agent",
    "x-request-id",
    "traceparent",
    "idempotency-key",
];
const RESPONSE_ALLOW: [&str; 11] = [
    "content-type",
    "content-length",
    "date",
    "retry-after",
    "x-request-id",
    "idempotency-replayed",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "ratelimit",
    "ratelimit-policy",
];

/// The client's logger.
#[derive(Clone, Default)]
pub(crate) struct Logger {
    pub(crate) level: LogLevel,
    pub(crate) headers: bool,
    pub(crate) allow: Vec<String>,
    pub(crate) sink: Option<Sink>,
    pub(crate) redact: Option<Redact>,
    pub(crate) profile: String,
}

impl fmt::Debug for Logger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Logger")
            .field("level", &self.level)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

impl Logger {
    pub(crate) fn enabled(&self, level: LogLevel) -> bool {
        level != LogLevel::Off && level <= self.level
    }

    /// Request or response headers as a record shows them: allowlisted values, other
    /// names with `REDACTED`, the never-log set left out.
    pub(crate) fn headers(&self, headers: &Headers, response: bool) -> Value {
        let allow: &[&str] = if response {
            &RESPONSE_ALLOW
        } else {
            &REQUEST_ALLOW
        };
        let mut out = Map::new();
        for (k, v) in headers.iter() {
            let k = k.to_ascii_lowercase();
            if NEVER.contains(&k.as_str()) {
                continue;
            }
            let shown = allow.contains(&k.as_str())
                || self.allow.iter().any(|a| a.eq_ignore_ascii_case(&k));
            out.insert(k, Value::from(if shown { v } else { "REDACTED" }));
        }
        Value::Object(out)
    }

    /// Sends `record` on, if its level is on: the profile added, `redact` applied.
    pub(crate) fn emit(&self, record: LogRecord) {
        if !self.enabled(record.level) {
            return;
        }
        let record = record.with("profile", self.profile.clone());
        let Some(record) = (match &self.redact {
            Some(r) => r(record),
            None => Some(record),
        }) else {
            return;
        };
        match &self.sink {
            Some(sink) => sink(&record),
            None => to_tracing(&record),
        }
    }
}

#[cfg(feature = "tracing")]
fn to_tracing(r: &LogRecord) {
    let f = |k: &str| r.field(k).unwrap_or_default();
    macro_rules! event {
        ($level:expr) => {
            tracing::event!(
                target: "inorbithr",
                $level,
                event = r.event,
                operation = %f("operation"),
                method = %f("method"),
                path = %f("path"),
                attempt = %f("attempt"),
                status = %f("status"),
                request_id = %f("request_id"),
                server_request_id = %f("server_request_id"),
                duration_ms = %f("duration_ms"),
                reason = %f("reason"),
                delay_ms = %f("delay_ms"),
                error_kind = %f("error_kind"),
                error_code = %f("error_code"),
                headers = %f("headers"),
                profile = %f("profile"),
            )
        };
    }
    match r.level {
        LogLevel::Error => event!(tracing::Level::ERROR),
        LogLevel::Warn => event!(tracing::Level::WARN),
        LogLevel::Info => event!(tracing::Level::INFO),
        LogLevel::Debug => event!(tracing::Level::DEBUG),
        LogLevel::Off => {}
    }
}

#[cfg(not(feature = "tracing"))]
fn to_tracing(_: &LogRecord) {}
