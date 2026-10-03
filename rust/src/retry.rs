//! The retry policy (`docs/design.md` section 6): idempotent calls only, after a
//! connection failure, a timeout, or a `429`, `503` or `504`; the server's
//! `Retry-After` up to 60 s, otherwise full jitter from 0.5 s up to 8 s.

use std::time::Duration;

use crate::error::Headers;

/// The longest wait a server may ask for.
pub(crate) const RETRY_AFTER_CAP: Duration = Duration::from_secs(60);
const BACKOFF_BASE: Duration = Duration::from_millis(500);
const BACKOFF_CAP: Duration = Duration::from_secs(8);

/// Whether a status is retried.
pub(crate) fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 503 | 504)
}

/// The server's `Retry-After` in seconds, capped.
pub(crate) fn retry_after(headers: &Headers) -> Option<Duration> {
    let secs: u64 = headers.get("retry-after")?.trim().parse().ok()?;
    Some(Duration::from_secs(secs).min(RETRY_AFTER_CAP))
}

/// Full jitter: a random wait between 0 and min(8 s, 0.5 s × 2^attempt).
pub(crate) fn backoff(attempt: u32) -> Duration {
    let ceiling = BACKOFF_BASE
        .saturating_mul(1 << attempt.min(5))
        .min(BACKOFF_CAP);
    let r = getrandom::u64().unwrap_or(0);
    let millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(if millis == 0 { 0 } else { r % (millis + 1) })
}

/// A fresh client request id, `iohr-` and 16 hex digits.
pub(crate) fn request_id() -> String {
    let n = getrandom::u64().unwrap_or(0);
    format!("iohr-{n:016x}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{backoff, retry_after, retryable_status};
    use crate::error::Headers;

    #[test]
    fn backoff_stays_under_its_ceiling() {
        for attempt in 0..10 {
            assert!(backoff(attempt) <= Duration::from_secs(8));
        }
        assert!(backoff(0) <= Duration::from_millis(500));
    }

    #[test]
    fn retry_after_is_read_and_capped() {
        let h = Headers::new([("retry-after".to_owned(), "3".to_owned())]);
        assert_eq!(retry_after(&h), Some(Duration::from_secs(3)));
        let h = Headers::new([("retry-after".to_owned(), "900".to_owned())]);
        assert_eq!(retry_after(&h), Some(Duration::from_secs(60)));
        let h = Headers::new([("retry-after".to_owned(), "Wed, 21 Oct".to_owned())]);
        assert_eq!(retry_after(&h), None);
        assert!(retryable_status(429) && retryable_status(503) && retryable_status(504));
        assert!(!retryable_status(500) && !retryable_status(400));
    }
}
