//! The runner's loop (RFC 0120, "Leases and heartbeats"): heartbeat on an interval,
//! accept an offered lease, run its turn while the heartbeats carry on, report the reply.
//!
//! One turn at a time (capacity 1 in this version): a lease offered while a turn runs is
//! left to expire and goes to another runner of the same person. A lease is accepted only
//! when its turn pack can be fetched. Finished turns are reported through an in-memory
//! outbox (lost if the runner exits; a durable one is a follow-up).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::claude::Reply;
use super::exec::{Exec, Finished};
use super::service::{
    AcceptFacts, Action, Beat, Lease, LeaseBeat, Pack, RunnerInfo, Service, TurnReport, Usage,
};

/// The RFC's runner heartbeat. The service's proto does not carry an interval yet, so
/// this is the default and `--interval` changes it.
pub(crate) const DEFAULT_INTERVAL: Duration = Duration::from_secs(20);
/// Reconnect backoff bounds (RFC: 1 s to 60 s, with jitter).
pub(crate) const BACKOFF_MIN: Duration = Duration::from_secs(1);
pub(crate) const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Providers are asked again after this many beats, and after every turn.
const PROVIDER_EVERY: u64 = 15;
/// Replies kept for delivery; at this many, no new lease is accepted.
const OUTBOX_MAX: usize = 8;

#[derive(Debug, Clone, Copy)]
pub(crate) struct LoopConfig {
    /// Pins the heartbeat interval (`--interval`); `None` follows what the service says
    /// (`heartbeat_interval_seconds`), else [`DEFAULT_INTERVAL`].
    pub(crate) interval: Option<Duration>,
    pub(crate) backoff_min: Duration,
    pub(crate) backoff_max: Duration,
    /// Stop after this many answered heartbeats (scripts and tests).
    pub(crate) max_beats: Option<u64>,
    /// The most one service call (heartbeat, accept, pack, report) may take.
    pub(crate) op_timeout: Duration,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            interval: None,
            backoff_min: BACKOFF_MIN,
            backoff_max: BACKOFF_MAX,
            max_beats: None,
            op_timeout: Duration::from_secs(30),
        }
    }
}

/// Delay after `failures` failures in a row: doubles from `min` to `max`, and `jitter`
/// (0.0 to 1.0) takes up to half of it off so runners do not retry in step.
pub(crate) fn backoff(failures: u32, min: Duration, max: Duration, jitter: f64) -> Duration {
    let exp = min.saturating_mul(2_u32.saturating_pow(failures.saturating_sub(1).min(16)));
    let capped = exp.min(max);
    capped.mul_f64(1.0 - 0.5 * jitter.clamp(0.0, 1.0))
}

fn jitter() -> f64 {
    let mut b = [0_u8; 2];
    let _ = getrandom::fill(&mut b);
    f64::from(u16::from_le_bytes(b)) / f64::from(u16::MAX)
}

/// Why the loop ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum End {
    Shutdown,
    MaxBeats,
    /// The service says this runner was revoked.
    Revoked,
}

/// What the loop did, for tests and the exit note.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub(crate) beats: u64,
    pub(crate) failures: u64,
    pub(crate) accepted: u64,
    pub(crate) turns: u64,
    pub(crate) reported: u64,
}

struct Busy {
    lease: Lease,
    pack: Pack,
    handle: JoinHandle<Finished>,
}

type Log<'a> = &'a (dyn Fn(&str) + Sync);
type Op<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// What taking a lease came to.
struct Taken {
    /// `AcceptLease` succeeded.
    accepted: bool,
    next: Next,
}

enum Next {
    /// Not accepted: the lease is left to expire and goes to another runner.
    Skip,
    Run(Box<Busy>),
    /// Accepted but no turn can run: reported as failed, never dropped.
    Report(Box<TurnReport>),
}

/// Stops what runs in the background, and waits until it is stopped. Aborting the turn
/// task drops its future, and with it the guard that kills `claude`'s process group.
async fn stop(busy: Option<Busy>, refresher: Option<JoinHandle<()>>) {
    if let Some(b) = busy {
        b.handle.abort();
        let _ = b.handle.await;
    }
    if let Some(r) = refresher {
        r.abort();
        let _ = r.await;
    }
}

