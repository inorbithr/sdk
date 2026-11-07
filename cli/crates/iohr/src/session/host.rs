//! The host's loop: it holds the lock, listens on the socket, recovers the queue, runs the
//! Roles, and stops when the last session leaves.
//!
//! A shell sends one line and gets one line back, except `session`, whose connection stays
//! open: that open connection is the session. The requests:
//!
//! - `session`: joins; the answer is `ready <pid>`, and the host stops when the last joined
//!   connection closes (a shell that is killed closes it too, so nothing can be left behind).
//! - `status`: `ok <pid> sessions=<n>`.
//! - `stop`: `stopping`; every Role finishes the task it holds, then the host ends.
//! - `enqueue <json>`: adds a task (`external_id`, `role_hint`, `payload`) and answers
//!   `queued <id> new` or `queued <id> repeated` (the same `external_id` adds nothing).

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::lifecycle::{Paths, take};
use super::pool::{self, Pace};
use super::queue::{NewTask, Queue, QueueError, Recovered, Task, now_ms};

/// The longest request line the host reads.
const MAX_LINE: u64 = 64 * 1024;

/// How the host paces itself, beyond the pool's.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub(crate) pace: Pace,
    /// A host nobody joined within this long ends (its starter died).
    pub(crate) first_join: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pace: Pace::default(),
            first_join: Duration::from_secs(30),
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
    let recovered = queue.recover(now_ms())?;
    queue.register_roles(roles, now_ms())?;
    let listener = UnixListener::bind(&paths.sock)?;
    listener.set_nonblocking(true)?;
    let sessions = AtomicUsize::new(0);
    let joined = AtomicBool::new(false);
    ready(recovered);
    let accept_queue = queue.handle()?;
    let result = std::thread::scope(|scope| {
        let (listening, counted, seen) = (&listener, &sessions, &joined);
        let acceptor = scope.spawn(move || {
            accept(
                listening,
                &accept_queue,
                stop,
                counted,
                seen,
                options.first_join,
            );
        });
        let out = pool::run(&queue, roles, handler, stop, options.pace);
        // Whatever ended the Roles, the acceptor and every connection end with them.
        stop.store(true, Ordering::Relaxed);
        let _ = acceptor.join();
        out
    });
    let _ = std::fs::remove_file(&paths.sock);
    Ok(result?)
}

fn accept(
    listener: &UnixListener,
    queue: &Queue,
    stop: &AtomicBool,
    sessions: &AtomicUsize,
    joined: &AtomicBool,
    first_join: Duration,
) {
    let started = Instant::now();
    std::thread::scope(|scope| {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let Ok(handle) = queue.handle() else { continue };
                    scope.spawn(move || talk(stream, &handle, stop, sessions, joined));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if !joined.load(Ordering::Relaxed) && started.elapsed() > first_join {
                        stop.store(true, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    });
}

/// Reads one line, waiting in short steps so a stopping host is noticed. `None` at the end of
/// the connection, an error, a line over [`MAX_LINE`], or when the host stops.
fn read_line(reader: &mut BufReader<UnixStream>, stop: &AtomicBool) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
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

fn talk(
    stream: UnixStream,
    queue: &Queue,
    stop: &AtomicBool,
    sessions: &AtomicUsize,
    joined: &AtomicBool,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let Some(line) = read_line(&mut reader, stop) else {
        return;
    };
    let line = String::from_utf8_lossy(&line);
    let line = line.trim_end();
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let reply = match word {
        "session" => {
            // Counted before the answer, so a shell that sees `ready` is in the count.
            sessions.fetch_add(1, Ordering::SeqCst);
            joined.store(true, Ordering::SeqCst);
            let _ = writeln!(out, "ready {}", std::process::id());
            // Held until the shell's end closes, however it closes.
            while read_line(&mut reader, stop).is_some() {}
            if sessions.fetch_sub(1, Ordering::SeqCst) == 1 {
                stop.store(true, Ordering::SeqCst);
            }
            return;
        }
        "status" => format!(
            "ok {} sessions={}",
            std::process::id(),
            sessions.load(Ordering::SeqCst)
        ),
        "stop" => {
            stop.store(true, Ordering::SeqCst);
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
        _ => "error unknown request".to_owned(),
    };
    let _ = writeln!(out, "{reply}");
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc;

    use super::super::link::{self, Stopped};
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
        }
    }

    /// A host running on its own thread in a folder of its own.
    struct Running {
        paths: Paths,
        stop: std::sync::Arc<AtomicBool>,
        done: mpsc::Receiver<Result<(), HostError>>,
        ran: std::sync::Arc<Mutex<Vec<String>>>,
        _dir: tempfile::TempDir,
    }

    impl Running {
        fn start(first_join: Duration) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::in_dir(dir.path().join("h"));
            paths.create().unwrap();
            Self::start_in(dir, paths, first_join)
        }

        fn start_in(dir: tempfile::TempDir, paths: Paths, first_join: Duration) -> Self {
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let ran = std::sync::Arc::new(Mutex::new(Vec::new()));
            let (ready_tx, ready_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            {
                let (paths, stop, ran) = (paths.clone(), stop.clone(), ran.clone());
                let handler = move |t: &Task, role: &str| {
                    ran.lock().unwrap().push(format!("{role}:{}", t.payload));
                    Ok("ok".to_owned())
                };
                std::thread::spawn(move || {
                    let out = serve(
                        &paths,
                        &roles(),
                        &handler,
                        &stop,
                        options(first_join),
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
