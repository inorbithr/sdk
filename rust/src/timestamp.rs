//! Timestamps as the API sends them: RFC 3339 strings, `""` when unset (rule N5 of
//! `spec/README.md`, settled behaviour). Models keep the string; this reads it.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Reads a timestamp field: `Ok(None)` for `""` (the field is unset), the instant for
/// an RFC 3339 string, an error for anything else.
///
/// ```
/// assert_eq!(inorbithr::parse_timestamp("").unwrap(), None);
/// let t = inorbithr::parse_timestamp("2026-10-04T08:00:00Z").unwrap().unwrap();
/// assert_eq!(t.unix_timestamp(), 1_791_100_800);
/// ```
///
/// # Errors
///
/// [`time::error::Parse`] when the value is neither `""` nor RFC 3339.
pub fn parse_timestamp(value: &str) -> Result<Option<OffsetDateTime>, time::error::Parse> {
    if value.is_empty() {
        return Ok(None);
    }
    OffsetDateTime::parse(value, &Rfc3339).map(Some)
}

#[cfg(test)]
mod tests {
    use super::parse_timestamp;

    #[test]
    fn empty_is_no_value_and_garbage_is_an_error() {
        assert_eq!(parse_timestamp("").ok(), Some(None));
        assert!(parse_timestamp("yesterday").is_err());
        let t = parse_timestamp("2026-10-04T08:00:00.5Z").ok().flatten();
        assert_eq!(t.map(time::OffsetDateTime::millisecond), Some(500));
    }
}