/// Runs until `shutdown` is set or `cfg.max_beats` heartbeats were answered.
///
/// Everything that can wait (the heartbeat, accepting a lease and fetching its pack,
/// delivering a report, refreshing the providers) is its own in-flight operation, each
/// under `cfg.op_timeout`, polled beside the others: a slow one never delays a heartbeat
/// or a shutdown. Reports wait in an in-memory outbox until the service takes them.
#[allow(
    clippy::too_many_lines,
    reason = "one select over the loop's operations"
)]
pub(crate) async fn run_loop<S: Service, X: Exec>(
    svc: &S,
    exec: &Arc<X>,
    runner: &RunnerInfo,
    cfg: LoopConfig,
    mut shutdown: watch::Receiver<bool>,
    log: Log<'_>,
) -> (End, Stats) {
    let mut stats = Stats::default();
    let providers = Arc::new(Mutex::new(exec.providers().await));
    let mut refresher: Option<JoinHandle<()>> = None;
    let mut refresh_wanted = false;
    let mut busy: Option<Busy> = None;
    let mut taking: Option<Op<'_, Taken>> = None;
    let mut beating: Option<Op<'_, Result<Beat, String>>> = None;
    let mut lease_beating: Option<Op<'_, Result<LeaseBeat, String>>> = None;
    let mut service_interval: Option<Duration> = None;
    let mut next_lease_beat = Instant::now();
    let mut lease_beat_started = next_lease_beat;
    let mut reporting: Option<Op<'_, Result<(), String>>> = None;
    let mut outbox: VecDeque<TurnReport> = VecDeque::new();
    // Delivery attempts of the report at the front of the outbox.
    let mut tries: u32 = 0;
    let mut failures: u32 = 0;
    let mut next_beat = Instant::now();
    let mut beat_started = next_beat;
    let mut next_report = next_beat;
    let interval = |service: Option<Duration>| cfg.interval.or(service).unwrap_or(DEFAULT_INTERVAL);

    loop {
        if lease_beating.is_none()
            && Instant::now() >= next_lease_beat
            && let Some(b) = busy.as_ref()
        {
            lease_beat_started = Instant::now();
            let lease = b.lease.clone();
            lease_beating = Some(Box::pin(async move {
                match tokio::time::timeout(cfg.op_timeout, svc.lease_heartbeat(&lease)).await {
                    Ok(Ok(b)) => Ok(b),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("timed out".into()),
                }
            }));
        }
        if reporting.is_none()
            && Instant::now() >= next_report
            && let Some(r) = outbox.front().cloned()
        {
            tries += 1;
            reporting = Some(Box::pin(async move {
                match tokio::time::timeout(cfg.op_timeout, svc.report_turn(&r)).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("timed out".into()),
                }
            }));
        }
        if refresh_wanted && refresher.as_ref().is_none_or(JoinHandle::is_finished) {
            refresh_wanted = false;
            let (e, shared) = (exec.clone(), providers.clone());
            refresher = Some(tokio::spawn(async move {
                let fresh = e.providers().await;
                *shared.lock().unwrap_or_else(PoisonError::into_inner) = fresh;
            }));
        }
        if cfg.max_beats.is_some_and(|m| stats.beats >= m)
            && busy.is_none()
            && taking.is_none()
            && reporting.is_none()
            && (outbox.is_empty() || tries > 0)
        {
            stop(None, refresher.take()).await;
            return (End::MaxBeats, stats);
        }
        if beating.is_none() && Instant::now() >= next_beat {
            beat_started = Instant::now();
            if stats.beats > 0 && stats.beats % PROVIDER_EVERY == 0 {
                refresh_wanted = true;
            }
            let snapshot = providers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let id = runner.runner_id.clone();
            beating = Some(Box::pin(async move {
                match tokio::time::timeout(cfg.op_timeout, svc.heartbeat(&id, &snapshot)).await {
                    Ok(Ok(b)) => Ok(b),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("timed out".into()),
                }
            }));
        }

        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                stop(busy.take(), refresher.take()).await;
                return (End::Shutdown, stats);
            }
            beat = async {
                match beating.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                beating = None;
                match beat {
                    Err(e) => {
                        failures = failures.saturating_add(1);
                        stats.failures += 1;
                        log(&format!("heartbeat failed ({failures} in a row): {e}"));
                        next_beat = Instant::now()
                            + backoff(failures, cfg.backoff_min, cfg.backoff_max, jitter());
                    }
                    Ok(beat) => {
                        failures = 0;
                        stats.beats += 1;
                        if beat.heartbeat_interval_seconds > 0 {
                            service_interval =
                                Some(Duration::from_secs(beat.heartbeat_interval_seconds));
                        }
                        next_beat = (beat_started + interval(service_interval)).max(Instant::now());
                        if beat.runner.status == "revoked" {
                            log("the service revoked this runner");
                            stop(busy.take(), refresher.take()).await;
                            return (End::Revoked, stats);
                        }
                        for lease in beat.offered {
                            if busy.is_some() || taking.is_some() {
                                log(&format!(
                                    "lease {} left for another runner: a turn is running",
                                    lease.lease_id
                                ));
                            } else if outbox.len() >= OUTBOX_MAX {
                                log(&format!(
                                    "lease {} left for another runner: {} replies are not delivered yet",
                                    lease.lease_id,
                                    outbox.len()
                                ));
                            } else if !svc.turn_pack_available() {
                                log(&format!(
                                    "lease {} left for another runner: the service cannot hand over a turn pack yet",
                                    lease.lease_id
                                ));
                            } else {
                                taking = Some(Box::pin(take(svc, exec, lease, runner, cfg.op_timeout, log)));
                            }
                        }
                    }
                }
            }
            joined = async {
                match busy.as_mut() {
                    Some(b) => (&mut b.handle).await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(b) = busy.take() else { continue };
                refresh_wanted = true;
                let finished = joined.unwrap_or_else(|e| {
                    Finished::refused("failed", format!("the turn's task stopped: {e}"))
                });
                stats.turns += 1;
                let report = report_of(&b.lease, &b.pack, runner, &finished);
                log(&format!("turn {} ended: {}", b.pack.turn_id, report.outcome));
                outbox.push_back(report);
                next_report = Instant::now();
            }
            t = async {
                match taking.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                taking = None;
                if t.accepted {
                    stats.accepted += 1;
                }
                match t.next {
                    Next::Skip => {}
                    Next::Run(b) => {
                        busy = Some(*b);
                        next_lease_beat = Instant::now();
                    }
                    Next::Report(r) => {
                        outbox.push_back(*r);
                        next_report = Instant::now();
                    }
                }
            }
            sent = async {
                match reporting.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                reporting = None;
                match sent {
                    Ok(()) => {
                        outbox.pop_front();
                        tries = 0;
                        stats.reported += 1;
                        next_report = Instant::now();
                    }
                    Err(e) => {
                        log(&format!(
                            "the reply {} was not delivered, kept to retry ({} waiting): {e}",
                            outbox.front().map(TurnReport::idempotency_key).unwrap_or_default(),
                            outbox.len()
                        ));
                        next_report = Instant::now() + interval(service_interval);
                    }
                }
            }
            beat = async {
                match lease_beating.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                lease_beating = None;
                next_lease_beat = (lease_beat_started + interval(service_interval)).max(Instant::now());
                match beat {
                    Ok(LeaseBeat { stop: true, stop_reason }) => {
                        // Past the wall-clock cap: the service says stop. The turn's task is
                        // aborted (its process group dies with it) and reported as capped.
                        if let Some(b) = busy.take() {
                            log(&format!("turn {} stopped by the service: {stop_reason}", b.pack.turn_id));
                            let (lease, pack) = (b.lease.clone(), b.pack.clone());
                            stop(Some(b), None).await;
                            stats.turns += 1;
                            let finished = Finished {
                                outcome: "cap_hit".into(),
                                detail: format!("stopped by the service: {stop_reason}"),
                                ..Finished::refused("cap_hit", "")
                            };
                            outbox.push_back(report_of(&lease, &pack, runner, &finished));
                            next_report = Instant::now();
                            refresh_wanted = true;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => log(&format!("lease heartbeat failed: {e}")),
                }
            }
            () = tokio::time::sleep_until(next_lease_beat), if lease_beating.is_none() && busy.is_some() => {}
            () = tokio::time::sleep_until(next_beat), if beating.is_none() => {}
            () = tokio::time::sleep_until(next_report), if reporting.is_none() && !outbox.is_empty() => {}
        }
    }
}

