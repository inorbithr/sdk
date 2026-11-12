//! The host's loop: it holds the lock, listens on the socket, recovers the queue, runs the
//! Roles, and stops when the last session leaves.
//!
//! A shell sends one line and gets one line back, except `session`, whose connection stays
//! open: that open connection is the session. The requests:
//!
//! - `session`: joins; the answer is `ready <pid>`, and the host stops when the last joined
//!   connection closes (a shell that is killed closes it too, so nothing can be left behind).
//! - `status`: `ok <pid> sessions=<n>`, or `ok <pid> stopping` once the host drains.
//! - `stop`: `stopping`; every Role finishes the task it holds, then the host ends.
//! - `enqueue <json>`: adds a task (`external_id`, `role_hint`, `payload`) and answers
//!   `queued <id> new` or `queued <id> repeated` (the same `external_id` adds nothing).
//!
//! - `roles`: `ok <name>,<name>...`, the Roles this host runs.
//! - `results <ask>`: one line of JSON, the tasks of that ask (see [`super::route`]), each with
//!   its Role, state and, once it landed, its answer or reason.
//!
//! Anything else is answered `error ...`, and so is every request past [`MAX_CONNECTIONS`]
//! (`error busy`).
//!
//! Stopping has two steps. `stop` (the flag the caller passes in, set by a signal, `stop`, the
//! last session leaving, or a host nobody joined) *drains*: the Roles finish the task they hold
//! and take no more. The socket is kept answering all through, so a shell that asks sees
//! `stopping`, not a silence, and `iohr stop` can wait for the lock. Only when the Roles are
//! done does `down` end the acceptor and close every connection.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::lifecycle::{Paths, take};
use super::pool::{self, Pace};
use super::queue::{NewTask, Queue, QueueError, Recovered, Task, now_ms};

/// The longest request line the host reads.
const MAX_LINE: u64 = 64 * 1024;

/// How many connections a host holds at once (every one is a thread). Sessions are one per
/// open shell, so this is far more than a person opens.
pub(crate) const MAX_CONNECTIONS: usize = 64;

/// How the host paces itself, beyond the pool's.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub(crate) pace: Pace,
    /// A host nobody joined within this long ends (its starter died).
    pub(crate) first_join: Duration,
    /// The process that started this host (its parent then). A host nobody has joined ends
    /// as soon as that process is gone, not [`Self::first_join`] later: a shell killed
    /// between starting the host and joining it leaves nobody to join.
    pub(crate) starter: Option<i32>,
}

impl Options {
    /// The options of a host started by the process that is its parent now.
    pub(crate) fn started_by_parent() -> Self {
        Self {
            starter: rustix::process::getppid().map(rustix::process::Pid::as_raw_pid),
            ..Self::default()
        }
    }

