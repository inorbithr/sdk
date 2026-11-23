//! The flood and smear guard (RFC 0230 section 5): what a node does with a report before it
//! keeps or forwards it. Pure state and a clock passed in, so every rule has an abuse test.
//!
//! The order of the checks is part of the design: shape, then time, then the signature,
//! and only then anything that changes state. A forged report therefore never spends a real
//! key's quota, never takes a nonce, and never counts toward a listing.
//!
//! A subject is listed only when `k` distinct reporters (the pseudonym the sign-in vouch
//! carries, never the key) reported it inside the window. One person with several keys is
//! one reporter, so a single account cannot smear a shop alone.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use super::record::{Kind, Reason, Report};
use crate::runner::approvals::DeviceKeys;

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// The numbers the guard holds to. RFC 0230 section 5 states the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Reports one key may sign in an hour.
    pub(crate) per_hour: usize,
    /// Reports one reporter (all their keys together) may sign in a day.
    pub(crate) per_day: usize,
    /// Distinct reporters before a subject is listed.
    pub(crate) k: usize,
    /// How far back reports count toward a listing, in seconds.
    pub(crate) window: i64,
    /// How old a stored report may be before it is dropped, in seconds.
    pub(crate) expiry: i64,
    /// How far a live report's time may be from ours, in seconds.
    pub(crate) skew: i64,
    /// Reports a node keeps at most.
    pub(crate) max_reports: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_hour: 10,
            per_day: 30,
            k: 3,
            window: 7 * DAY,
            expiry: 30 * DAY,
            skew: 600,
            max_reports: 100_000,
        }
    }
}

/// How a report reached this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Freshness {
    /// Just published: its time must be within `skew` of ours.
    Live,
    /// From a peer's store while catching up: only `expiry` applies.
    Stored,
}

/// Why a report was refused. Never carries the subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A field is not in its canonical form.
    Shape,
    /// Older than `expiry`, or older than `skew` when live.
    Stale,
    /// More than `skew` ahead of our clock.
    Future,
    /// The key is unknown or the signature does not verify.
    Signature,
    /// This report (or this key's nonce) was seen before.
    Replay,
    /// The key signed `per_hour` reports in the last hour.
    KeyRate,
    /// The reporter signed `per_day` reports in the last day.
    ReporterRate,
    /// The store is full.
    Full,
    /// A retraction from a key that did not sign the report.
    NotOwner,
    /// A retraction for a report this node does not hold.
    Unknown,
}

impl Refusal {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Shape => "malformed",
            Self::Stale => "too old",
            Self::Future => "dated in the future",
            Self::Signature => "signature does not verify",
            Self::Replay => "already seen",
            Self::KeyRate => "key over its hourly limit",
            Self::ReporterRate => "reporter over its daily limit",
            Self::Full => "store full",
            Self::NotOwner => "not signed by this key",
            Self::Unknown => "no such report",
        }
    }
}

/// A listed subject: how many distinct reporters, and the reasons they gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listing {
    pub(crate) reporters: usize,
    pub(crate) reasons: BTreeSet<Reason>,
}

#[derive(Debug, Clone)]
struct Held {
    report: Report,
    reporter: String,
}

/// One node's view of the reports it holds.
#[derive(Debug)]
pub(crate) struct Guard {
    limits: Limits,
    keys: DeviceKeys,
    /// (key id, nonce) pairs seen, with the report's time, so they can expire.
    nonces: HashMap<(String, String), i64>,
    /// Signed-at times per key, oldest first.
    by_key: HashMap<String, VecDeque<i64>>,
    /// Signed-at times per reporter, oldest first.
    by_reporter: HashMap<String, VecDeque<i64>>,
    /// Report id to the report.
    reports: HashMap<String, Held>,
    /// (kind, subject) to the ids of its reports.
    subjects: BTreeMap<(Kind, String), BTreeSet<String>>,
}

impl Guard {
    pub(crate) fn new(limits: Limits, keys: DeviceKeys) -> Self {
        Self {
            limits,
            keys,
            nonces: HashMap::new(),
            by_key: HashMap::new(),
            by_reporter: HashMap::new(),
            reports: HashMap::new(),
            subjects: BTreeMap::new(),
        }
    }

    /// How many reports the node holds.
    pub(crate) fn len(&self) -> usize {
        self.reports.len()
    }