/// Accepts a lease and gets its pack. The caller has checked the service can hand a pack
/// over; if the fetch fails all the same, the accepted lease is reported failed (through
/// the outbox) and never dropped silently.
async fn take<S: Service, X: Exec>(
    svc: &S,
    exec: &Arc<X>,
    lease: Lease,
    runner: &RunnerInfo,
    op_timeout: Duration,
    log: Log<'_>,
) -> Taken {
    let facts = AcceptFacts {
        // A coordinate turn has no worktree: it runs in the runner's own empty directory.
        worktree_fresh: true,
        worktree_is_main_checkout: false,
        env_scrubbed: exec.env_is_clean(),
        // The argument builder refuses a turn with no `--max-turns`.
        caps_set: true,
    };
    let skip = Taken {
        accepted: false,
        next: Next::Skip,
    };
    match tokio::time::timeout(op_timeout, svc.accept(&lease, facts)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            log(&format!("lease {} not accepted: {e}", lease.lease_id));
            return skip;
        }
        Err(_) => {
            log(&format!("lease {} not accepted: timed out", lease.lease_id));
            return skip;
        }
    }
    let pack = match tokio::time::timeout(op_timeout, svc.fetch_pack(&lease)).await {
        Ok(Ok(p)) => p,
        other => {
            let why = match other {
                Ok(Err(e)) => e.to_string(),
                _ => "timed out".into(),
            };
            log(&format!(
                "lease {} accepted but its pack could not be fetched, reporting it failed: {why}",
                lease.lease_id
            ));
            let pack = Pack {
                turn_id: lease.turn_id.clone(),
                ..Pack::default()
            };
            let finished =
                Finished::refused("failed", format!("the turn pack was not fetched: {why}"));
            return Taken {
                accepted: true,
                next: Next::Report(Box::new(report_of(&lease, &pack, runner, &finished))),
            };
        }
    };
    let task = exec.clone();
    let p = pack.clone();
    Taken {
        accepted: true,
        next: Next::Run(Box::new(Busy {
            lease,
            pack,
            handle: tokio::spawn(async move { task.execute(p).await }),
        })),
    }
}