    /// Whether the starter is gone: this process was handed to another parent.
    fn starter_gone(&self) -> bool {
        self.starter.is_some_and(|starter| {
            rustix::process::getppid().map(rustix::process::Pid::as_raw_pid) != Some(starter)
        })
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pace: Pace::default(),
            first_join: Duration::from_secs(30),
            starter: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostError {
    #[error("a session host already runs for this project")]
    AlreadyRunning,
    #[error("the session host: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Queue(#[from] QueueError),
}

#[derive(Deserialize)]
struct Enqueue {
    #[serde(default)]
    external_id: Option<String>,
    #[serde(default)]
    role_hint: Option<String>,
    payload: String,
}

/// Runs the host until `stop` is set (by a signal, by `stop`, or by the last session leaving).
///
/// `handler` is what a Role does with a task: `Ok(reference)` ends it `done`, `Err(reason)`
/// `failed`. `ready` is called once the socket accepts connections.
pub(crate) fn serve<H>(
    paths: &Paths,
    roles: &[String],
    handler: &H,
    stop: &AtomicBool,
    options: Options,
    ready: impl FnOnce(Recovered),
) -> Result<(), HostError>
where
    H: Fn(&Task, &str) -> Result<String, String> + Sync,
{
    let Some(_lock) = take(paths)? else {
        return Err(HostError::AlreadyRunning);
    };
    // The lock is ours, so a socket file left here belongs to a host that died.
    let _ = std::fs::remove_file(&paths.sock);
    let queue = Queue::open(&paths.db)?;
    // The lock is ours: no other host is alive, so every claim in the file is an orphan.
    let recovered = queue.recover_all()?;
    queue.register_roles(roles, now_ms())?;
    let listener = UnixListener::bind(&paths.sock)?;
    // The folder is the owner's alone already; the socket says so too.
    std::fs::set_permissions(&paths.sock, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let net = Net {
        stop,
        down: AtomicBool::new(false),
        sessions: AtomicUsize::new(0),
        connections: AtomicUsize::new(0),
        joined: AtomicBool::new(false),
    };
    ready(recovered);
    let accept_queue = queue.handle()?;
    let result = std::thread::scope(|scope| {
        let (listening, shared) = (&listener, &net);
        let acceptor = scope.spawn(move || {
            accept(listening, &accept_queue, shared, options);
        });
        let out = pool::run(&queue, roles, handler, stop, options.pace);
        // The Roles are done (whatever ended them): now the acceptor and every connection end.
        stop.store(true, Ordering::SeqCst);
        net.down.store(true, Ordering::SeqCst);
        let _ = acceptor.join();
        out
    });
    let _ = std::fs::remove_file(&paths.sock);
    Ok(result?)
}

/// What the acceptor and the connections share.
struct Net<'a> {
    /// The host drains: Roles finish what they hold and take nothing more.
    stop: &'a AtomicBool,
    /// The Roles are done: connections close.
    down: AtomicBool,
    /// Open `session` connections.
    sessions: AtomicUsize,
    /// Every open connection.
    connections: AtomicUsize,
    /// Somebody has joined, ever.
    joined: AtomicBool,
}

impl Net<'_> {
    fn draining(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// Counts a connection while it lives.
struct Held<'a>(&'a AtomicUsize);

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept(listener: &UnixListener, queue: &Queue, net: &Net<'_>, options: Options) {
    let started = Instant::now();
    std::thread::scope(|scope| {
        while !net.down.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if net.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        net.connections.fetch_sub(1, Ordering::SeqCst);
                        refuse(stream);
                        continue;
                    }
                    let held = Held(&net.connections);
                    let Ok(handle) = queue.handle() else { continue };
                    scope.spawn(move || {
                        let _held = held;
                        talk(stream, &handle, net);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if !net.joined.load(Ordering::Relaxed)
                        && (started.elapsed() > options.first_join || options.starter_gone())
                    {
                        net.stop.store(true, Ordering::SeqCst);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    });
}

/// Says the host is full, without reading anything and without a thread.
fn refuse(mut stream: UnixStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = writeln!(stream, "error busy");
}

/// Reads one line, waiting in short steps so a stopping host is noticed. `None` at the end of
/// the connection, an error, a line over [`MAX_LINE`], or when the host is down.
fn read_line(reader: &mut BufReader<UnixStream>, down: &AtomicBool) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        if down.load(Ordering::Relaxed) {
            return None;
        }
        let mut limited = reader.by_ref().take(MAX_LINE);
        match limited.read_until(b'\n', &mut line) {
            Ok(0) => return None,
            Ok(_) if line.ends_with(b"\n") => return Some(line),
            Ok(_) if line.len() as u64 >= MAX_LINE => return None,
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return None,
        }
    }
}

fn talk(stream: UnixStream, queue: &Queue, net: &Net<'_>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let Some(line) = read_line(&mut reader, &net.down) else {
        return;
    };
    let line = String::from_utf8_lossy(&line);
    let line = line.trim_end();
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let reply = match word {
        "session" if net.draining() => "error stopping".to_owned(),
        "session" => {
            // Counted before the answer, so a shell that sees `ready` is in the count.
            net.sessions.fetch_add(1, Ordering::SeqCst);
            net.joined.store(true, Ordering::SeqCst);
            let _ = writeln!(out, "ready {}", std::process::id());
            // Held until the shell's end closes, however it closes.
            while read_line(&mut reader, &net.down).is_some() {}
            if net.sessions.fetch_sub(1, Ordering::SeqCst) == 1 {
                net.stop.store(true, Ordering::SeqCst);
            }
            return;
        }
        "status" if net.draining() => format!("ok {} stopping", std::process::id()),
        "status" => format!(
            "ok {} sessions={}",
            std::process::id(),
            net.sessions.load(Ordering::SeqCst)
        ),
        "stop" => {
            net.stop.store(true, Ordering::SeqCst);
            "stopping".to_owned()
        }
        "enqueue" => match serde_json::from_str::<Enqueue>(rest) {
            Ok(t) => match queue.enqueue(
                &NewTask {
                    external_id: t.external_id.as_deref(),
                    role_hint: t.role_hint.as_deref(),
                    payload: &t.payload,
                },
                now_ms(),
            ) {
                Ok(e) => format!(
                    "queued {} {}",
                    e.id,
                    if e.repeated { "repeated" } else { "new" }
                ),
                Err(e) => format!("error {e}"),
            },
            Err(e) => format!("error bad task: {e}"),
        },
        "roles" => match queue.role_names() {
            Ok(names) => format!("ok {}", names.join(",")),
            Err(e) => format!("error {e}"),
        },
        "results" => results_line(queue, rest.trim()),
        _ => "error unknown request".to_owned(),
    };
    let _ = writeln!(out, "{reply}");
}

/// The `results` answer: the ask's tasks as one JSON array. An ask id is made of letters and
/// digits and is not long, so nothing else is looked up.
fn results_line(queue: &Queue, ask: &str) -> String {
    if !super::route::valid_ask(ask) {
        return "error bad ask id".to_owned();
    }
    match queue.answers(&super::route::prefix(ask)) {
        Ok(tasks) => serde_json::Value::Array(
            tasks
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "role": t.role,
                        "state": t.state,
                        "result": t.result,
                        "reason": t.reason,
                    })
                })
                .collect(),
        )
        .to_string(),
        Err(e) => format!("error {e}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc;

    use super::super::link::{self, Stopped};
    use super::super::queue::{LEASE, MAX_ATTEMPTS};
    use super::*;

    fn roles() -> Vec<String> {
        vec!["rust".to_owned(), "security".to_owned()]
    }

    fn options(first_join: Duration) -> Options {
        Options {
            pace: Pace {
                lease: Duration::from_secs(30),
                poll: Duration::from_millis(5),
            },
            first_join,
            starter: None,
        }
    }

    /// A host running on its own thread in a folder of its own.
    struct Running {
        paths: Paths,
        stop: std::sync::Arc<AtomicBool>,
        done: mpsc::Receiver<Result<(), HostError>>,
        ran: std::sync::Arc<Mutex<Vec<String>>>,
        /// Set when a handler call has returned.
        finished: std::sync::Arc<AtomicBool>,
        _dir: tempfile::TempDir,
    }

    impl Running {
        fn start(first_join: Duration) -> Self {
            Self::start_busy(first_join, Duration::ZERO)
        }

        /// A host whose handler takes `busy` for every task.
        fn start_busy(first_join: Duration, busy: Duration) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::in_dir(dir.path().join("h"));
            paths.create().unwrap();
            Self::start_with(dir, paths, first_join, busy, None)
        }

        fn start_in(dir: tempfile::TempDir, paths: Paths, first_join: Duration) -> Self {
            Self::start_with(dir, paths, first_join, Duration::ZERO, None)
        }

        fn start_with(
            dir: tempfile::TempDir,
            paths: Paths,
            first_join: Duration,
            busy: Duration,
            starter: Option<i32>,
        ) -> Self {
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let ran = std::sync::Arc::new(Mutex::new(Vec::new()));
            let finished = std::sync::Arc::new(AtomicBool::new(false));
            let (ready_tx, ready_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            {
                let (paths, stop, ran) = (paths.clone(), stop.clone(), ran.clone());
                let finished = finished.clone();
                let handler = move |t: &Task, role: &str| {
                    ran.lock().unwrap().push(format!("{role}:{}", t.payload));
                    std::thread::sleep(busy);
                    finished.store(true, Ordering::SeqCst);
                    Ok("ok".to_owned())
                };
                std::thread::spawn(move || {
                    let out = serve(
                        &paths,
                        &roles(),
                        &handler,
                        &stop,
                        Options {
                            starter,
                            ..options(first_join)
                        },
                        |_| {
                            let _ = ready_tx.send(());
                        },
                    );
                    let _ = done_tx.send(out);
                });
            }
            // `ready` never comes when the host cannot start: the answer is then `done`.
            let give_up = Instant::now() + Duration::from_secs(20);
            while ready_rx.try_recv().is_err() && Instant::now() < give_up {
                if let Ok(early) = done_rx.try_recv() {
                    let (tx, rx) = mpsc::channel();
                    tx.send(early).unwrap();
                    return Self {
                        paths,
                        stop,
                        done: rx,
                        ran,
                        finished,
                        _dir: dir,
                    };
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Self {
                paths,
                stop,
                done: done_rx,
                ran,
                finished,
                _dir: dir,
            }
        }

        fn ended(&self, within: Duration) -> Option<Result<(), HostError>> {
            self.done.recv_timeout(within).ok()
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = self.done.recv_timeout(Duration::from_secs(10));
        }
    }

    const LONG: Duration = Duration::from_secs(60);
    const SOON: Duration = Duration::from_secs(10);

    #[test]
    fn the_host_ends_when_its_only_session_closes_and_leaves_no_socket() {
        let host = Running::start(LONG);
        let session = link::join(&host.paths).unwrap();
        assert_eq!(session.host_pid, std::process::id());
        assert_eq!(link::running(&host.paths), Some(std::process::id()));
        drop(session);
        assert!(host.ended(SOON).unwrap().is_ok(), "the host must end");
        assert!(!host.paths.sock.exists(), "the socket is removed");
        assert_eq!(link::running(&host.paths), None);
    }

    #[test]
    fn with_two_sessions_the_host_ends_after_the_last_one() {
        let host = Running::start(LONG);
        let first = link::join(&host.paths).unwrap();
        let second = link::join(&host.paths).unwrap();
        assert_eq!(
            link::ask(&host.paths, "status").unwrap(),
            format!("ok {} sessions=2", std::process::id())
        );
        drop(first);
        assert!(host.ended(Duration::from_millis(500)).is_none());
        assert_eq!(
            link::ask(&host.paths, "status").unwrap(),
            format!("ok {} sessions=1", std::process::id())
        );
        drop(second);
        assert!(host.ended(SOON).unwrap().is_ok());
    }

    #[test]
    fn stop_ends_a_host_and_the_sessions_see_it_go() {
        let host = Running::start(LONG);
        let session = link::join(&host.paths).unwrap();
        let watcher = session.watcher().unwrap();
        let seen = std::thread::spawn(move || watcher.wait_for_host());
        assert_eq!(
            link::stop(&host.paths, SOON),
            Stopped::Gone(std::process::id())
        );
        assert!(seen.join().unwrap(), "the session learns the host stopped");
        assert!(host.ended(SOON).unwrap().is_ok());
        assert_eq!(link::stop(&host.paths, SOON), Stopped::NotRunning);
    }

    /// `iohr stop` must not say "stopped" while a Role is still working: the host keeps
    /// answering `status` (as stopping) through the drain, and `stop` returns only when the
    /// lock is free. The handler takes longer than the 5 s a shell waits for one answer.
    #[test]
    fn stop_waits_for_a_busy_role_and_status_answers_while_it_drains() {
        let host = Running::start_busy(LONG, Duration::from_secs(6));
        let pid = std::process::id();
        let queued = link::ask(
            &host.paths,
            r#"enqueue {"external_id":"a","payload":"slow"}"#,
        );
        assert!(queued.unwrap().starts_with("queued"));
        let give_up = Instant::now() + SOON;
        while host.ran.lock().unwrap().is_empty() {
            assert!(Instant::now() < give_up, "no Role took the task");
            std::thread::sleep(Duration::from_millis(10));
        }
        // Told to stop, with no time to wait: the Role is still busy, so it says so.
        assert_eq!(
            link::stop(&host.paths, Duration::from_millis(300)),
            Stopped::Stopping(pid)
        );
        assert!(!host.finished.load(Ordering::SeqCst));
        // Draining, the host still answers, and says what it is doing.
        assert_eq!(
            link::ask(&host.paths, "status").unwrap(),
            format!("ok {pid} stopping")
        );
        // Waiting returns only after the Role has finished its task.
        assert_eq!(
            link::stop(&host.paths, Duration::from_secs(30)),
            Stopped::Gone(pid)
        );
        assert!(
            host.finished.load(Ordering::SeqCst),
            "stopped while a Role was busy"
        );
        assert!(host.ended(SOON).unwrap().is_ok());
        assert!(!host.paths.sock.exists());
        assert_eq!(link::stop(&host.paths, SOON), Stopped::NotRunning);
    }

    #[test]
    fn the_socket_is_for_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let host = Running::start(LONG);
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&host.paths.sock), 0o600);
        assert_eq!(mode(&host.paths.dir), 0o700);
    }

    /// A host holds at most [`MAX_CONNECTIONS`] connections: the next one is told so and
    /// dropped, the others go on, and a place is free again once a session leaves.
    #[test]
    fn connections_are_capped_and_a_place_frees_up_when_one_leaves() {
        let host = Running::start(LONG);
        let mut open: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| link::join(&host.paths).unwrap())
            .collect();
        assert_eq!(link::ask(&host.paths, "status").unwrap(), "error busy");
        open.pop();
        let give_up = Instant::now() + SOON;
        loop {
            let said = link::ask(&host.paths, "status").unwrap();
            if said.starts_with("ok ") {
                assert!(
                    said.ends_with(&format!("sessions={}", MAX_CONNECTIONS - 1)),
                    "{said}"
                );
                break;
            }
            assert!(Instant::now() < give_up, "no place freed: {said}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A host started on a file where a task was killed on its last try must not leave it
    /// `running` for ever: the lock says nobody else holds it, so it fails at once instead of
    /// waiting for a lease that no one renews.
    #[test]
    fn a_task_killed_on_its_last_try_is_failed_by_the_next_host_not_left_running() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().join("h"));
        paths.create().unwrap();
        let id = {
            let q = Queue::open(&paths.db).unwrap();
            q.register_roles(&roles(), now_ms()).unwrap();
            let id = q
                .enqueue(
                    &NewTask {
                        external_id: None,
                        role_hint: None,
                        payload: "p",
                    },
                    now_ms(),
                )
                .unwrap()
                .id;
            for n in 1..=MAX_ATTEMPTS {
                let t = q.claim("rust", LEASE, now_ms()).unwrap().unwrap();
                assert_eq!(t.attempts, n);
                q.start(id, "rust", now_ms()).unwrap();
                if n < MAX_ATTEMPTS {
                    // Killed, and the next host came back after the lease.
                    q.recover_all().unwrap();
                }
            }
            // Killed on the third try, and the next host starts within the lease.
            id
        };
        let host = Running::start_in(dir, paths, LONG);
        host.stop.store(true, Ordering::SeqCst);
        assert!(host.ended(SOON).unwrap().is_ok());
        let q = Queue::open(&host.paths.db).unwrap();
        let (state, attempts, _) = q.task(id).unwrap().unwrap();
        assert_eq!((state.as_str(), attempts), ("failed", MAX_ATTEMPTS));
        assert!(host.ran.lock().unwrap().is_empty(), "it must not run again");
    }

    #[test]
    fn a_second_host_for_the_same_project_is_refused() {
        let host = Running::start(LONG);
        let stop = AtomicBool::new(false);
        let handler = |_: &Task, _: &str| Ok(String::new());
        let again = serve(
            &host.paths,
            &roles(),
            &handler,
            &stop,
            options(LONG),
            |_| panic!("it must not start"),
        );
        assert!(matches!(again, Err(HostError::AlreadyRunning)));
        // The first one is untouched.
        assert_eq!(link::running(&host.paths), Some(std::process::id()));
    }

    /// A host whose starter is gone ends at once when nobody joined, not after `first_join`.
    #[test]
    fn a_host_whose_starter_is_gone_and_nobody_joined_ends_at_once() {
        // A pid that is not this process's parent: the starter is gone from the host's view.
        let gone = Options {
            starter: Some(i32::MAX),
            ..options(LONG)
        };
        assert!(gone.starter_gone());
        assert!(
            !Options::started_by_parent().starter_gone(),
            "the parent is still the parent"
        );
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().join("h"));
        paths.create().unwrap();
        let host = Running::start_with(dir, paths, LONG, Duration::ZERO, gone.starter);
        assert!(
            host.ended(Duration::from_secs(5)).is_some(),
            "the host outlived its starter with nobody joined"
        );
        assert!(!host.paths.sock.exists());
    }

    #[test]
    fn a_host_nobody_joins_ends_by_itself() {
        let host = Running::start(Duration::from_millis(300));
        assert!(host.ended(SOON).unwrap().is_ok());
        assert!(!host.paths.sock.exists());
    }

    #[test]
    fn a_stale_socket_and_lock_file_do_not_block_a_start() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().join("h"));
        paths.create().unwrap();
        // What a killed host leaves: a socket file nobody listens on, and the lock file.
        drop(UnixListener::bind(&paths.sock).unwrap());
        std::fs::write(&paths.lock, "").unwrap();
        assert!(paths.sock.exists());
        let host = Running::start_in(dir, paths, LONG);
        assert_eq!(link::running(&host.paths), Some(std::process::id()));
    }

    #[test]
    fn enqueue_over_the_socket_runs_on_a_role_and_a_repeat_adds_nothing() {
        let host = Running::start(LONG);
        let req = r#"enqueue {"external_id":"ask_7","role_hint":"security","payload":"look"}"#;
        let first = link::ask(&host.paths, req).unwrap();
        assert!(first.ends_with(" new"), "{first}");
        let again = link::ask(&host.paths, req).unwrap();
        assert_eq!(again, first.replace(" new", " repeated"));
        let give_up = Instant::now() + SOON;
        while host.ran.lock().unwrap().is_empty() {
            assert!(Instant::now() < give_up, "no Role took the task");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(*host.ran.lock().unwrap(), vec!["security:look".to_owned()]);
    }

    #[test]
    fn a_bad_request_is_answered_and_the_host_goes_on() {
        let host = Running::start(LONG);
        assert_eq!(
            link::ask(&host.paths, "dance").unwrap(),
            "error unknown request"
        );
        assert!(
            link::ask(&host.paths, "enqueue {not json")
                .unwrap()
                .starts_with("error bad task")
        );
        assert!(
            link::ask(&host.paths, r#"enqueue {"payload":1}"#)
                .unwrap()
                .starts_with("error bad task")
        );
        assert_eq!(link::running(&host.paths), Some(std::process::id()));
    }

    #[test]
    fn a_line_with_no_end_cannot_hang_the_host() {
        let host = Running::start(LONG);
        let mut stream = std::os::unix::net::UnixStream::connect(&host.paths.sock).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        // More than the longest line, and no newline.
        let junk = vec![b'x'; usize::try_from(MAX_LINE).unwrap() + 4096];
        let _ = stream.write_all(&junk);
        let mut answer = String::new();
        // The host drops the connection: the read ends (empty or reset), it does not wait.
        let _ = std::io::BufReader::new(&stream).read_line(&mut answer);
        assert_eq!(answer, "");
        assert_eq!(link::running(&host.paths), Some(std::process::id()));
    }
}
