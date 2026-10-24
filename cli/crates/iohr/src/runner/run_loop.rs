//! The runner's loop (RFC 0120, "Leases and heartbeats"): heartbeat on an interval,
//! accept an offered lease, run its turn while the heartbeats carry on, report the reply.
//!
//! Up to `capacity` turns at once (the declared capacity, else the registered one, 1 to
//! 32; a reload changes it between beats, and lowering it never stops a running turn): a lease
//! offered while every slot is taken is left to expire and goes to another runner of the
//! same person. The service counts a slot from the moment it creates a lease until the
//! report is delivered, so it never offers more than the runner has room for; the cap here
//! is the runner's own guard. Each held lease has its own pack fetch, turn and heartbeat,
//! and one turn failing or crashing never stops the others. A lease is accepted only when
//! its turn pack can be fetched. Finished turns are reported through an in-memory outbox
//! (lost if the runner exits; a durable one is a follow-up).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::access::Access;
use super::claude::Reply;
use super::exec::{Exec, Finished};
use super::gate::{self, Checked, Held, Skip};
use super::service::{
    AcceptFacts, Action, Beat, Lease, LeaseBeat, Pack, RunnerInfo, Service, ServiceError,
    TurnReport, Usage,
};
use super::settings::{self, BuildFloor, Declared};
use super::workspace::{self, Prepared, Shipped, Workspaces};

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
/// The longest a job's checkout may take to make (a first fetch of a large repository).
const PREPARE_TIMEOUT: Duration = Duration::from_secs(600);
/// The most turns one runner holds at once; the service clamps to the same.
pub(crate) const MAX_CAPACITY: usize = settings::MAX_CAPACITY as usize;

#[derive(Debug, Clone)]
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
    /// The most turns held at once (accepting, fetching a pack or running); clamped to
    /// 1 to [`MAX_CAPACITY`]. The service's own slot count for the runner is the limit
    /// that matters; this only keeps the runner from taking more than it registered.
    /// A declared capacity replaces it.
    pub(crate) capacity: usize,
    /// What each heartbeat declares, read again before every beat (a SIGHUP reload sends
    /// a new value). `None`: no capacity, this build's version, every turn class.
    pub(crate) declared: Option<watch::Receiver<Declared>>,
    /// The build-disk guard; `None` turns it off.
    pub(crate) build_floor: Option<BuildFloor>,
    /// The Checked gate's check, run in the job's checkout after a job turn finishes and
    /// sent as a signed gate report before the turn's own report; `None` turns it off.
    pub(crate) gate: Option<GateCheck>,
    /// The checkouts of job leases (RFC 0147 slice A): made before the accept, shipped
    /// as a pull request after a reply; `None` keeps the old behaviour (no checkout).
    pub(crate) workspaces: Option<Arc<Workspaces>>,
}

/// What the run loop runs for the Checked gate (RFC 0120); the same check and the same
/// report as `iohr runner gate-report`.
#[derive(Debug, Clone)]
pub(crate) struct GateCheck {
    /// The turns' working directory (`ClaudeExec::workdir`): the pack's `checkout_path` is
    /// relative to it.
    pub(crate) workdir: PathBuf,
    pub(crate) command: String,
    pub(crate) timeout: Duration,
    /// The runner's own environment, scrubbed by the check before the command sees it.
    pub(crate) env: Vec<(OsString, OsString)>,
}

impl Default for GateCheck {
    fn default() -> Self {
        Self {
            workdir: std::env::temp_dir(),
            command: gate::DEFAULT_COMMAND.to_owned(),
            timeout: Duration::from_secs(gate::DEFAULT_TIMEOUT_SECS),
            env: std::env::vars_os().collect(),
        }
    }
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
            capacity: 1,
            declared: None,
            build_floor: None,
            gate: None,
            workspaces: None,
        }
    }
}

