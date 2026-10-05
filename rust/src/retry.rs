//! The retry policy (`docs/design.md` section 6, `docs/config.md` section 7.4):
//! idempotent calls (and writes with an idempotency key) only, after a connection
//! failure, a timeout, or a `429`, `503` or `504`; the server's `Retry-After` (seconds
//! or an HTTP date), otherwise full jitter from the base delay up to the cap.

use std::time::Duration;

use crate::error::Headers;

/// The longest wait a server may ask for, by default.
pub(crate) const RETRY_AFTER_CAP: Duration = Duration::from_secs(60);
pub(crate) const BACKOFF_BASE: Duration = Duration::from_millis(500);
pub(crate) const BACKOFF_CAP: Duration = Duration::from_secs(8);

/// Whether a status is retried.
pub(crate) fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 503 | 504)
}

/// The server's `Retry-After`: delay-seconds, or an HTTP date (RFC 9110 section
/// 10.2.3), as the wait from now.
pub(crate) fn retry_after(headers: &Headers) -> Option<Duration> {
    let v = headers.get("retry-after")?.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let format = time::format_description::parse_borrowed::<2>(
        "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT",
    )
    .ok()?;
    let at = time::PrimitiveDateTime::parse(v, &format)
        .ok()?
        .assume_utc();
    let left = at - time::OffsetDateTime::now_utc();
    Some(Duration::try_from(left).unwrap_or(Duration::ZERO))
}

/// Full jitter: a random wait between 0 and min(`cap`, `base` × 2^attempt).
pub(crate) fn backoff(attempt: u32, base: Duration, cap: Duration) -> Duration {
    let ceiling = base.saturating_mul(1 << attempt.min(16)).min(cap);
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
        let (base, cap) = (super::BACKOFF_BASE, super::BACKOFF_CAP);
        for attempt in 0..40 {
            assert!(backoff(attempt, base, cap) <= Duration::from_secs(8));
        }
        assert!(backoff(0, base, cap) <= Duration::from_millis(500));
    }

    #[test]
    fn retry_after_is_seconds_or_a_date() {
        let h = Headers::new([("retry-after".to_owned(), "3".to_owned())]);
        assert_eq!(retry_after(&h), Some(Duration::from_secs(3)));
        let h = Headers::new([("retry-after".to_owned(), "900".to_owned())]);
        assert_eq!(retry_after(&h), Some(Duration::from_mins(15)));
        let h = Headers::new([("retry-after".to_owned(), "Wed, 21 Oct".to_owned())]);
        assert_eq!(retry_after(&h), None);
        let past = Headers::new([(
            "retry-after".to_owned(),
            "Sun, 06 Nov 1994 08:49:37 GMT".to_owned(),
        )]);
        assert_eq!(retry_after(&past), Some(Duration::ZERO));
        assert!(retryable_status(429) && retryable_status(503) && retryable_status(504));
        assert!(!retryable_status(500) && !retryable_status(400));
    }
}
