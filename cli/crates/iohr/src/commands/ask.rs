//! `iohr ask`: the developer's own agent answers once in this folder, streamed to the terminal.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncBufReadExt as _;

use crate::ask::stream::{self, Look, Renderer};
use crate::ask::{self, Agent, Via};
use crate::card::{self, Body, Card, Look as CardLook};
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
    let model = ask::choose_model(agent, args.model.as_deref(), cfg.model.as_deref())
        .map_err(Error::Usage)?;
    let thinking = ask::choose_thinking(args.thinking.as_deref(), cfg.thinking.as_deref())
        .map_err(Error::Usage)?;
    let allow_edits = args.allow_edits || cfg.allow_edits.unwrap_or(false);
    if via == Via::Io {
        return Err(Error::Failed(
            "`--via io` is not built yet; use `--via local`".to_owned(),
        ));
    }
    if agent == Agent::Codex && headless_wanted(&args.headless) {
        return Err(Error::Failed(
            "`--agent codex` is not built yet for `--headless`, `--card` or a context flag; use `--agent claude`"
                .to_owned(),
        ));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let bin = ask::find_on_path(agent.name(), &path);
    // The question is only ever an argument of the agent; nothing below prints or stores it
    // except `--dry-run`, which prints the command line you asked to see.
    let question = args.question.join(" ");
    let command_line = ask::argv(agent, model.as_deref(), allow_edits, None, &question);
    if args.dry_run {
        let program = bin
            .as_ref()
            .map_or_else(|| agent.name().to_owned(), |b| b.display().to_string());
        if out.json {
            Out::print_json(&serde_json::json!({
                "dry_run": true,
                "via": via.name(),
                "agent": agent.name(),
                "found": bin.is_some(),
                "program": program,
                "args": command_line,
            }));
        } else {
            Out::raw(format!("{}\n", ask::shell_line(&program, &command_line)).as_bytes());
        }
        return Ok(());
    }
    let Some(bin) = bin else {
        return Err(Error::Failed(format!(
            "`{}` is not on your PATH: {}",
            agent.name(),
            agent.install_hint()
        )));
    };
    if headless_wanted(&args.headless) {
        return headless(&bin, agent, via, &question, &args.headless, out).await;
    }
    Out::note(&ask::label(agent));
    let started = Instant::now();
    let ui = Ui::detect(out.json);
    let look = Look {
        fancy: ui.fancy(),
        thinking: Some(thinking),
        cwd: std::env::current_dir()
            .ok()
            .map(|d| d.display().to_string()),
        model,
        ..Look::default()
    };
    let Ended { code, failure, .. } = stream(Run {
        agent,
        bin: &bin,
        args: &command_line,
        cwd: None,
        json: out.json,
        look,
        ui,
        waiting: "thinking",
        timeout: None,
    })
    .await?;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if out.json {
        // One more line in the stream, so a program reading the lines knows how it ended.
        let line = serde_json::json!({
            "type": "iohr_summary",
            "via": via.name(),
            "agent": agent.name(),
            "exit_code": code,
            "duration_ms": duration_ms,
        });
        Out::raw(format!("{line}\n").as_bytes());
    }
    if let Some(f) = failure {
        Out::note(&format!("iohr ask: the agent reports: {f}"));
    }
    match u8::try_from(code) {
        Ok(0) => Ok(()),
        Ok(c) => Err(Error::Child(c)),
        Err(_) => Err(Error::Child(1)),
    }
}

/// One run of the agent in print mode: what to start, where, and how to draw it.
pub(crate) struct Run<'a> {
    pub(crate) agent: Agent,
    pub(crate) bin: &'a std::path::Path,
    /// The arguments from [`ask::argv`], the prompt last.
    pub(crate) args: &'a [String],
    /// The folder to run in; the current one when unset.
    pub(crate) cwd: Option<&'a std::path::Path>,
    /// Pass each line of the agent's stream through untouched instead of drawing it.
    pub(crate) json: bool,
    pub(crate) look: Look,
    pub(crate) ui: Ui,
    /// What the spinner says until the agent's first event.
    pub(crate) waiting: &'a str,
    /// Stop the agent after this long; `None` waits for it.
    pub(crate) timeout: Option<Duration>,
}

/// How a run ended.
pub(crate) struct Ended {
    /// The agent's exit code (1 when it exited 0 but said the run failed).
    pub(crate) code: i32,
    /// The reason the agent gave when it said the run failed.
    pub(crate) failure: Option<String>,
    /// iohr stopped the agent because `timeout` passed.
    pub(crate) timed_out: bool,
}

/// Runs the agent once with its JSON stream piped to iohr and draws it as it arrives (or, with
/// `json`, passes each line through untouched). No terminal is handed to the agent and its
/// stdin is closed, so it cannot open a session or prompt. The one launch code of `iohr ask`
/// and `iohr project init`. Ctrl-C reaches the agent too (same process group); iohr waits
/// for it to end. When `timeout` passes, the agent is killed and the run is `timed_out`.
pub(crate) async fn stream(run: Run<'_>) -> Result<Ended, Error> {
    let Run {
        agent,
        bin,
        args,
        cwd,
        json,
        look,
        ui,
        waiting,
        timeout,
    } = run;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Failed(format!("cannot start `{}`: {}", agent.name(), e.kind())))?;
    let Some(stdout) = child.stdout.take() else {
        return Err(Error::Failed(format!(
            "cannot read the output of `{}`",
            agent.name()
        )));
    };
    let spinner = (!json).then(|| ui.step(waiting));
    let mut render = Renderer::new(std::io::stdout(), std::io::stderr(), look, spinner);
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    // No limit is a year away, far inside what the timer takes.
    let deadline = tokio::time::sleep(timeout.unwrap_or(Duration::from_hours(8760)));
    tokio::pin!(deadline);
    let mut timed_out = false;
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(l)) if json => Out::raw(format!("{l}\n").as_bytes()),
                Ok(Some(l)) => {
                    for e in stream::parse(agent, &l) {
                        render.draw(e);
                    }
                }
                _ => break,
            },
            () = &mut deadline, if timeout.is_some() => {
                timed_out = true;
                let _ = child.start_kill();
                break;
            }
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    render.finish();
    let status = loop {
        tokio::select! {
            s = child.wait() => break s,
            () = &mut deadline, if timeout.is_some() && !timed_out => {
                timed_out = true;
                let _ = child.start_kill();
            }
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
    let failure = render.failure().map(str::to_owned);
    let mut code = exit_code(status);
    if code == 0 && failure.is_some() {
        code = 1;
    }
    Ok(Ended {
        code,
        failure,
        timed_out,
    })
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

/// Answer once and print as a card or plain text (the `?` prompt hook, a script, the app),
/// instead of streaming the run? Only when asked for: `--headless`, `--card` or a context
/// flag. `--interactive` says no even then.
fn headless_wanted(h: &HeadlessArgs) -> bool {
    !h.interactive && (h.headless || h.card || h.has_context())
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
        let look = CardLook::detect(fancy);
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
    look: CardLook,
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