fn report_of(lease: &Lease, pack: &Pack, runner: &RunnerInfo, f: &Finished) -> TurnReport {
    let p = f.parsed.clone();
    let reply = p.as_ref().map(|p| p.reply.clone()).unwrap_or_default();
    let Reply {
        say,
        state,
        actions,
        evidence,
        left,
    } = reply;
    let count = |k: &str| {
        p.as_ref()
            .and_then(|p| p.usage.get(k))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    TurnReport {
        turn_id: pack.turn_id.clone(),
        lease_id: lease.lease_id.clone(),
        fencing_token: lease.fencing_token,
        runner_id: runner.runner_id.clone(),
        pack_sha256: pack.pack_sha256.clone(),
        outcome: f.outcome.clone(),
        say,
        state,
        actions: actions.iter().map(action_of).collect(),
        evidence: evidence.iter().map(text_of).collect(),
        left: left.iter().map(text_of).collect(),
        provider: "claude".into(),
        provider_session_id: f.session_id.clone(),
        resumed: f.resumed,
        provider_version: f.provider_version.clone(),
        num_turns: p.as_ref().map_or(0, |p| p.num_turns),
        duration_ms: p.as_ref().map_or(0, |p| p.duration_ms),
        total_cost_usd: p.as_ref().and_then(|p| p.total_cost_usd),
        usage: Usage {
            input: count("input_tokens"),
            output: count("output_tokens"),
            cache_read: count("cache_read_input_tokens"),
            cache_write: count("cache_creation_input_tokens"),
        },
        denied_tools: p
            .as_ref()
            .map(|p| p.denied_tools.clone())
            .unwrap_or_default(),
        detail: f.detail.clone(),
        // A turn that never got its pack (no attempt known) is the first.
        attempt: pack.attempt.max(1),
        resets_at: p.map(|p| p.resets_at).unwrap_or_default(),
    }
}

/// A list element as text: a string as is, anything else as its JSON.
fn text_of(v: &serde_json::Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}

/// An action as the contract has it: its `kind` and its payload as JSON text (the
/// `payload` field, else every other field).
fn action_of(v: &serde_json::Value) -> Action {
    let kind = v
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let payload = v.get("payload").cloned().unwrap_or_else(|| {
        let mut rest = v.clone();
        if let Some(o) = rest.as_object_mut() {
            o.remove("kind");
        }
        rest
    });
    Action {
        kind,
        payload_json: payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    #![allow(
        clippy::unused_async_trait_impl,
        reason = "a fake with the real trait's shape"
    )]
    use super::*;
    use crate::runner::service::{Challenge, ProviderState, Registration, ServiceError};
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    #[test]
    fn backoff_doubles_to_the_cap_and_jitter_only_shortens() {
        let (min, max) = (BACKOFF_MIN, BACKOFF_MAX);
        let secs = |n, j| backoff(n, min, max, j).as_secs_f64();
        assert!((secs(1, 0.0) - 1.0).abs() < 1e-9);
        assert!((secs(2, 0.0) - 2.0).abs() < 1e-9);
        assert!((secs(4, 0.0) - 8.0).abs() < 1e-9);
        assert!((secs(7, 0.0) - 60.0).abs() < 1e-9);
        assert!((secs(500, 0.0) - 60.0).abs() < 1e-9);
        assert!((secs(7, 1.0) - 30.0).abs() < 1e-9);
        assert!(secs(3, 0.5) > 2.0 && secs(3, 0.5) < 4.0);
    }

    /// A service with scripted answers that records what it was asked.
    #[derive(Default)]
    struct Fake {
        beats: Mutex<VecDeque<Result<Vec<Lease>, ()>>>,
        calls: Mutex<Vec<String>>,
        reports: Mutex<Vec<TurnReport>>,
        pack: Mutex<Option<Pack>>,
        pack_unavailable: bool,
        /// Lease heartbeats say stop.
        stop_turn: bool,
        /// Report deliveries that fail before one succeeds.
        report_failures: Mutex<u32>,
        /// Heartbeats answer `revoked` once this file exists.
        revoke_when: Option<std::path::PathBuf>,
        /// Met by the first heartbeat after an accept, and by the turn.
        gate: Option<Arc<tokio::sync::Barrier>>,
        gated: Mutex<bool>,
    }

    impl Fake {
        fn note(&self, s: String) {
            self.calls.lock().unwrap().push(s);
        }
    }

    impl Service for Fake {
        fn turn_pack_available(&self) -> bool {
            !self.pack_unavailable
        }
        async fn challenge(&self) -> Result<Challenge, ServiceError> {
            Err(ServiceError::NotImplemented("not used"))
        }
        async fn lease_heartbeat(&self, l: &Lease) -> Result<LeaseBeat, ServiceError> {
            self.note(format!("lease_heartbeat {}", l.lease_id));
            // Let the turn's task start before the answer comes back.
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            Ok(LeaseBeat {
                stop: self.stop_turn,
                stop_reason: "past the cap".into(),
            })
        }
        async fn register(&self, _: &Registration) -> Result<RunnerInfo, ServiceError> {
            Err(ServiceError::NotImplemented("not used"))
        }
        async fn list_runners(&self, _: bool) -> Result<Vec<RunnerInfo>, ServiceError> {
            Ok(vec![])
        }
        async fn heartbeat(&self, id: &str, p: &[ProviderState]) -> Result<Beat, ServiceError> {
            self.note(format!("heartbeat {id} providers={}", p.len()));
            if self.revoke_when.as_ref().is_some_and(|f| f.exists()) {
                let runner = RunnerInfo {
                    status: "revoked".into(),
                    ..RunnerInfo::default()
                };
                return Ok(Beat {
                    runner,
                    offered: vec![],
                    heartbeat_interval_seconds: 0,
                });
            }
            let after_accept = self
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("accept"));
            if let Some(g) = self.gate.as_ref().filter(|_| after_accept) {
                let first = !std::mem::replace(&mut *self.gated.lock().unwrap(), true);
                if first {
                    // The turn is held until this heartbeat is in flight beside it.
                    tokio::time::timeout(Duration::from_secs(2), g.wait())
                        .await
                        .map_err(|_| {
                            ServiceError::NotImplemented("no turn beside the heartbeat")
                        })?;
                }
            }
            match self.beats.lock().unwrap().pop_front() {
                Some(Err(())) => Err(ServiceError::NotImplemented("down")),
                Some(Ok(offered)) => Ok(Beat {
                    runner: RunnerInfo::default(),
                    offered,
                    heartbeat_interval_seconds: 0,
                }),
                None => Ok(Beat::default()),
            }
        }
        async fn accept(&self, l: &Lease, f: AcceptFacts) -> Result<(), ServiceError> {
            assert!(
                f.env_scrubbed && f.caps_set && f.worktree_fresh && !f.worktree_is_main_checkout
            );
            self.note(format!("accept {} token={}", l.lease_id, l.fencing_token));
            Ok(())
        }
        async fn fetch_pack(&self, l: &Lease) -> Result<Pack, ServiceError> {
            self.note(format!("pack {}", l.lease_id));
            self.pack
                .lock()
                .unwrap()
                .clone()
                .ok_or(ServiceError::NotImplemented("no pack"))
        }
        async fn report_turn(&self, r: &TurnReport) -> Result<(), ServiceError> {
            {
                let mut n = self.report_failures.lock().unwrap();
                if *n > 0 {
                    *n -= 1;
                    return Err(ServiceError::NotImplemented("down"));
                }
            }
            self.reports.lock().unwrap().push(r.clone());
            Ok(())
        }
    }

    /// Sets its flag when dropped: a turn's future that was dropped.
    struct Dropped(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[derive(Default)]
    struct FakeExec {
        gate: Option<Arc<tokio::sync::Barrier>>,
        /// Provider refreshes after the first never answer.
        refresh_hangs: bool,
        /// The turn never ends on its own; set to true when its future is dropped.
        hang: Option<Arc<std::sync::atomic::AtomicBool>>,
        asked: std::sync::atomic::AtomicU32,
    }
    impl Exec for FakeExec {
        fn execute(&self, pack: Pack) -> Pin<Box<dyn Future<Output = Finished> + Send>> {
            let gate = self.gate.clone();
            // Made before the future first runs: a stop may come before its first poll.
            let hang = self.hang.clone().map(Dropped);
            Box::pin(async move {
                if let Some(_dropped) = hang {
                    std::future::pending::<()>().await;
                }
                let mut outcome = "replied";
                match gate {
                    Some(g) => {
                        if tokio::time::timeout(Duration::from_secs(2), g.wait())
                            .await
                            .is_err()
                        {
                            outcome = "failed";
                        }
                    }
                    None => tokio::time::sleep(Duration::from_millis(30)).await,
                }
                Finished {
                    outcome: outcome.into(),
                    parsed: None,
                    resumed: true,
                    session_id: format!("sess-{}", pack.turn_id),
                    detail: String::new(),
                    provider_version: "x".into(),
                }
            })
        }
        fn providers(&self) -> Pin<Box<dyn Future<Output = Vec<ProviderState>> + Send>> {
            let hang = self.refresh_hangs
                && self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0;
            Box::pin(async move {
                if hang {
                    std::future::pending::<()>().await;
                }
                vec![ProviderState {
                    provider: "claude".into(),
                    method: "subscription".into(),
                    signed_in: true,
                }]
            })
        }
        fn env_is_clean(&self) -> bool {
            true
        }
    }

    fn lease(id: &str, token: i64) -> Lease {
        Lease {
            lease_id: id.into(),
            job_id: "job_1".into(),
            turn_id: "turn_1".into(),
            fencing_token: token,
        }
    }

    fn cfg(max_beats: u64) -> LoopConfig {
        LoopConfig {
            interval: Some(Duration::from_millis(10)),
            backoff_min: Duration::from_millis(5),
            backoff_max: Duration::from_millis(20),
            max_beats: Some(max_beats),
            op_timeout: Duration::from_secs(2),
        }
    }

    fn runner() -> RunnerInfo {
        RunnerInfo {
            runner_id: "run_1".into(),
            user: "usr_me".into(),
            ..RunnerInfo::default()
        }
    }

    async fn go(fake: &Fake, c: LoopConfig) -> (End, Stats) {
        let (_tx, rx) = watch::channel(false);
        run_loop(
            fake,
            &Arc::new(FakeExec::default()),
            &runner(),
            c,
            rx,
            &|_| {},
        )
        .await
    }

    #[tokio::test]
    async fn heartbeats_on_the_interval_until_the_beat_count() {
        let fake = Fake::default();
        let started = std::time::Instant::now();
        let (end, stats) = go(&fake, cfg(4)).await;
        assert_eq!(end, End::MaxBeats);
        assert_eq!(stats.beats, 4);
        assert!(
            started.elapsed() >= Duration::from_millis(25),
            "beats were not spaced"
        );
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(calls.iter().all(|c| c == "heartbeat run_1 providers=1"));
    }

    #[tokio::test]
    async fn an_offered_lease_is_accepted_run_and_reported() {
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(Pack {
            turn_id: "turn_1".into(),
            pack_sha256: "abc".into(),
            ..Pack::default()
        });
        let (_, stats) = go(&fake, cfg(3)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 1, 1));
        let calls = fake.calls.lock().unwrap().clone();
        let accept = calls
            .iter()
            .position(|c| c == "accept lease_1 token=7")
            .unwrap();
        assert_eq!(calls[accept + 1], "pack lease_1");
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (
                r[0].lease_id.as_str(),
                r[0].fencing_token,
                r[0].outcome.as_str()
            ),
            ("lease_1", 7, "replied")
        );
        assert!(r[0].resumed);
        assert_eq!(r[0].provider_session_id, "sess-turn_1");
        assert_eq!(r[0].pack_sha256, "abc");
    }

    #[tokio::test]
    async fn heartbeats_carry_on_while_a_turn_runs() {
        // The turn waits at a barrier that only a heartbeat sent after the accept can
        // meet: a loop that waits for the turn before beating again deadlocks and the
        // turn fails.
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let fake = Fake {
            gate: Some(gate.clone()),
            ..Fake::default()
        };
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        *fake.pack.lock().unwrap() = Some(Pack::default());
        let exec = Arc::new(FakeExec {
            gate: Some(gate),
            ..FakeExec::default()
        });
        let (_tx, rx) = watch::channel(false);
        let (_, stats) = run_loop(&fake, &exec, &runner(), cfg(6), rx, &|_| {}).await;
        assert_eq!(stats.reported, 1);
        let r = fake.reports.lock().unwrap();
        assert_eq!(
            r[0].outcome, "replied",
            "the turn never saw a heartbeat beside it"
        );
    }

    #[tokio::test]
    async fn the_lease_is_kept_alive_during_a_turn_and_a_stop_ends_the_turn() {
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake {
            stop_turn: true,
            ..Fake::default()
        };
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 3)]));
        *fake.pack.lock().unwrap() = Some(Pack {
            turn_id: "turn_1".into(),
            attempt: 2,
            ..Pack::default()
        });
        let exec = Arc::new(FakeExec {
            hang: Some(dropped.clone()),
            ..FakeExec::default()
        });
        let (_tx, rx) = watch::channel(false);
        let (end, _) = tokio::time::timeout(
            Duration::from_secs(5),
            run_loop(&fake, &exec, &runner(), cfg(8), rx, &|_| {}),
        )
        .await
        .unwrap();
        assert_eq!(end, End::MaxBeats);
        assert!(
            fake.calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c == "lease_heartbeat lease_1")
        );
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "the turn was not stopped"
        );
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].outcome.as_str(), r[0].attempt), ("cap_hit", 2));
        assert!(r[0].detail.contains("past the cap"));
    }

    #[tokio::test]
    async fn a_failed_heartbeat_backs_off_and_the_loop_survives() {
        let fake = Fake::default();
        for _ in 0..3 {
            fake.beats.lock().unwrap().push_back(Err(()));
        }
        let (end, stats) = go(&fake, cfg(2)).await;
        assert_eq!(end, End::MaxBeats);
        assert_eq!(stats.failures, 3);
        assert_eq!(stats.beats, 2);
    }

    #[tokio::test]
    async fn no_lease_is_accepted_while_the_service_has_no_turn_pack() {
        let fake = Fake {
            pack_unavailable: true,
            ..Fake::default()
        };
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        let (_, stats) = go(&fake, cfg(2)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (0, 0, 0));
        assert!(
            !fake
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("accept"))
        );
    }

    #[tokio::test]
    async fn an_accepted_lease_whose_pack_fails_is_reported_failed_not_dropped() {
        let fake = Fake::default(); // available, but the fetch fails: no pack scripted
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 4)]));
        let (_, stats) = go(&fake, cfg(2)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 0, 1));
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (
                r[0].lease_id.as_str(),
                r[0].fencing_token,
                r[0].outcome.as_str()
            ),
            ("lease_1", 4, "failed")
        );
        assert!(r[0].detail.contains("pack"));
    }

    #[tokio::test]
    async fn a_report_that_fails_is_kept_and_retried_with_the_same_key() {
        let fake = Fake {
            report_failures: Mutex::new(2),
            ..Fake::default()
        };
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(Pack::default());
        let (_, stats) = go(&fake, cfg(12)).await;
        assert_eq!(stats.reported, 1);
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].idempotency_key(), "lease_1:7::1");
        assert_eq!(*fake.report_failures.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_provider_refresh_that_never_answers_does_not_stop_heartbeats() {
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        *fake.pack.lock().unwrap() = Some(Pack::default());
        // After the turn the refresh hangs for good; beats must still reach 6.
        let exec = Arc::new(FakeExec {
            refresh_hangs: true,
            ..FakeExec::default()
        });
        let (_tx, rx) = watch::channel(false);
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            run_loop(&fake, &exec, &runner(), cfg(6), rx, &|_| {}),
        )
        .await
        .expect("a hanging refresh blocked the loop");
        assert_eq!(r.0, End::MaxBeats);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn revocation_kills_the_running_claude() {
        use crate::runner::exec::ClaudeExec;
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let (pid, kid) = (dir.path().join("pid"), dir.path().join("kid"));
        let claude = dir.path().join("claude");
        std::fs::write(
            &claude,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  auth) echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\"apiProvider\":\"firstParty\"}}'; exit 0;;\n  --version) echo 1; exit 0;;\nesac\nsleep 60 &\necho $! > {}\necho $$ > {}\nwait\n",
                kid.display(),
                pid.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let exec = Arc::new(ClaudeExec::new(
            claude,
            dir.path().join("work"),
            vec![],
            None,
            "run_1".into(),
            "usr_me".into(),
        ));
        let fake = Fake {
            revoke_when: Some(pid.clone()),
            ..Fake::default()
        };
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        *fake.pack.lock().unwrap() = Some(Pack {
            user: "usr_me".into(),
            model: "sonnet".into(),
            max_turns: 5,
            allowed_tools: vec!["Read".into()],
            ..Pack::default()
        });
        let (_tx, rx) = watch::channel(false);
        let (end, _) = tokio::time::timeout(
            Duration::from_secs(10),
            run_loop(&fake, &exec, &runner(), cfg(1000), rx, &|_| {}),
        )
        .await
        .unwrap();
        assert_eq!(end, End::Revoked);
        let read = |f: &std::path::Path| -> i32 {
            std::fs::read_to_string(f).unwrap().trim().parse().unwrap()
        };
        for p in [read(&pid), read(&kid)] {
            let mut dead = false;
            for _ in 0..100 {
                let alive = std::process::Command::new("kill")
                    .args(["-0", &p.to_string()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if !alive {
                    dead = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(dead, "process {p} survived the revocation");
        }
    }

    #[tokio::test]
    async fn shutdown_ends_the_loop() {
        let fake = Fake::default();
        let (tx, rx) = watch::channel(false);
        let c = LoopConfig {
            max_beats: None,
            ..cfg(0)
        };
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(35)).await;
            let _ = tx.send(true);
            tx
        });
        let (end, stats) = run_loop(
            &fake,
            &Arc::new(FakeExec::default()),
            &runner(),
            c,
            rx,
            &|_| {},
        )
        .await;
        assert_eq!(end, End::Shutdown);
        assert!(stats.beats >= 2);
        let _ = t.await;
    }
}
