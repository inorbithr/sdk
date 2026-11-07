//! The shell's side of the host's socket: ask it something, join it, stop it.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use super::lifecycle::Paths;

/// How long a host gets to answer one line.
const ANSWER: Duration = Duration::from_secs(5);

fn connect(paths: &Paths) -> std::io::Result<UnixStream> {
    let s = UnixStream::connect(&paths.sock)?;
    s.set_read_timeout(Some(ANSWER))?;
    s.set_write_timeout(Some(ANSWER))?;
    Ok(s)
}

fn ask_line(stream: &UnixStream, request: &str) -> std::io::Result<String> {
    let mut w = stream;
    writeln!(w, "{request}")?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    if answer.is_empty() {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    Ok(answer.trim_end().to_owned())
}

/// One request and its one-line answer, on a connection of its own.
pub(crate) fn ask(paths: &Paths, request: &str) -> std::io::Result<String> {
    ask_line(&connect(paths)?, request)
}

/// The pid of the host that answers, `None` when none does.
pub(crate) fn running(paths: &Paths) -> Option<u32> {
    let answer = ask(paths, "status").ok()?;
    answer.split(' ').nth(1)?.parse().ok()
}

/// An open session: the host counts this connection until it is dropped.
#[derive(Debug)]
pub(crate) struct Session {
    stream: UnixStream,
    pub(crate) host_pid: u32,
}

/// Joins the host, which must be running.
pub(crate) fn join(paths: &Paths) -> std::io::Result<Session> {
    let stream = connect(paths)?;
    let answer = ask_line(&stream, "session")?;
    let host_pid = answer
        .strip_prefix("ready ")
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| std::io::Error::other(format!("the host answered: {answer}")))?;
    // Held open from here on: no more reads time out.
    stream.set_read_timeout(None)?;
    Ok(Session { stream, host_pid })
}

impl Session {
    /// Blocks until the host closes the connection (it stopped); `true` when it did.
    pub(crate) fn wait_for_host(&self) -> bool {
        let mut line = String::new();
        matches!(
            BufReader::new(&self.stream).read_line(&mut line),
            Ok(0) | Err(_)
        )
    }

    /// A second handle on the connection, to wait on from another thread.
    pub(crate) fn watcher(&self) -> std::io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
            host_pid: self.host_pid,
        })
    }
}

/// What `stop` found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Stopped {
    /// No host was running.
    NotRunning,
    /// The host is gone.
    Gone(u32),
    /// The host was told, and still finishes the tasks its Roles hold.
    Stopping(u32),
}

/// Tells the host to stop and waits up to `wait` for it to go.
pub(crate) fn stop(paths: &Paths, wait: Duration) -> Stopped {
    let Some(pid) = running(paths) else {
        return Stopped::NotRunning;
    };
    match ask(paths, "stop") {
        Ok(_) => {}
        // It ended between the two questions.
        Err(_) => return Stopped::Gone(pid),
    }
    let end = Instant::now() + wait;
    while Instant::now() < end {
        if running(paths).is_none() {
            return Stopped::Gone(pid);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Stopped::Stopping(pid)
}
