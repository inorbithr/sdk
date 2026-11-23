//! `iohr atlas scrum`: a scrum board kept as one JSON file, `atlas/scrum/board.json`
//! (RFC 0003, RFC 0234).
//!
//! The board is the authority for the plan: specs, sizes, dates and priorities. The InOrbit
//! Planner is the authority for execution: claims, leases, approvals and verified evidence. They
//! meet only through a card's `planner` field. Cards and boards are checked against the
//! `scrum/card/v0` and `scrum/board/v0` schemas the binary embeds ([`crate::schema`]); what a
//! schema cannot say is in [`check`].

pub(crate) mod board;
pub(crate) mod change;
pub(crate) mod check;
pub(crate) mod diff;
pub(crate) mod sync;
pub(crate) mod view;
pub(crate) mod watch;

use std::fmt;

use chrono::NaiveDate;

use crate::error::Error as CliError;

/// The six lists, in board order.
pub(crate) const LISTS: [&str; 6] = ["backlog", "ready", "doing", "review", "done", "dropped"];
/// Where a board lives under a project root.
pub(crate) const BOARD_PATH: &str = "atlas/scrum/board.json";

/// Why a command did not do what it was asked.
#[derive(Debug)]
pub(crate) enum Error {
    /// Wrong usage: an unknown card or list, a missing argument. Exit 2.
    Usage(String),
    /// A failed check or a refused change, one line per reason. Exit 1.
    Refused(Vec<String>),
    /// The file could not be read or written. Exit 1.
    Io(String),
}

impl Error {
    /// The exit code: 2 for wrong usage, 1 for everything else.
    #[cfg(test)]
    pub(crate) fn code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::Refused(_) | Error::Io(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) | Error::Io(m) => f.write_str(m),
            Error::Refused(lines) => f.write_str(&lines.join("\n")),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for CliError {
    fn from(e: Error) -> Self {
        match e {
            Error::Usage(m) => CliError::Usage(m),
            other => CliError::Failed(other.to_string()),
        }
    }
}

/// A strict `YYYY-MM-DD`; `None` for any other shape or an impossible day.
pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !s
        .bytes()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// A date as `YYYY-MM-DD`.
pub(crate) fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Today in UTC.
pub(crate) fn today() -> String {
    fmt_date(chrono::Utc::now().date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_and_bad_ones_fail() {
        for s in ["1970-01-01", "2000-02-29", "2026-11-14", "2026-12-31"] {
            assert_eq!(fmt_date(parse_date(s).expect(s)), s);
        }
        let a = parse_date("2026-11-15").expect("a");
        let b = parse_date("2026-11-14").expect("b");
        assert_eq!((a - b).num_days(), 1);
        for s in [
            "2026-02-30",
            "2026-13-01",
            "2026-1-01",
            "26-11-14",
            "2026-11-1x",
            "2100-02-29",
        ] {
            assert_eq!(parse_date(s), None, "{s}");
        }
    }
}
