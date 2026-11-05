//! `iohr ask`: run the developer's own agent in this folder with the question.

use std::process::Stdio;
use std::time::Instant;

use crate::ask::{self, Agent, Via};
use crate::cli::{AskArgs, Global};
use crate::context::Ctx;
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn run(g: &Global, args: AskArgs, out: Out) -> Result<(), Error> {
    let cfg = Ctx::load(g)?.config.ask;
    let via = ask::choose_via(args.via.as_deref(), cfg.via.as_deref()).map_err(Error::Usage)?;
    let agent =
        ask::choose_agent(args.agent.as_deref(), cfg.agent.as_deref()).map_err(Error::Usage)?;
    if via == Via::Io {
        return Err(Error::Failed(
            "`--via io` is not built yet; use `--via local`".to_owned(),
        ));
    }
    if agent == Agent::Codex {
        return Err(Error::Failed(
            "`--agent codex` is not built yet; use `--agent claude`".to_owned(),
        ));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(bin) = ask::find_on_path(agent.name(), &path) else {
        return Err(Error::Failed(format!(
            "`{}` is not on your PATH: {}",
            agent.name(),
            agent.install_hint()
        )));
    };
    // The question is only ever an argument of the agent; nothing below prints or stores it.
    let question = args.question.join(" ");
    Out::note(&ask::label(agent));
    let started = Instant::now();
    let mut child = tokio::process::Command::new(bin)
        .arg(question)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| Error::Failed(format!("cannot start `{}`: {}", agent.name(), e.kind())))?;
    // Ctrl-C reaches the agent too (same process group); iohr waits for it instead of dying
    // first and leaving the terminal to a half-stopped agent.
    let status = loop {
        tokio::select! {
            s = child.wait() => break s,
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    .map_err(|e| {
        Error::Failed(format!(
            "waiting for `{}` failed: {}",
            agent.name(),
            e.kind()
        ))
    })?;
    let code = exit_code(status);
    if out.json {
        Out::print_json(&serde_json::json!({
            "via": via.name(),
            "agent": agent.name(),
            "exit_code": code,
            "duration_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }));
    }
    match u8::try_from(code) {
        Ok(0) => Ok(()),
        Ok(c) => Err(Error::Child(c)),
        Err(_) => Err(Error::Child(1)),
    }
}

#[cfg(unix)]
fn exit_code(s: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0))
}

#[cfg(not(unix))]
fn exit_code(s: std::process::ExitStatus) -> i32 {
    s.code().unwrap_or(1)
}
