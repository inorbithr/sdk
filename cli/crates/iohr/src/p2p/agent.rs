//! `iohr p2p register`: a background agent that keeps this machine in touch with the
//! bootstrap node (RFC 0230 section 4, slice 1).
//!
//! Slice 1 only proves the plumbing: the agent starts detached, reaches the bootstrap
//! address over TCP, backs off when it cannot, writes its state as a heartbeat and stops on
//! request. The Noise handshake and the report exchange are slice 2 (ADR 0242). The agent
//! writes no log: its only output is the state file, which holds the bootstrap address,
//! times and counts, never a report or a subject.
//!
//! The bootstrap host is the one named exception to "the SDK talks only to the API host"
//! (ADR 0242, sdk ADR 0018): HTTPS addresses only, plain HTTP on loopback for tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::runner::approvals::unix_now;

/// The environment variable that names the bootstrap node when `--bootstrap` is not given.
pub(crate) const BOOTSTRAP_ENV: &str = "IOHR_P2P_BOOTSTRAP";

/// How long a heartbeat stays fresh, in seconds. The agent writes at least every 35 s.
pub(crate) const FRESH: i64 = 90;

const STATE_FILE: &str = "state.json";
const STOP_FILE: &str = "stop";

/// The bootstrap node's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bootstrap {
    url: Url,
    host: String,
    port: u16,
}

impl Bootstrap {
    /// The address `--bootstrap` (or [`BOOTSTRAP_ENV`]) named.
    ///
    /// # Errors
    /// None was named, or the one named breaks a rule of [`Bootstrap::parse`].
    pub(crate) fn named(s: Option<&str>) -> Result<Self, String> {
        match s.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => Self::parse(s),
            None => Err(format!(
                "name the bootstrap node with --bootstrap https://HOST[:PORT] or {BOOTSTRAP_ENV}"
            )),
        }
    }

    /// `s` as a bootstrap address: `https://host[:port]`, or `http://` on loopback only; no
    /// user, query, fragment or path.
    ///
    /// # Errors
    /// The rule it breaks.
    pub(crate) fn parse(s: &str) -> Result<Self, String> {
        let url =
            Url::parse(s.trim()).map_err(|_| "the bootstrap address is not a URL".to_owned())?;
        let host = url
            .host()
            .ok_or_else(|| "the bootstrap address has no host".to_owned())?;
        let loopback = match &host {
            Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(a) => a.is_loopback(),
            Host::Ipv6(a) => a.is_loopback(),
        };
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            "http" => {
                return Err("the bootstrap address must be https (http only on loopback)".into());
            }
            _ => return Err("the bootstrap address must be https://HOST[:PORT]".into()),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("the bootstrap address cannot carry a user or password".into());
        }
        if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
            return Err("the bootstrap address is a host and port only".into());
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| "the bootstrap address has no port".to_owned())?;
        let host = match host {
            Host::Domain(d) => d.to_ascii_lowercase(),
            Host::Ipv4(a) => a.to_string(),
            Host::Ipv6(a) => a.to_string(),
        };
        Ok(Self { url, host, port })
    }

    /// The address as given, normalised (`https://host:port/`).
    pub(crate) fn as_str(&self) -> &str {
        self.url.as_str()
    }
}

/// Where the agent stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Starting,
    Reachable,
    Unreachable,
    Stopped,
}

impl Phase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Reachable => "reachable",
            Self::Unreachable => "unreachable",
            Self::Stopped => "stopped",
        }
    }
}

/// What the agent writes after every probe, and what `status` reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct State {
    pub(crate) bootstrap: String,
    pub(crate) pid: u32,
    pub(crate) started_at: i64,
    pub(crate) phase: Phase,
    pub(crate) last_contact_at: Option<i64>,
    pub(crate) attempts: u64,
    pub(crate) updated_at: i64,
}

impl State {
    /// Whether an agent is alive behind this state at `now`: not stopped, and written
    /// within [`FRESH`] seconds.
    pub(crate) fn running(&self, now: i64) -> bool {
        self.phase != Phase::Stopped && now - self.updated_at <= FRESH
    }
}

/// The agent's folder in the config directory.
pub(crate) fn dir(config_dir: &Path) -> PathBuf {
    config_dir.join("p2p")
}

