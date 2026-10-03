//! One error family, rooted in [`Error`] (`docs/design.md` section 5).

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The largest answer read into memory, 16 MiB (SR-19).
pub const MAX_BODY: usize = 16 * 1024 * 1024;

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The API answered with the problem envelope (or a plain-text refusal).
    #[error(transparent)]
    Api(Box<ApiError>),
    /// The API could not be reached: DNS, TCP, TLS, or a reset before an answer.
    #[error("cannot reach {host}: {reason}")]
    Connection {
        /// The host that was called.
        host: String,
        /// What went wrong, from the HTTP stack, without the URL.
        reason: String,
    },
    /// No answer within the per-attempt timeout.
    #[error("{host} did not answer within {secs} s")]
    Timeout {
        /// The host that was called.
        host: String,
        /// The timeout in seconds.
        secs: u64,
    },
    /// The token exchange failed.
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// The client was built with options that cannot work.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The answer was larger than [`MAX_BODY`].
    #[error("the answer is larger than {} MiB; refusing to read it", MAX_BODY / 1024 / 1024)]
    TooLarge,
    /// The answer was not the JSON this version of the SDK expects.
    #[error("the API answered something this version of inorbithr cannot read: {reason}")]
    Decode {
        /// What did not parse.
        reason: String,
        /// The answer, for the raw path.
        raw: Box<RawResponse>,
    },
}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Self::Api(Box::new(e))
    }
}

impl Error {
    /// The HTTP status, when the API answered.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api(e) => Some(e.status),
            Self::Decode { raw, .. } => Some(raw.status),
            _ => None,
        }
    }

    /// Whether another attempt could succeed: a connection failure, a timeout, or a
    /// `429`, `503` or `504`.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Connection { .. } | Self::Timeout { .. } => true,
            Self::Api(e) => matches!(e.status, 429 | 503 | 504),
            _ => false,
        }
    }
}

/// The answer as it came: status, headers, body, and the request ids both ways
/// (SR-17).
#[derive(Clone)]
#[non_exhaustive]
pub struct RawResponse {
    /// The HTTP status.
    pub status: u16,
    /// The response headers.
    pub headers: Headers,
    /// The body, at most [`MAX_BODY`] bytes.
    pub body: Vec<u8>,
    /// The request id the SDK sent (`x-request-id`).
    pub request_id: String,
    /// The request id the API returned, when it did.
    pub server_request_id: Option<String>,
    /// How many attempts the call took.
    pub attempts: u32,
}

/// `Debug` shows the body's size, not the body: an answer may hold data a log must not
/// (SR-13).
impl fmt::Debug for RawResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .field("request_id", &self.request_id)
            .field("server_request_id", &self.server_request_id)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

impl RawResponse {
    /// The body as `T`.
    ///
    /// # Errors
    ///
    /// [`Error::Decode`] when the body is not that JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_slice(&self.body).map_err(|e| Error::Decode {
            reason: e.to_string(),
            raw: Box::new(self.clone()),
        })
    }

    /// The body as text, when it is UTF-8.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }
}

/// Response headers, with case-insensitive lookup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    /// Builds a header list; names are kept in lower case.
    pub fn new(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect(),
        )
    }

    /// The first value of `name`, whatever its case.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every header, in the order received.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// An error answer from the API.
///
/// The platform's envelope is `{"code": ..., "error": ..., "details": [...]}`; a
/// gateway refusal in plain text is mapped by its status and keeps the text as the
/// message.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ApiError {
    /// The HTTP status.
    pub status: u16,
    /// The stable error code.
    pub code: Code,
    /// The message the API gave, written for the reader.
    pub message: String,
    /// Typed details; unknown types are kept raw.
    pub details: Vec<Detail>,
    /// The answer as it came.
    pub raw: RawResponse,
}

