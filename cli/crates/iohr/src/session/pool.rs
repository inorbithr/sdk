//! The Roles at work: one thread per Role, each with its own connection to the queue, each
//! taking the next task for it, renewing the lease while it works, and finishing it.
//!
//! What a task does is the `handler`'s: `Ok(reference)` is `done`, `Err(reason)` is
//! `failed`. A handler that never returns (the host was killed) leaves the claim to run out;
//! the next host recovers it ([`super::queue::Queue::recover`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::queue::{Queue, QueueError, Task, now_ms};

/// How the pool paces itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pace {
    /// A claim lasts this long without a heartbeat.
    pub(crate) lease: Duration,
    /// An idle Role looks for work this often.
    pub(crate) poll: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            lease: super::queue::LEASE,
            poll: Duration::from_millis(500),
        }
    }
}

/// Sleeps `total`, a little at a time, and stops early when `stop` is set.
fn nap(total: Duration, stop: &AtomicBool) {
    let end = Instant::now() + total;
    while !stop.load(Ordering::Relaxed) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(20)));
    }
}

/// Runs every Role until `stop` is set. A Role finishes the task it holds before it stops.
pub(crate) fn run<H>(
    queue: &Queue,
    roles: &[String],
    handler: &H,
    stop: &AtomicBool,
    pace: Pace,
) -> Result<(), QueueError>
where
    H: Fn(&Task, &str) -> Result<String, String> + Sync,
{
    let conns = roles
        .iter()
        .map(|name| queue.handle().map(|h| (name.as_str(), h)))
        .collect::<Result<Vec<_>, _>>()?;
    std::thread::scope(|scope| {
        let workers: Vec<_> = conns
            .into_iter()
            .map(|(role, q)| scope.spawn(move || work(&q, role, handler, stop, pace)))
            .collect();
        let mut first = Ok(());
        for w in workers {
            // A panicking handler is a bug; it must not hide the others' results.
            if let Ok(Err(e)) = w.join() {
                first = Err(e);
            }
        }
        first
    })
}

fn work<H>(
    q: &Queue,
    role: &str,
    handler: &H,
    stop: &AtomicBool,
    pace: Pace,
) -> Result<(), QueueError>
where
    H: Fn(&Task, &str) -> Result<String, String> + Sync,
{
    while !stop.load(Ordering::Relaxed) {
        let Some(task) = q.claim(role, pace.lease, now_ms())? else {
            q.role_state(role, "idle", now_ms())?;
            nap(pace.poll, stop);
            continue;
        };
        q.role_state(role, "working", now_ms())?;
        q.start(task.id, role, now_ms())?;
        let outcome = with_heartbeat(q, role, task.id, pace.lease, || handler(&task, role))?;
        match outcome {
            Ok(reference) => q.complete(task.id, role, Some(&reference), now_ms())?,
            Err(why) => q.fail(task.id, role, &why, now_ms())?,
        };
    }
    q.role_state(role, "stopped", now_ms())
}

