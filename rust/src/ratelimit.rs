//! Rate-limit headers (`docs/config.md` section 7.8): the edge's `X-RateLimit-*`
//! (Envoy's draft 03) and the IETF `RateLimit` and `RateLimit-Policy` fields (draft
//! 11), which win when both are sent.

use std::str::FromStr;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::Headers;

/// What the `rate_limit` middleware does with the headers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RateLimitMode {
    /// Read them into a snapshot on every answer; nothing waits (the default).
    #[default]
    Observe,
    /// As `Observe`, and hold an attempt until the window resets when the latest
    /// snapshot says nothing remains.
    Wait,
    /// Do not read them.
    Off,
}

impl FromStr for RateLimitMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "observe" => Ok(Self::Observe),
            "wait" => Ok(Self::Wait),
            "off" => Ok(Self::Off),
            _ => Err(()),
        }
    }
}

/// A rate-limit snapshot from one answer's headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RateLimit {
    /// Requests allowed in the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// Requests left in the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u64>,
    /// Time until the window resets.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "reset_ms",
        with = "ms"
    )]
    pub reset: Option<Duration>,
    /// The policy, when the IETF `RateLimit-Policy` field was sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<RateLimitPolicy>,
}

/// A quota policy (`RateLimit-Policy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RateLimitPolicy {
    /// The policy's name.
    pub name: String,
    /// Requests allowed per window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<u64>,
    /// The window.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "window_ms",
        with = "ms"
    )]
    pub window: Option<Duration>,
}

mod ms {
    use std::time::Duration;

    #[allow(clippy::ref_option, reason = "the serde `with` signature")]
    pub(super) fn serialize<S: serde::Serializer>(
        d: &Option<Duration>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            None => s.serialize_none(),
        }
    }
}

impl RateLimit {
    /// The snapshot `headers` describe, or `None` when they carry none or a value does
    /// not parse (a malformed header is ignored, never an error).
    #[must_use]
    pub fn from_headers(headers: &Headers) -> Option<Self> {
        if let Some(field) = headers.get("ratelimit") {
            return ietf(field, headers.get("ratelimit-policy"));
        }
        let num = |k: &str| -> Option<Option<u64>> {
            match headers.get(k) {
                None => Some(None),
                Some(v) => v.trim().parse::<u64>().ok().map(Some),
            }
        };
        let limit = num("x-ratelimit-limit")?;
        let remaining = num("x-ratelimit-remaining")?;
        let reset = num("x-ratelimit-reset")?;
        if limit.is_none() && remaining.is_none() && reset.is_none() {
            return None;
        }
        Some(Self {
            limit,
            remaining,
            reset: reset.map(Duration::from_secs),
            policy: None,
        })
    }
}

/// One structured-field list item: its name (a string or token) and its parameters.
fn item(field: &str) -> Option<(String, Vec<(String, String)>)> {
    let first = field.split(',').next()?.trim();
    let mut parts = first.split(';');
    let name = parts.next()?.trim();
    let name = name
        .strip_prefix('"')
        .and_then(|n| n.strip_suffix('"'))
        .unwrap_or(name)
        .to_owned();
    let mut params = Vec::new();
    for p in parts {
        let (k, v) = p.trim().split_once('=')?;
        params.push((k.trim().to_owned(), v.trim().to_owned()));
    }
    Some((name, params))
}

fn ietf(field: &str, policy: Option<&str>) -> Option<RateLimit> {
    let (_, params) = item(field)?;
    let get = |ps: &[(String, String)], k: &str| -> Option<Option<u64>> {
        match ps.iter().find(|(n, _)| n == k) {
            None => Some(None),
            Some((_, v)) => v.parse::<u64>().ok().map(Some),
        }
    };
    let remaining = get(&params, "r")?;
    let reset = get(&params, "t")?;
    let policy = match policy {
        None => None,
        Some(p) => {
            let (name, ps) = item(p)?;
            Some(RateLimitPolicy {
                name,
                quota: get(&ps, "q")?,
                window: get(&ps, "w")?.map(Duration::from_secs),
            })
        }
    };
    Some(RateLimit {
        limit: policy.as_ref().and_then(|p| p.quota),
        remaining,
        reset: reset.map(Duration::from_secs),
        policy,
    })
}

/// The client's latest snapshot and when it was taken, shared by its calls.
#[derive(Debug, Default)]
pub(crate) struct Latest(std::sync::Mutex<Option<(RateLimit, Instant)>>);

impl Latest {
    pub(crate) fn set(&self, snapshot: RateLimit) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((snapshot, Instant::now()));
    }

    pub(crate) fn get(&self) -> Option<RateLimit> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|(s, _)| s.clone())
    }

    /// When nothing remains and the window has not reset: the instant it resets.
    pub(crate) fn hold_until(&self) -> Option<Instant> {
        let guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (s, at) = guard.as_ref()?;
        if s.remaining != Some(0) {
            return None;
        }
        let until = *at + s.reset?;
        (until > Instant::now()).then_some(until)
    }
}