impl ApiError {
    /// Reads an error answer. Never fails: an unreadable body is described by its
    /// status alone, and a plain-text body becomes the message.
    #[must_use]
    pub fn parse(raw: RawResponse) -> Self {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            code: String,
            #[serde(default)]
            error: String,
            #[serde(default)]
            details: Vec<Detail>,
        }
        let wire = serde_json::from_slice::<Wire>(&raw.body)
            .ok()
            .filter(|w| !w.code.is_empty() || !w.error.is_empty());
        let (code, message, details) = match wire {
            Some(w) => (
                if w.code.is_empty() {
                    Code::for_status(raw.status)
                } else {
                    Code::from(w.code.as_str())
                },
                w.error,
                w.details,
            ),
            None => (
                Code::for_status(raw.status),
                raw.text().map(str::trim).unwrap_or_default().to_owned(),
                Vec::new(),
            ),
        };
        let message = if message.is_empty() {
            gateway_message(raw.status).to_owned()
        } else {
            truncate(&message, 300)
        };
        Self {
            status: raw.status,
            code,
            message,
            details,
            raw,
        }
    }

    /// The server's `retry` detail or `Retry-After` header, in seconds, when present.
    #[must_use]
    pub fn retry_after_seconds(&self) -> Option<u64> {
        self.details
            .iter()
            .find_map(|d| match d {
                Detail::Retry { after_seconds } => Some(*after_seconds),
                _ => None,
            })
            .or_else(|| self.raw.headers.get("retry-after")?.trim().parse().ok())
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}, HTTP {})", self.message, self.code, self.status)?;
        if let Some(id) = &self.raw.server_request_id {
            write!(f, ", request id {id}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// The platform's error codes (`spec/problem.json`), with a fallback that keeps a
/// code this version does not know.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Code {
    /// 400: the request is malformed.
    BadRequest,
    /// 400: the request is well formed but the state does not allow it.
    FailedPrecondition,
    /// 401: the token is missing, expired or revoked.
    Unauthenticated,
    /// 403: the credential may not do this.
    Forbidden,
    /// 404: no such route or resource.
    NotFound,
    /// 405: the method is not allowed on this route.
    MethodNotAllowed,
    /// 409: it already exists.
    AlreadyExists,
    /// 409: a conflicting change happened first.
    Conflict,
    /// 413: the body is too large.
    PayloadTooLarge,
    /// 415: the media type is not accepted.
    UnsupportedMediaType,
    /// 429: too many requests.
    RateLimited,
    /// 429: the account's allowance is used up.
    QuotaExceeded,
    /// 499: the caller went away.
    Cancelled,
    /// 500: the platform failed.
    Internal,
    /// 501: not implemented.
    Unimplemented,
    /// 503: a backend is unavailable.
    Unavailable,
    /// 504: a backend did not answer in time.
    Timeout,
    /// A code this version of the SDK does not know; the raw slug is kept.
    Unknown(String),
}

impl Code {
    const KNOWN: [(Code, &'static str, u16); 17] = [
        (Code::BadRequest, "bad_request", 400),
        (Code::FailedPrecondition, "failed_precondition", 400),
        (Code::Unauthenticated, "unauthenticated", 401),
        (Code::Forbidden, "forbidden", 403),
        (Code::NotFound, "not_found", 404),
        (Code::MethodNotAllowed, "method_not_allowed", 405),
        (Code::AlreadyExists, "already_exists", 409),
        (Code::Conflict, "conflict", 409),
        (Code::PayloadTooLarge, "payload_too_large", 413),
        (Code::UnsupportedMediaType, "unsupported_media_type", 415),
        (Code::RateLimited, "rate_limited", 429),
        (Code::QuotaExceeded, "quota_exceeded", 429),
        (Code::Cancelled, "cancelled", 499),
        (Code::Internal, "internal", 500),
        (Code::Unimplemented, "unimplemented", 501),
        (Code::Unavailable, "unavailable", 503),
        (Code::Timeout, "timeout", 504),
    ];

    /// The code's slug, as the API writes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(s) => s,
            known => Self::KNOWN
                .iter()
                .find(|(c, _, _)| c == known)
                .map_or("unknown", |(_, s, _)| s),
        }
    }

    /// The HTTP status this code is answered with, for a known code.
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        Self::KNOWN
            .iter()
            .find(|(c, _, _)| c == self)
            .map(|(_, _, s)| *s)
    }

    /// The code a plain-text answer with `status` stands for.
    #[must_use]
    pub fn for_status(status: u16) -> Self {
        match status {
            400 => Self::BadRequest,
            401 => Self::Unauthenticated,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            405 => Self::MethodNotAllowed,
            409 => Self::Conflict,
            413 => Self::PayloadTooLarge,
            415 => Self::UnsupportedMediaType,
            429 => Self::RateLimited,
            499 => Self::Cancelled,
            501 => Self::Unimplemented,
            503 => Self::Unavailable,
            504 => Self::Timeout,
            s if s >= 500 => Self::Internal,
            _ => Self::Unknown(format!("http_{status}")),
        }
    }
}

impl From<&str> for Code {
    fn from(slug: &str) -> Self {
        Self::KNOWN
            .iter()
            .find(|(_, s, _)| *s == slug)
            .map_or_else(|| Self::Unknown(slug.to_owned()), |(c, _, _)| c.clone())
    }
}

impl FromStr for Code {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s))
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|s| Self::from(s.as_str()))
    }
}

/// One entry of an error's `details`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Detail {
    /// A field of the request is wrong.
    Field {
        /// The field, in wire names (`org_id`).
        field: String,
        /// What is wrong with it.
        description: String,
    },
    /// More about the failure.
    Info {
        /// A short reason, stable.
        reason: String,
        /// Who defines the reason.
        domain: String,
        /// Extra key-value context.
        #[serde(default)]
        metadata: BTreeMap<String, String>,
    },
    /// When to try again.
    Retry {
        /// Seconds to wait.
        after_seconds: u64,
    },
    /// A detail type this version does not know, kept as it came.
    #[serde(untagged)]
    Unknown(serde_json::Value),
}