    /// Checks `report` from `reporter` at `now` and keeps it. Returns its id.
    ///
    /// # Errors
    /// The first rule it breaks; nothing is changed then.
    pub(crate) fn accept(
        &mut self,
        report: &Report,
        reporter: &str,
        now: i64,
        freshness: Freshness,
    ) -> Result<String, Refusal> {
        self.prune(now);
        if report.check().is_err() || reporter.is_empty() {
            return Err(Refusal::Shape);
        }
        let age = now - report.signed_at;
        if age > self.limits.expiry {
            return Err(Refusal::Stale);
        }
        if -age > self.limits.skew {
            return Err(Refusal::Future);
        }
        if freshness == Freshness::Live && age > self.limits.skew {
            return Err(Refusal::Stale);
        }
        // The signature before any state: a forger spends nothing of the victim's.
        if !report.verifies(&self.keys) {
            return Err(Refusal::Signature);
        }
        let id = report.id();
        let nonce = (report.key_id.clone(), report.nonce.clone());
        if self.reports.contains_key(&id) || self.nonces.contains_key(&nonce) {
            return Err(Refusal::Replay);
        }
        let hour_from = report.signed_at - HOUR;
        if count_after(self.by_key.get(&report.key_id), hour_from) >= self.limits.per_hour {
            return Err(Refusal::KeyRate);
        }
        let day_from = report.signed_at - DAY;
        if count_after(self.by_reporter.get(reporter), day_from) >= self.limits.per_day {
            return Err(Refusal::ReporterRate);
        }
        if self.reports.len() >= self.limits.max_reports {
            return Err(Refusal::Full);
        }
        self.nonces.insert(nonce, report.signed_at);
        insert_sorted(
            self.by_key.entry(report.key_id.clone()).or_default(),
            report.signed_at,
        );
        insert_sorted(
            self.by_reporter.entry(reporter.to_owned()).or_default(),
            report.signed_at,
        );
        self.subjects
            .entry((report.kind, report.subject.clone()))
            .or_default()
            .insert(id.clone());
        self.reports.insert(
            id.clone(),
            Held {
                report: report.clone(),
                reporter: reporter.to_owned(),
            },
        );
        Ok(id)
    }

    /// Takes back report `id`. Only the key that signed it may. The rate counters are not
    /// given back, so sign-retract cycles cannot dodge the limits.
    ///
    /// # Errors
    /// The node does not hold `id`, or `key_id` did not sign it.
    pub(crate) fn retract(&mut self, id: &str, key_id: &str) -> Result<(), Refusal> {
        let Some(held) = self.reports.get(id) else {
            return Err(Refusal::Unknown);
        };
        if held.report.key_id != key_id {
            return Err(Refusal::NotOwner);
        }
        self.remove(id);
        Ok(())
    }

    /// Whether `subject` is listed at `now`: `k` distinct reporters inside the window.
    pub(crate) fn listed(&self, kind: Kind, subject: &str, now: i64) -> Option<Listing> {
        let ids = self.subjects.get(&(kind, subject.to_owned()))?;
        let from = now - self.limits.window;
        let mut reporters = BTreeSet::new();
        let mut reasons = BTreeSet::new();
        for held in ids.iter().filter_map(|id| self.reports.get(id)) {
            if held.report.signed_at > from {
                reporters.insert(held.reporter.as_str());
                reasons.insert(held.report.reason);
            }
        }
        (reporters.len() >= self.limits.k).then_some(Listing {
            reporters: reporters.len(),
            reasons,
        })
    }

    /// Drops what has expired: reports past `expiry`, nonces with them, and rate entries
    /// older than a day.
    pub(crate) fn prune(&mut self, now: i64) {
        let oldest = now - self.limits.expiry;
        let gone: Vec<String> = self
            .reports
            .iter()
            .filter(|(_, h)| h.report.signed_at < oldest)
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            self.remove(&id);
        }
        self.nonces.retain(|_, at| *at >= oldest);
        let day_ago = now - DAY - self.limits.skew;
        for times in self
            .by_key
            .values_mut()
            .chain(self.by_reporter.values_mut())
        {
            while times.front().is_some_and(|t| *t < day_ago) {
                times.pop_front();
            }
        }
        self.by_key.retain(|_, t| !t.is_empty());
        self.by_reporter.retain(|_, t| !t.is_empty());
    }

    fn remove(&mut self, id: &str) {
        if let Some(held) = self.reports.remove(id) {
            let key = (held.report.kind, held.report.subject);
            if let Some(ids) = self.subjects.get_mut(&key) {
                ids.remove(id);
                if ids.is_empty() {
                    self.subjects.remove(&key);
                }
            }
        }
    }
}

fn count_after(times: Option<&VecDeque<i64>>, from: i64) -> usize {
    times.map_or(0, |t| t.iter().filter(|at| **at > from).count())
}

fn insert_sorted(times: &mut VecDeque<i64>, at: i64) {
    let i = times.partition_point(|t| *t <= at);
    times.insert(i, at);
}
