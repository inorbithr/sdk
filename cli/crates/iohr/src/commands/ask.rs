//! `iohr ask`: the developer's own agent answers once in this folder, streamed to the terminal.

use std::process::Stdio;
use std::time::Instant;

use tokio::io::AsyncBufReadExt as _;

use crate::ask::stream::{self, Look, Renderer};
use crate::ask::{self, Agent, Via};
use crate::cli::{AskArgs, Global};
use crate::context::Ctx;
use crate::error::Error;
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
    let path = std::env::var_os("PATH").unwrap_or_default();
    let bin = ask::find_on_path(agent.name(), &path);
    // The question is only ever an argument of the agent; nothing below prints or stores it
    // except `--dry-run`, which prints the command line you asked to see.
    let question = args.question.join(" ");
    let command_line = ask::argv(agent, model.as_deref(), allow_edits, &question);
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
    };
    let (code, failure) = stream(agent, &bin, &command_line, out.json, look, ui).await?;
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

/// Runs the agent once with its JSON stream piped to iohr and draws it as it arrives (or, with
/// `json`, passes each line through untouched). No terminal is handed to the agent and its
/// stdin is closed, so it cannot open a session or prompt. Its exit code, and the reason it
/// gave when it said the run failed. Ctrl-C reaches the agent too (same process group); iohr
/// waits for it to end.
async fn stream(
    agent: Agent,
    bin: &std::path::Path,
    command_line: &[String],
    json: bool,
    look: Look,
    ui: Ui,
) -> Result<(i32, Option<String>), Error> {
    let mut child = tokio::process::Command::new(bin)
        .args(command_line)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| Error::Failed(format!("cannot start `{}`: {}", agent.name(), e.kind())))?;
    let Some(stdout) = child.stdout.take() else {
        return Err(Error::Failed(format!(
            "cannot read the output of `{}`",
            agent.name()
        )));
    };
    let spinner = (!json).then(|| ui.step("thinking"));
    let mut render = Renderer::new(std::io::stdout(), std::io::stderr(), look, spinner);
    let mut lines = tokio::io::BufReader::new(stdout).lines();
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
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    render.finish();
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
    let failure = render.failure().map(str::to_owned);
    let mut code = exit_code(status);
    if code == 0 && failure.is_some() {
        code = 1;
    }
    Ok((code, failure))
}

/// Starts the agent with `prompt` as its argument (in `cwd` when given), stdio inherited, and
/// waits for it; its exit code. The one launch code of `iohr ask` and `iohr project init`.
/// Ctrl-C reaches the agent too (same process group); iohr waits for it instead of dying
/// first and leaving the terminal to a half-stopped agent.
pub(crate) async fn start(
    agent: Agent,
    bin: &std::path::Path,
    prompt: String,
    cwd: Option<&std::path::Path>,
) -> Result<i32, Error> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg(prompt)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(false);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Failed(format!("cannot start `{}`: {}", agent.name(), e.kind())))?;
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
    Ok(exit_code(status))
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
