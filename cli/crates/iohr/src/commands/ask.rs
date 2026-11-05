//! `iohr ask`: run the developer's own agent in this folder with the question.

use std::process::Stdio;
use std::time::{Duration, Instant};

use std::io::IsTerminal as _;
use std::path::Path;

use crate::ask::{self, Agent, Via};
use crate::card::{self, Body, Card, Look};
use crate::cli::{AskArgs, Global, HeadlessArgs};
use crate::context::Ctx;
use crate::error::Error;
use crate::headless::{self, Answer, Context};
use crate::output::Out;
use crate::ui::Ui;

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
    if headless_wanted(&args.headless, out.json)? {
        return headless(&bin, agent, via, &question, &args.headless, out).await;
    }
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

/// Answer once and print, or open the agent? Printing is what a prompt hook, a script or
/// the app needs, so it is the answer to `--headless`, any context flag, `--json`, and a
/// stdin or stdout that is not a terminal; only a person at a terminal gets the agent.
fn headless_wanted(h: &HeadlessArgs, json: bool) -> Result<bool, Error> {
    if h.interactive {
        if json {
            return Err(Error::Usage(
                "`--json` prints one answer and never opens the agent; drop `--interactive`"
                    .to_owned(),
            ));
        }
        return Ok(false);
    }
    Ok(h.headless
        || h.card
        || h.has_context()
        || json
        || !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()))
}

/// One capped `claude -p` run (see [`headless`]). The label goes first on stderr so a pipe
/// of the answer stays clean; under `--json` it is a field instead.
async fn headless(
    bin: &Path,
    agent: Agent,
    via: Via,
    question: &str,
    h: &HeadlessArgs,
    out: Out,
) -> Result<(), Error> {
    let ctx = Context {
        cwd: match &h.cwd {
            Some(c) => c.clone(),
            None => std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
        },
        command: h.last_command.clone().unwrap_or_default(),
        exit_code: h.exit_code,
        output: match &h.output_file {
            // Only the kind of failure: the path is the person's, what is in it is not said.
            Some(f) => std::fs::read_to_string(f)
                .map_err(|e| Error::Usage(format!("cannot read --output-file: {}", e.kind())))?,
            None => String::new(),
        },
        prior: h.prior_question.clone().zip(h.prior_answer.clone()),
    };
    let message = headless::assemble(question, &ctx).map_err(Error::Usage)?;
    let read = headless::read_line(&ctx);
    let fancy = Ui::detect(out.json).fancy();
    let stdout_tty = std::io::stdout().is_terminal();
    // A person reads a card; a pipe, a script and `--json` read the plain answer.
    if !out.json && (h.card || (stdout_tty && fancy)) {
        let look = Look::detect(fancy);
        let started = Instant::now();
        let run = headless::run(bin, &message, started);
        let done = if look.fancy {
            spin_while(run, agent, look, started).await?
        } else {
            run.await
        };
        let label = ask::label(agent);
        let notice = format!("An AI model {} {}", ask::PROVIDER, ask::CHECK);
        let mut shown = Card {
            agent: agent.display(),
            model: headless::MODEL,
            elapsed: None,
            follows: h.prior_question.as_deref(),
            read: &read,
            notice: &notice,
            label: &label,
            hint: h.card.then_some("`??` follows up."),
            body: Body::Failed(""),
        };
        let width = card::stderr_width();
        return match &done {
            Ok((text, duration_ms)) => {
                shown.elapsed = Some(Duration::from_millis(*duration_ms));
                shown.body = Body::Answer(text);
                card::draw(&card::render(&shown, width, look));
                if !stdout_tty {
                    Out::raw(format!("{text}\n").as_bytes());
                }
                Ok(())
            }
            Err(why) => {
                shown.elapsed = Some(started.elapsed());
                shown.body = Body::Failed(why);
                card::draw(&card::render(&shown, width, look));
                Err(Error::Child(1))
            }
        };
    }
    let label = format!("{} {read}", ask::label(agent));
    if !out.json {
        Out::note(&label);
    }
    let started = Instant::now();
    let (text, duration_ms) = headless::run(bin, &message, started)
        .await
        .map_err(Error::Failed)?;
    if out.json {
        Out::print_json(&Answer {
            text,
            agent: agent.name(),
            via: via.name(),
            label,
            model: headless::MODEL,
            exit_code: 0,
            duration_ms,
        });
    } else {
        Out::raw(format!("{text}\n").as_bytes());
    }
    Ok(())
}

/// Waits for `work` with a spinner on stderr. Ctrl-C clears the line and stops the run
/// (dropping `work` kills the agent) with the shell's 130.
async fn spin_while<T>(
    work: impl std::future::Future<Output = T>,
    agent: Agent,
    look: Look,
    started: Instant,
) -> Result<T, Error> {
    tokio::pin!(work);
    let stop = tokio::signal::ctrl_c();
    tokio::pin!(stop);
    let mut tick = tokio::time::interval(Duration::from_millis(90));
    let mut frame = 0usize;
    loop {
        tokio::select! {
            done = &mut work => {
                card::spin_clear();
                return Ok(done);
            }
            _ = &mut stop => {
                card::spin_clear();
                return Err(Error::Child(130));
            }
            _ = tick.tick() => {
                card::spin(frame, started.elapsed(), agent.display(), look);
                frame += 1;
            }
        }
    }
}
