//! The session host's durable inbox (RFC 0213): one `DuckDB` file that only the host process
//! opens for writing. Roles work from it at the same time, each with its own connection
//! to the one database, and nothing in it is lost when the host stops or is killed.
//!
//! `DuckDB` allows one read-write process; inside it many threads write under MVCC with
//! optimistic concurrency control. Appends never conflict. Two threads editing one row do:
//! the second gets a conflict error, and every statement here runs through [`Queue::retrying`],
//! which tries it again after a short wait (`DuckDB`, "Concurrency",
//! <https://duckdb.org/docs/current/connect/concurrency>).
//!
//! A task is `queued`, then `claimed` by a Role (a lease starts), `running`, and ends `done`
//! or `failed`. Every change appends a row to `task_events`, which is never edited.
//!
//! Two clocks, on purpose. The lease runs on [`tick`], the host's own monotonic clock, which
//! neither a clock step nor a laptop's sleep moves (the threads sleep with it), so a Role that
//! is alive is never judged late. Stamps (`created_at`, `updated_at`, the events) are wall
//! clock, for people to read. A lease is only meaningful to the process that wrote it, so:
//!
//! - a new host holds the lock, which means no other host is alive: every claim in the file is
//!   an orphan, and [`Queue::recover_all`] gives each back at once, whatever its lease says;
//! - while a host runs, a claim is taken from its Role only when that Role has `stopped` (its
//!   thread ended) and the lease has run out ([`Queue::claim`], [`Queue::recover`]). A Role
//!   that is alive keeps its task, and a task is never run by two Roles.
//!
//! A task given back goes to `queued` until it has been tried [`MAX_ATTEMPTS`] times, then it
//! is `failed` with the reason.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use duckdb::{Connection, params};

/// How long a claim lasts without a heartbeat (the coordinator's lease, RFC 0120).
pub(crate) const LEASE: Duration = Duration::from_secs(90);

/// How many times a task is tried before it is `failed`.
pub(crate) const MAX_ATTEMPTS: i32 = 3;

/// How many times a statement is tried again after a conflict before it gives up.
const CONFLICT_RETRIES: u32 = 24;

#[derive(Debug, thiserror::Error)]
pub(crate) enum QueueError {
    #[error("the session queue: {0}")]
    Db(#[from] duckdb::Error),
    #[error("the session queue: still in conflict after {0} tries")]
    Conflict(u32),
}

/// A task as a Role is handed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Task {
    pub(crate) id: i64,
    pub(crate) external_id: Option<String>,
    pub(crate) role_hint: Option<String>,
    pub(crate) payload: String,
    pub(crate) attempts: i32,
}

/// What to put in the queue. `external_id` makes it idempotent: the same id is the same task
/// (the coordinator's `QueueAsk` id, so a retried mirror adds nothing).
#[derive(Debug, Clone, Default)]
pub(crate) struct NewTask<'a> {
    pub(crate) external_id: Option<&'a str>,
    pub(crate) role_hint: Option<&'a str>,
    pub(crate) payload: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Enqueued {
    pub(crate) id: i64,
    /// The `external_id` was already in the queue: nothing was added.
    pub(crate) repeated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Recovered {
    pub(crate) requeued: usize,
    pub(crate) failed: usize,
}

/// A handle to the queue. Each thread takes its own with [`Queue::handle`].
pub(crate) struct Queue {
    conn: Connection,
    conflicts: Arc<AtomicU64>,
}

/// How long to wait before try number `tries`: up to `2 ms << tries` (at most 128 ms), a
/// random part of it, so threads that met each other do not meet again at the same moment.
/// The randomness is the clock, a counter every call moves on and the calling thread, run
/// through a mixer: a coarse clock (a loaded Linux runner) gives two threads the same
/// nanoseconds, and they must still wait differently.
fn backoff(tries: u32) -> Duration {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let noise = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::from(d.subsec_nanos()));
    let mut thread = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&std::thread::current().id(), &mut thread);
    let salt = std::hash::Hasher::finish(&thread) ^ CALLS.fetch_add(1, Ordering::Relaxed);
    backoff_from(tries, noise, salt)
}