/// Runs `work` while a second connection renews the lease every third of it.
fn with_heartbeat<T>(
    q: &Queue,
    role: &str,
    id: i64,
    lease: Duration,
    work: impl FnOnce() -> T,
) -> Result<T, QueueError> {
    let beat = q.handle()?;
    let done = AtomicBool::new(false);
    let finished = &done;
    std::thread::scope(|scope| {
        let ticker = scope.spawn(move || -> Result<(), QueueError> {
            while !finished.load(Ordering::Relaxed) {
                nap(lease / 3, finished);
                if !finished.load(Ordering::Relaxed)
                    && !beat.heartbeat(id, role, lease, now_ms())?
                {
                    return Ok(());
                }
            }
            Ok(())
        });
        let out = work();
        done.store(true, Ordering::Relaxed);
        match ticker.join() {
            Ok(Ok(())) | Err(_) => Ok(out),
            Ok(Err(e)) => Err(e),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::session::queue::NewTask;

    const ROLES: [&str; 4] = ["rust", "security", "docs", "release"];

    fn names(n: usize) -> Vec<String> {
        ROLES.iter().take(n).map(ToString::to_string).collect()
    }

    fn fill(q: &Queue, n: usize) -> Vec<i64> {
        (0..n)
            .map(|i| {
                q.enqueue(
                    &NewTask {
                        external_id: Some(&format!("ask_{i}")),
                        role_hint: None,
                        payload: "p",
                    },
                    now_ms(),
                )
                .unwrap()
                .id
            })
            .collect()
    }

    fn quick() -> Pace {
        Pace {
            lease: Duration::from_secs(30),
            poll: Duration::from_millis(5),
        }
    }

    /// Test (b) of RFC 0213: four Roles take twenty tasks off one queue at the same time;
    /// every task ends `done` once, and the claims overlap in time.
    #[test]
    fn four_roles_run_twenty_tasks_at_once_and_each_task_is_done_once() {
        let q = Queue::open_in_memory().unwrap();
        let roles = names(4);
        q.register_roles(&roles, now_ms()).unwrap();
        let ids = fill(&q, 20);
        let now = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let runs: Mutex<BTreeMap<i64, Vec<String>>> = Mutex::new(BTreeMap::new());
        let stop = AtomicBool::new(false);
        let handler = |t: &Task, role: &str| {
            let inside = now.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(inside, Ordering::SeqCst);
            // Hold the task until every Role is inside one, so an overlap is proven, not
            // hoped for (a deadline keeps a broken pool from hanging the test).
            let give_up = Instant::now() + Duration::from_secs(10);
            while peak.load(Ordering::SeqCst) < 4 && Instant::now() < give_up {
                std::thread::sleep(Duration::from_millis(5));
            }
            runs.lock()
                .unwrap()
                .entry(t.id)
                .or_default()
                .push(role.to_owned());
            now.fetch_sub(1, Ordering::SeqCst);
            Ok(format!("by {role}"))
        };
        std::thread::scope(|scope| {
            let own = q.handle().unwrap();
            let (roles, handler, stop) = (&roles, &handler, &stop);
            scope.spawn(move || run(&own, roles, handler, stop, quick()).unwrap());
            let give_up = Instant::now() + Duration::from_secs(60);
            while q.counts().unwrap() != vec![("done".to_owned(), 20)] {
                assert!(Instant::now() < give_up, "{:?}", q.counts());
                std::thread::sleep(Duration::from_millis(20));
            }
            stop.store(true, Ordering::Relaxed);
        });
        assert_eq!(peak.load(Ordering::SeqCst), 4, "four Roles worked at once");
        let runs = runs.into_inner().unwrap();
        assert_eq!(runs.keys().copied().collect::<Vec<_>>(), ids);
        assert!(
            runs.values().all(|by| by.len() == 1),
            "a task ran more than once: {runs:?}"
        );
        for id in ids {
            let (state, attempts, _) = q.task(id).unwrap().unwrap();
            assert_eq!((state.as_str(), attempts), ("done", 1));
        }
        for role in roles {
            assert_eq!(q.role_state_of(&role).unwrap().as_deref(), Some("stopped"));
        }
    }

    /// A task that takes longer than its lease is not taken by another Role: the heartbeat
    /// renews the claim while the handler works.
    #[test]
    fn the_heartbeat_keeps_a_long_task_with_its_role() {
        let q = Queue::open_in_memory().unwrap();
        let roles = names(2);
        q.register_roles(&roles, now_ms()).unwrap();
        let ids = fill(&q, 1);
        let ran = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let handler = |_: &Task, _: &str| {
            ran.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(1500));
            stop.store(true, Ordering::Relaxed);
            Ok("slow".to_owned())
        };
        let pace = Pace {
            lease: Duration::from_millis(400),
            poll: Duration::from_millis(5),
        };
        run(&q, &roles, &handler, &stop, pace).unwrap();
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(
            q.task(ids[0]).unwrap(),
            Some(("done".to_owned(), 1, Some("slow".to_owned())))
        );
    }

    #[test]
    fn an_error_from_the_handler_fails_the_task_with_its_reason() {
        let q = Queue::open_in_memory().unwrap();
        let roles = names(1);
        q.register_roles(&roles, now_ms()).unwrap();
        let ids = fill(&q, 1);
        let stop = AtomicBool::new(false);
        let handler = |_: &Task, _: &str| {
            stop.store(true, Ordering::Relaxed);
            Err("claude exited 1".to_owned())
        };
        run(&q, &roles, &handler, &stop, quick()).unwrap();
        let (state, ..) = q.task(ids[0]).unwrap().unwrap();
        assert_eq!(state, "failed");
        let last = q.events(ids[0]).unwrap().pop().unwrap();
        assert_eq!(
            last,
            ("failed".to_owned(), Some("claude exited 1".to_owned()))
        );
    }
}
