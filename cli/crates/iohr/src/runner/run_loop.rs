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
    AcceptFacts, Action, Beat, Lease, LeaseBeat, Pack, RunnerInfo, Service, ServiceError,
    TurnReport, Usage,
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
/// Tries at fetching a pack before the lease is given up (it then expires).
const FETCH_TRIES: u32 = 10;

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
    /// How long shutdown may spend delivering what is in the outbox.
    pub(crate) flush_timeout: Duration,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            interval: None,
            backoff_min: BACKOFF_MIN,
            backoff_max: BACKOFF_MAX,
            max_beats: None,
            op_timeout: Duration::from_secs(30),
            flush_timeout: Duration::from_secs(10),
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
    /// Reports the service refused for good and the runner dropped.
    pub(crate) rejected: u64,
}

struct Busy {
    lease: Lease,
    pack: Pack,
    handle: JoinHandle<Finished>,
}

/// An accepted lease whose pack is not fetched yet.
struct Fetch {
    lease: Lease,
    tries: u32,
}

type Log<'a> = &'a (dyn Fn(&str) + Sync);
type Op<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
/// The answers to one round of lease heartbeats, each with the lease it was asked for.
type LeaseAnswers = Vec<(Lease, Result<LeaseBeat, OpErr>)>;

/// Why a service call failed, and whether asking again could help.
struct OpErr {
    msg: String,
    permanent: bool,
}

impl From<ServiceError> for OpErr {
    fn from(e: ServiceError) -> Self {
        Self {
            permanent: e.is_permanent(),
            msg: e.to_string(),
        }
    }
}

impl OpErr {
    fn transient(msg: &str) -> Self {
        Self {
            msg: msg.into(),
            permanent: false,
        }
    }
}

/// A call under `limit`.
async fn bounded<T>(
    limit: Duration,
    call: impl Future<Output = Result<T, ServiceError>>,
) -> Result<T, OpErr> {
    match tokio::time::timeout(limit, call).await {
        Ok(r) => r.map_err(OpErr::from),
        Err(_) => Err(OpErr::transient("timed out")),
    }
}

/// A lease is named by its id and the fencing token it was taken with.
fn is_lease(l: &Lease, id: &str, token: i64) -> bool {
    l.lease_id == id && l.fencing_token == token
}