/// The wait for try number `tries` from a clock reading and a salt (splitmix64 mixes them).
fn backoff_from(tries: u32, noise: u64, salt: u64) -> Duration {
    let span = 2u64 << tries.min(6);
    let mut z = noise
        .wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(u64::from(tries) << 13);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    Duration::from_micros(span * 500 + z % (span * 500))
}

const SCHEMA: &str = "
CREATE SEQUENCE IF NOT EXISTS task_ids START 1;
CREATE SEQUENCE IF NOT EXISTS event_ids START 1;
CREATE SEQUENCE IF NOT EXISTS role_ids START 1;
CREATE TABLE IF NOT EXISTS roles (
    id BIGINT PRIMARY KEY DEFAULT nextval('role_ids'),
    role VARCHAR NOT NULL,
    state VARCHAR NOT NULL,
    started_at BIGINT NOT NULL,
    heartbeat_at BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    id BIGINT PRIMARY KEY DEFAULT nextval('task_ids'),
    external_id VARCHAR UNIQUE,
    role_hint VARCHAR,
    payload VARCHAR NOT NULL,
    state VARCHAR NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    lease_until BIGINT,
    claimed_by VARCHAR,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    result_ref VARCHAR,
    reason VARCHAR
);
CREATE TABLE IF NOT EXISTS task_events (
    seq BIGINT PRIMARY KEY DEFAULT nextval('event_ids'),
    task_id BIGINT NOT NULL,
    happened_at BIGINT NOT NULL,
    event VARCHAR NOT NULL,
    detail VARCHAR
);
";

/// The Role that holds a claim is not alive: it has `stopped`, or this host never ran it.
macro_rules! owner_gone {
    () => {
        "claimed_by NOT IN (SELECT role FROM roles WHERE state <> 'stopped')"
    };
}

/// A claim can be taken by a Role when it is waiting, or its lease ran out and its Role is
/// gone. `$3` is the lease clock, [`tick`].
const TAKEABLE: &str = concat!(
    "(state = 'queued' OR (state IN ('claimed', 'running') AND lease_until < $3 AND ",
    owner_gone!(),
    "))"
);

/// The lease clock: milliseconds on this process's monotonic clock since it first asked.
/// It never goes back, and it does not run while the machine sleeps (neither do the threads
/// that renew leases). Leases written with it mean nothing to another process.
pub(crate) fn tick() -> i64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let ms = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// The current wall-clock time in milliseconds since the epoch, for stamps.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

pub(crate) fn lease_ms(lease: Duration) -> i64 {
    i64::try_from(lease.as_millis()).unwrap_or(i64::MAX)
}

fn is_conflict(e: &duckdb::Error) -> bool {
    let text = e.to_string();
    text.contains("onflict")
}

