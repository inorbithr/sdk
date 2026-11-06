//! `iohr ask`: the developer's own agent answers once in this folder, streamed to the terminal.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncBufReadExt as _;

use crate::actions::{self, MenuKey};
use crate::ask::stream::{self, Look, Renderer};
use crate::ask::{self, Agent, Via};
use crate::askback;
use crate::card::{self, Body, Card, Look as CardLook};
use crate::cli::{AskArgs, Global, HeadlessArgs};
use crate::commands::asks;
use crate::context::Ctx;
use crate::error::Error;
use crate::headless::{self, Answer, Context};
use crate::hints;
use crate::output::Out;
use crate::ui::Ui;
use crate::{Env, prompt};

pub(crate) async fn run(g: &Global, env: &Env, args: AskArgs, out: Out) -> Result<(), Error> {
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
    let command_line = ask::argv(agent, model.as_deref(), allow_edits, &[], None, &question);
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
            (g, env),
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

/// What the shell hook told this call, as the context the prompt is built from, and the
/// project the folder is in (where an answer to a question is kept), when it is in one. The
/// git facts and the project's notes are iohr's own reads, never the model's.
fn context_of(h: &HeadlessArgs) -> Result<(Context, Option<std::path::PathBuf>), Error> {
    let cwd = match &h.cwd {
        Some(c) => c.clone(),
        None => std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let root = crate::memory::find_root(Path::new(&cwd));
    let ctx = Context {
        files: headless::list_folder(Path::new(&cwd)),
        reads: headless::can_read(Path::new(&cwd), home.as_deref()),
        git: askback::git_facts(Path::new(&cwd)),
        project: root
            .as_deref()
            .map(askback::project_notes)
            .unwrap_or_default(),
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
        refresh: false,
    };
    Ok((ctx, root))
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
    (g, env): (&Global, &Env),
) -> Result<(), Error> {
    let Chosen {
        bin,
        agent,
        via,
        model,
        ..
    } = *chosen;
    let (ctx, root) = context_of(h)?;
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
        let (text, questions) = askback::split(&text);
        if out.json {
            Out::print_json(&Answer {
                command: card::command_of(&text),
                text,
                questions,
                agent: agent.name(),
                via: via.name(),
                label,
                model: model.to_owned(),
                exit_code: 0,
                duration_ms,
            });
        } else {
            Out::raw(format!("{text}\n").as_bytes());
            // No terminal to ask on: the questions are plain lines, apart from the answer.
            for line in askback::plain_lines(&questions) {
                Out::note(&line);
            }
        }
        return Ok(());
    }
    // A person reads a card.
    card_run(
        chosen,
        question,
        h,
        (g, env),
        Prepared {
            ctx,
            root,
            message,
            read,
            label,
            started,
            fancy,
            stdout_tty,
        },
    )
    .await
}

/// What `headless` settled before it knew whether a card or a plain answer was wanted.
struct Prepared {
    ctx: Context,
    root: Option<std::path::PathBuf>,
    message: String,
    read: String,
    label: String,
    started: Instant,
    fancy: bool,
    stdout_tty: bool,
}

/// The card: the model runs under a spinner while the asks waiting are counted, the answer is
/// drawn, a question the model asked back is put to the person (kept, then the answer is
/// refreshed once), and the one line about waiting asks closes it.
async fn card_run(
    chosen: &Chosen<'_>,
    question: &str,
    h: &HeadlessArgs,
    (g, env): (&Global, &Env),
    prepared: Prepared,
) -> Result<(), Error> {
    let Chosen {
        bin,
        agent,
        model,
        hints,
        menu_key,
        dir,
        ..
    } = *chosen;
    let Prepared {
        mut ctx,
        root,
        message,
        read,
        label,
        started,
        fancy,
        stdout_tty,
    } = prepared;
    let look = CardLook::detect(fancy);
    let folder = ctx.reads.then(|| std::path::PathBuf::from(&ctx.cwd));
    let run = headless::run(bin, &message, model, started, folder.as_deref());
    // The asks waiting are looked up while the model thinks, so they add no time.
    let (done, waiting) = if look.fancy {
        let (done, waiting) =
            tokio::join!(spin_while(run, agent, look, started), asks::waiting(g, env));
        (done?, waiting)
    } else {
        (run.await, None)
    };
    let notice = format!("An AI model {} {}", ask::PROVIDER, ask::CHECK);
    // Only a card that is drawn moves the footer on.
    // A plain look has no footer (its label carries the disclosure), so it moves nothing.
    let footer = if look.fancy {
        hints::next(dir, hints, &notice, hints::now())
    } else {
        hints::Footer::Full
    };
    let shown = Card {
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
    let cwd = Path::new(&ctx.cwd).to_path_buf();
    match &done {
        Ok((raw, duration_ms)) => {
            let (mut text, questions) = askback::split(raw);
            let took = Duration::from_millis(*duration_ms);
            draw_answer(&shown, &text, took, (h, look, menu_key), &cwd, width);
            // The model asked back: ask the person, keep the answer, and answer once more.
            let can_ask = look.fancy
                && prompt::interactive()
                && (h.hook || h.card || !h.headless)
                && !questions.is_empty();
            if let Some(root) = root.as_deref().filter(|_| can_ask)
                && let Some((again, ms)) =
                    ask_back(chosen, question, &mut ctx, (root, &questions), look).await?
            {
                text = again;
                let took = Duration::from_millis(ms);
                draw_answer(&shown, &text, took, (h, look, menu_key), &cwd, width);
            }
            if let Some(w) = &waiting {
                Out::note(&waiting_line(w, h.hook, menu_key, look, width));
            }
            if h.hook {
                let command = card::command_of(&text);
                let link = card::link_of(&text, &cwd);
                Out::raw(actions::record(&text, command.as_deref(), link.as_deref()).as_bytes());
            } else if !stdout_tty {
                Out::raw(format!("{text}\n").as_bytes());
            }
            Ok(())
        }
        Err(why) => {
            let failed = Card {
                elapsed: Some(started.elapsed()),
                body: Body::Failed(why),
                ..shown
            };
            card::draw(&card::render(&failed, width, look));
            Err(Error::Child(1))
        }
    }
}

/// Puts the model's questions to the person, keeps what they answer (iohr writes it, the
/// model writes nothing) and asks the model once more with it in hand. `Some((answer, ms))`
/// is the refreshed answer; `None` when they skipped or the refresh failed (the first answer
/// stands, and a note says why).
async fn ask_back(
    chosen: &Chosen<'_>,
    question: &str,
    ctx: &mut Context,
    (root, questions): (&Path, &[askback::Question]),
    look: CardLook,
) -> Result<Option<(String, u64)>, Error> {
    let Some(answers) = askback::ask_person(questions) else {
        return Ok(None);
    };
    for line in askback::save(root, &answers) {
        Out::note(&line);
    }
    ctx.project = askback::project_notes(root);
    ctx.refresh = true;
    let again = headless::assemble(question, ctx).map_err(Error::Usage)?;
    let started = Instant::now();
    let folder = ctx.reads.then(|| std::path::PathBuf::from(&ctx.cwd));
    let run = headless::run(chosen.bin, &again, chosen.model, started, folder.as_deref());
    match spin_while(run, chosen.agent, look, started).await? {
        // The second answer asks nothing: a question in it is dropped.
        Ok((raw, ms)) => Ok(Some((askback::split(&raw).0, ms))),
        Err(why) => {
            Out::note(&format!("could not refresh the answer: {why}"));
            Ok(None)
        }
    }
}

/// Draws one answer card: the row under it only when the hook is there to bind its key.
fn draw_answer(
    shown: &Card<'_>,
    text: &str,
    took: Duration,
    (h, look, menu_key): (&HeadlessArgs, CardLook, MenuKey),
    cwd: &Path,
    width: usize,
) {
    let command = card::command_of(text);
    let link = card::link_of(text, cwd);
    // The row promises a key that only the hook binds, so only the hook draws it.
    let row = (h.hook && look.fancy)
        .then(|| actions::row(menu_key, command.is_some(), link.is_some()))
        .flatten();
    let card = Card {
        elapsed: Some(took),
        row: row.as_deref(),
        body: Body::Answer(text),
        ..shown.clone()
    };
    card::draw(&card::render(&card, width, look));
}

/// The fewest columns a title is worth showing in: under this the line keeps the count.
const TITLE_MIN: usize = 12;

/// The one quiet line under a card when asks wait for the person: how to open them from
/// here. The menu's own `a` is the key; without the hook (or its key) the command is named.
/// One ask is named by its title, cut to the card's `width`; several keep the count. Asks
/// the person already answered (their turn is over) are not in `waiting`.
fn waiting_line(
    waiting: &asks::Waiting,
    hook: bool,
    key: MenuKey,
    look: CardLook,
    width: usize,
) -> String {
    let n = waiting.count;
    let noun = if n == 1 { "ask" } else { "asks" };
    let how = match key.label().filter(|_| hook) {
        Some(k) => format!("{k} then a to open"),
        None => "`iohr asks list` opens them".to_owned(),
    };
    let head = format!("{n} {noun} waiting");
    let tail = format!(" \u{b7} {how}");
    let named = waiting.only.as_deref().and_then(|title| {
        // ": " between the count and the title.
        let room = width.checked_sub(console::measure_text_width(&head) + 2)?;
        let room = room.checked_sub(console::measure_text_width(&tail))?;
        (room >= TITLE_MIN).then(|| console::truncate_str(title, room, "...").into_owned())
    });
    let line = match named {
        Some(title) => format!("{head}: {title}{tail}"),
        None => format!("{head}{tail}"),
    };
    if look.fancy {
        console::style(line).dim().to_string()
    } else {
        line
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

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: CardLook = CardLook {
        fancy: false,
        truecolor: false,
    };

    fn one(title: &str) -> asks::Waiting {
        asks::Waiting {
            count: 1,
            only: Some(title.to_owned()),
        }
    }

    #[test]
    fn one_waiting_ask_is_named() {
        let line = waiting_line(&one("Merge #12"), true, MenuKey::Down, PLAIN, 80);
        assert_eq!(
            line,
            "1 ask waiting: Merge #12 \u{b7} \u{2193} then a to open"
        );
    }

    #[test]
    fn a_long_title_is_cut_to_the_card_width() {
        let title = "Roll the accounts service to production after the migration has run";
        for width in [50, 60, 80] {
            let line = waiting_line(&one(title), true, MenuKey::Down, PLAIN, width);
            assert!(
                console::measure_text_width(&line) <= width,
                "{width}: {line}"
            );
            assert!(line.contains("..."), "{line}");
            assert!(line.ends_with("then a to open"), "{line}");
        }
    }

    #[test]
    fn a_narrow_card_keeps_the_count_alone() {
        let line = waiting_line(&one("Merge #12"), true, MenuKey::Down, PLAIN, 30);
        assert_eq!(line, "1 ask waiting \u{b7} \u{2193} then a to open");
    }

    #[test]
    fn several_keep_the_count_and_name_none() {
        let w = asks::Waiting {
            count: 3,
            only: None,
        };
        let line = waiting_line(&w, false, MenuKey::Down, PLAIN, 80);
        assert_eq!(line, "3 asks waiting \u{b7} `iohr asks list` opens them");
    }

    #[test]
    fn plain_look_carries_no_escape_codes() {
        let line = waiting_line(&one("Merge #12"), true, MenuKey::CtrlG, PLAIN, 80);
        assert!(!line.contains('\u{1b}'), "{line:?}");
    }
}
