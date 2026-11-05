//! `iohr ask`: run the developer's own agent in this folder with the question.

use std::process::Stdio;
use std::time::{Duration, Instant};

use std::io::IsTerminal as _;
use std::path::Path;

use crate::actions::{self, MenuKey};
use crate::ask::{self, Agent, Via};
use crate::card::{self, Body, Card, Look};
use crate::cli::{AskArgs, Global, HeadlessArgs};
use crate::context::Ctx;
use crate::error::Error;
use crate::headless::{self, Answer, Context};
use crate::hints;
use crate::output::Out;
use crate::ui::Ui;

pub(crate) async fn run(g: &Global, args: AskArgs, out: Out) -> Result<(), Error> {
    if args.menu {
        // The menu under a card asks no model and needs no sign-in or agent.
        let cwd = args.headless.cwd.as_deref().map_or_else(
            || std::env::current_dir().unwrap_or_default(),
            std::path::PathBuf::from,
        );
        return actions::menu(args.command.as_deref(), args.link.as_deref(), &cwd);
    }
    let ctx = Ctx::load(g)?;
    let cfg = ctx.config.ask;
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
    // One reading of the model for the card and the run alike; Codex returned above.
    let model = ask::choose_model(agent, args.model.as_deref(), cfg.model.as_deref())
        .map_err(Error::Usage)?
        .unwrap_or_else(|| ask::DEFAULT_MODEL.to_owned());
    let hints = hints::Mode::parse(cfg.hints.as_deref()).map_err(Error::Usage)?;
    let menu_key = MenuKey::parse(cfg.menu_key.as_deref()).map_err(Error::Usage)?;
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
        return headless(
            &Chosen {
                bin: &bin,
                agent,
                via,
                model: &model,
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
        || h.hook
        || h.has_context()
        || json
        || !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()))
}

/// What the shell hook told this call, as the context the prompt is built from.
fn context_of(h: &HeadlessArgs) -> Result<Context, Error> {
    Ok(Context {
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
    let fancy = Ui::detect(out.json).fancy();
    let stdout_tty = std::io::stdout().is_terminal();
    let label = ask::label(agent);
    let started = Instant::now();
    if out.json || !(h.hook || (fancy && (h.card || stdout_tty))) {
        let label = format!("{label} {read}");
        if !out.json {
            Out::note(&label);
        }
        let (text, duration_ms) = headless::run(bin, &message, model, started)
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
    let look = Look::detect(fancy);
    let run = headless::run(bin, &message, model, started);
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
