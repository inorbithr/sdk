//! The ask path end to end over the host's socket (RFC 0213, S2): an ask fanned out to the
//! Roles, answered concurrently, read back, and kept across a stop and a restart. The Roles'
//! turn is a closure here; its real form (`iohr ask --role` as a child) is [`super::turn`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::host::{self, HostError, Options};
use super::lifecycle::Paths;
use super::link;
use super::pool::{self, Pace};
use super::queue::{NewTask, Queue, Task, now_ms};
use super::route::{self, Landed, Outcome, Payload, Reply};

const SOON: Duration = Duration::from_secs(15);

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// A host on a thread of its own, answering with `handler`.
struct Host {
    paths: Paths,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), HostError>>>,
}

impl Host {
    fn start<H>(paths: &Paths, roles: &[&str], stop: &Arc<AtomicBool>, handler: H) -> Self
    where
        H: Fn(&Task, &str) -> Result<String, String> + Send + Sync + 'static,
    {
        paths.create().unwrap();
        let (p, s, r) = (paths.clone(), stop.clone(), names(roles));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            host::serve(
                &p,
                &r,
                &handler,
                &s,
                Options {
                    pace: Pace {
                        lease: Duration::from_secs(30),
                        poll: Duration::from_millis(5),
                    },
                    first_join: Duration::from_secs(120),
                    starter: None,
                },
                |_| {
                    let _ = ready_tx.send(());
                },
            )
        });
        ready_rx.recv_timeout(SOON).expect("the host did not start");
        Self {
            paths: paths.clone(),
            stop: stop.clone(),
            thread: Some(thread),
        }
    }

    /// `iohr stop`'s request, then waits for the host to end.
    fn stop_and_wait(&mut self) -> Result<(), HostError> {
        assert_eq!(link::ask(&self.paths, "stop").unwrap(), "stopping");
        self.thread.take().unwrap().join().unwrap()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn paths_in(dir: &tempfile::TempDir) -> Paths {
    Paths::in_dir(dir.path().join("h"))
}

/// A reply the way a Role stores it.
fn replied(text: &str) -> String {
    Reply {
        text: text.to_owned(),
        model: "m".into(),
        duration_ms: 1,
    }
    .stored()
}

fn wait_for<T>(what: &str, mut look: impl FnMut() -> Option<T>) -> T {
    let give_up = Instant::now() + SOON;
    loop {
        if let Some(v) = look() {
            return v;
        }
        assert!(Instant::now() < give_up, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Reads an ask back until none of its Roles is still working.
fn landed(paths: &Paths, ask: &str) -> Vec<Landed> {
    wait_for("the Roles' answers", || {
        let rows = route::poll(paths, ask).unwrap();
        (!rows.is_empty() && rows.iter().all(|r| !matches!(r.outcome, Outcome::Working)))
            .then_some(rows)
    })
}

fn answered(row: &Landed) -> &str {
    match &row.outcome {
        Outcome::Answered(r) => &r.text,
        other => panic!("{} has no answer: {other:?}", row.role),
    }
}

fn ask_roles(paths: &Paths, ask: &str, question: &str, roles: &[&str]) {
    let payload = Payload::new(ask, question, "/work");
    route::fan_out(paths, &payload, &names(roles)).unwrap();
}

#[test]
fn two_roles_answer_one_ask_at_the_same_time_and_the_card_reads_each_back() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(&dir);
    // Neither Role leaves the handler until both are inside it: one Role after the other
    // could never get past this, so passing means they ran at once.
    let inside = Arc::new(AtomicUsize::new(0));
    let both = inside.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let _host = Host::start(
        &paths,
        &["rust-engineer", "security"],
        &stop,
        move |t, role| {
            let ask = Payload::parse(&t.payload)?;
            both.fetch_add(1, Ordering::SeqCst);
            let give_up = Instant::now() + Duration::from_secs(10);
            while both.load(Ordering::SeqCst) < 2 {
                if Instant::now() > give_up {
                    return Err("the other Role never ran beside this one".to_owned());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(replied(&format!("{role} on {}", ask.question)))
        },
    );

    assert_eq!(
        link::ask(&paths, "roles").unwrap(),
        "ok rust-engineer,security"
    );
    assert_eq!(
        route::hosted_roles(&paths),
        Some(names(&["rust-engineer", "security"]))
    );
    ask_roles(&paths, "ask1", "why?", &["rust-engineer", "security"]);
    let rows = landed(&paths, "ask1");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].role, "rust-engineer");
    assert_eq!(answered(&rows[0]), "rust-engineer on why?");
    assert_eq!(rows[1].role, "security");
    assert_eq!(answered(&rows[1]), "security on why?");
}

#[test]
fn two_asks_at_once_keep_their_answers_apart_and_a_repeat_adds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(&dir);
    let stop = Arc::new(AtomicBool::new(false));
    let ran = Arc::new(AtomicUsize::new(0));
    let count = ran.clone();
    let _host = Host::start(&paths, &["a", "b"], &stop, move |t, role| {
        count.fetch_add(1, Ordering::SeqCst);
        let ask = Payload::parse(&t.payload)?;
        Ok(replied(&format!("{role}:{}:{}", ask.ask, ask.question)))
    });
    ask_roles(&paths, "one", "first", &["a", "b"]);
    ask_roles(&paths, "two", "second", &["a"]);
    // The same ask again is the same tasks.
    ask_roles(&paths, "one", "first", &["a", "b"]);
    let one = landed(&paths, "one");
    let two = landed(&paths, "two");
    assert_eq!(
        one.iter().map(answered).collect::<Vec<_>>(),
        ["a:one:first", "b:one:first"]
    );
    assert_eq!(
        two.iter().map(answered).collect::<Vec<_>>(),
        ["a:two:second"]
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(ran.load(Ordering::SeqCst), 3, "a repeat must not run again");
}

#[test]
fn an_ask_survives_a_stop_that_cancels_what_is_in_flight_and_the_next_host_answers_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(&dir);

    // The first host: the Role is mid-answer when `iohr stop` comes.
    let stop = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let (flag, seen) = (stop.clone(), started.clone());
    let mut first = Host::start(&paths, &["rust-engineer"], &stop, move |_, _| {
        seen.store(true, Ordering::SeqCst);
        while !flag.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(pool::INTERRUPTED.to_owned())
    });
    ask_roles(&paths, "keep", "still there?", &["rust-engineer"]);
    wait_for("the Role to start", || {
        started.load(Ordering::SeqCst).then_some(())
    });
    assert!(
        first.stop_and_wait().is_ok(),
        "stop must end the host cleanly"
    );
    assert!(!paths.sock.exists());
    drop(first);

    // On disk: queued again, no try spent, no failure.
    {
        let q = Queue::open(&paths.db).unwrap();
        let rows = q.answers(&route::prefix("keep")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "queued");
        assert_eq!(rows[0].result, None);
    }

    // The next host answers it, and the answer is the same one a first host would have given.
    let stop = Arc::new(AtomicBool::new(false));
    let _second = Host::start(&paths, &["rust-engineer"], &stop, |t, role| {
        let ask = Payload::parse(&t.payload)?;
        if t.attempts != 1 {
            return Err(format!("the stop spent a try: attempts {}", t.attempts));
        }
        Ok(replied(&format!("{role}: {}", ask.question)))
    });
    let rows = landed(&paths, "keep");
    assert_eq!(answered(&rows[0]), "rust-engineer: still there?");
}

#[test]
fn what_a_stopped_host_never_started_is_answered_by_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(&dir);
    {
        // Queued while the host ran, and the host went before any Role took it.
        paths.create().unwrap();
        let q = Queue::open(&paths.db).unwrap();
        q.register_roles(&names(&["security"]), now_ms()).unwrap();
        let body = serde_json::to_string(&Payload::new("late", "anyone?", "/work")).unwrap();
        q.enqueue(
            &NewTask {
                external_id: Some("ask:late:security"),
                role_hint: Some("security"),
                payload: &body,
            },
            now_ms(),
        )
        .unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let _host = Host::start(&paths, &["security"], &stop, |t, role| {
        Ok(replied(&format!(
            "{role} read {}",
            Payload::parse(&t.payload)?.question
        )))
    });
    assert_eq!(
        answered(&landed(&paths, "late")[0]),
        "security read anyone?"
    );
}

#[test]
fn a_role_that_fails_shows_its_reason_beside_the_ones_that_answered() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(&dir);
    let stop = Arc::new(AtomicBool::new(false));
    let _host = Host::start(&paths, &["a", "b"], &stop, |_, role| {
        if role == "b" {
            Err("claude is not on your PATH".to_owned())
        } else {
            Ok(replied("fine"))
        }
    });
    ask_roles(&paths, "mixed", "q", &["a", "b"]);
    let rows = landed(&paths, "mixed");
    assert_eq!(answered(&rows[0]), "fine");
    assert_eq!(
        rows[1].outcome,
        Outcome::Failed("claude is not on your PATH".into())
    );
}

#[test]
fn the_host_refuses_a_bad_ask_id_and_no_host_means_no_routing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(&dir);
    let stop = Arc::new(AtomicBool::new(false));
    let _host = Host::start(&paths, &["a"], &stop, |_, _| Ok(String::new()));
    assert_eq!(
        link::ask(&paths, "results a:b").unwrap(),
        "error bad ask id"
    );
    assert_eq!(link::ask(&paths, "results").unwrap(), "error bad ask id");
    assert_eq!(link::ask(&paths, "results nothing").unwrap(), "[]");
    assert!(route::poll(&paths, "a:b").is_err());
    // No host runs for a folder nobody started one in, so its asks go the way they always did.
    assert!(route::find_host(dir.path(), dir.path()).is_none());
}

