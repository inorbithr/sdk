//! `iohr p2p`: the background agent that joins this machine to the report network
//! (RFC 0230). Calls no API: the agent reaches only the bootstrap node (ADR 0242).

use std::time::Duration;

use serde_json::json;

use crate::Env;
use crate::cli::{Global, P2pCommand};
use crate::context::config_dir;
use crate::error::Error;
use crate::output::Out;
use crate::p2p::agent::{self, Bootstrap, Registered, Timing};
use crate::runner::approvals::unix_now;

/// How long `unregister` waits for the agent to stop.
const STOP_WAIT: Duration = Duration::from_secs(5);

pub(crate) async fn run(g: &Global, _env: &Env, cmd: P2pCommand, out: Out) -> Result<(), Error> {
    match cmd {
        P2pCommand::Register { bootstrap } => register(g, bootstrap.as_deref(), out),
        P2pCommand::Status => status(g, out),
        P2pCommand::Unregister => unregister(g, out).await,
        P2pCommand::Peers => Err(not_yet("peers")),
        P2pCommand::Publish => Err(not_yet("publish")),
        P2pCommand::Subscribe => Err(not_yet("subscribe")),
        P2pCommand::Retract => Err(not_yet("retract")),
        P2pCommand::Agent { bootstrap } => {
            let b = Bootstrap::named(Some(&bootstrap)).map_err(Error::Usage)?;
            let dir = agent::dir(&config_dir(g)?);
            agent::run(&dir, &b, Timing::default())
                .await
                .map_err(Error::Failed)
        }
    }
}

fn not_yet(verb: &str) -> Error {
    Error::Failed(format!(
        "iohr p2p {verb} is not implemented yet: it comes with the peer exchange (RFC 0230 slice 2)"
    ))
}

fn register(g: &Global, bootstrap: Option<&str>, out: Out) -> Result<(), Error> {
    let b = Bootstrap::named(bootstrap).map_err(Error::Usage)?;
    let config = config_dir(g)?;
    let dir = agent::dir(&config);
    let exe = std::env::current_exe()
        .map_err(|e| Error::Failed(format!("cannot find this program: {}", e.kind())))?;
    let done = agent::register(&dir, &b, unix_now(), || {
        agent::spawn_detached(&exe, &config, &b)
    })
    .map_err(Error::Failed)?;
    let (started, pid, address) = match &done {
        Registered::AlreadyRunning(s) => (false, s.pid, s.bootstrap.clone()),
        Registered::Started(pid) => (true, *pid, b.as_str().to_owned()),
    };
    if out.json {
        Out::print_json(&json!({"started": started, "pid": pid, "bootstrap": address}));
    } else {
        Out::pairs(&[
            (
                "agent",
                if started {
                    "started"
                } else {
                    "already running"
                }
                .to_owned(),
            ),
            ("pid", pid.to_string()),
            ("bootstrap", address),
        ]);
        Out::note("`iohr p2p status` shows when it reaches the bootstrap node.");
    }
    Ok(())
}

fn when(t: Option<i64>) -> String {
    t.and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map_or_else(|| "never".to_owned(), |d| d.to_rfc3339())
}

fn status(g: &Global, out: Out) -> Result<(), Error> {
    let dir = agent::dir(&config_dir(g)?);
    let now = unix_now();
    let Some(s) = agent::read_state(&dir) else {
        if out.json {
            Out::print_json(&json!({"registered": false}));
        } else {
            Out::pairs(&[("state", "not registered".to_owned())]);
            Out::note("`iohr p2p register --bootstrap https://HOST` starts the agent.");
        }
        return Ok(());
    };
    let running = s.running(now);
    // A state nobody has written for a while: the agent died without saying so.
    let state = if running || s.phase == agent::Phase::Stopped {
        s.phase.as_str()
    } else {
        "not running"
    };
    if out.json {
        Out::print_json(&json!({
            "registered": true,
            "running": running,
            "state": state,
            "bootstrap": s.bootstrap,
            "pid": s.pid,
            "started_at": s.started_at,
            "last_contact_at": s.last_contact_at,
            "attempts": s.attempts,
            "updated_at": s.updated_at,
        }));
    } else {
        Out::pairs(&[
            ("state", state.to_owned()),
            ("bootstrap", s.bootstrap.clone()),
            ("pid", s.pid.to_string()),
            ("last contact", when(s.last_contact_at)),
            ("attempts", s.attempts.to_string()),
            ("updated", when(Some(s.updated_at))),
        ]);
    }
    Ok(())
}

async fn unregister(g: &Global, out: Out) -> Result<(), Error> {
    let dir = agent::dir(&config_dir(g)?);
    let was_running = agent::unregister(&dir, STOP_WAIT, Duration::from_millis(100))
        .await
        .map_err(Error::Failed)?;
    if out.json {
        Out::print_json(&json!({"stopped": was_running, "removed": true}));
    } else {
        Out::pairs(&[(
            "agent",
            if was_running {
                "stopped"
            } else {
                "was not running"
            }
            .to_owned(),
        )]);
    }
    Ok(())
}