impl<'de> Deserialize<'de> for Detail {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum Known {
            Field {
                #[serde(default)]
                field: String,
                #[serde(default)]
                description: String,
            },
            Info {
                #[serde(default)]
                reason: String,
                #[serde(default)]
                domain: String,
                #[serde(default)]
                metadata: BTreeMap<String, String>,
            },
            Retry {
                #[serde(default)]
                after_seconds: u64,
            },
        }
        let value = serde_json::Value::deserialize(d)?;
        Ok(match serde_json::from_value::<Known>(value.clone()) {
            Ok(Known::Field { field, description }) => Self::Field { field, description },
            Ok(Known::Info {
                reason,
                domain,
                metadata,
            }) => Self::Info {
                reason,
                domain,
                metadata,
            },
            Ok(Known::Retry { after_seconds }) => Self::Retry { after_seconds },
            Err(_) => Self::Unknown(value),
        })
    }
}

/// Why no token could be produced.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// The token endpoint refused the key. The message names the key id, never the
    /// secret.
    #[error("the token exchange for key {key_id} failed: {error}{}", description_suffix(.description))]
    Exchange {
        /// The key that was refused.
        key_id: String,
        /// The OAuth error code (`invalid_client`, `invalid_scope`).
        error: String,
        /// The description the token endpoint gave, if any.
        description: String,
        /// The HTTP status of the refusal.
        status: u16,
    },
    /// The token endpoint could not be reached, or did not answer in time.
    #[error("the token endpoint: {0}")]
    Transport(String),
    /// A caller's token provider failed.
    #[error("the token provider failed: {0}")]
    Provider(String),
}

fn description_suffix(description: &str) -> String {
    if description.is_empty() {
        String::new()
    } else {
        format!(" ({description})")
    }
}

/// Why a client could not be built.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A required option is missing; the message names the environment variables.
    #[error("no {what}: set {env}")]
    Missing {
        /// What is missing (`credentials`, `scopes`).
        what: &'static str,
        /// The environment variables that would supply it.
        env: String,
    },
    /// A URL cannot be used.
    #[error("{what} is not usable: {reason}")]
    InvalidUrl {
        /// Which URL (`base_url`, `token_url`).
        what: &'static str,
        /// What is wrong with it.
        reason: String,
    },
    /// A request body could not be serialised.
    #[error("the request body cannot be serialised: {reason}")]
    Body {
        /// What went wrong.
        reason: String,
    },
    /// The HTTP stack could not start.
    #[error("cannot start the HTTP client: {0}")]
    Http(String),
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

#[cfg(test)]
mod tests {
    use super::{ApiError, Code, Detail, Headers, RawResponse};

    fn raw(status: u16, body: &str) -> RawResponse {
        RawResponse {
            status,
            headers: Headers::new([("Content-Type".to_owned(), "text/plain".to_owned())]),
            body: body.as_bytes().to_vec(),
            request_id: "iohr-1".into(),
            server_request_id: Some("r1".into()),
            attempts: 1,
        }
    }

    #[test]
    fn every_known_code_round_trips_and_unknown_is_kept() {
        for (code, slug, status) in &Code::KNOWN {
            assert_eq!(Code::from(*slug), *code);
            assert_eq!(code.as_str(), *slug);
            assert_eq!(code.http_status(), Some(*status));
            let json = serde_json::to_string(code).unwrap();
            assert_eq!(serde_json::from_str::<Code>(&json).unwrap(), *code);
        }
        let new = Code::from("brand_new_code");
        assert_eq!(new, Code::Unknown("brand_new_code".into()));
        assert_eq!(new.as_str(), "brand_new_code");
        assert_eq!(new.http_status(), None);
    }

    #[test]
    fn reads_the_envelope_with_typed_and_unknown_details() {
        let e = ApiError::parse(raw(
            429,
            r#"{"code":"rate_limited","error":"Too many requests.","details":[{"type":"retry","after_seconds":3},{"type":"future_detail","x":1}]}"#,
        ));
        assert_eq!(e.code, Code::RateLimited);
        assert_eq!(e.message, "Too many requests.");
        assert_eq!(e.details[0], Detail::Retry { after_seconds: 3 });
        assert!(matches!(e.details[1], Detail::Unknown(_)));
        assert_eq!(e.retry_after_seconds(), Some(3));
        assert_eq!(
            e.to_string(),
            "Too many requests. (rate_limited, HTTP 429), request id r1"
        );
    }

    #[test]
    fn maps_a_plain_text_refusal_by_status_and_keeps_the_text() {
        let e = ApiError::parse(raw(403, "RBAC: access denied"));
        assert_eq!(e.code, Code::Forbidden);
        assert_eq!(e.message, "RBAC: access denied");
        let e = ApiError::parse(raw(502, ""));
        assert_eq!(e.code, Code::Internal);
        assert_eq!(e.message, "the API failed to answer");
    }

    #[test]
    fn debug_shows_the_body_size_not_the_body() {
        let shown = format!("{:?}", raw(200, "{\"email\":\"someone@example.com\"}"));
        assert!(
            shown.contains("body_bytes: 31") && !shown.contains("example.com"),
            "{shown}"
        );
    }

    #[test]
    fn headers_look_up_without_case() {
        let h = Headers::new([("Retry-After".to_owned(), "7".to_owned())]);
        assert_eq!(h.get("retry-after"), Some("7"));
        assert_eq!(h.get("RETRY-AFTER"), Some("7"));
        assert_eq!(h.get("x"), None);
    }
}