#[test]
fn the_queue_reads_an_asks_tasks_gives_a_task_back_without_a_try_and_lists_its_roles() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(&dir.path().join("own").join("q.duckdb")).unwrap();
    q.register_roles(&names(&["b", "a"]), now_ms()).unwrap();
    assert_eq!(q.role_names().unwrap(), ["a", "b"]);
    for (ext, role) in [("ask:x:a", "a"), ("ask:x:b", "b"), ("ask:xy:a", "a")] {
        q.enqueue(
            &NewTask {
                external_id: Some(ext),
                role_hint: Some(role),
                payload: "p",
            },
            now_ms(),
        )
        .unwrap();
    }
    // The prefix ends at the colon: ask `x` is not ask `xy`.
    let x = q.answers("ask:x:").unwrap();
    assert_eq!(
        x.iter().map(|r| r.role.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );

    let t = q
        .claim("a", Duration::from_secs(30), now_ms())
        .unwrap()
        .unwrap();
    assert_eq!(t.attempts, 1);
    assert!(!q.release(t.id, "b", "not mine", now_ms()).unwrap());
    assert!(q.release(t.id, "a", "stopping", now_ms()).unwrap());
    let again = q
        .claim("a", Duration::from_secs(30), now_ms())
        .unwrap()
        .unwrap();
    assert_eq!((again.id, again.attempts), (t.id, 1));
    q.complete(again.id, "a", Some("r"), now_ms()).unwrap();
    let done = &q.answers("ask:x:").unwrap()[0];
    assert_eq!(
        (done.state.as_str(), done.result.as_deref()),
        ("done", Some("r"))
    );
}
