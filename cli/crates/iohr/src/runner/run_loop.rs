//! The runner's loop (RFC 0120, "Leases and heartbeats"): heartbeat on an interval,
//! accept an offered lease, run its turn while the heartbeats carry on, report the reply.
//!
//! One turn at a time (capacity 1 in this version): a lease offered while a turn runs is
//! left to expire and goes to another runner of the same person.

use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::exec::{Exec, Finished};
use super::service::{AcceptFacts, Lease, Pack, ProviderState, RunnerInfo, Service, TurnReport};

/// The RFC's runner heartbeat. The service's proto does not carry an interval yet, so
/// this is the default and `--interval` changes it.
pub(crate) const DEFAULT_INTERVAL: Duration = Duration::from_secs(20);
/// Reconnect backoff bounds (RFC: 1 s to 60 s, with jitter).
pub(crate) const BACKOFF_MIN: Duration = Duration::from_secs(1);
pub(crate) const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Providers are asked again after this many beats, and after every turn.
const PROVIDER_EVERY: u64 = 15;

#[derive(Debug, Clone, Copy)]
pub(crate) struct LoopConfig {
    pub(crate) interval: Duration,
    pub(crate) backoff_min: Duration,
    pub(crate) backoff_max: Duration,
    /// Stop after this many answered heartbeats (scripts and tests).
    pub(crate) max_beats: Option<u64>,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_INTERVAL,
            backoff_min: BACKOFF_MIN,
            backoff_max: BACKOFF_MAX,
            max_beats: None,
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

/// Runs until `shutdown` is set or `cfg.max_beats` heartbeats were answered.
pub(crate) async fn run_loop<S: Service, X: Exec>(
    svc: &S,
    exec: &std::sync::Arc<X>,
    runner: &RunnerInfo,
    cfg: LoopConfig,
    mut shutdown: watch::Receiver<bool>,
    log: Log<'_>,
) -> (End, Stats) {
    let mut stats = Stats::default();
    let mut providers: Vec<ProviderState> = exec.providers().await;
    let mut tick = tokio::time::interval(cfg.interval);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut busy: Option<Busy> = None;
    let mut failures: u32 = 0;
    let mut refresh_providers = false;

    loop {
        // A pending delay after a failure, or the next tick, or a turn ending.
        let wait = if failures == 0 {
            None
        } else {
            Some(backoff(
                failures,
                cfg.backoff_min,
                cfg.backoff_max,
                jitter(),
            ))
        };
        tokio::select! {
            biased;
            _ = shutdown.changed() => return (End::Shutdown, stats),
            joined = async {
                match busy.as_mut() {
                    Some(b) => (&mut b.handle).await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(b) = busy.take() else { continue };
                refresh_providers = true;
                let finished = joined.unwrap_or_else(|e| Finished {
                    outcome: "failed".into(),
                    parsed: None,
                    resumed: false,
                    session_id: String::new(),
                    detail: format!("the turn's task stopped: {e}"),
                    provider_version: String::new(),
                });
                stats.turns += 1;
                let report = report_of(&b.lease, &b.pack, runner, &finished);
                log(&format!("turn {} ended: {}", b.pack.turn_id, report.outcome));
                match svc.report_turn(&report).await {
                    Ok(()) => stats.reported += 1,
                    Err(e) => log(&format!("the reply was not delivered: {e}")),
                }
                continue;
            }
            () = async {
                match wait {
                    Some(d) => tokio::time::sleep(d).await,
                    None => { tick.tick().await; }
                }
            } => {}
        }

        if refresh_providers || stats.beats % PROVIDER_EVERY == 0 && stats.beats > 0 {
            providers = exec.providers().await;
            refresh_providers = false;
        }
        match svc.heartbeat(&runner.runner_id, &providers).await {
            Err(e) => {
                failures = failures.saturating_add(1);
                stats.failures += 1;
                log(&format!("heartbeat failed ({failures} in a row): {e}"));
            }
            Ok(beat) => {
                failures = 0;
                stats.beats += 1;
                if beat.runner.status == "revoked" {
                    log("the service revoked this runner");
                    return (End::Revoked, stats);
                }
                for lease in beat.offered {
                    if busy.is_some() {
                        log(&format!(
                            "lease {} left for another runner: a turn is running",
                            lease.lease_id
                        ));
                        continue;
                    }
                    busy = take(svc, exec, lease, log, &mut stats).await;
                }
                if cfg.max_beats.is_some_and(|m| stats.beats >= m) && busy.is_none() {
                    return (End::MaxBeats, stats);
                }
            }
        }
    }
}

/// Accepts a lease, gets its pack and starts the turn.
async fn take<S: Service, X: Exec>(
    svc: &S,
    exec: &std::sync::Arc<X>,
    lease: Lease,
    log: Log<'_>,
    stats: &mut Stats,
) -> Option<Busy> {
    let facts = AcceptFacts {
        // A coordinate turn has no worktree: it runs in the runner's own empty directory.
        worktree_fresh: true,
        worktree_is_main_checkout: false,
        env_scrubbed: exec.env_is_clean(),
        // The argument builder refuses a turn with no `--max-turns`.
        caps_set: true,
    };
    if let Err(e) = svc.accept(&lease, facts).await {
        log(&format!("lease {} not accepted: {e}", lease.lease_id));
        return None;
    }
    stats.accepted += 1;
    let pack = match svc.fetch_pack(&lease).await {
        Ok(p) => p,
        Err(e) => {
            log(&format!(
                "lease {} accepted but no turn to run: {e}",
                lease.lease_id
            ));
            return None;
        }
    };
    let task = exec.clone();
    let p = pack.clone();
    Some(Busy {
        lease,
        pack,
        handle: tokio::spawn(async move { task.execute(p).await }),
    })
}

fn report_of(lease: &Lease, pack: &Pack, runner: &RunnerInfo, f: &Finished) -> TurnReport {
    let p = f.parsed.clone();
    let reply = p.as_ref().map(|p| p.reply.clone()).unwrap_or_default();
    TurnReport {
        turn_id: pack.turn_id.clone(),
        lease_id: lease.lease_id.clone(),
        fencing_token: lease.fencing_token,
        runner_id: runner.runner_id.clone(),
        pack_sha256: pack.pack_sha256.clone(),
        outcome: f.outcome.clone(),
        say: reply.say,
        state: reply.state,
        actions: reply.actions,
        evidence: reply.evidence,
        left: reply.left,
        provider: "claude".into(),
        provider_session_id: f.session_id.clone(),
        resumed: f.resumed,
        provider_version: f.provider_version.clone(),
        num_turns: p.as_ref().map_or(0, |p| p.num_turns),
        duration_ms: p.as_ref().map_or(0, |p| p.duration_ms),
        total_cost_usd: p.as_ref().and_then(|p| p.total_cost_usd),
        usage: p
            .as_ref()
            .map_or(serde_json::Value::Null, |p| p.usage.clone()),
        denied_tools: p.map(|p| p.denied_tools).unwrap_or_default(),
        detail: f.detail.clone(),
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
    use crate::runner::service::{Beat, Registration, ServiceError};
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
    }

    impl Fake {
        fn note(&self, s: String) {
            self.calls.lock().unwrap().push(s);
        }
    }

    impl Service for Fake {
        async fn register(&self, _: &Registration) -> Result<RunnerInfo, ServiceError> {
            Err(ServiceError::NotImplemented("not used"))
        }
        async fn list_runners(&self, _: bool) -> Result<Vec<RunnerInfo>, ServiceError> {
            Ok(vec![])
        }
        async fn heartbeat(&self, id: &str, p: &[ProviderState]) -> Result<Beat, ServiceError> {
            self.note(format!("heartbeat {id} providers={}", p.len()));
            match self.beats.lock().unwrap().pop_front() {
                Some(Err(())) => Err(ServiceError::NotImplemented("down")),
                Some(Ok(offered)) => Ok(Beat {
                    runner: RunnerInfo::default(),
                    offered,
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
            self.reports.lock().unwrap().push(r.clone());
            Ok(())
        }
    }

    struct FakeExec;
    impl Exec for FakeExec {
        fn execute(&self, pack: Pack) -> Pin<Box<dyn Future<Output = Finished> + Send>> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Finished {
                    outcome: "replied".into(),
                    parsed: None,
                    resumed: true,
                    session_id: format!("sess-{}", pack.turn_id),
                    detail: String::new(),
                    provider_version: "x".into(),
                }
            })
        }
        fn providers(&self) -> Pin<Box<dyn Future<Output = Vec<ProviderState>> + Send>> {
            Box::pin(async {
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
            interval: Duration::from_millis(10),
            backoff_min: Duration::from_millis(5),
            backoff_max: Duration::from_millis(20),
            max_beats: Some(max_beats),
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
        run_loop(fake, &Arc::new(FakeExec), &runner(), c, rx, &|_| {}).await
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
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        *fake.pack.lock().unwrap() = Some(Pack::default());
        let (_, stats) = go(&fake, cfg(6)).await;
        // The turn takes 30 ms and the interval is 10: beats continue during it.
        assert!(stats.beats >= 6);
        let calls = fake.calls.lock().unwrap().clone();
        let accept = calls.iter().position(|c| c.starts_with("accept")).unwrap();
        assert!(
            calls[accept..]
                .iter()
                .filter(|c| c.starts_with("heartbeat"))
                .count()
                >= 1,
            "{calls:?}"
        );
        assert_eq!(stats.reported, 1);
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
    async fn a_lease_with_no_pack_is_accepted_but_runs_nothing() {
        // The service has no GetTurnPack yet: the stub says so and nothing is run.
        let fake = Fake::default();
        fake.beats
            .lock()
            .unwrap()
            .push_back(Ok(vec![lease("lease_1", 1)]));
        let (_, stats) = go(&fake, cfg(2)).await;
        assert_eq!((stats.accepted, stats.turns), (1, 0));
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
        let (end, stats) = run_loop(&fake, &Arc::new(FakeExec), &runner(), c, rx, &|_| {}).await;
        assert_eq!(end, End::Shutdown);
        assert!(stats.beats >= 2);
        let _ = t.await;
    }
}