/// Every lease the runner holds and must keep alive: the turn running, the pack being
/// fetched, and each finished turn whose report is not delivered yet.
fn held_leases(
    busy: Option<&Busy>,
    fetching: Option<&Fetch>,
    outbox: &VecDeque<TurnReport>,
) -> Vec<Lease> {
    let mut all: Vec<Lease> = busy.map(|b| b.lease.clone()).into_iter().collect();
    all.extend(fetching.map(|f| f.lease.clone()));
    for r in outbox {
        if !all
            .iter()
            .any(|l| is_lease(l, &r.lease_id, r.fencing_token))
        {
            all.push(Lease {
                lease_id: r.lease_id.clone(),
                job_id: String::new(),
                turn_id: r.turn_id.clone(),
                fencing_token: r.fencing_token,
            });
        }
    }
    all
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

/// Delivers what is in the outbox, in order, until `limit` is up. A permanent refusal
/// drops that report; a transient one ends the attempt and keeps the rest.
async fn flush<S: Service>(
    svc: &S,
    outbox: &mut VecDeque<TurnReport>,
    limit: Duration,
    op_timeout: Duration,
    stats: &mut Stats,
    log: Log<'_>,
) {
    let deadline = Instant::now() + limit;
    while let Some(r) = outbox.front().cloned() {
        let left = deadline
            .saturating_duration_since(Instant::now())
            .min(op_timeout);
        if left.is_zero() {
            break;
        }
        match bounded(left, svc.report_turn(&r)).await {
            Ok(()) => stats.reported += 1,
            Err(e) if e.permanent => {
                stats.rejected += 1;
                log(&format!(
                    "report {} refused for good, dropped: {}",
                    r.idempotency_key(),
                    e.msg
                ));
            }
            Err(e) => {
                log(&format!(
                    "{} replies left undelivered at exit: {}",
                    outbox.len(),
                    e.msg
                ));
                break;
            }
        }
        outbox.pop_front();
    }
}

/// The report of a turn that did not finish: the turn the pack named, `failed` or
/// `cap_hit`, with the pack's own hash and attempt.
fn unfinished(b: &Busy, runner: &RunnerInfo, outcome: &str, detail: String) -> TurnReport {
    let finished = Finished {
        outcome: outcome.into(),
        detail,
        ..Finished::refused(outcome, "")
    };
    report_of(&b.lease, &b.pack, runner, &finished).limited()
}

/// Runs until `shutdown` is set or `cfg.max_beats` heartbeats were answered.
///
/// Everything that can wait (the heartbeat, accepting a lease, fetching its pack,
/// keeping leases alive, delivering a report, refreshing the providers) is its own
/// in-flight operation, each under `cfg.op_timeout`, polled beside the others: a slow
/// one never delays a heartbeat or a shutdown.
///
/// A lease is kept alive from its accept until its report is delivered (or the service
/// refuses it for good): while the pack is fetched (retried), while the turn runs, and
/// while the report waits in the in-memory outbox. A lease answer is matched to the
/// lease and fencing token it was asked for; a stop or refusal ends only that lease's
/// work. On shutdown the turn is aborted, its cancellation is recorded as a report, and
/// the outbox is flushed for up to `cfg.flush_timeout`.
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
    let mut accepting: Option<Op<'_, Option<Lease>>> = None;
    let mut fetching: Option<Fetch> = None;
    let mut fetch_op: Option<Op<'_, Result<Pack, OpErr>>> = None;
    let mut beating: Option<Op<'_, Result<Beat, OpErr>>> = None;
    let mut lease_beating: Option<Op<'_, LeaseAnswers>> = None;
    let mut reporting: Option<Op<'_, (String, Result<(), OpErr>)>> = None;
    let mut service_interval: Option<Duration> = None;
    let mut outbox: VecDeque<TurnReport> = VecDeque::new();
    // Delivery attempts since the last success.
    let mut tries: u32 = 0;
    let mut failures: u32 = 0;
    let start = Instant::now();
    let (mut next_beat, mut next_report, mut next_lease_beat, mut next_fetch) =
        (start, start, start, start);
    let mut beat_started = start;
    let mut lease_beat_started = start;
    let interval = |service: Option<Duration>| cfg.interval.or(service).unwrap_or(DEFAULT_INTERVAL);

    loop {
        let held = held_leases(busy.as_ref(), fetching.as_ref(), &outbox);
        let has_held = !held.is_empty();
        let now = Instant::now();
        if lease_beating.is_none() && now >= next_lease_beat && has_held {
            lease_beat_started = now;
            let limit = cfg.op_timeout;
            lease_beating = Some(Box::pin(async move {
                let mut out = Vec::new();
                for l in held {
                    let r = bounded(limit, svc.lease_heartbeat(&l)).await;
                    out.push((l, r));
                }
                out
            }));
        }
        if fetch_op.is_none()
            && now >= next_fetch
            && let Some(f) = fetching.as_ref()
        {
            let lease = f.lease.clone();
            let limit = cfg.op_timeout;
            fetch_op = Some(Box::pin(async move {
                let pack = bounded(limit, svc.fetch_pack(&lease)).await?;
                check_pack(&pack, &lease)?;
                Ok(pack)
            }));
        }
        if reporting.is_none()
            && now >= next_report
            && let Some(r) = outbox.front().cloned()
        {
            tries += 1;
            let limit = cfg.op_timeout;
            reporting = Some(Box::pin(async move {
                let key = r.idempotency_key();
                (key, bounded(limit, svc.report_turn(&r)).await)
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
            && accepting.is_none()
            && fetching.is_none()
            && reporting.is_none()
            && (outbox.is_empty() || tries > 0)
        {
            stop(None, refresher.take()).await;
            return (End::MaxBeats, stats);
        }
        if beating.is_none() && now >= next_beat {
            beat_started = now;
            if stats.beats > 0 && stats.beats % PROVIDER_EVERY == 0 {
                refresh_wanted = true;
            }
            let snapshot = providers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let id = runner.runner_id.clone();
            let limit = cfg.op_timeout;
            beating = Some(Box::pin(async move {
                bounded(limit, svc.heartbeat(&id, &snapshot)).await
            }));
        }

        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                drop(reporting.take());
                if let Some(b) = busy.take() {
                    log(&format!("turn {} cancelled by shutdown", b.pack.turn_id));
                    let report = unfinished(&b, runner, "failed", "the runner was stopped before the turn finished".into());
                    stop(Some(b), refresher.take()).await;
                    stats.turns += 1;
                    outbox.push_back(report);
                } else {
                    stop(None, refresher.take()).await;
                }
                flush(svc, &mut outbox, cfg.flush_timeout, cfg.op_timeout, &mut stats, log).await;
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
                        log(&format!("heartbeat failed ({failures} in a row): {}", e.msg));
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
                            if busy.is_some() || accepting.is_some() || fetching.is_some() {
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
                                accepting = Some(Box::pin(accept_lease(svc, exec, lease, cfg.op_timeout, log)));
                            }
                        }
                    }
                }
            }
            answers = async {
                match lease_beating.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                lease_beating = None;
                next_lease_beat = (lease_beat_started + interval(service_interval)).max(Instant::now());
                for (lease, answer) in answers {
                    let (id, token) = (lease.lease_id.as_str(), lease.fencing_token);
                    // Only for a lease still held: an answer that comes after its
                    // report was delivered, or its turn was stopped, is ignored.
                    if !held_leases(busy.as_ref(), fetching.as_ref(), &outbox)
                        .iter()
                        .any(|l| is_lease(l, id, token))
                    {
                        continue;
                    }
                    let (ended, reason) = match answer {
                        Ok(LeaseBeat { stop: true, stop_reason }) => (Some(true), stop_reason),
                        Ok(_) => (None, String::new()),
                        Err(e) if e.permanent => (Some(false), e.msg),
                        Err(e) => {
                            log(&format!("lease {id} heartbeat failed, will retry: {}", e.msg));
                            (None, String::new())
                        }
                    };
                    let Some(stopped) = ended else { continue };
                    if fetching.as_ref().is_some_and(|f| is_lease(&f.lease, id, token)) {
                        log(&format!("lease {id} ended before its pack came: {reason}"));
                        fetching = None;
                    }
                    if busy.as_ref().is_some_and(|b| is_lease(&b.lease, id, token))
                        && let Some(b) = busy.take()
                    {
                        refresh_wanted = true;
                        stats.turns += 1;
                        if stopped {
                            // Past the wall-clock cap: the turn is aborted (its process
                            // group dies with it) and reported as capped.
                            log(&format!("turn {} stopped by the service: {reason}", b.pack.turn_id));
                            let report = unfinished(&b, runner, "cap_hit", format!("stopped by the service: {reason}"));
                            stop(Some(b), None).await;
                            outbox.push_back(report);
                            next_report = Instant::now();
                        } else {
                            // Released, reassigned or fenced: the work is no longer ours
                            // and a report would be refused.
                            log(&format!("lease {id} is gone ({reason}): turn {} aborted", b.pack.turn_id));
                            stop(Some(b), None).await;
                        }
                    }
                    if !stopped {
                        outbox.retain(|r| !is_lease_of(r, id, token));
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
                let report = report_of(&b.lease, &b.pack, runner, &finished).limited();
                log(&format!("turn {} ended: {}", b.pack.turn_id, report.outcome));
                outbox.push_back(report);
                next_report = Instant::now();
            }
            taken = async {
                match accepting.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                accepting = None;
                if let Some(lease) = taken {
                    stats.accepted += 1;
                    fetching = Some(Fetch { lease, tries: 0 });
                    next_fetch = Instant::now();
                    next_lease_beat = Instant::now();
                }
            }
            got = async {
                match fetch_op.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                fetch_op = None;
                let Some(mut f) = fetching.take() else { continue };
                match got {
                    Ok(pack) => {
                        let (task, p) = (exec.clone(), pack.clone());
                        busy = Some(Busy {
                            lease: f.lease,
                            pack,
                            handle: tokio::spawn(async move { task.execute(p).await }),
                        });
                    }
                    Err(e) => {
                        f.tries += 1;
                        if e.permanent || f.tries >= FETCH_TRIES {
                            // No pack, no hash and no attempt to report with: the lease
                            // is no longer kept alive and expires for another runner.
                            log(&format!(
                                "lease {} given up after {} tries, its pack could not be fetched: {}",
                                f.lease.lease_id, f.tries, e.msg
                            ));
                        } else {
                            log(&format!("lease {} pack not fetched, will retry: {}", f.lease.lease_id, e.msg));
                            next_fetch = Instant::now() + interval(service_interval);
                            fetching = Some(f);
                        }
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
                let (key, result) = sent;
                match result {
                    Ok(()) => {
                        outbox.retain(|r| r.idempotency_key() != key);
                        tries = 0;
                        stats.reported += 1;
                        next_report = Instant::now();
                    }
                    Err(e) if e.permanent => {
                        // Asking again will not change the answer: drop it, or it would
                        // block every report behind it.
                        outbox.retain(|r| r.idempotency_key() != key);
                        stats.rejected += 1;
                        tries = 0;
                        log(&format!("report {key} refused for good, dropped: {}", e.msg));
                        next_report = Instant::now();
                    }
                    Err(e) => {
                        log(&format!(
                            "report {key} not delivered, kept to retry ({} waiting): {}",
                            outbox.len(),
                            e.msg
                        ));
                        next_report = Instant::now() + interval(service_interval);
                    }
                }
            }
            () = tokio::time::sleep_until(next_lease_beat), if lease_beating.is_none() && has_held => {}
            () = tokio::time::sleep_until(next_fetch), if fetch_op.is_none() && fetching.is_some() => {}
            () = tokio::time::sleep_until(next_beat), if beating.is_none() => {}
            () = tokio::time::sleep_until(next_report), if reporting.is_none() && !outbox.is_empty() => {}
        }
    }
}

fn is_lease_of(r: &TurnReport, id: &str, token: i64) -> bool {
    r.lease_id == id && r.fencing_token == token
}

/// A pack the service sent must carry what a report needs: its hash and attempt, and be
/// the lease's own turn.
fn check_pack(p: &Pack, lease: &Lease) -> Result<(), OpErr> {
    let hash_ok = p.pack_sha256.len() == 64 && p.pack_sha256.bytes().all(|b| b.is_ascii_hexdigit());
    if !hash_ok || p.attempt == 0 || (!lease.turn_id.is_empty() && p.turn_id != lease.turn_id) {
        return Err(OpErr {
            msg: "the pack has no valid hash and attempt, or is another turn's".into(),
            permanent: true,
        });
    }
    Ok(())
}

/// `AcceptLease` under the call limit; `None` when it was not accepted (the lease is
/// left to expire and goes to another runner).
async fn accept_lease<S: Service, X: Exec>(
    svc: &S,
    exec: &Arc<X>,
    lease: Lease,
    op_timeout: Duration,
    log: Log<'_>,
) -> Option<Lease> {
    let facts = AcceptFacts {
        // A coordinate turn has no worktree: it runs in the runner's own empty directory.
        worktree_fresh: true,
        worktree_is_main_checkout: false,
        env_scrubbed: exec.env_is_clean(),
        // The argument builder refuses a turn with no `--max-turns`.
        caps_set: true,
    };
    match bounded(op_timeout, svc.accept(&lease, facts)).await {
        Ok(()) => Some(lease),
        Err(e) => {
            log(&format!("lease {} not accepted: {}", lease.lease_id, e.msg));
            None
        }
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
        attempt: pack.attempt,
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
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "a fake with the real trait's shape; the lint is newer than the MSRV toolchain's clippy"
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
    #[allow(
        clippy::struct_excessive_bools,
        reason = "one switch per failure a test injects"
    )]
    struct Fake {
        beats: Mutex<VecDeque<Result<Vec<Lease>, ()>>>,
        calls: Mutex<Vec<String>>,
        reports: Mutex<Vec<TurnReport>>,
        pack: Mutex<Option<Pack>>,
        pack_unavailable: bool,
        /// Pack fetches that fail (transiently) before one succeeds.
        pack_failures: Mutex<u32>,
        /// The service refuses to give a pack, for good.
        pack_refused: bool,
        /// Lease heartbeats say stop.
        stop_turn: bool,
        /// Lease heartbeats for this lease answer a permanent refusal (released).
        refuse_lease: Option<String>,
        /// Lease heartbeats fail, transiently.
        lease_beats_down: bool,
        /// This lease's heartbeat answers `stop` only after the delay: a late answer.
        late_stop: Option<(String, Duration)>,
        /// The first report is refused for good.
        refuse_first_report: Mutex<bool>,
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
            Err(ServiceError::Refused("not used"))
        }
        async fn lease_heartbeat(&self, l: &Lease) -> Result<LeaseBeat, ServiceError> {
            self.note(format!("lease_heartbeat {}", l.lease_id));
            if self.lease_beats_down {
                return Err(down());
            }
            // Released while its turn runs: the service refuses once the pack was handed over.
            let packed = self
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| *c == format!("pack {}", l.lease_id));
            if packed && self.refuse_lease.as_deref() == Some(l.lease_id.as_str()) {
                return Err(ServiceError::Refused("the lease was released"));
            }
            if let Some((id, delay)) = &self.late_stop
                && *id == l.lease_id
            {
                tokio::time::sleep(*delay).await;
                return Ok(LeaseBeat {
                    stop: true,
                    stop_reason: "late".into(),
                });
            }
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
            Err(ServiceError::Refused("not used"))
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
                        .map_err(|_| ServiceError::Refused("no turn beside the heartbeat"))?;
                }
            }
            match self.beats.lock().unwrap().pop_front() {
                Some(Err(())) => Err(down()),
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
            if self.pack_refused {
                return Err(ServiceError::Refused("no such pack"));
            }
            {
                let mut n = self.pack_failures.lock().unwrap();
                if *n > 0 {
                    *n -= 1;
                    return Err(down());
                }
            }
            Ok(self.pack.lock().unwrap().clone().unwrap_or_else(|| Pack {
                turn_id: l.turn_id.clone(),
                ..good_pack()
            }))
        }
        async fn report_turn(&self, r: &TurnReport) -> Result<(), ServiceError> {
            self.note(format!("report {}", r.idempotency_key()));
            if std::mem::take(&mut *self.refuse_first_report.lock().unwrap()) {
                return Err(ServiceError::Refused("over a limit"));
            }
            {
                let mut n = self.report_failures.lock().unwrap();
                if *n > 0 {
                    *n -= 1;
                    return Err(down());
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
        /// This turn takes this long instead of 30 ms.
        slow_turn: Option<(String, Duration)>,
        /// The turn's reply has a `say` over the service's limit.
        big_say: bool,
        asked: std::sync::atomic::AtomicU32,
    }
    impl Exec for FakeExec {
        fn execute(&self, pack: Pack) -> Pin<Box<dyn Future<Output = Finished> + Send>> {
            let gate = self.gate.clone();
            // Made before the future first runs: a stop may come before its first poll.
            let hang = self.hang.clone().map(Dropped);
            let big_say = self.big_say;
            let slow = self
                .slow_turn
                .as_ref()
                .filter(|(id, _)| *id == pack.turn_id)
                .map(|(_, d)| *d);
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
                    None => tokio::time::sleep(slow.unwrap_or(Duration::from_millis(30))).await,
                }
                let parsed = big_say.then(|| crate::runner::claude::Parsed {
                    outcome: crate::runner::claude::Outcome::Replied,
                    reply: crate::runner::claude::Reply {
                        say: "x".repeat(20_001),
                        state: String::new(),
                        actions: vec![],
                        evidence: vec![],
                        left: vec![],
                    },
                    session_id: String::new(),
                    num_turns: 1,
                    duration_ms: 1,
                    total_cost_usd: None,
                    usage: serde_json::json!({}),
                    denied_tools: vec![],
                    resets_at: String::new(),
                });
                Finished {
                    outcome: outcome.into(),
                    parsed,
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

    /// A transient failure: a call that did not answer.
    fn down() -> ServiceError {
        ServiceError::Api(inorbithr::Error::Timeout {
            host: "fake".into(),
            secs: 1,
        })
    }

    /// A pack with what a report needs: the hash and the attempt.
    fn good_pack() -> Pack {
        Pack {
            turn_id: "turn_1".into(),
            pack_sha256: "a".repeat(64),
            attempt: 1,
            ..Pack::default()
        }
    }

    fn lease(id: &str, token: i64) -> Lease {
        Lease {
            lease_id: id.into(),
            job_id: "job_1".into(),
            turn_id: id.replacen("lease", "turn", 1),
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
            flush_timeout: Duration::from_secs(1),
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
            pack_sha256: "ab".repeat(32),
            ..good_pack()
        });
        let (_, stats) = go(&fake, cfg(3)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 1, 1));
        let calls = fake.calls.lock().unwrap().clone();
        let accept = calls
            .iter()
            .position(|c| c == "accept lease_1 token=7")
            .unwrap();
        assert!(calls[accept + 1..].iter().any(|c| c == "pack lease_1"));
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
        assert_eq!(r[0].pack_sha256, "ab".repeat(32));
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
        *fake.pack.lock().unwrap() = Some(good_pack());
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
            ..good_pack()
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

    fn calls_of(fake: &Fake, prefix: &str) -> usize {
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }

    async fn go_with(fake: &Fake, exec: FakeExec, c: LoopConfig) -> (End, Stats) {
        let (_tx, rx) = watch::channel(false);
        tokio::time::timeout(
            Duration::from_secs(10),
            run_loop(fake, &Arc::new(exec), &runner(), c, rx, &|_| {}),
        )
        .await
        .expect("the loop hung")
    }

    fn offer(fake: &Fake, leases: &[Lease]) {
        fake.beats.lock().unwrap().push_back(Ok(leases.to_vec()));
    }

    #[tokio::test]
    async fn a_pack_that_cannot_be_fetched_is_retried_with_the_lease_kept_alive_then_run() {
        let fake = Fake {
            pack_failures: Mutex::new(3),
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 4)]);
        let (_, stats) = go(&fake, cfg(10)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 1, 1));
        assert_eq!(calls_of(&fake, "pack "), 4, "three failures, then the pack");
        assert_ne!(calls_of(&fake, "lease_heartbeat"), 0);
        // Reported with the real pack's hash and attempt, never an empty or guessed one.
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].pack_sha256.len(), r[0].attempt), (64, 1));
        assert_eq!(r[0].outcome, "replied");
    }

    #[tokio::test]
    async fn a_pack_the_service_refuses_gives_up_the_lease_and_reports_nothing() {
        let fake = Fake {
            pack_refused: true,
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 4)]);
        let (_, stats) = go(&fake, cfg(4)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 0, 0));
        // No report with an empty hash or a guessed attempt.
        assert!(fake.reports.lock().unwrap().is_empty());
        assert_eq!(calls_of(&fake, "report "), 0);
        assert_eq!(calls_of(&fake, "pack "), 1);
    }

    #[tokio::test]
    async fn a_pack_without_a_valid_hash_and_attempt_is_not_run() {
        let fake = Fake::default();
        *fake.pack.lock().unwrap() = Some(Pack {
            turn_id: "turn_1".into(),
            ..Pack::default()
        });
        offer(&fake, &[lease("lease_1", 4)]);
        let (_, stats) = go(&fake, cfg(4)).await;
        assert_eq!((stats.turns, stats.reported), (0, 0));
    }

    #[tokio::test]
    async fn the_lease_is_kept_alive_until_its_report_is_delivered() {
        let fake = Fake {
            report_failures: Mutex::new(4),
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 4)]);
        let (_, stats) = go(&fake, cfg(20)).await;
        assert_eq!(stats.reported, 1);
        let calls = fake.calls.lock().unwrap().clone();
        let first_failed = calls.iter().position(|c| c.starts_with("report ")).unwrap();
        let last_report = calls
            .iter()
            .rposition(|c| c.starts_with("report "))
            .unwrap();
        assert!(
            calls[first_failed..last_report]
                .iter()
                .any(|c| c == "lease_heartbeat lease_1"),
            "no lease heartbeat while the report was undelivered: {calls:?}"
        );
    }

    #[tokio::test]
    async fn a_late_stop_for_a_finished_lease_never_aborts_the_next_turn() {
        // lease_1's heartbeat answers `stop` only after 400 ms; by then its turn is done
        // and reported, and lease_2's turn (600 ms) is the one running.
        let fake = Fake {
            late_stop: Some(("lease_1".into(), Duration::from_millis(400))),
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 1)]);
        for _ in 0..8 {
            fake.beats.lock().unwrap().push_back(Ok(vec![]));
        }
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_2", 2)]));
        let exec = FakeExec {
            slow_turn: Some(("turn_2".into(), Duration::from_millis(600))),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg(80)).await;
        let r = fake.reports.lock().unwrap();
        let by = |t: &str| r.iter().find(|x| x.turn_id == t).map(|x| x.outcome.clone());
        assert_eq!(by("turn_1").as_deref(), Some("replied"));
        assert_eq!(by("turn_2").as_deref(), Some("replied"), "{stats:?}");
    }

    #[tokio::test]
    async fn a_lease_the_service_released_aborts_that_turn_and_reports_nothing() {
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake {
            refuse_lease: Some("lease_1".into()),
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 3)]);
        let exec = FakeExec {
            hang: Some(dropped.clone()),
            ..FakeExec::default()
        };
        let (end, _) = go_with(&fake, exec, cfg(8)).await;
        assert_eq!(end, End::MaxBeats);
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "the fenced turn kept running"
        );
        assert!(fake.reports.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_lease_heartbeat_that_only_fails_for_now_does_not_abort_the_turn() {
        let fake = Fake {
            lease_beats_down: true,
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 3)]);
        let (_, stats) = go(&fake, cfg(6)).await;
        assert_eq!((stats.turns, stats.reported), (1, 1));
        assert_eq!(fake.reports.lock().unwrap()[0].outcome, "replied");
    }

    #[tokio::test]
    async fn shutdown_records_the_cancelled_turn_and_flushes_it() {
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 3)]);
        let exec = Arc::new(FakeExec {
            hang: Some(dropped.clone()),
            ..FakeExec::default()
        });
        let (tx, rx) = watch::channel(false);
        let c = LoopConfig {
            max_beats: None,
            ..cfg(0)
        };
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(true);
            tx
        });
        let (end, stats) = run_loop(&fake, &exec, &runner(), c, rx, &|_| {}).await;
        let _ = t.await;
        assert_eq!(end, End::Shutdown);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(stats.reported, 1);
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].outcome, "failed");
        assert!(r[0].detail.contains("stopped"));
        assert_eq!((r[0].pack_sha256.len(), r[0].attempt), (64, 1));
    }

    #[tokio::test]
    async fn a_report_refused_for_good_is_dropped_and_does_not_block_the_next() {
        let fake = Fake {
            refuse_first_report: Mutex::new(true),
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 1)]);
        for _ in 0..6 {
            fake.beats.lock().unwrap().push_back(Ok(vec![]));
        }
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_2", 2)]));
        let (_, stats) = go(&fake, cfg(30)).await;
        assert_eq!((stats.rejected, stats.reported), (1, 1));
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].lease_id, "lease_2");
        // Asked once for the refused one, never again.
        assert_eq!(calls_of(&fake, "report lease_1"), 1);
    }

    #[tokio::test]
    async fn a_reply_over_the_limits_is_reported_as_a_bounded_failure() {
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1)]);
        let exec = FakeExec {
            big_say: true,
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg(4)).await;
        assert_eq!(stats.reported, 1);
        let r = fake.reports.lock().unwrap();
        assert_eq!(r[0].outcome, "failed");
        assert!(r[0].say.is_empty() && r[0].detail.contains("limits"));
        assert_eq!((r[0].pack_sha256.len(), r[0].attempt), (64, 1));
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
        *fake.pack.lock().unwrap() = Some(good_pack());
        let (_, stats) = go(&fake, cfg(12)).await;
        assert_eq!(stats.reported, 1);
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].idempotency_key(), "lease_1:7:turn_1:1");
        assert_eq!(*fake.report_failures.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_provider_refresh_that_never_answers_does_not_stop_heartbeats() {
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        *fake.pack.lock().unwrap() = Some(good_pack());
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
            ..good_pack()
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
            assert!(
                crate::runner::exec::process_gone(p).await,
                "process {p} survived the revocation"
            );
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