/// What the next heartbeat declares before the build-disk guard.
fn declared_now(cfg: &LoopConfig) -> Declared {
    cfg.declared.as_ref().map_or_else(
        || Declared {
            capacity: None,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            kinds: settings::CLASSES.iter().map(|c| (*c).to_owned()).collect(),
        },
        |rx| rx.borrow().clone(),
    )
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

/// A finished job turn whose lease is kept alive while the gate check runs and its report
/// is sent; the turn's own report waits here and goes to the outbox after.
struct Gating {
    lease: Lease,
    job_id: String,
    report: TurnReport,
    /// The shipping and the check, until they end; then the report is being sent.
    check: Option<JoinHandle<Gated>>,
}

/// What a finished job turn's after-work did: the pull request (when the runner owns the
/// checkout and the turn replied) and the check (when the gate runs).
#[derive(Debug)]
struct Gated {
    shipped: Option<Result<Option<Shipped>, String>>,
    checked: Option<Result<Checked, Skip>>,
}

/// A lease accepted, with its checkout when the runner made one; else its id.
type Accepted = Result<(Lease, Option<Owned>), String>;

/// A job lease's checkout, made by this runner before it accepted the lease, and the
/// document the job serves (for the pull request's body).
#[derive(Debug, Clone)]
struct Owned {
    checkout: Prepared,
    document_id: String,
    /// How git and gh reach GitHub for this lease (RFC 0120): its own installation token,
    /// or this host's sign-in where the platform says the App is not installed.
    access: Access,
}

/// The sending of one gate report, with the lease it was for.
type Sent = (String, i64, String);
/// Tries at sending a gate report that did not get an answer.
const GATE_SEND_TRIES: u32 = 3;
/// The pause between those tries.
const GATE_SEND_PAUSE: Duration = Duration::from_secs(2);

struct Busy {
    lease: Lease,
    pack: Pack,
    owned: Option<Owned>,
    handle: JoinHandle<Finished>,
}

/// An accepted lease whose pack is not fetched yet.
struct Fetch {
    lease: Lease,
    owned: Option<Owned>,
    tries: u32,
    /// Not before this instant is the pack asked for (again).
    next: Instant,
    /// A fetch for this lease is in flight.
    asking: bool,
}

/// The answer to one lease's pack fetch, with the lease it was asked for.
type Fetched = (String, i64, Result<Pack, OpErr>);

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
    busy: &[Busy],
    fetching: &[Fetch],
    gating: &[Gating],
    outbox: &VecDeque<TurnReport>,
) -> Vec<Lease> {
    let mut all: Vec<Lease> = busy.iter().map(|b| b.lease.clone()).collect();
    all.extend(fetching.iter().map(|f| f.lease.clone()));
    all.extend(gating.iter().map(|g| g.lease.clone()));
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

/// Stops what runs in the background, and waits until it is stopped. Aborting a turn
/// task drops its future, and with it the guard that kills `claude`'s process group.
async fn stop(busy: Vec<Busy>, refresher: Option<JoinHandle<()>>) {
    for b in busy {
        stop_turn(b).await;
    }
    if let Some(r) = refresher {
        r.abort();
        let _ = r.await;
    }
}

/// Stops one turn and waits until its task is gone.
async fn stop_turn(b: Busy) {
    b.handle.abort();
    let _ = b.handle.await;
}

/// The first of `ops` to finish, removed from the list; waits for ever when there are none.
/// Nothing is lost if this future is dropped before it finishes.
async fn first_of<T>(ops: &mut Vec<Op<'_, T>>) -> T {
    std::future::poll_fn(|cx| {
        for i in 0..ops.len() {
            if let Poll::Ready(v) = ops[i].as_mut().poll(cx) {
                drop(ops.remove(i));
                return Poll::Ready(v);
            }
        }
        Poll::Pending
    })
    .await
}

/// Every one of `ops`, run side by side, answered in the order given.
async fn all_of<T>(ops: Vec<Op<'_, T>>) -> Vec<T> {
    let mut ops: Vec<(Op<'_, T>, Option<T>)> = ops.into_iter().map(|o| (o, None)).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (op, out) in &mut ops {
            if out.is_none() {
                match op.as_mut().poll(cx) {
                    Poll::Ready(v) => *out = Some(v),
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            return Poll::Pending;
        }
        Poll::Ready(ops.iter_mut().filter_map(|(_, o)| o.take()).collect())
    })
    .await
}

/// The first running turn to end, taken off the list, with how its task ended. Waits for
/// ever when none runs.
async fn next_finished(busy: &mut Vec<Busy>) -> (Busy, Result<Finished, tokio::task::JoinError>) {
    std::future::poll_fn(|cx| {
        for i in 0..busy.len() {
            if let Poll::Ready(r) = Pin::new(&mut busy[i].handle).poll(cx) {
                return Poll::Ready((busy.remove(i), r));
            }
        }
        Poll::Pending
    })
    .await
}

/// The first gate check to end, with its lease and how its task ended. Waits for ever when
/// none runs.
async fn next_checked(
    gating: &mut [Gating],
) -> (String, i64, Result<Gated, tokio::task::JoinError>) {
    std::future::poll_fn(|cx| {
        for g in gating.iter_mut() {
            if let Some(h) = g.check.as_mut()
                && let Poll::Ready(r) = Pin::new(h).poll(cx)
            {
                g.check = None;
                return Poll::Ready((g.lease.lease_id.clone(), g.lease.fencing_token, r));
            }
        }
        Poll::Pending
    })
    .await
}

/// Where the pack's checkout is on this machine: the pack names it relative to the turn's
/// working directory (core: `checkouts/<job_id>`). A path that is absolute or climbs out of
/// the working directory is refused: a pack is data, and the check runs a command there.
fn checkout_in(workdir: &Path, checkout: &str) -> Result<PathBuf, String> {
    let rel = Path::new(checkout);
    if rel.components().all(|c| matches!(c, Component::Normal(_))) {
        Ok(workdir.join(rel))
    } else {
        Err(format!(
            "its checkout path {checkout:?} is not inside the working directory"
        ))
    }
}

/// Why a finished job turn gets no gate check, or `None` when it does: it must have
/// replied, and its pack must name a checkout inside the working directory.
fn no_gate_because(pack: &Pack, f: &Finished, workdir: &Path) -> Option<String> {
    if !matches!(f.outcome.as_str(), "replied" | "unshaped") {
        return Some(format!("the turn ended {}, not with a reply", f.outcome));
    }
    if pack.checkout.is_empty() {
        return Some("its pack names no checkout to check".into());
    }
    checkout_in(workdir, &pack.checkout).err()
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

/// Runs until `shutdown` is set or `cfg.max_beats` heartbeats were answered. Up to
/// `cfg.capacity` turns are held at once, each through the same steps below.
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
/// work. On shutdown every running turn is aborted, its cancellation is recorded as a
/// report, and the outbox is flushed for up to `cfg.flush_timeout`.
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
    let mut capacity = cfg.capacity.clamp(1, MAX_CAPACITY);
    // Whether the last beat declared `build`; the guard's changes are logged once each.
    let mut declared_build: Option<bool> = None;
    let mut busy: Vec<Busy> = Vec::new();
    // Accepts in flight: each answers with its lease when taken, else with its lease id.
    let mut accepting: Vec<Op<'_, Accepted>> = Vec::new();
    // The ids of the leases being accepted, so a lease offered twice is taken once.
    let mut accepting_ids: Vec<String> = Vec::new();
    let mut fetching: Vec<Fetch> = Vec::new();
    // Finished job turns whose gate check runs or whose gate report is being sent.
    let mut gating: Vec<Gating> = Vec::new();
    let mut gate_sends: Vec<Op<'_, Sent>> = Vec::new();
    let mut fetch_ops: Vec<Op<'_, Fetched>> = Vec::new();
    let mut beating: Option<Op<'_, (bool, Result<Beat, OpErr>)>> = None;
    let mut lease_beating: Option<Op<'_, LeaseAnswers>> = None;
    let mut reporting: Option<Op<'_, (String, Result<(), OpErr>)>> = None;
    let mut service_interval: Option<Duration> = None;
    let mut outbox: VecDeque<TurnReport> = VecDeque::new();
    // Delivery attempts since the last success.
    let mut tries: u32 = 0;
    let mut failures: u32 = 0;
    let start = Instant::now();
    let (mut next_beat, mut next_report, mut next_lease_beat) = (start, start, start);
    let mut beat_started = start;
    let mut lease_beat_started = start;
    let interval = |service: Option<Duration>| cfg.interval.or(service).unwrap_or(DEFAULT_INTERVAL);

    loop {
        let held = held_leases(&busy, &fetching, &gating, &outbox);
        let has_held = !held.is_empty();
        let now = Instant::now();
        if lease_beating.is_none() && now >= next_lease_beat && has_held {
            lease_beat_started = now;
            let limit = cfg.op_timeout;
            // Side by side: one lease's slow answer never holds back another's beat.
            let beats: Vec<Op<'_, _>> = held
                .into_iter()
                .map(|l| -> Op<'_, _> {
                    Box::pin(async move {
                        let r = bounded(limit, svc.lease_heartbeat(&l)).await;
                        (l, r)
                    })
                })
                .collect();
            lease_beating = Some(Box::pin(all_of(beats)));
        }
        for f in fetching.iter_mut().filter(|f| !f.asking && now >= f.next) {
            f.asking = true;
            let lease = f.lease.clone();
            let owned = f.owned.as_ref().map(|o| o.checkout.rel.clone());
            let limit = cfg.op_timeout;
            fetch_ops.push(Box::pin(async move {
                let got = async {
                    let pack = bounded(limit, svc.fetch_pack(&lease)).await?;
                    check_pack(&pack, &lease, owned.as_deref())?;
                    Ok(pack)
                }
                .await;
                (lease.lease_id, lease.fencing_token, got)
            }));
        }
        // The next pack fetch due, if any waits for its time.
        let next_fetch = fetching.iter().filter(|f| !f.asking).map(|f| f.next).min();
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
            && busy.is_empty()
            && accepting.is_empty()
            && fetching.is_empty()
            && gating.is_empty()
            && reporting.is_none()
            && (outbox.is_empty() || tries > 0)
        {
            stop(Vec::new(), refresher.take()).await;
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
            let declared = declared_now(&cfg);
            if let Some(c) = declared.capacity {
                let c = usize::try_from(c).unwrap_or(1).clamp(1, MAX_CAPACITY);
                if c != capacity {
                    log(&format!(
                        "capacity is now {c} turns at once (was {capacity}); running turns carry on"
                    ));
                    capacity = c;
                }
            }
            let floor = cfg.build_floor.clone();
            beating = Some(Box::pin(async move {
                let declared = match floor {
                    Some(f) => {
                        let free = settings::free_bytes(&f.volume).await;
                        declared.guarded(free, f.bytes)
                    }
                    None => declared,
                };
                let has_build = declared.kinds.iter().any(|k| k == settings::BUILD);
                (
                    has_build,
                    bounded(limit, svc.heartbeat(&id, &snapshot, &declared)).await,
                )
            }));
        }

        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                drop(reporting.take());
                let running = std::mem::take(&mut busy);
                for b in &running {
                    log(&format!("turn {} cancelled by shutdown", b.pack.turn_id));
                    stats.turns += 1;
                    outbox.push_back(unfinished(b, runner, "failed", "the runner was stopped before the turn finished".into()));
                }
                for g in gating.drain(..) {
                    if let Some(h) = g.check {
                        h.abort();
                        let _ = h.await;
                    }
                    outbox.push_back(g.report);
                }
                drop(gate_sends);
                stop(running, refresher.take()).await;
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
                let (has_build, beat) = beat;
                if declared_build == Some(true) && !has_build {
                    log("the build volume is under its free-space floor: no new build turn until it has room");
                } else if declared_build == Some(false) && has_build {
                    log("the build volume has room again: build turns are taken");
                }
                declared_build = Some(has_build);
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
                            stop(std::mem::take(&mut busy), refresher.take()).await;
                            return (End::Revoked, stats);
                        }
                        for lease in beat.offered {
                            let taken = busy.len() + accepting.len() + fetching.len();
                            let held_already = accepting_ids.contains(&lease.lease_id)
                                || held_leases(&busy, &fetching, &gating, &outbox)
                                    .iter()
                                    .any(|l| l.lease_id == lease.lease_id);
                            if held_already {
                                // Offered again before the service saw the accept.
                            } else if taken >= capacity {
                                log(&format!(
                                    "lease {} left for another runner: {taken} of {capacity} turns are taken",
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
                                accepting_ids.push(lease.lease_id.clone());
                                accepting.push(Box::pin(accept_lease(svc, exec, cfg.workspaces.as_deref(), lease, cfg.op_timeout, log)));
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
                    if !held_leases(&busy, &fetching, &gating, &outbox)
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
                    if let Some(i) = gating.iter().position(|g| is_lease(&g.lease, id, token)) {
                        let g = gating.remove(i);
                        if let Some(h) = g.check {
                            h.abort();
                            let _ = h.await;
                        }
                        if stopped {
                            // Capped from above: the turn did finish, so its reply goes.
                            log(&format!("lease {id} stopped by the service ({reason}): no gate report"));
                            outbox.push_back(g.report);
                            next_report = Instant::now();
                        } else {
                            log(&format!("lease {id} is gone ({reason}): no gate report"));
                        }
                    }
                    if let Some(i) = fetching.iter().position(|f| is_lease(&f.lease, id, token)) {
                        log(&format!("lease {id} ended before its pack came: {reason}"));
                        fetching.remove(i);
                    }
                    if let Some(i) = busy.iter().position(|b| is_lease(&b.lease, id, token)) {
                        let b = busy.remove(i);
                        refresh_wanted = true;
                        stats.turns += 1;
                        if stopped {
                            // Past the wall-clock cap: the turn is aborted (its process
                            // group dies with it) and reported as capped.
                            log(&format!("turn {} stopped by the service: {reason}", b.pack.turn_id));
                            let report = unfinished(&b, runner, "cap_hit", format!("stopped by the service: {reason}"));
                            stop_turn(b).await;
                            outbox.push_back(report);
                            next_report = Instant::now();
                        } else {
                            // Released, reassigned or fenced: the work is no longer ours
                            // and a report would be refused.
                            log(&format!("lease {id} is gone ({reason}): turn {} aborted", b.pack.turn_id));
                            stop_turn(b).await;
                        }
                    }
                    if !stopped {
                        outbox.retain(|r| !is_lease_of(r, id, token));
                    }
                }
            }
            (b, joined) = next_finished(&mut busy) => {
                refresh_wanted = true;
                let finished = joined.unwrap_or_else(|e| {
                    Finished::refused("failed", format!("the turn's task stopped: {e}"))
                });
                stats.turns += 1;
                let report = report_of(&b.lease, &b.pack, runner, &finished).limited();
                log(&report.journal_line());
                // The runner ships a replied turn of a checkout it owns: commit, push
                // `job/<id>`, open or update the pull request; before the check, so the
                // check sees the pushed head.
                let ship = match (&b.owned, cfg.workspaces.as_ref(), &finished.parsed) {
                    (Some(o), Some(w), Some(parsed)) if finished.outcome == "replied" => {
                        let m = workspace::compose(&o.checkout.job_id, &o.document_id, &parsed.reply);
                        // A token close to its end is minted again for the push; one that
                        // cannot be is a ship that fails, said in the log.
                        let access = if o.access.fresh(time::OffsetDateTime::now_utc()) {
                            Ok(o.access.clone())
                        } else {
                            github_access(svc, &b.lease, &o.checkout.repository, cfg.op_timeout).await
                        };
                        Some((Arc::clone(w), o.checkout.clone(), m, access))
                    }
                    (Some(_), Some(_), _) => {
                        log(&format!("lease {}: nothing shipped, the turn ended {}", b.lease.lease_id, finished.outcome));
                        None
                    }
                    _ => None,
                };
                let gate_run = cfg.gate.as_ref().and_then(|g| {
                    let wanted = b.pack.kind == "job" || b.pack.kind == settings::BUILD;
                    let why = no_gate_because(&b.pack, &finished, &g.workdir);
                    if let Some(why) = &why
                        && wanted
                    {
                        log(&format!("lease {}: no gate report, {why}", b.lease.lease_id));
                    }
                    let repo = checkout_in(&g.workdir, &b.pack.checkout).ok()?;
                    (wanted && why.is_none()).then_some((g, repo))
                });
                if gate_run.is_some() || ship.is_some() {
                    let check = gate_run.map(|(g, repo)| {
                        log(&format!("lease {}: checking {} in {}", b.lease.lease_id, g.command, repo.display()));
                        (repo, g.command.clone(), g.timeout, g.env.clone())
                    });
                    let job_id = if b.pack.job_id.is_empty() { b.lease.job_id.clone() } else { b.pack.job_id.clone() };
                    gating.push(Gating {
                        lease: b.lease,
                        job_id,
                        report,
                        check: Some(tokio::spawn(async move {
                            let shipped = match ship {
                                Some((w, p, m, Ok(access))) => Some(w.ship(&p, &m, &access).await),
                                Some((_, _, _, Err(e))) => Some(Err(format!("nothing pushed: {e}"))),
                                None => None,
                            };
                            let checked = match check {
                                Some((repo, command, timeout, env)) => {
                                    Some(gate::check(&repo, &command, timeout, env).await)
                                }
                                None => None,
                            };
                            Gated { shipped, checked }
                        })),
                    });
                    next_lease_beat = Instant::now();
                } else {
                    outbox.push_back(report);
                    next_report = Instant::now();
                }
            }
            (id, token, joined) = next_checked(&mut gating) => {
                let Some(g) = gating.iter_mut().find(|g| is_lease(&g.lease, &id, token)) else { continue };
                let joined = joined.map(|gated| {
                    match gated.shipped {
                        Some(Ok(Some(s))) => {
                            log(&format!("lease {id}: {}", s.evidence()));
                            if !s.withheld.is_empty() {
                                log(&format!(
                                    "lease {id}: not committed, the names look like secrets: {}",
                                    s.withheld.join(", ")
                                ));
                            }
                            // The service takes at most 50 evidence lines.
                            if g.report.evidence.len() < 50 {
                                g.report.evidence.push(s.evidence());
                            }
                        }
                        Some(Ok(None)) => log(&format!("lease {id}: nothing to ship, the branch has no change from main")),
                        Some(Err(e)) => log(&format!("lease {id}: the pull request was not shipped: {e}")),
                        None => {}
                    }
                    gated.checked
                });
                let g = &*g;
                match joined {
                    Ok(None) => {
                        if let Some(i) = gating.iter().position(|g| is_lease(&g.lease, &id, token)) {
                            outbox.push_back(gating.remove(i).report);
                            next_report = Instant::now();
                        }
                    }
                    Ok(Some(Ok(checked))) => {
                        let gr = gate::report_of(
                            Held {
                                job_id: &g.job_id,
                                lease_id: &id,
                                fencing_token: token,
                                runner_id: &runner.runner_id,
                            },
                            &checked,
                        );
                        let limit = cfg.op_timeout;
                        gate_sends.push(Box::pin(async move {
                            let mut tries = 0;
                            let said = loop {
                                tries += 1;
                                match bounded(limit, gate::send(svc, &gr)).await {
                                    Ok(Some(a)) => break format!(
                                        "gate report for {} sent (exit {}, evidence {}, job is now {})",
                                        gr.job_id, gr.exit_code, a.evidence_id, a.stage
                                    ),
                                    Ok(None) => break "this coordinator does not take gate reports yet; nothing was recorded".into(),
                                    Err(e) if e.permanent || tries >= GATE_SEND_TRIES => {
                                        break format!("gate report for {} not sent: {}", gr.job_id, e.msg);
                                    }
                                    Err(_) => tokio::time::sleep(GATE_SEND_PAUSE).await,
                                }
                            };
                            (gr.lease_id.clone(), gr.fencing_token, said)
                        }));
                    }
                    other => {
                        let why = match other {
                            Ok(Some(Err(skip))) => skip.to_string(),
                            Err(e) => format!("the check's task stopped: {e}"),
                            Ok(_) => String::new(),
                        };
                        log(&format!("lease {id}: no gate report was sent: {why}"));
                        if let Some(i) = gating.iter().position(|g| is_lease(&g.lease, &id, token)) {
                            outbox.push_back(gating.remove(i).report);
                            next_report = Instant::now();
                        }
                    }
                }
            }
            (id, token, said) = first_of(&mut gate_sends) => {
                log(&format!("lease {id}: {said}"));
                if let Some(i) = gating.iter().position(|g| is_lease(&g.lease, &id, token)) {
                    outbox.push_back(gating.remove(i).report);
                    next_report = Instant::now();
                }
            }
            taken = first_of(&mut accepting) => {
                match taken {
                    Ok((lease, owned)) => {
                        accepting_ids.retain(|id| *id != lease.lease_id);
                        stats.accepted += 1;
                        fetching.push(Fetch { lease, owned, tries: 0, next: Instant::now(), asking: false });
                        next_lease_beat = Instant::now();
                    }
                    // Not accepted: `accept_lease` logged it and the lease expires.
                    Err(id) => accepting_ids.retain(|a| *a != id),
                }
            }
            (id, token, got) = first_of(&mut fetch_ops) => {
                let Some(i) = fetching.iter().position(|f| is_lease(&f.lease, &id, token)) else { continue };
                let mut f = fetching.remove(i);
                f.asking = false;
                match got {
                    Ok(pack) => {
                        let (task, p) = (exec.clone(), pack.clone());
                        busy.push(Busy {
                            lease: f.lease,
                            pack,
                            owned: f.owned,
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
                            f.next = Instant::now() + interval(service_interval);
                            fetching.push(f);
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
            () = tokio::time::sleep_until(next_fetch.unwrap_or(start)), if next_fetch.is_some() => {}
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
fn check_pack(p: &Pack, lease: &Lease, owned: Option<&str>) -> Result<(), OpErr> {
    let hash_ok = p.pack_sha256.len() == 64 && p.pack_sha256.bytes().all(|b| b.is_ascii_hexdigit());
    if !hash_ok || p.attempt == 0 || (!lease.turn_id.is_empty() && p.turn_id != lease.turn_id) {
        return Err(OpErr {
            msg: "the pack has no valid hash and attempt, or is another turn's".into(),
            permanent: true,
        });
    }
    // The turn is told where its checkout is; it must be the one the runner made.
    if let Some(rel) = owned
        && p.checkout != rel
    {
        return Err(OpErr {
            msg: format!(
                "the pack names the checkout {:?}, the runner made {rel:?}",
                p.checkout
            ),
            permanent: true,
        });
    }
    Ok(())
}

/// How git and gh reach GitHub for `lease` on `repository` (RFC 0120): the installation
/// token connections mints for that one repository, or this host's sign-in only where
/// the platform says our App is not installed there, or the platform is older than the
/// call. Any other refusal (a permission the App lacks, a lease the coordinator did not
/// confirm, a token for another repository) is an error: never a silent fallback. The
/// token never reaches a log line or an error.
async fn github_access<S: Service>(
    svc: &S,
    lease: &Lease,
    repository: &str,
    op_timeout: Duration,
) -> Result<Access, String> {
    match tokio::time::timeout(op_timeout, svc.runner_github_token(lease)).await {
        Ok(Ok(m)) => m.access(repository, time::OffsetDateTime::now_utc()),
        Ok(Err(e)) if e.is_missing() => Ok(Access::host(
            "this platform does not mint runner tokens yet",
        )),
        Ok(Err(e)) => Err(format!("no GitHub token for the lease: {e}")),
        Err(_) => Err("no GitHub token for the lease: timed out".into()),
    }
}

/// `AcceptLease` under the call limit; the lease's id when it was not accepted (the lease
/// is left to expire and goes to another runner).
///
/// A job lease whose job names a repository gets its checkout first
/// (`checkouts/<job_id>` on `job/<job_id>`, [`Workspaces::prepare`]); a checkout that
/// cannot be made means the lease is not accepted. A coordinator without `GetJob` keeps
/// the old behaviour.
async fn accept_lease<S: Service, X: Exec>(
    svc: &S,
    exec: &Arc<X>,
    workspaces: Option<&Workspaces>,
    lease: Lease,
    op_timeout: Duration,
    log: Log<'_>,
) -> Result<(Lease, Option<Owned>), String> {
    let refuse = |why: &str| {
        log(&format!("lease {} not accepted: {why}", lease.lease_id));
        lease.lease_id.clone()
    };
    let mut owned = None;
    if let Some(w) = workspaces
        && !lease.job_id.is_empty()
        && lease.turn_id.is_empty()
    {
        let job = match tokio::time::timeout(op_timeout, svc.get_job(&lease.job_id)).await {
            Ok(Ok(j)) => Some(j),
            Ok(Err(e)) if e.is_missing() => None,
            Ok(Err(e)) => return Err(refuse(&format!("its job could not be read: {e}"))),
            Err(_) => return Err(refuse("its job could not be read: timed out")),
        };
        if let Some(job) = job.filter(|j| !j.repository.is_empty()) {
            let access = match github_access(svc, &lease, &job.repository, op_timeout).await {
                Ok(a) => a,
                Err(e) => return Err(refuse(&e)),
            };
            log(&format!(
                "lease {}: git reaches {} through {}",
                lease.lease_id,
                job.repository,
                access.describe()
            ));
            let made = tokio::time::timeout(
                PREPARE_TIMEOUT,
                w.prepare(&lease.job_id, &job.repository, &access),
            )
            .await
            .unwrap_or_else(|_| Err("making the checkout took too long".into()));
            match made {
                Ok(p) => {
                    log(&format!(
                        "lease {}: {} checkout {} on {} ({})",
                        lease.lease_id,
                        if p.reused { "reusing" } else { "made" },
                        p.rel,
                        p.branch,
                        p.repository
                    ));
                    owned = Some(Owned {
                        checkout: p,
                        document_id: job.document_id,
                        access,
                    });
                }
                Err(e) => return Err(refuse(&format!("its checkout could not be made: {e}"))),
            }
        }
    }
    let facts = AcceptFacts {
        // A job's checkout is a worktree this runner made for the job (`job/<id>`, never
        // a main checkout; `prepare` verified both); a coordinate turn has no worktree
        // and runs in the runner's own empty directory.
        worktree_fresh: true,
        worktree_is_main_checkout: false,
        env_scrubbed: exec.env_is_clean(),
        // The argument builder refuses a turn with no `--max-turns`.
        caps_set: true,
    };
    match bounded(op_timeout, svc.accept(&lease, facts)).await {
        Ok(()) => Ok((lease, owned)),
        Err(e) => Err(refuse(&e.msg)),
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
    use crate::runner::service::{
        Challenge, GateAnswer, GateReport, JobInfo, ProviderState, Registration, ServiceError,
    };
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
        /// What each heartbeat declared, in order.
        declared: Mutex<Vec<Declared>>,
        /// Every gate report sent.
        gate_reports: Mutex<Vec<GateReport>>,
        /// Lease heartbeats are refused for good once this file exists.
        release_when: Option<std::path::PathBuf>,
        /// What `GetJob` answers; `None` is an older coordinator without it.
        job: Option<JobInfo>,
        /// `GetJob` fails, transiently.
        job_down: bool,
        /// What `MintRunnerGithubToken` answers; `None` is an older platform without it.
        minted: Option<crate::runner::access::Minted>,
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
        async fn submit_gate_report(&self, r: &GateReport) -> Result<GateAnswer, ServiceError> {
            self.note(format!("gate_report {}", r.job_id));
            self.gate_reports.lock().unwrap().push(r.clone());
            Ok(GateAnswer {
                evidence_id: "ev_1".into(),
                duplicate: false,
                stage: "checked".into(),
            })
        }
        async fn lease_heartbeat(&self, l: &Lease) -> Result<LeaseBeat, ServiceError> {
            self.note(format!("lease_heartbeat {}", l.lease_id));
            if self.release_when.as_ref().is_some_and(|f| f.exists()) {
                return Err(ServiceError::Refused("the lease was released"));
            }
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
        async fn heartbeat(
            &self,
            id: &str,
            p: &[ProviderState],
            d: &Declared,
        ) -> Result<Beat, ServiceError> {
            self.note(format!("heartbeat {id} providers={}", p.len()));
            self.declared.lock().unwrap().push(d.clone());
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
        async fn runner_github_token(
            &self,
            l: &Lease,
        ) -> Result<crate::runner::access::Minted, ServiceError> {
            self.note(format!("runner_token {}", l.lease_id));
            self.minted.clone().ok_or_else(|| {
                let raw = inorbithr::RawResponse::for_tests(
                    404,
                    inorbithr::Headers::new([]),
                    br#"{"code":"not_found","error":"no such route","details":[]}"#.to_vec(),
                );
                ServiceError::Api(inorbithr::Error::Api(Box::new(inorbithr::ApiError::parse(
                    raw,
                ))))
            })
        }
        async fn get_job(&self, job_id: &str) -> Result<JobInfo, ServiceError> {
            self.note(format!("job {job_id}"));
            if self.job_down {
                return Err(down());
            }
            self.job.clone().ok_or_else(|| {
                let raw = inorbithr::RawResponse::for_tests(
                    404,
                    inorbithr::Headers::new([]),
                    br#"{"code":"not_found","error":"no such route","details":[]}"#.to_vec(),
                );
                ServiceError::Api(inorbithr::Error::Api(Box::new(inorbithr::ApiError::parse(
                    raw,
                ))))
            })
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
        /// The same, for this turn only.
        hang_turn: Option<(String, Arc<std::sync::atomic::AtomicBool>)>,
        /// This turn takes this long instead of 30 ms.
        slow_turn: Option<(String, Duration)>,
        /// Every turn takes this long instead of 30 ms.
        turn_time: Option<Duration>,
        /// This turn's task panics.
        crash_turn: Option<String>,
        /// The turn's reply has a `say` over the service's limit.
        big_say: bool,
        /// The turn writes `made.txt` in this directory and replies with a pull request.
        writes_in: Option<PathBuf>,
        asked: std::sync::atomic::AtomicU32,
    }
    impl Exec for FakeExec {
        fn execute(&self, pack: Pack) -> Pin<Box<dyn Future<Output = Finished> + Send>> {
            let gate = self.gate.clone();
            // Made before the future first runs: a stop may come before its first poll.
            let hang = self
                .hang
                .clone()
                .or_else(|| {
                    self.hang_turn
                        .as_ref()
                        .filter(|(id, _)| *id == pack.turn_id)
                        .map(|(_, f)| f.clone())
                })
                .map(Dropped);
            let big_say = self.big_say;
            let writes_in = self.writes_in.clone();
            let slow = self
                .slow_turn
                .as_ref()
                .filter(|(id, _)| *id == pack.turn_id)
                .map(|(_, d)| *d);
            let all = self.turn_time;
            let crash = self.crash_turn.as_deref() == Some(pack.turn_id.as_str());
            Box::pin(async move {
                if let Some(_dropped) = hang {
                    std::future::pending::<()>().await;
                }
                assert!(!crash, "this turn crashes");
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
                    None => {
                        tokio::time::sleep(slow.or(all).unwrap_or(Duration::from_millis(30))).await;
                    }
                }
                if let Some(dir) = &writes_in {
                    std::fs::write(dir.join("made.txt"), "made\n").unwrap();
                }
                let parsed =
                    (big_say || writes_in.is_some()).then(|| crate::runner::claude::Parsed {
                        outcome: crate::runner::claude::Outcome::Replied,
                        reply: crate::runner::claude::Reply {
                            say: if big_say {
                                "x".repeat(20_001)
                            } else {
                                "Made it.".into()
                            },
                            state: String::new(),
                            actions: vec![
                                serde_json::json!({"kind": "open_pull_request", "payload": {
                            "title": "feat(app): make it", "implements": "RFC 0147",
                            "prd": "0152", "adr": "0141"}}),
                            ],
                            evidence: vec![serde_json::json!("made.txt written")],
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
            capacity: 1,
            declared: None,
            build_floor: None,
            gate: None,
            workspaces: None,
        }
    }

    fn cfg_cap(max_beats: u64, capacity: usize) -> LoopConfig {
        LoopConfig {
            capacity,
            ..cfg(max_beats)
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

    /// [`go_with`], keeping every log line.
    async fn go_logged(fake: &Fake, exec: FakeExec, c: LoopConfig) -> (Stats, Vec<String>) {
        let (_tx, rx) = watch::channel(false);
        let lines = Mutex::new(Vec::new());
        let log = |l: &str| lines.lock().unwrap().push(l.to_owned());
        let (_, stats) = tokio::time::timeout(
            Duration::from_secs(10),
            run_loop(fake, &Arc::new(exec), &runner(), c, rx, &log),
        )
        .await
        .expect("the loop hung");
        (stats, lines.into_inner().unwrap())
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

    fn pos(calls: &[String], what: &str) -> usize {
        calls
            .iter()
            .position(|c| c == what)
            .unwrap_or_else(|| panic!("no call {what}: {calls:?}"))
    }

    #[tokio::test]
    async fn capacity_two_runs_two_leases_at_once() {
        // Both turns wait at a barrier only the other can meet: one at a time, the
        // first times out and fails.
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        let exec = FakeExec {
            gate: Some(Arc::new(tokio::sync::Barrier::new(2))),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg_cap(8, 2)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (2, 2, 2));
        let r = fake.reports.lock().unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.outcome == "replied"), "{r:?}");
    }

    fn declaring(capacity: Option<u32>, kinds: &[&str]) -> (watch::Sender<Declared>, LoopConfig) {
        let (tx, rx) = watch::channel(Declared {
            capacity,
            version: "9.9.9".into(),
            kinds: kinds.iter().map(|k| (*k).to_owned()).collect(),
        });
        let c = LoopConfig {
            declared: Some(rx),
            ..cfg(8)
        };
        (tx, c)
    }

    #[tokio::test]
    async fn a_declared_capacity_takes_effect_on_the_first_beat() {
        // Registered with 1; the config declares 2. Both turns wait at a barrier only
        // the other can meet, so they pass only when the second lease of the very first
        // offer is taken beside the first.
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        let exec = FakeExec {
            gate: Some(Arc::new(tokio::sync::Barrier::new(2))),
            ..FakeExec::default()
        };
        let (_tx, c) = declaring(Some(2), &["coordinate", "build"]);
        let (_, stats) = go_with(&fake, exec, c).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (2, 2, 2));
        assert!(
            fake.reports
                .lock()
                .unwrap()
                .iter()
                .all(|x| x.outcome == "replied")
        );
        let d = fake.declared.lock().unwrap();
        assert_eq!(d[0].capacity, Some(2));
        assert_eq!(d[0].version, "9.9.9");
        assert_eq!(d[0].kinds, ["coordinate", "build"]);
    }

    #[tokio::test]
    async fn lowering_the_capacity_never_stops_a_running_turn() {
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        let exec = FakeExec {
            turn_time: Some(Duration::from_millis(150)),
            ..FakeExec::default()
        };
        let (tx, c) = declaring(Some(2), &["coordinate"]);
        let lower = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            tx.send_modify(|d| d.capacity = Some(1));
            tx
        });
        let c = LoopConfig {
            max_beats: Some(30),
            ..c
        };
        let (_, stats) = go_with(&fake, exec, c).await;
        let _ = lower.await;
        assert_eq!((stats.accepted, stats.reported), (2, 2));
        assert!(
            fake.reports
                .lock()
                .unwrap()
                .iter()
                .all(|x| x.outcome == "replied")
        );
        let d = fake.declared.lock().unwrap();
        assert_eq!(d.last().and_then(|x| x.capacity), Some(1), "{d:?}");
    }

    #[tokio::test]
    async fn under_the_build_floor_the_beat_declares_no_build() {
        let fake = Fake::default();
        let c = LoopConfig {
            // No volume has this much free: the guard always holds.
            build_floor: Some(BuildFloor {
                volume: std::env::temp_dir(),
                bytes: u64::MAX,
            }),
            max_beats: Some(2),
            ..declaring(None, &["coordinate", "build"]).1
        };
        let (_, stats) = go_with(&fake, FakeExec::default(), c).await;
        assert_eq!(stats.beats, 2);
        let d = fake.declared.lock().unwrap();
        assert!(d.iter().all(|x| x.kinds == ["coordinate"]), "{d:?}");
        assert!(d.iter().all(|x| x.capacity.is_none()), "{d:?}");
    }

    #[tokio::test]
    async fn one_slot_still_runs_one_turn_and_leaves_the_second_lease() {
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        let exec = FakeExec {
            turn_time: Some(Duration::from_millis(200)),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg(40)).await;
        assert_eq!((stats.accepted, stats.reported), (1, 1));
        assert_eq!(calls_of(&fake, "accept lease_2"), 0);
    }

    #[tokio::test]
    async fn a_third_lease_is_left_to_expire_while_both_slots_are_taken() {
        let fake = Fake::default();
        offer(
            &fake,
            &[
                lease("lease_1", 1),
                lease("lease_2", 2),
                lease("lease_3", 3),
            ],
        );
        let exec = FakeExec {
            turn_time: Some(Duration::from_millis(300)),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg_cap(60, 2)).await;
        assert_eq!((stats.accepted, stats.reported), (2, 2));
        assert_eq!(calls_of(&fake, "accept lease_3"), 0);
        assert_eq!(calls_of(&fake, "pack lease_3"), 0);
        // Both held leases were kept alive on their own.
        let calls = fake.calls.lock().unwrap().clone();
        for id in ["lease_1", "lease_2"] {
            assert!(
                calls.iter().any(|c| *c == format!("lease_heartbeat {id}")),
                "{id} was not kept alive: {calls:?}"
            );
        }
        let mut turns: Vec<_> = fake
            .reports
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.turn_id.clone())
            .collect();
        turns.sort();
        assert_eq!(turns, ["turn_1", "turn_2"]);
    }

    #[tokio::test]
    async fn a_lease_offered_again_after_a_slot_frees_is_taken() {
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        // Beat 2 offers the third while both turns run (150 ms): declined. Beat 25
        // (about 250 ms) offers it again: both are done and reported.
        offer(&fake, &[lease("lease_3", 3)]);
        for _ in 0..22 {
            offer(&fake, &[]);
        }
        offer(&fake, &[lease("lease_3", 3)]);
        let exec = FakeExec {
            turn_time: Some(Duration::from_millis(150)),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg_cap(60, 2)).await;
        assert_eq!((stats.accepted, stats.reported), (3, 3));
        assert_eq!(calls_of(&fake, "accept lease_3"), 1);
        let calls = fake.calls.lock().unwrap().clone();
        assert!(
            pos(&calls, "accept lease_3 token=3") > pos(&calls, "report lease_1:1:turn_1:1"),
            "the third lease was taken before a slot was free: {calls:?}"
        );
    }

    #[tokio::test]
    async fn a_lease_offered_twice_is_taken_once() {
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_1", 1)]);
        offer(&fake, &[lease("lease_1", 1)]);
        let exec = FakeExec {
            turn_time: Some(Duration::from_millis(100)),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg_cap(20, 2)).await;
        assert_eq!((stats.accepted, stats.reported), (1, 1));
        assert_eq!(calls_of(&fake, "accept lease_1"), 1);
    }

    #[tokio::test]
    async fn one_turn_crashing_leaves_the_other_running_and_reporting() {
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        // turn_1 panics at once; turn_2 is still running (200 ms) when it does.
        let exec = FakeExec {
            crash_turn: Some("turn_1".into()),
            slow_turn: Some(("turn_2".into(), Duration::from_millis(200))),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg_cap(40, 2)).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (2, 2, 2));
        let r = fake.reports.lock().unwrap();
        let by = |t: &str| r.iter().find(|x| x.turn_id == t).map(|x| x.outcome.clone());
        assert_eq!(by("turn_1").as_deref(), Some("failed"));
        assert_eq!(by("turn_2").as_deref(), Some("replied"));
        let crashed = r.iter().find(|x| x.turn_id == "turn_1").unwrap();
        assert!(crashed.detail.contains("stopped"), "{}", crashed.detail);
    }

    #[tokio::test]
    async fn a_lease_the_service_released_stops_only_its_own_turn() {
        // lease_1 is refused for good once its pack is out; lease_2 carries on.
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake {
            refuse_lease: Some("lease_1".into()),
            ..Fake::default()
        };
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        let exec = FakeExec {
            slow_turn: Some(("turn_2".into(), Duration::from_millis(250))),
            hang_turn: Some(("turn_1".into(), dropped.clone())),
            ..FakeExec::default()
        };
        let (_, stats) = go_with(&fake, exec, cfg_cap(60, 2)).await;
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "turn_1 kept running"
        );
        assert_eq!(stats.reported, 1);
        let r = fake.reports.lock().unwrap();
        assert_eq!(
            (r[0].turn_id.as_str(), r[0].outcome.as_str()),
            ("turn_2", "replied")
        );
    }

    #[tokio::test]
    async fn shutdown_records_every_cancelled_turn() {
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake::default();
        offer(&fake, &[lease("lease_1", 1), lease("lease_2", 2)]);
        let exec = Arc::new(FakeExec {
            hang: Some(dropped.clone()),
            ..FakeExec::default()
        });
        let (tx, rx) = watch::channel(false);
        let c = LoopConfig {
            max_beats: None,
            ..cfg_cap(0, 2)
        };
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(true);
            tx
        });
        let (end, stats) = run_loop(&fake, &exec, &runner(), c, rx, &|_| {}).await;
        let _ = t.await;
        assert_eq!(end, End::Shutdown);
        assert_eq!((stats.turns, stats.reported), (2, 2));
        assert!(
            fake.reports
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.outcome == "failed")
        );
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

    fn gate_cfg(command: &str, workdir: &Path) -> GateCheck {
        GateCheck {
            workdir: workdir.to_owned(),
            command: command.into(),
            timeout: Duration::from_secs(20),
            env: std::env::vars_os().collect(),
        }
    }

    /// A job pack whose checkout is `work` under the working directory, as core names it.
    fn job_pack() -> Pack {
        Pack {
            kind: "job".into(),
            job_id: "job_1".into(),
            checkout: "work".into(),
            ..good_pack()
        }
    }

    #[tokio::test]
    async fn a_finished_job_turn_sends_one_gate_report_with_its_lease_before_its_own_report() {
        let (root, _origin, _work) = gate::tests::repos();
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(job_pack());
        let c = LoopConfig {
            gate: Some(gate_cfg("exit 3", root.path())),
            ..cfg(6)
        };
        let (_, stats) = go(&fake, c).await;
        assert_eq!((stats.turns, stats.reported), (1, 1));
        let g = fake.gate_reports.lock().unwrap().clone();
        assert_eq!(g.len(), 1, "exactly one gate report");
        assert_eq!(
            (
                g[0].job_id.as_str(),
                g[0].lease_id.as_str(),
                g[0].fencing_token
            ),
            ("job_1", "lease_1", 7)
        );
        assert_eq!(g[0].runner_id, "run_1");
        // A failing check is sent as it is.
        assert_eq!((g[0].command.as_str(), g[0].exit_code), ("exit 3", 3));
        let calls = fake.calls.lock().unwrap().clone();
        let gate_at = calls.iter().position(|c| c == "gate_report job_1").unwrap();
        let report_at = calls.iter().position(|c| c.starts_with("report ")).unwrap();
        assert!(
            gate_at < report_at,
            "the lease is held until the gate is sent"
        );
    }

    #[tokio::test]
    async fn a_refused_turn_or_one_with_no_checkout_sends_no_gate_report() {
        let (root, _origin, _work) = gate::tests::repos();
        // The turn crashes: no reply, no check.
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(job_pack());
        let (_tx, rx) = watch::channel(false);
        let exec = Arc::new(FakeExec {
            crash_turn: Some("turn_1".into()),
            ..FakeExec::default()
        });
        let logged = Mutex::new(Vec::<String>::new());
        let c = LoopConfig {
            gate: Some(gate_cfg("true", root.path())),
            ..cfg(4)
        };
        run_loop(&fake, &exec, &runner(), c, rx, &|m| {
            logged.lock().unwrap().push(m.to_owned());
        })
        .await;
        assert!(fake.gate_reports.lock().unwrap().is_empty());
        assert!(
            logged
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains("no gate report"))
        );
        // A pack with no checkout path: the turn is reported, nothing is checked.
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(Pack {
            kind: "job".into(),
            ..good_pack()
        });
        let c = LoopConfig {
            gate: Some(gate_cfg("true", root.path())),
            ..cfg(4)
        };
        let (_, stats) = go(&fake, c).await;
        assert_eq!(stats.reported, 1);
        assert!(fake.gate_reports.lock().unwrap().is_empty());
    }

    #[test]
    fn a_checkout_is_found_under_the_working_directory_and_nowhere_else() {
        let w = Path::new("/runner/coordinate");
        assert_eq!(
            checkout_in(w, "checkouts/job_1"),
            Ok(PathBuf::from("/runner/coordinate/checkouts/job_1"))
        );
        for bad in ["/etc", "../x", "checkouts/../../x", "./.."] {
            assert!(checkout_in(w, bad).is_err(), "{bad} must be refused");
        }
    }

    #[tokio::test]
    async fn a_pack_whose_checkout_climbs_out_of_the_working_directory_sends_no_gate_report() {
        let (root, _origin, _work) = gate::tests::repos();
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(Pack {
            checkout: "../work".into(),
            ..job_pack()
        });
        let inner = root.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let c = LoopConfig {
            gate: Some(gate_cfg("true", &inner)),
            ..cfg(4)
        };
        let (_, stats) = go(&fake, c).await;
        assert_eq!(stats.reported, 1);
        assert!(fake.gate_reports.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_lease_released_during_the_check_sends_no_gate_report() {
        let (root, _origin, _work) = gate::tests::repos();
        let flag = root.path().join("running");
        let fake = Fake {
            release_when: Some(flag.clone()),
            ..Fake::default()
        };
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 7)]));
        *fake.pack.lock().unwrap() = Some(job_pack());
        // The check says it has started, then runs long: the lease is gone meanwhile.
        let c = LoopConfig {
            gate: Some(gate_cfg(
                &format!("touch {} && sleep 20", flag.display()),
                root.path(),
            )),
            ..cfg(6)
        };
        let started = std::time::Instant::now();
        let (_, stats) = go(&fake, c).await;
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the check was stopped"
        );
        assert!(fake.gate_reports.lock().unwrap().is_empty());
        assert_eq!(stats.reported, 0, "a gone lease reports nothing");
    }

    /// A job lease as core offers it: a job, no turn yet.
    fn job_lease(id: &str, token: i64) -> Lease {
        Lease {
            lease_id: id.into(),
            job_id: "job_1".into(),
            turn_id: String::new(),
            fencing_token: token,
        }
    }

    #[tokio::test]
    async fn a_job_lease_gets_its_checkout_before_the_accept_and_ships_its_reply_as_a_pull_request()
    {
        use crate::runner::workspace::tests::{FakeForge, identity, place, spaces};
        let p = place();
        let forge = Arc::new(FakeForge::default());
        let spaced = spaces(&p, forge.clone());
        // The mirror, made once, with the test's commit identity in its own config.
        spaced
            .prepare("job_0", "acme/app", &Access::host("a test"))
            .await
            .unwrap();
        identity(&p);
        let fake = Fake {
            job: Some(JobInfo {
                repository: "acme/app".into(),
                document_id: "ldoc_1".into(),
            }),
            minted: Some(crate::runner::access::Minted::token(
                crate::runner::access::tests::FAKE,
                "2099-01-01T00:00:00Z",
                "acme/app",
            )),
            ..Fake::default()
        };
        offer(&fake, &[job_lease("lease_1", 7)]);
        *fake.pack.lock().unwrap() = Some(Pack {
            kind: "job".into(),
            job_id: "job_1".into(),
            checkout: "checkouts/job_1".into(),
            ..good_pack()
        });
        let checkout = p.workdir.join("checkouts/job_1");
        let exec = FakeExec {
            writes_in: Some(checkout.clone()),
            ..FakeExec::default()
        };
        let c = LoopConfig {
            gate: Some(gate_cfg("exit 0", &p.workdir)),
            workspaces: Some(Arc::new(spaced)),
            ..cfg(8)
        };
        let (stats, lines) = go_logged(&fake, exec, c).await;
        assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 1, 1));
        // The lease's own token, minted before the checkout, given to gh for the pull
        // request, and never in a log line.
        let fake_token = crate::runner::access::tests::FAKE;
        assert!(lines.iter().all(|l| !l.contains(fake_token)), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("the lease's GitHub App token")),
            "{lines:?}"
        );
        assert_eq!(*forge.tokens.lock().unwrap(), [true]);
        let calls = fake.calls.lock().unwrap().clone();
        let mint_at = calls
            .iter()
            .position(|c| c == "runner_token lease_1")
            .unwrap();
        let job_at = calls.iter().position(|c| c == "job job_1").unwrap();
        assert!(job_at < mint_at, "the job names the repository first");
        let accept_at = calls
            .iter()
            .position(|c| c.starts_with("accept lease_1"))
            .unwrap();
        assert!(job_at < accept_at, "the checkout is made before the accept");
        assert!(mint_at < accept_at, "the token is minted before the accept");
        // Pushed on the job's branch, never on main, with the turn's file.
        let head = gate::tests::sh(&p.origin, &["rev-parse", "refs/heads/job/job_1"]);
        let files = gate::tests::sh(&p.origin, &["ls-tree", "--name-only", &head]);
        assert!(files.lines().any(|f| f == "made.txt"), "{files}");
        let asked = forge.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].title, "feat(app): make it");
        assert!(
            asked[0]
                .body
                .contains("Implements: RFC 0147\nPRD: 0152\nADR: 0141")
        );
        assert!(asked[0].body.contains("Document: ldoc_1"));
        // The check ran on the pushed head and passed.
        let g = fake.gate_reports.lock().unwrap().clone();
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].head.as_str(), g[0].exit_code), (head.as_str(), 0));
        // The turn's report names the pull request.
        let r = fake.reports.lock().unwrap().clone();
        assert!(
            r[0].evidence
                .iter()
                .any(|e| e
                    == &format!("runner: pull request acme/app#7 opened on job/job_1 at {head}")),
            "{:?}",
            r[0].evidence
        );
    }

    #[tokio::test]
    async fn a_job_lease_whose_checkout_cannot_be_made_is_not_accepted() {
        use crate::runner::workspace::tests::{FakeForge, place, spaces};
        let p = place();
        for fake in [
            // The origin has no such repository.
            Fake {
                job: Some(JobInfo {
                    repository: "acme/missing".into(),
                    document_id: String::new(),
                }),
                ..Fake::default()
            },
            // The job could not be read.
            Fake {
                job_down: true,
                ..Fake::default()
            },
        ] {
            offer(&fake, &[job_lease("lease_1", 7)]);
            let c = LoopConfig {
                workspaces: Some(Arc::new(spaces(&p, Arc::new(FakeForge::default())))),
                ..cfg(4)
            };
            let (_, stats) = go(&fake, c).await;
            assert_eq!((stats.accepted, stats.turns), (0, 0));
            assert_eq!(calls_of(&fake, "accept "), 0);
            assert_eq!(calls_of(&fake, "job job_1"), 1);
        }
        assert!(!p.workdir.join("checkouts/job_1").exists());
    }

    #[tokio::test]
    async fn a_coordinator_without_get_job_or_a_job_without_a_repository_is_accepted_as_before() {
        use crate::runner::workspace::tests::{FakeForge, place, spaces};
        let p = place();
        for fake in [
            Fake::default(),
            Fake {
                job: Some(JobInfo::default()),
                ..Fake::default()
            },
        ] {
            offer(&fake, &[job_lease("lease_1", 7)]);
            let c = LoopConfig {
                workspaces: Some(Arc::new(spaces(&p, Arc::new(FakeForge::default())))),
                ..cfg(6)
            };
            let (_, stats) = go(&fake, c).await;
            assert_eq!((stats.accepted, stats.turns, stats.reported), (1, 1, 1));
        }
        assert!(!p.workdir.join("checkouts").exists());
    }
    #[tokio::test]
    async fn a_lease_without_its_own_token_uses_the_host_only_where_the_app_is_not_installed() {
        use crate::runner::access::Minted;
        use crate::runner::access::tests::FAKE;
        use crate::runner::workspace::tests::{FakeForge, place, spaces};
        let job = || {
            Some(JobInfo {
                repository: "acme/app".into(),
                document_id: String::new(),
            })
        };
        // Refused: the App lacks a permission, or the token is for another repository.
        // Nothing is prepared, nothing accepted, the host's sign-in is never tried.
        for minted in [
            Minted::refused("needs_permission", "contents: write"),
            Minted::refused("unavailable", "GitHub did not answer"),
            Minted::token(FAKE, "2099-01-01T00:00:00Z", "acme/other"),
        ] {
            let p = place();
            let fake = Fake {
                job: job(),
                minted: Some(minted),
                ..Fake::default()
            };
            offer(&fake, &[job_lease("lease_1", 7)]);
            let c = LoopConfig {
                workspaces: Some(Arc::new(spaces(&p, Arc::new(FakeForge::default())))),
                ..cfg(4)
            };
            let (stats, lines) = go_logged(&fake, FakeExec::default(), c).await;
            assert_eq!((stats.accepted, stats.turns), (0, 0));
            assert_eq!(calls_of(&fake, "accept "), 0);
            assert!(!p.workdir.join("checkouts/job_1").exists());
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("lease lease_1 not accepted")),
                "{lines:?}"
            );
            assert!(lines.iter().all(|l| !l.contains(FAKE)), "{lines:?}");
        }
        // Not installed (or a platform older than the call): the host's sign-in, said.
        for (minted, why) in [
            (
                Some(Minted::refused(
                    "not_installed",
                    "this account has not installed our GitHub App",
                )),
                "no GitHub App installation",
            ),
            (None, "does not mint runner tokens yet"),
        ] {
            let p = place();
            let fake = Fake {
                job: job(),
                minted,
                ..Fake::default()
            };
            offer(&fake, &[job_lease("lease_1", 7)]);
            let c = LoopConfig {
                workspaces: Some(Arc::new(spaces(&p, Arc::new(FakeForge::default())))),
                ..cfg(4)
            };
            let (stats, lines) = go_logged(&fake, FakeExec::default(), c).await;
            assert_eq!(stats.accepted, 1, "{lines:?}");
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("this host's gh sign-in") && l.contains(why)),
                "{lines:?}"
            );
        }
    }
}