impl Queue {
    /// Opens (and creates) the queue file. The host is the only process that does this.
    pub(crate) fn open(path: &Path) -> Result<Self, QueueError> {
        Self::with(Connection::open(path)?)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, QueueError> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(conn: Connection) -> Result<Self, QueueError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            conflicts: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Another connection to the same database, for another thread.
    pub(crate) fn handle(&self) -> Result<Self, QueueError> {
        Ok(Self {
            conn: self.conn.try_clone()?,
            conflicts: self.conflicts.clone(),
        })
    }

    #[cfg(test)]
    /// How many statements were tried again after a conflict, over every handle.
    pub(crate) fn conflicts(&self) -> u64 {
        self.conflicts.load(Ordering::Relaxed)
    }

    /// Runs `f` in one transaction; after a conflict (another thread edited the same row)
    /// the whole of it is tried again, waiting a little longer each time.
    pub(crate) fn retrying<T>(
        &self,
        mut f: impl FnMut(&duckdb::Transaction<'_>) -> duckdb::Result<T>,
    ) -> Result<T, QueueError> {
        let mut tries = 0;
        loop {
            let attempt = self.conn.unchecked_transaction().and_then(|tx| {
                let out = f(&tx)?;
                tx.commit()?;
                Ok(out)
            });
            match attempt {
                Ok(v) => return Ok(v),
                Err(e) if is_conflict(&e) => {
                    tries += 1;
                    self.conflicts.fetch_add(1, Ordering::Relaxed);
                    if tries > CONFLICT_RETRIES {
                        return Err(QueueError::Conflict(tries));
                    }
                    std::thread::sleep(backoff(tries));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Adds a task, or finds the one with the same `external_id`.
    pub(crate) fn enqueue(&self, new: &NewTask<'_>, now: i64) -> Result<Enqueued, QueueError> {
        self.retrying(|tx| {
            let added: Option<i64> = match tx.query_row(
                "INSERT INTO tasks (external_id, role_hint, payload, state, created_at, updated_at) \
                 VALUES ($1, $2, $3, 'queued', $4, $4) ON CONFLICT DO NOTHING RETURNING id",
                params![new.external_id, new.role_hint, new.payload, now],
                |r| r.get(0),
            ) {
                Ok(id) => Some(id),
                Err(duckdb::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e),
            };
            if let Some(id) = added {
                event(tx, id, now, "queued", new.role_hint)?;
                return Ok(Enqueued {
                    id,
                    repeated: false,
                });
            }
            let id: i64 = tx.query_row(
                "SELECT id FROM tasks WHERE external_id = $1",
                params![new.external_id],
                |r| r.get(0),
            )?;
            Ok(Enqueued { id, repeated: true })
        })
    }

    /// The next task for `role` (its own, a hint of a Role this host does not run, or none),
    /// claimed for `lease`; `None` when nothing waits. One statement: of two Roles claiming
    /// at once, one wins the row and the other, after the retry, takes the next. `tick` is
    /// the lease clock ([`tick`]).
    pub(crate) fn claim(
        &self,
        role: &str,
        lease: Duration,
        tick: i64,
    ) -> Result<Option<Task>, QueueError> {
        let until = tick.saturating_add(lease_ms(lease));
        let now = now_ms();
        let sql = format!(
            "UPDATE tasks SET state = 'claimed', claimed_by = $1, lease_until = $2, \
                 attempts = attempts + 1, updated_at = $5 \
             WHERE id = (SELECT id FROM tasks WHERE {TAKEABLE} AND attempts < $4 \
                 AND (role_hint IS NULL OR role_hint = '' OR role_hint = $1 \
                      OR role_hint NOT IN (SELECT role FROM roles WHERE state <> 'stopped')) \
                 ORDER BY created_at, id LIMIT 1) \
             AND {TAKEABLE} AND attempts < $4 \
             RETURNING id, external_id, role_hint, payload, attempts"
        );
        self.retrying(|tx| {
            let got = tx.query_row(&sql, params![role, until, tick, MAX_ATTEMPTS, now], |r| {
                Ok(Task {
                    id: r.get(0)?,
                    external_id: r.get(1)?,
                    role_hint: r.get(2)?,
                    payload: r.get(3)?,
                    attempts: r.get(4)?,
                })
            });
            match got {
                Ok(t) => {
                    event(tx, t.id, now, "claimed", Some(role))?;
                    Ok(Some(t))
                }
                Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// A Role starts the work of a task it holds.
    pub(crate) fn start(&self, id: i64, role: &str, now: i64) -> Result<bool, QueueError> {
        self.held(
            id,
            role,
            now,
            "state = 'running', updated_at = $3",
            "running",
            None,
        )
    }

    /// A Role is still working on `id`: the lease runs on (`tick` is the lease clock).
    /// `false` when the task is no longer its own.
    pub(crate) fn heartbeat(
        &self,
        id: i64,
        role: &str,
        lease: Duration,
        tick: i64,
    ) -> Result<bool, QueueError> {
        let until = tick.saturating_add(lease_ms(lease));
        let now = now_ms();
        self.retrying(|tx| {
            let n = tx.execute(
                "UPDATE tasks SET lease_until = $1, updated_at = $2 \
                 WHERE id = $3 AND claimed_by = $4 AND state IN ('claimed', 'running')",
                params![until, now, id, role],
            )?;
            Ok(n == 1)
        })
    }

    /// The task is done. Only the Role that holds it can finish it, so a task whose lease
    /// ran out and went to another Role is finished once.
    pub(crate) fn complete(
        &self,
        id: i64,
        role: &str,
        result_ref: Option<&str>,
        now: i64,
    ) -> Result<bool, QueueError> {
        self.held(
            id,
            role,
            now,
            "state = 'done', lease_until = NULL, result_ref = $4, updated_at = $3",
            "done",
            result_ref,
        )
    }

    /// The task failed for good, with the reason.
    pub(crate) fn fail(
        &self,
        id: i64,
        role: &str,
        why: &str,
        now: i64,
    ) -> Result<bool, QueueError> {
        self.held(
            id,
            role,
            now,
            "state = 'failed', lease_until = NULL, reason = $4, updated_at = $3",
            "failed",
            Some(why),
        )
    }

    fn held(
        &self,
        id: i64,
        role: &str,
        now: i64,
        set: &str,
        name: &str,
        extra: Option<&str>,
    ) -> Result<bool, QueueError> {
        let sql = format!(
            "UPDATE tasks SET {set} \
             WHERE id = $1 AND claimed_by = $2 AND state IN ('claimed', 'running')"
        );
        self.retrying(|tx| {
            let n = if sql.contains("$4") {
                tx.execute(&sql, params![id, role, now, extra])?
            } else {
                tx.execute(&sql, params![id, role, now])?
            };
            if n == 1 {
                event(tx, id, now, name, extra)?;
            }
            Ok(n == 1)
        })
    }

    /// While the host runs: a claim whose lease ran out (`tick` is the lease clock) and whose
    /// Role has stopped goes back to `queued`, or is `failed` when it has been tried
    /// [`MAX_ATTEMPTS`] times. A claim of a Role that is alive is left alone.
    pub(crate) fn recover(&self, tick: i64) -> Result<Recovered, QueueError> {
        let sql = concat!(
            "SELECT id, attempts FROM tasks WHERE state IN ('claimed', 'running') \
             AND lease_until < $1 AND ",
            owner_gone!()
        );
        self.give_back(sql, Some(tick), "the lease ran out")
    }

    /// What a host does first, holding the lock: no other host is alive, so every claim in
    /// the file is an orphan (its lease was written by a process that is gone, on a clock
    /// this one does not share). Each is given back at once.
    pub(crate) fn recover_all(&self) -> Result<Recovered, QueueError> {
        let sql = "SELECT id, attempts FROM tasks WHERE state IN ('claimed', 'running')";
        self.give_back(sql, None, "the host that held it stopped")
    }

    fn give_back(
        &self,
        select: &str,
        tick: Option<i64>,
        cause: &str,
    ) -> Result<Recovered, QueueError> {
        let now = now_ms();
        self.retrying(|tx| {
            let mut stmt = tx.prepare(select)?;
            let map = |r: &duckdb::Row<'_>| Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?));
            let stale: Vec<(i64, i32)> = match tick {
                Some(t) => stmt.query_map(params![t], map)?.collect::<Result<_, _>>()?,
                None => stmt.query_map([], map)?.collect::<Result<_, _>>()?,
            };
            drop(stmt);
            let mut out = Recovered::default();
            for (id, attempts) in stale {
                if attempts >= MAX_ATTEMPTS {
                    let why = format!("{cause}, after {attempts} tries");
                    tx.execute(
                        "UPDATE tasks SET state = 'failed', lease_until = NULL, reason = $1, \
                         updated_at = $2 WHERE id = $3",
                        params![why, now, id],
                    )?;
                    event(tx, id, now, "failed", Some(&why))?;
                    out.failed += 1;
                } else {
                    tx.execute(
                        "UPDATE tasks SET state = 'queued', lease_until = NULL, claimed_by = NULL, \
                         updated_at = $1 WHERE id = $2",
                        params![now, id],
                    )?;
                    event(tx, id, now, "requeued", Some(cause))?;
                    out.requeued += 1;
                }
            }
            Ok(out)
        })
    }

    /// The Roles this host runs, all `idle`. The host is the only process on the file, so
    /// the rows of an earlier host are replaced.
    pub(crate) fn register_roles(&self, roles: &[String], now: i64) -> Result<(), QueueError> {
        self.retrying(|tx| {
            tx.execute("DELETE FROM roles", [])?;
            for r in roles {
                tx.execute(
                    "INSERT INTO roles (role, state, started_at, heartbeat_at) \
                     VALUES ($1, 'idle', $2, $2)",
                    params![r, now],
                )?;
            }
            Ok(())
        })
    }

    /// A Role's state (`idle`, `working`, `stopped`), and that it is alive.
    pub(crate) fn role_state(&self, role: &str, state: &str, now: i64) -> Result<(), QueueError> {
        self.retrying(|tx| {
            tx.execute(
                "UPDATE roles SET state = $1, heartbeat_at = $2 WHERE role = $3",
                params![state, now, role],
            )?;
            Ok(())
        })
    }

    #[cfg(test)]
    pub(crate) fn role_state_of(&self, role: &str) -> Result<Option<String>, QueueError> {
        self.retrying(|tx| {
            match tx.query_row(
                "SELECT state FROM roles WHERE role = $1",
                params![role],
                |r| r.get(0),
            ) {
                Ok(state) => Ok(Some(state)),
                Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn task(
        &self,
        id: i64,
    ) -> Result<Option<(String, i32, Option<String>)>, QueueError> {
        self.retrying(|tx| {
            match tx.query_row(
                "SELECT state, attempts, result_ref FROM tasks WHERE id = $1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ) {
                Ok(v) => Ok(Some(v)),
                Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    #[cfg(test)]
    /// The events of one task, oldest first, as `(event, detail)`.
    pub(crate) fn events(&self, id: i64) -> Result<Vec<(String, Option<String>)>, QueueError> {
        self.retrying(|tx| {
            let mut stmt = tx
                .prepare("SELECT event, detail FROM task_events WHERE task_id = $1 ORDER BY seq")?;
            let rows = stmt
                .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    #[cfg(test)]
    /// How many tasks are in each state.
    pub(crate) fn counts(&self) -> Result<Vec<(String, i64)>, QueueError> {
        self.retrying(|tx| {
            let mut stmt =
                tx.prepare("SELECT state, count(*) FROM tasks GROUP BY state ORDER BY state")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

fn event(
    tx: &duckdb::Transaction<'_>,
    id: i64,
    now: i64,
    name: &str,
    detail: Option<&str>,
) -> duckdb::Result<()> {
    tx.execute(
        "INSERT INTO task_events (task_id, happened_at, event, detail) VALUES ($1, $2, $3, $4)",
        params![id, now, name, detail],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;

    const SHORT: Duration = Duration::from_secs(60);

    fn put(q: &Queue, external: Option<&str>, hint: Option<&str>) -> i64 {
        q.enqueue(
            &NewTask {
                external_id: external,
                role_hint: hint,
                payload: "do it",
            },
            now_ms(),
        )
        .unwrap()
        .id
    }

    fn roles(q: &Queue, names: &[&str]) {
        let names: Vec<String> = names.iter().map(ToString::to_string).collect();
        q.register_roles(&names, now_ms()).unwrap();
    }

    /// The lease clock is the host's own: it counts from the process's start, so it is not
    /// the wall clock (a clock step cannot move it), and it never goes back.
    #[test]
    fn the_lease_clock_is_monotonic_and_is_not_the_wall_clock() {
        let a = tick();
        std::thread::sleep(Duration::from_millis(20));
        let b = tick();
        assert!(b >= a + 20, "{a} then {b}");
        // Milliseconds since this process began its work: far from the epoch's 1.7e12.
        assert!(b < 10_i64.pow(11), "{b}");
        assert!(now_ms() > 10_i64.pow(12));
    }

    #[test]
    fn the_same_external_id_is_the_same_task() {
        let q = Queue::open_in_memory().unwrap();
        let first = q
            .enqueue(
                &NewTask {
                    external_id: Some("ask_1"),
                    role_hint: Some("rust"),
                    payload: "p",
                },
                1,
            )
            .unwrap();
        let again = q
            .enqueue(
                &NewTask {
                    external_id: Some("ask_1"),
                    role_hint: None,
                    payload: "other",
                },
                2,
            )
            .unwrap();
        assert!(!first.repeated);
        assert!(again.repeated);
        assert_eq!(first.id, again.id);
        assert_eq!(q.counts().unwrap(), vec![("queued".to_owned(), 1)]);
        // Tasks without an external id are all distinct.
        assert_ne!(put(&q, None, None), put(&q, None, None));
    }

    #[test]
    fn a_task_goes_queued_claimed_running_done_and_only_its_holder_can_finish_it() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust", "security"]);
        let id = put(&q, None, None);
        let t = q.claim("rust", LEASE, 10).unwrap().unwrap();
        assert_eq!((t.id, t.attempts), (id, 1));
        assert!(q.claim("security", LEASE, 11).unwrap().is_none());
        assert!(!q.complete(id, "security", None, 12).unwrap());
        assert!(q.start(id, "rust", 13).unwrap());
        assert!(q.complete(id, "rust", Some("ref"), 14).unwrap());
        // Finished once: a second finish finds nothing to finish.
        assert!(!q.complete(id, "rust", Some("ref"), 15).unwrap());
        assert_eq!(
            q.task(id).unwrap(),
            Some(("done".to_owned(), 1, Some("ref".to_owned())))
        );
        let events: Vec<String> = q.events(id).unwrap().into_iter().map(|e| e.0).collect();
        assert_eq!(events, ["queued", "claimed", "running", "done"]);
    }

    #[test]
    fn a_hint_goes_to_that_role_unless_it_is_not_running() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust", "security"]);
        let id = put(&q, None, Some("security"));
        assert!(q.claim("rust", LEASE, 1).unwrap().is_none());
        assert_eq!(q.claim("security", LEASE, 1).unwrap().unwrap().id, id);
        // A hint for a Role this host does not run is anyone's.
        let other = put(&q, None, Some("go"));
        assert_eq!(q.claim("rust", LEASE, 2).unwrap().unwrap().id, other);
    }

    #[test]
    fn recovery_requeues_what_ran_out_and_fails_what_was_tried_too_often() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust"]);
        let id = put(&q, None, None);
        let lease = Duration::from_millis(1000);
        q.claim("rust", lease, 0).unwrap().unwrap();
        // Its Role's thread ended, so only the lease is left to wait for.
        q.role_state("rust", "stopped", 1).unwrap();
        // Not run out yet: nothing happens.
        assert_eq!(q.recover(999).unwrap(), Recovered::default());
        assert_eq!(
            q.recover(1001).unwrap(),
            Recovered {
                requeued: 1,
                failed: 0
            }
        );
        // Tried again, and again: the third claim is the last.
        for n in 2..=MAX_ATTEMPTS {
            let t = q.claim("rust", lease, 2000).unwrap().unwrap();
            assert_eq!(t.attempts, n);
            q.recover(4000).unwrap();
        }
        let (state, attempts, _) = q.task(id).unwrap().unwrap();
        assert_eq!((state.as_str(), attempts), ("failed", MAX_ATTEMPTS));
        let last = q.events(id).unwrap().pop().unwrap();
        assert_eq!(last.0, "failed");
        assert!(last.1.unwrap().contains("lease ran out"));
        assert!(q.claim("rust", lease, 9000).unwrap().is_none());
    }

    /// A live Role's claim is never taken from it, however late its heartbeat is: that would
    /// run one task twice. An expired claim is taken only when its Role has stopped.
    #[test]
    fn an_expired_claim_is_taken_only_from_a_role_that_has_stopped() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust", "security"]);
        let id = put(&q, None, None);
        q.claim("rust", Duration::from_millis(10), 0)
            .unwrap()
            .unwrap();
        q.role_state("rust", "working", 1).unwrap();
        // The lease ran out, but the Role is alive: not another Role's, not recovery's.
        assert!(q.claim("security", LEASE, 11).unwrap().is_none());
        assert_eq!(q.recover(11).unwrap(), Recovered::default());
        assert_eq!(q.task(id).unwrap().unwrap().0, "claimed");
        // Its thread is gone: now the claim can go to another Role.
        q.role_state("rust", "stopped", 12).unwrap();
        let t = q.claim("security", LEASE, 13).unwrap().unwrap();
        assert_eq!((t.id, t.attempts), (id, 2));
        // The first Role's late answer does not count: the task is another's now.
        assert!(!q.complete(id, "rust", None, 14).unwrap());
        assert!(q.complete(id, "security", None, 15).unwrap());
    }

    /// What a new host does with the file it finds: the lock says no other host is alive, so
    /// every claim in it is an orphan, whatever its lease says (it was written by a process
    /// that is gone, on a clock this one does not share).
    #[test]
    fn a_new_host_takes_back_every_claim_at_once_and_fails_the_exhausted_ones() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust"]);
        let id = put(&q, None, None);
        let waiting = put(&q, None, None);
        for n in 1..=MAX_ATTEMPTS {
            // Killed with a lease that has most of its time left.
            let t = q.claim("rust", LEASE, 0).unwrap().unwrap();
            assert_eq!((t.id, t.attempts), (id, n));
            let back = q.recover_all().unwrap();
            let want = if n < MAX_ATTEMPTS { (1, 0) } else { (0, 1) };
            assert_eq!((back.requeued, back.failed), want, "try {n}");
        }
        assert_eq!(q.task(id).unwrap().unwrap().0, "failed");
        assert_eq!(q.task(waiting).unwrap().unwrap().0, "queued");
    }

    /// A statement that meets another connection's uncommitted edit of the same row is
    /// tried again and succeeds once that edit is committed.
    #[test]
    fn a_conflict_on_the_same_row_is_retried_not_failed() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust"]);
        let id = put(&q, None, None);
        q.claim("rust", LEASE, 0).unwrap().unwrap();
        let other = q.handle().unwrap();
        let mut blocker = Some(other.conn.unchecked_transaction().unwrap());
        blocker
            .as_ref()
            .unwrap()
            .execute("UPDATE tasks SET updated_at = 5 WHERE id = $1", params![id])
            .unwrap();
        let mut tries = 0;
        let done = q.retrying(|tx| {
            tries += 1;
            if tries == 2 {
                // The other edit lands between our attempts.
                blocker.take().unwrap().commit()?;
            }
            tx.execute("UPDATE tasks SET updated_at = 6 WHERE id = $1", params![id])
        });
        assert_eq!(done.unwrap(), 1);
        assert!(tries >= 2, "the first attempt must have met the conflict");
        assert!(q.conflicts() >= 1);
    }

    /// A coarse clock gives two threads the same reading: their waits must still differ, or
    /// they meet again and again until the retries run out (a loaded CI runner did this).
    #[test]
    fn two_threads_with_the_same_clock_reading_wait_differently() {
        let waits: std::collections::HashSet<_> = (0..8u64)
            .map(|salt| backoff_from(3, 123_456_789, salt))
            .collect();
        assert!(waits.len() >= 6, "{waits:?}");
        for w in &waits {
            assert!(
                *w >= Duration::from_millis(8) && *w < Duration::from_millis(16),
                "{w:?}"
            );
        }
    }

    /// Four threads editing one row at once (a heavier load than the host ever makes: one
    /// Role holds a task): every edit goes through, none ends in an error.
    #[test]
    fn threads_hammering_one_row_all_succeed() {
        let q = Queue::open_in_memory().unwrap();
        roles(&q, &["rust"]);
        let id = put(&q, None, None);
        q.claim("rust", LEASE, 0).unwrap().unwrap();
        let handles: Vec<Queue> = (0..4).map(|_| q.handle().unwrap()).collect();
        std::thread::scope(|scope| {
            let workers: Vec<_> = handles
                .into_iter()
                .map(|h| {
                    scope.spawn(move || {
                        (0..15).try_for_each(|i| {
                            h.heartbeat(id, "rust", LEASE, 1000 + i).map(|held| {
                                assert!(held);
                            })
                        })
                    })
                })
                .collect();
            for w in workers {
                w.join().unwrap().unwrap();
            }
        });
    }

    // ----- a host that is killed

    const CHILD_DIR: &str = "IOHR_TEST_QUEUE_CHILD_DIR";

    /// Not a test on its own: the process the next test kills. It claims a task, starts it
    /// and then works for ever, as a host would in the middle of a turn.
    #[test]
    #[ignore = "run by a_killed_host_loses_nothing, as a child process"]
    fn child_host_that_never_finishes() {
        let Ok(dir) = std::env::var(CHILD_DIR) else {
            return;
        };
        let dir = PathBuf::from(dir);
        let q = Queue::open(&dir.join("queue.duckdb")).unwrap();
        roles(&q, &["rust"]);
        put(&q, Some("ask_kill"), None);
        let t = q.claim("rust", SHORT, tick()).unwrap().unwrap();
        assert!(q.start(t.id, "rust", now_ms()).unwrap());
        std::fs::write(dir.join("claimed"), t.id.to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(120));
    }

    /// Test (a) of RFC 0213: kill the host (SIGKILL, no cleanup) in the middle of a task,
    /// start another on the same file, and the task resumes and ends `done` once.
    #[test]
    fn a_killed_host_loses_nothing_and_the_task_finishes_once() {
        use super::super::pool::{self, Pace};
        use std::sync::Mutex;
        use std::sync::atomic::AtomicBool;

        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::queue::tests::child_host_that_never_finishes",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_DIR, dir.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let marker = dir.path().join("claimed");
        let give_up = Instant::now() + Duration::from_secs(60);
        while !marker.exists() {
            assert!(Instant::now() < give_up, "the child never claimed its task");
            std::thread::sleep(Duration::from_millis(20));
        }
        let id: i64 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();

        // The next host opens the same file: the claim is still held, its lease has most of
        // its time left, and the host that made it is gone. It is given back at once.
        let q = Queue::open(&dir.path().join("queue.duckdb")).unwrap();
        assert_eq!(q.task(id).unwrap().unwrap().0, "running");
        assert_eq!(
            q.recover_all().unwrap(),
            Recovered {
                requeued: 1,
                failed: 0
            }
        );
        roles(&q, &["rust"]);
        let ran = Mutex::new(Vec::new());
        let stop = AtomicBool::new(false);
        let handler = |t: &Task, role: &str| {
            ran.lock()
                .unwrap()
                .push((t.id, role.to_owned(), t.attempts));
            stop.store(true, Ordering::Relaxed);
            Ok("result".to_owned())
        };
        let pace = Pace {
            lease: Duration::from_secs(5),
            poll: Duration::from_millis(10),
        };
        pool::run(&q, &["rust".to_owned()], &handler, &stop, pace).unwrap();
        assert_eq!(*ran.lock().unwrap(), vec![(id, "rust".to_owned(), 2)]);
        assert_eq!(
            q.task(id).unwrap(),
            Some(("done".to_owned(), 2, Some("result".to_owned())))
        );
        let events: Vec<String> = q.events(id).unwrap().into_iter().map(|e| e.0).collect();
        assert_eq!(
            events,
            [
                "queued", "claimed", "running", "requeued", "claimed", "running", "done"
            ]
        );
        assert_eq!(events.iter().filter(|e| *e == "done").count(), 1);
    }

    #[test]
    fn what_was_queued_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("queue.duckdb");
        let id = {
            let q = Queue::open(&file).unwrap();
            put(&q, Some("ask_9"), Some("rust"))
        };
        let q = Queue::open(&file).unwrap();
        assert_eq!(q.task(id).unwrap().unwrap().0, "queued");
        let again = q
            .enqueue(
                &NewTask {
                    external_id: Some("ask_9"),
                    ..NewTask::default()
                },
                now_ms(),
            )
            .unwrap();
        assert!(
            again.repeated,
            "the mirror's retry after a restart adds nothing"
        );
    }
}
