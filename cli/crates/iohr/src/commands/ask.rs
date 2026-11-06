//! `iohr ask`: the developer's own agent answers once in this folder, streamed to the terminal.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncBufReadExt as _;

use crate::actions::{self, MenuKey};
use crate::ask::stream::{self, Look, Renderer};
use crate::ask::{self, Agent, Via};
use crate::card::{self, Body, Card, Look as CardLook};
use crate::cli::{AskArgs, Global, HeadlessArgs};
use crate::context::Ctx;
use crate::error::Error;
use crate::headless::{self, Answer, Context};
use crate::hints;
use crate::output::Out;
use crate::ui::Ui;

pub(crate) async fn run(g: &Global, args: AskArgs, out: Out) -> Result<(), Error> {
    if args.menu {
        return menu(&args);
    }
    let ctx = Ctx::load(g)?;
    let cfg = ctx.config.ask;
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
    let hints = hints::Mode::parse(cfg.hints.as_deref()).map_err(Error::Usage)?;
    let menu_key = MenuKey::parse(cfg.menu_key.as_deref()).map_err(Error::Usage)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let bin = ask::find_on_path(agent.name(), &path);
    // The question is only ever an argument of the agent; nothing below prints or stores it
    // except `--dry-run`, which prints the command line you asked to see.
    let question = args.question.join(" ");
    let command_line = ask::argv(agent, model.as_deref(), allow_edits, None, &question);
    if args.dry_run {
        dry_run(via, agent, bin.as_deref(), &command_line, out);
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
        return headless(
            &Chosen {
                bin: &bin,
                agent,
                via,
                // One reading of the setting for the card and the streamed run alike.
                model: model.as_deref().unwrap_or(ask::DEFAULT_MODEL),
                hints,
                menu_key,
                dir: &ctx.dir,
            },
            &question,
            &args.headless,
            out,
        )
        .await;
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
    ended(code)
}

/// The agent's exit code as this command's result.
fn ended(code: i32) -> Result<(), Error> {
    match u8::try_from(code) {
        Ok(0) => Ok(()),
        Ok(c) => Err(Error::Child(c)),
        Err(_) => Err(Error::Child(1)),
    }
}

/// `--menu`: the menu under a card asks no model and needs no sign-in or agent.
fn menu(args: &AskArgs) -> Result<(), Error> {
    let cwd = args.headless.cwd.as_deref().map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        std::path::PathBuf::from,
    );
    actions::menu(args.command.as_deref(), args.link.as_deref(), &cwd)
}

/// `--dry-run`: the exact command line, one object under `--json`; nothing is started.
fn dry_run(via: Via, agent: Agent, bin: Option<&Path>, command_line: &[String], out: Out) {
    let program = bin.map_or_else(|| agent.name().to_owned(), |b| b.display().to_string());
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
        Out::raw(format!("{}\n", ask::shell_line(&program, command_line)).as_bytes());
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
/// instead of streaming the run? Only when asked for: `--headless`, `--card`, `--hook` or a
/// context flag. `--interactive` says no even then.
fn headless_wanted(h: &HeadlessArgs) -> bool {
    !h.interactive && (h.headless || h.card || h.hook || h.has_context())
}

/// What the shell hook told this call, as the context the prompt is built from.
fn context_of(h: &HeadlessArgs) -> Result<Context, Error> {
    let cwd = match &h.cwd {
        Some(c) => c.clone(),
        None => std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    Ok(Context {
        files: headless::list_folder(Path::new(&cwd)),
        reads: headless::can_read(Path::new(&cwd), home.as_deref()),
        cwd,
        command: h.last_command.clone().unwrap_or_default(),
        exit_code: h.exit_code,
        output: match &h.output_file {
            // Only the kind of failure: the path is the person's, what is in it is not said.
            Some(f) => std::fs::read_to_string(f)
                .map_err(|e| Error::Usage(format!("cannot read --output-file: {}", e.kind())))?,
            None => String::new(),
        },
        prior: h.prior_question.clone().zip(h.prior_answer.clone()),
    })
}

/// What the settings and flags settled for one call.
struct Chosen<'a> {
    bin: &'a Path,
    agent: Agent,
    via: Via,
    model: &'a str,
    hints: hints::Mode,
    menu_key: MenuKey,
    dir: &'a Path,
}

/// One capped `claude -p` run (see [`headless`]), shown one way (RFC 0203.1): `--json` is the
/// answer's fields; `--hook` is a card on stderr and a record on stdout; a person at a
/// terminal gets a card on stderr; anything else gets the label on stderr and the plain
/// answer once on stdout, so a pipe of the answer stays clean.
async fn headless(
    chosen: &Chosen<'_>,
    question: &str,
    h: &HeadlessArgs,
    out: Out,
) -> Result<(), Error> {
    let Chosen {
        bin,
        agent,
        via,
        model,
        hints,
        menu_key,
        dir,
    } = *chosen;
    let ctx = context_of(h)?;
    let message = headless::assemble(question, &ctx).map_err(Error::Usage)?;
    let read = headless::read_line(&ctx);
    let folder = ctx.reads.then(|| Path::new(&ctx.cwd));
    let fancy = Ui::detect(out.json).fancy();
    let stdout_tty = std::io::stdout().is_terminal();
    let label = ask::label(agent);
    let started = Instant::now();
    if out.json || !(h.hook || (fancy && (h.card || stdout_tty))) {
        let label = format!("{label} {read}");
        if !out.json {
            Out::note(&label);
        }
        let (text, duration_ms) = headless::run(bin, &message, model, started, folder)
            .await
            .map_err(Error::Failed)?;
        if out.json {
            Out::print_json(&Answer {
                command: card::command_of(&text),
                text,
                agent: agent.name(),
                via: via.name(),
                label,
                model: model.to_owned(),
                exit_code: 0,
                duration_ms,
            });
        } else {
            Out::raw(format!("{text}\n").as_bytes());
        }
        return Ok(());
    }
    // A person reads a card.
    let look = CardLook::detect(fancy);
    let run = headless::run(bin, &message, model, started, folder);
    let done = if look.fancy {
        spin_while(run, agent, look, started).await?
    } else {
        run.await
    };
    let notice = format!("An AI model {} {}", ask::PROVIDER, ask::CHECK);
    // Only a card that is drawn moves the footer on.
    // A plain look has no footer (its label carries the disclosure), so it moves nothing.
    let footer = if look.fancy {
        hints::next(dir, hints, &notice, hints::now())
    } else {
        hints::Footer::Full
    };
    let mut shown = Card {
        agent: agent.display(),
        model,
        elapsed: None,
        follows: h.prior_question.as_deref(),
        read: &read,
        notice: &notice,
        full: footer == hints::Footer::Full,
        label: &label,
        hint: match footer {
            hints::Footer::Short(h) => h,
            hints::Footer::Full => None,
        },
        row: None,
        body: Body::Failed(""),
    };
    let width = card::stderr_width();
    match &done {
        Ok((text, duration_ms)) => {
            let command = card::command_of(text);
            let link = card::link_of(text, Path::new(&ctx.cwd));
            // The row promises a key that only the hook binds, so only the hook draws it.
            let row = (h.hook && look.fancy)
                .then(|| actions::row(menu_key, command.is_some(), link.is_some()))
                .flatten();
            shown.elapsed = Some(Duration::from_millis(*duration_ms));
            shown.row = row.as_deref();
            shown.body = Body::Answer(text);
            card::draw(&card::render(&shown, width, look));
            if h.hook {
                Out::raw(actions::record(text, command.as_deref(), link.as_deref()).as_bytes());
            } else if !stdout_tty {
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
    }
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