/// The state in `dir`, if there is one that parses.
pub(crate) fn read_state(dir: &Path) -> Option<State> {
    let bytes = std::fs::read(dir.join(STATE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_state(dir: &Path, s: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| format!("p2p state: {e}"))?;
    iohr_auth::write_private(dir, STATE_FILE, &bytes)
        .map_err(|e| format!("cannot write the p2p state: {}", e.kind()))
}

fn stop_requested(dir: &Path) -> bool {
    dir.join(STOP_FILE).exists()
}

/// What `register` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Registered {
    /// An agent was already running; nothing was started.
    AlreadyRunning(State),
    /// A new agent was started with this process id.
    Started(u32),
}

/// Starts the agent for `bootstrap` unless one is already running, and returns at once.
/// `spawn` starts it (a detached `iohr p2p agent` in the program; a task in tests) and
/// returns its process id.
///
/// # Errors
/// The state cannot be written, or `spawn` failed.
pub(crate) fn register(
    dir: &Path,
    bootstrap: &Bootstrap,
    now: i64,
    spawn: impl FnOnce() -> Result<u32, String>,
) -> Result<Registered, String> {
    if let Some(s) = read_state(dir).filter(|s| s.running(now)) {
        return Ok(Registered::AlreadyRunning(s));
    }
    match std::fs::remove_file(dir.join(STOP_FILE)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot clear the old stop request: {}", e.kind())),
    }
    write_state(
        dir,
        &State {
            bootstrap: bootstrap.as_str().to_owned(),
            pid: 0,
            started_at: now,
            phase: Phase::Starting,
            last_contact_at: None,
            attempts: 0,
            updated_at: now,
        },
    )?;
    spawn().map(Registered::Started)
}

/// Starts `exe p2p agent` detached: its own process group, no terminal, no inherited
/// output. `config_dir` is passed on so the agent writes where `status` reads.
///
/// # Errors
/// The program could not be started.
pub(crate) fn spawn_detached(
    exe: &Path,
    config_dir: &Path,
    bootstrap: &Bootstrap,
) -> Result<u32, String> {
    use std::process::{Command, Stdio};
    let mut c = Command::new(exe);
    c.arg("--config-dir")
        .arg(config_dir)
        .args(["p2p", "agent", "--bootstrap", bootstrap.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        c.process_group(0);
    }
    c.spawn()
        .map(|child| child.id())
        .map_err(|e| format!("could not start the p2p agent: {}", e.kind()))
}

/// The agent's clock: how often it looks for a stop request, how long a probe may take,
/// how often it probes when the node answers, and how far it backs off when it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    pub(crate) tick: Duration,
    pub(crate) probe_timeout: Duration,
    pub(crate) reachable_every: Duration,
    pub(crate) first_retry: Duration,
    pub(crate) max_backoff: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            probe_timeout: Duration::from_secs(5),
            reachable_every: Duration::from_secs(30),
            first_retry: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Whether a TCP connection to the bootstrap node opens within `timeout`. Slice 2 puts the
/// Noise handshake on this connection; slice 1 closes it at once.
async fn probe(b: &Bootstrap, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(
            timeout,
            tokio::net::TcpStream::connect((b.host.as_str(), b.port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// The agent's loop: probe, write the state, wait, until a stop request appears. Writes
/// `stopped` before it returns.
///
/// # Errors
/// The state cannot be written.
pub(crate) async fn run(dir: &Path, bootstrap: &Bootstrap, timing: Timing) -> Result<(), String> {
    let now = unix_now();
    let mut state = State {
        bootstrap: bootstrap.as_str().to_owned(),
        pid: std::process::id(),
        started_at: now,
        phase: Phase::Starting,
        last_contact_at: None,
        attempts: 0,
        updated_at: now,
    };
    write_state(dir, &state)?;
    let mut backoff = timing.first_retry;
    loop {
        if stop_requested(dir) {
            break;
        }
        let reached = probe(bootstrap, timing.probe_timeout).await;
        let now = unix_now();
        state.attempts += 1;
        state.updated_at = now;
        let wait = if reached {
            state.phase = Phase::Reachable;
            state.last_contact_at = Some(now);
            backoff = timing.first_retry;
            timing.reachable_every
        } else {
            state.phase = Phase::Unreachable;
            let wait = backoff;
            backoff = (backoff * 2).min(timing.max_backoff);
            wait
        };
        write_state(dir, &state)?;
        let mut waited = Duration::ZERO;
        while waited < wait && !stop_requested(dir) {
            let step = timing.tick.min(wait.saturating_sub(waited));
            tokio::time::sleep(step).await;
            waited += step;
        }
    }
    state.phase = Phase::Stopped;
    state.updated_at = unix_now();
    write_state(dir, &state)
}

/// Asks the agent in `dir` to stop, waits up to `wait` for it to say `stopped`, then
/// removes the folder. Returns whether an agent was running.
///
/// # Errors
/// The stop request or the folder removal failed, or a running agent did not stop in time
/// (the folder is then kept so `status` still shows it).
pub(crate) async fn unregister(dir: &Path, wait: Duration, tick: Duration) -> Result<bool, String> {
    let running = read_state(dir).is_some_and(|s| s.running(unix_now()));
    if running {
        iohr_auth::write_private(dir, STOP_FILE, b"stop\n")
            .map_err(|e| format!("cannot ask the p2p agent to stop: {}", e.kind()))?;
        let mut waited = Duration::ZERO;
        while read_state(dir).is_some_and(|s| s.phase != Phase::Stopped) {
            if waited >= wait {
                return Err(format!(
                    "the p2p agent did not stop within {} s; run `iohr p2p unregister` again",
                    wait.as_secs()
                ));
            }
            tokio::time::sleep(tick).await;
            waited += tick;
        }
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(running),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(running),
        Err(e) => Err(format!("cannot remove the p2p folder: {}", e.kind())),
    }
}
