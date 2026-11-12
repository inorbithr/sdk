//! The Roles at work: one thread per Role, each with its own connection to the queue, each
//! taking the next task for it, renewing the lease while it works, and finishing it.
//!
//! What a task does is the `handler`'s: `Ok(reference)` is `done`, `Err(reason)` is
//! `failed`, and `Err(`[`INTERRUPTED`]`)` gives the task back to the queue untried. A handler that never returns because the host was killed leaves its claim to the
//! next host, which gives every claim back when it starts
//! ([`super::queue::Queue::recover_all`]). A Role whose thread ends (an error, a panic) is
//! marked `stopped`, and while the host runs a reaper gives back what such a Role held once
//! its lease has run out ([`super::queue::Queue::recover`]), failing it when it has been tried
//! too often. A Role that is alive is never robbed of its task.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::queue::{Queue, QueueError, Task, now_ms, tick};

/// What a handler returns as its `Err` when the host is stopping and it gave up the task
/// untried: the task goes back to the queue (`released`, the try not counted) instead of
/// `failed`, so the next host answers it.
pub(crate) const INTERRUPTED: &str = "interrupted: the host is stopping";

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
    let over = AtomicBool::new(false);
    let reaping = queue.handle()?;
    std::thread::scope(|scope| {
        let ended = &over;
        let reaper = scope.spawn(move || -> Result<(), QueueError> {
            // Looks as often as an idle Role does, and ends with the last Role.
            while !ended.load(Ordering::Relaxed) {
                nap(pace.poll, ended);
                reaping.recover(tick())?;
            }
            Ok(())
        });
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
        over.store(true, Ordering::Relaxed);
        if let Ok(Err(e)) = reaper.join() {
            first = first.and(Err(e));
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
    // However this thread ends (an error, a panic), its Role is marked `stopped`: that is what
    // lets its claim be given back.
    let _stopped = MarkStopped { q, role };
    while !stop.load(Ordering::Relaxed) {
        let Some(task) = q.claim(role, pace.lease, tick())? else {
            q.role_state(role, "idle", now_ms())?;
            nap(pace.poll, stop);
            continue;
        };
        q.role_state(role, "working", now_ms())?;
        q.start(task.id, role, now_ms())?;
        let outcome = with_heartbeat(q, role, task.id, pace.lease, || handler(&task, role))?;
        match outcome {
            Ok(reference) => q.complete(task.id, role, Some(&reference), now_ms())?,
            Err(why) if why == INTERRUPTED => q.release(
                task.id,
                role,
                "the host stopped before it answered",
                now_ms(),
            )?,
            Err(why) => q.fail(task.id, role, &why, now_ms())?,
        };
    }
    Ok(())
}

/// Marks a Role `stopped` when its thread ends, however it ends.
struct MarkStopped<'a> {
    q: &'a Queue,
    role: &'a str,
}

impl Drop for MarkStopped<'_> {
    fn drop(&mut self) {
        // Nothing more can be done about a failure here: the next host gives every claim back.
        let _ = self.q.role_state(self.role, "stopped", now_ms());
    }
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
        // Set however `work` ends: a handler that panics must not leave the ticker renewing
        // the lease for ever (the scope would wait on it, and the task would never be given back).
        let _end = End(finished);
        let ticker = scope.spawn(move || -> Result<(), QueueError> {
            while !finished.load(Ordering::Relaxed) {
                nap(lease / 3, finished);
                if !finished.load(Ordering::Relaxed) && !beat.heartbeat(id, role, lease, tick())? {
                    return Ok(());
                }
            }
            Ok(())
        });
        let out = work();
        finished.store(true, Ordering::Relaxed);
        match ticker.join() {
            Ok(Ok(())) | Err(_) => Ok(out),
            Ok(Err(e)) => Err(e),
        }
    })
}

/// Sets a flag when dropped.
struct End<'a>(&'a AtomicBool);

impl Drop for End<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::session::queue::{LEASE, MAX_ATTEMPTS, NewTask};

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

    fn hinted(q: &Queue, hint: &str) -> i64 {
        q.enqueue(
            &NewTask {
                external_id: None,
                role_hint: Some(hint),
                payload: "p",
            },
            now_ms(),
        )
        .unwrap()
        .id
    }

    fn short_lease() -> Pace {
        Pace {
            lease: Duration::from_millis(300),
            poll: Duration::from_millis(5),
        }
    }

    /// A Role whose handler panics is marked `stopped` (and the heartbeat of its task ends
    /// with it, so the host does not hang); its task, once the lease has run out, goes to a
    /// Role that is alive.
    #[test]
    fn a_role_that_dies_is_marked_stopped_and_its_task_goes_to_another() {
        let q = Queue::open_in_memory().unwrap();
        let roles = names(2);
        q.register_roles(&roles, now_ms()).unwrap();
        let id = hinted(&q, "rust");
        let stop = AtomicBool::new(false);
        let handler = |_: &Task, role: &str| {
            assert!(role != "rust", "the Role's thread dies here, on purpose");
            stop.store(true, Ordering::Relaxed);
            Ok("by security".to_owned())
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let own = q.handle().unwrap();
            let (roles, handler, stop) = (&roles, &handler, &stop);
            scope.spawn(move || {
                let _ = tx.send(run(&own, roles, handler, stop, short_lease()));
            });
            let ended = rx.recv_timeout(Duration::from_secs(20));
            stop.store(true, Ordering::Relaxed);
            assert!(ended.is_ok(), "the host hung on a Role that died");
        });
        assert_eq!(
            q.task(id).unwrap(),
            Some(("done".to_owned(), 2, Some("by security".to_owned())))
        );
        assert_eq!(q.role_state_of("rust").unwrap().as_deref(), Some("stopped"));
    }

    /// A task whose Role died on its last try stays `claimed` for good unless something
    /// looks: the reaper does, while the host runs, and fails it with the reason.
    #[test]
    fn the_reaper_fails_what_a_dead_role_held_on_its_last_try() {
        let q = Queue::open_in_memory().unwrap();
        let roles = names(2);
        q.register_roles(&roles, now_ms()).unwrap();
        let id = hinted(&q, "rust");
        for _ in 1..MAX_ATTEMPTS {
            q.claim("rust", LEASE, tick()).unwrap().unwrap();
            q.recover_all().unwrap();
        }
        let stop = AtomicBool::new(false);
        let handler = |_: &Task, role: &str| -> Result<String, String> {
            panic!("{role}'s thread dies here, on purpose");
        };
        std::thread::scope(|scope| {
            let own = q.handle().unwrap();
            let (roles, handler, stop) = (&roles, &handler, &stop);
            scope.spawn(move || run(&own, roles, handler, stop, short_lease()));
            let give_up = Instant::now() + Duration::from_secs(20);
            while q.task(id).unwrap().unwrap().0 != "failed" {
                assert!(Instant::now() < give_up, "{:?}", q.task(id));
                std::thread::sleep(Duration::from_millis(20));
            }
            stop.store(true, Ordering::Relaxed);
        });
        let last = q.events(id).unwrap().pop().unwrap();
        assert_eq!(last.0, "failed");
        assert!(last.1.unwrap().contains("after 3 tries"));
        assert_eq!(
            q.role_state_of("security").unwrap().as_deref(),
            Some("stopped")
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
