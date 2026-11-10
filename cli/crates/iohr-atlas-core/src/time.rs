//! Time (ADR 0004): bitemporal records, on the intervals and observed time of
//! the `interval` module (re-exported here).
//!
//! Two clocks matter. **Valid time** is when something held in the world. **Recorded
//! time** is when Atlas learned it or changed its mind. A record carries both, and
//! records are never updated in place: a new belief closes the old row's recorded
//! interval and opens a new row. Asking "what did we believe at T" reads recorded time;
//! asking "what was true at T" reads valid time.

mod interval;

pub use interval::*;
use serde::{Deserialize, Serialize};

/// A value with its two times. The store keeps every version; none is overwritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bitemporal<T> {
    /// When the value held in the world.
    pub valid: Interval,
    /// When Atlas held this belief.
    pub recorded: Interval,
    /// The value.
    pub value: T,
}

impl<T> Bitemporal<T> {
    /// A new belief, recorded from `recorded_at` and open.
    pub const fn new(value: T, valid: Interval, recorded_at: Timestamp) -> Self {
        Self {
            valid,
            recorded: Interval::open(recorded_at),
            value,
        }
    }

    /// Whether Atlas held this belief at `t`.
    #[must_use]
    pub fn believed_at(&self, t: Timestamp) -> bool {
        self.recorded.contains(t)
    }

    /// Supersede this belief at `at`: this row's recorded interval is closed, and the
    /// new row starts at the same instant, so there is never a gap or an overlap in
    /// what Atlas believed.
    ///
    /// # Errors
    /// This row is already superseded, or `at` is not after it was recorded.
    pub fn supersede(
        self,
        next: T,
        next_valid: Interval,
        at: Timestamp,
    ) -> Result<(Self, Self), CloseError> {
        let closed = Self {
            recorded: self.recorded.close_at(at)?,
            ..self
        };
        let opened = Self::new(next, next_valid, at);
        Ok((closed, opened))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone as _, Utc};

    fn at(s: i64) -> Timestamp {
        Utc.timestamp_opt(1_800_000_000 + s, 0).unwrap()
    }

    #[test]
    fn supersede_leaves_no_gap_and_no_overlap() {
        let first = Bitemporal::new("a", Interval::open(at(0)), at(0));
        let (old, new) = first.supersede("b", Interval::open(at(0)), at(10)).unwrap();
        assert!(old.believed_at(at(9)) && !old.believed_at(at(10)));
        assert!(new.believed_at(at(10)) && !new.believed_at(at(9)));
    }

    #[test]
    fn a_superseded_belief_cannot_be_superseded_again() {
        let first = Bitemporal::new(1, Interval::open(at(0)), at(0));
        let (old, _) = first.supersede(2, Interval::open(at(0)), at(10)).unwrap();
        assert_eq!(
            old.supersede(3, Interval::open(at(0)), at(20)).unwrap_err(),
            CloseError::AlreadyClosed
        );
    }
}
