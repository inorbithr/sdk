//! `iohr ask`: the developer's own agent answers once in this folder, streamed to the terminal.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncBufReadExt as _;

use crate::access;
use crate::actions::{self, MenuKey};
use crate::ask::stream::{self, Look, Renderer};
use crate::ask::{self, Agent, Via};
use crate::askback;
use crate::attach;
use crate::card::{self, Body, Card, Look as CardLook, Status};
use crate::cli::{AskArgs, Global, HeadlessArgs};
use crate::commands::ask_roles;
use crate::commands::asks;
use crate::context::Ctx;
use crate::error::Error;
use crate::headless::{self, Answer, Context, OutputHow};
use crate::hints;
use crate::output::Out;
#[cfg(unix)]
use crate::permissions::broker::Opts;
use crate::permissions::broker::{self, Session};
#[cfg(unix)]
use crate::permissions::engine::Place;
use crate::permissions::store::{self, Files};
use crate::permissions::{Grant, Mode, Settled, Words, trust};
use crate::roles::runtime;
use crate::steps::{self, Step};
use crate::ui::Ui;
use crate::{Env, prompt};

/// How an editing run ended, for the closing lines.
struct Done<'a> {
    /// The Role the run worked in (`--role`), as `--json` names it.
    role: Option<serde_json::Value>,
    via: Via,
    agent: Agent,
    code: i32,
    failure: Option<String>,
    started: Instant,
    note_root: Option<&'a std::path::Path>,
}

/// The closing lines of a streamed run: the JSON summary, the agent's reason when it failed, and
/// the handoff note it went on from, removed when it ended well.
fn finish(d: &Done<'_>, out: Out) {
    let (code, via, agent) = (d.code, d.via, d.agent);
    let duration_ms = u64::try_from(d.started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if out.json {
        // One more line in the stream, so a program reading the lines knows how it ended.
        let mut line = serde_json::json!({
            "type": "iohr_summary",
            "via": via.name(),
            "agent": agent.name(),
            "exit_code": code,
            "duration_ms": duration_ms,
        });
        if let (Some(role), Some(o)) = (&d.role, line.as_object_mut()) {
            o.insert("role".to_owned(), role.clone());
        }
        Out::raw(format!("{line}\n").as_bytes());
    }
    if let Some(f) = &d.failure {
        Out::note(&format!("iohr ask: the agent reports: {f}"));
    }
    if let (0, Some(root)) = (code, d.note_root) {
        // The run went on from the note and ended well: the note is done with.
        crate::handoff::clear(root);
        Out::note(&format!(
            "iohr ask: went on from {} and removed it",
            crate::handoff::FILE
        ));
    }
}

/// What is not built yet, said before anything starts.
fn built(via: Via, agent: Agent, args: &AskArgs) -> Result<(), Error> {
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
    Ok(())
}

/// The model of the run: the flag, then `[ask] model`, then the Role's, then the agent's own.
fn model_of(
    agent: Agent,
    args: &AskArgs,
    configured: Option<&str>,
    role: Option<&runtime::Applied>,
) -> Result<Option<String>, Error> {
    ask::choose_model(
        agent,
        ask::ModelChoice {
            flag: args.model.as_deref(),
            configured,
            role: role.and_then(|r| r.model.as_deref()),
            default: None,
        },
    )
    .map_err(Error::Usage)
}

/// The Role the run works in (`--role`, RFC 0211): the project's pack, the account's, or the
/// built-in one. Its model comes after the flag and `[ask] model`; its text joins the prompt.
async fn role_of(g: &Global, env: &Env, args: &AskArgs) -> Result<Option<runtime::Applied>, Error> {
    let Some(name) = args.role.as_deref() else {
        return Ok(None);
    };
    let root = crate::memory::find_root(&role_cwd(&args.headless));
    runtime::resolve(
        g,
        env,
        runtime::Lookup {
            name,
            job: args.job.as_deref(),
            root: root.as_deref(),
            online: !args.dry_run,
        },
    )
    .await
    .map(Some)
}

/// `iohr ask`: Io answers, and where a session host runs, the Roles answer beside it
/// ([`super::ask_roles`], RFC 0213). The Roles take the ask first, so they work while Io does;
/// Io's card is the one it always was, and each Role's card follows as its answer lands. When
/// the Roles answer, Io's own run is not in a Role (`--role` chose which of them answer).
pub(crate) async fn run(g: &Global, env: &Env, mut args: AskArgs, out: Out) -> Result<(), Error> {
    let plan = ask_roles::plan(g, &args, out.json);
    let routed = match plan {
        Some(p) => ask_roles::start(p).await,
        None => None,
    };
    if routed.is_some() {
        args.role = None;
        args.job = None;
    }
    let result = run_io(g, env, args, out).await;
    if let (Some(routed), Ok(())) = (routed, &result) {
        routed.show().await;
    }
    result
}

async fn run_io(g: &Global, env: &Env, args: AskArgs, out: Out) -> Result<(), Error> {
    if args.menu {
        return menu(g, &args);
    }
    let ctx = Ctx::load(g)?;
    let cfg = ctx.config.ask;
    let via = ask::choose_via(args.via.as_deref(), cfg.via.as_deref()).map_err(Error::Usage)?;
    let agent =
        ask::choose_agent(args.agent.as_deref(), cfg.agent.as_deref()).map_err(Error::Usage)?;
    let role = role_of(g, env, &args).await?;
    let model = model_of(agent, &args, cfg.model.as_deref(), role.as_ref())?;
    let thinking = ask::choose_thinking(args.thinking.as_deref(), cfg.thinking.as_deref())
        .map_err(Error::Usage)?;
    let named = words(&args, &cfg);
    let (allow_edits, plan) = streamed(named, &args, &cfg, prompt::interactive())?;
    built(via, agent, &args)?;
    let hints = hints::Mode::parse(cfg.hints.as_deref()).map_err(Error::Usage)?;
    let menu_key = MenuKey::parse(cfg.menu_key.as_deref()).map_err(Error::Usage)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let bin = ask::find_on_path(agent.name(), &path);
    // The question is only ever an argument of the agent; nothing below prints or stores it
    // except `--dry-run`, which prints the command line you asked to see.
    let question = args.question.join(" ");
    // An editing run in a project that has a handoff note (an earlier run stopped at its cap)
    // goes on from it: the note leads the prompt.
    let (note_root, question) =
        crate::handoff::continue_from(allow_edits && !args.dry_run, question);
    let role_text = role.as_ref().map(runtime::Applied::system_text);
    let role_json = role.as_ref().map(runtime::Applied::json);
    let command_line = ask::argv(
        agent,
        model.as_deref(),
        (allow_edits, plan),
        &[],
        None,
        role_text.as_deref(),
        &question,
    );
    if args.dry_run {
        dry_run(via, agent, bin.as_deref(), &command_line, role_json, out);
        return Ok(());
    }
    let Some(bin) = bin else {
        return Err(Error::Failed(format!(
            "`{}` is not on your PATH: {}",
            agent.name(),
            agent.install_hint()
        )));
    };
    let folders = folders_from_everywhere(&args.headless.read, &cfg);
    if headless_wanted(&args.headless) || folders_need_tools(&args.headless, &folders) {
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
                modes: named,
                role: role.as_ref(),
                folders,
            },
            &question,
            &args.headless,
            out,
            (g, env),
        )
        .await;
    }
    if !folders.is_empty() {
        Out::note(FOLDERS_IGNORED);
    }
    Out::note(&ask::label(agent));
    let started = Instant::now();
    let ui = Ui::detect(out.json);
    let look = look_of(ui, thinking, model);
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
    finish(
        &Done {
            role: role_json,
            via,
            agent,
            code,
            failure,
            started,
            note_root: note_root.as_deref(),
        },
        out,
    );
    ended(code)
}

/// How the plain run draws the agent's stream: in this folder, with this model.
fn look_of(ui: Ui, thinking: stream::Thinking, model: Option<String>) -> Look {
    Look {
        fancy: ui.fancy(),
        thinking: Some(thinking),
        cwd: std::env::current_dir()
            .ok()
            .map(|d| d.display().to_string()),
        model,
        ..Look::default()
    }
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
fn menu(g: &Global, args: &AskArgs) -> Result<(), Error> {
    let cwd = args.headless.cwd.as_deref().map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        std::path::PathBuf::from,
    );
    // Undo takes back only what the project file still holds, so the row never promises more.
    let files = Files::find(&cwd, &Ctx::load(g)?.dir);
    let undo: Vec<String> = args
        .undo
        .iter()
        .filter(|r| !r.is_empty())
        .cloned()
        .collect();
    let undo = actions::Takeback {
        rules: store::saved_project_allows(&files, &undo),
        files: &files,
    };
    // The offer to attach output stands only for a failed command and a question that asks why.
    let failed = args
        .failed_command
        .as_deref()
        .zip(args.failed_exit)
        .and_then(|(command, code)| {
            attach::offer(
                args.asked.as_deref().unwrap_or_default(),
                command,
                Some(code),
                false,
            )
        });
    let attach = args
        .output_to
        .as_deref()
        .zip(failed.as_ref())
        .map(|(to, f)| actions::Attach {
            failed: f,
            asked: args.asked.as_deref().unwrap_or_default(),
            to,
        });
    // Only a folder Io could be allowed to read, and only steps that decode, are offered.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let allow = args
        .access
        .as_deref()
        .filter(|f| access::allowable_at(f, &cwd, home.as_deref(), access::Source::Card).is_ok());
    let steps: Vec<Step> = args
        .step
        .iter()
        .filter_map(|s| Step::decode(s))
        .take(steps::OFFERED)
        .collect();
    let next = actions::Next {
        allow,
        steps: &steps,
    };
    let dir = Ctx::load(g)?.dir;
    actions::menu(
        (args.command.as_deref(), args.link.as_deref()),
        &cwd,
        &undo,
        attach.as_ref(),
        (&next, args.asked.as_deref().unwrap_or_default(), &dir),
    )
}

/// `--dry-run`: the exact command line, one object under `--json`; nothing is started.
fn dry_run(
    via: Via,
    agent: Agent,
    bin: Option<&Path>,
    command_line: &[String],
    role: Option<serde_json::Value>,
    out: Out,
) {
    let program = bin.map_or_else(|| agent.name().to_owned(), |b| b.display().to_string());
    if out.json {
        let mut v = serde_json::json!({
            "dry_run": true,
            "via": via.name(),
            "agent": agent.name(),
            "found": bin.is_some(),
            "program": program,
            "args": command_line,
        });
        if let (Some(role), Some(o)) = (role, v.as_object_mut()) {
            o.insert("role".to_owned(), role);
        }
        Out::print_json(&v);
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
    /// The agent stopped at its turn cap: its work is on disk, unfinished.
    pub(crate) turn_capped: bool,
    /// The agent's last words (empty when it said none), for the handoff note.
    pub(crate) last_said: String,
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
    // The live view draws the closing words on stderr, so it is for a person: with stdout
    // piped, the agent's words go there as they always did.
    let mut look = look;
    look.summary &= std::io::IsTerminal::is_terminal(&std::io::stdout());
    let mut render = Renderer::new(std::io::stdout(), std::io::stderr(), look, spinner);
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    // No limit is a year away, far inside what the timer takes.
    let deadline = tokio::time::sleep(timeout.unwrap_or(Duration::from_hours(8760)));
    tokio::pin!(deadline);
    let mut timed_out = false;
    let mut tail = stream::Tail::default();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(l)) if json => {
                    Out::raw(format!("{l}\n").as_bytes());
                    for e in stream::parse(agent, &l) {
                        tail.see(&e);
                    }
                }
                Ok(Some(l)) => {
                    for e in stream::parse(agent, &l) {
                        tail.see(&e);
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
        turn_capped: tail.capped,
        last_said: tail.said(),
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

/// `IOHR_ASK_READ` and `[ask] read` select the run that has file tools, as `--read` does: the
/// streamed run has no secret denies and no folder grants, so it is never given them.
fn folders_need_tools(h: &HeadlessArgs, folders: &[(access::Source, std::path::PathBuf)]) -> bool {
    !folders.is_empty() && !h.interactive
}

/// What `--interactive` says when folders were named: it streams without them.
const FOLDERS_IGNORED: &str = "--interactive streams the run without folder grants: --read, IOHR_ASK_READ and [ask] read are ignored";

/// Answer once and print as a card or plain text (the `?` prompt hook, a script, the app),
/// instead of streaming the run? Only when asked for: `--headless`, `--card`, `--hook` or a
/// context flag. `--interactive` says no even then.
fn headless_wanted(h: &HeadlessArgs) -> bool {
    !h.interactive && (h.headless || h.card || h.hook || h.has_context())
}

/// What the shell hook told this call, as the context the prompt is built from, and the
/// project the folder is in (where an answer to a question is kept), when it is in one. The
/// git facts and the project's notes are iohr's own reads, never the model's.
/// The folder a run starts in: `--cwd`, else the current one.
fn role_cwd(h: &HeadlessArgs) -> std::path::PathBuf {
    h.cwd.as_deref().map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        std::path::PathBuf::from,
    )
}

fn context_of(
    h: &HeadlessArgs,
    g: &Global,
) -> Result<(Context, Option<std::path::PathBuf>), Error> {
    let cwd = match &h.cwd {
        Some(c) => c.clone(),
        None => std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let root = crate::memory::find_root(Path::new(&cwd));
    // One walk up for the project root serves the notes and the IO.md chain.
    let io_md = crate::iomd::load_in(
        Path::new(&cwd),
        crate::iomd::user_dir(g).as_deref(),
        root.as_deref(),
    );
    let ctx =
        Context {
            files: headless::list_folder(Path::new(&cwd)),
            reads: headless::can_read(Path::new(&cwd), home.as_deref()),
            also: Vec::new(),
            granted: Vec::new(),
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
                // Secrets are scrubbed here, whatever put the output in the file.
                Some(f) => attach::scrub(&std::fs::read_to_string(f).map_err(|e| {
                    Error::Usage(format!("cannot read --output-file: {}", e.kind()))
                })?),
                None => String::new(),
            },
            output_how: match h.output_kind.as_deref() {
                Some("captured") => OutputHow::Captured,
                Some("pasted") => OutputHow::Pasted,
                _ => OutputHow::Attached,
            },
            prior: h.prior_question.clone().zip(h.prior_answer.clone()),
            refresh: false,
            role: String::new(),
            io_md,
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
    /// The flag layer (`--mode`, `--permissions`) and `[ask]`, as written; [`Mode::choose`]
    /// settles them in [`headless`].
    modes: (Words<'a>, Words<'a>),
    /// The Role the run works in (`--role`).
    role: Option<&'a runtime::Applied>,
    /// The folders asked for besides the working one and where each came from: [`asked_folders`].
    folders: Vec<(access::Source, std::path::PathBuf)>,
}

/// The most folders one ask may read besides the working one. A flag, variable or config
/// list over it is refused; trusted folders beyond it are left out.
const MAX_FOLDERS: usize = 8;

/// Where a folder asked for came from.
const FROM_FLAG: access::Source = access::Source::Flag;
const FROM_ENV: access::Source = access::Source::Env;
const FROM_CONFIG: access::Source = access::Source::Config;

/// How an error names where a folder was asked for.
fn asked_by(source: access::Source) -> &'static str {
    match source {
        access::Source::Flag => "--read",
        access::Source::Env => "IOHR_ASK_READ",
        access::Source::Config => "[ask] read",
        access::Source::Trust => "trusted.toml",
        access::Source::Card => "the card",
    }
}

/// The folders asked for beside the working one, with their source: the flags (`--read`,
/// `--add-dir`), then `IOHR_ASK_READ` (an OS path list: `:` apart on Unix, `;` on Windows),
/// then `[ask] read`. The sources add up; a leading `~/` in the last two is the home folder.
/// Nothing here is checked: [`grant_access`] runs every one through the same rules.
fn asked_folders(
    flags: &[std::path::PathBuf],
    env: Option<&std::ffi::OsStr>,
    config: &[String],
    home: Option<&Path>,
) -> Vec<(access::Source, std::path::PathBuf)> {
    let from_env = env
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| (FROM_ENV, access::expand_home(&p, home)));
    let from_config = config
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| (FROM_CONFIG, access::expand_home(Path::new(p.trim()), home)));
    flags
        .iter()
        .map(|p| (FROM_FLAG, p.clone()))
        .chain(from_env)
        .chain(from_config)
        .collect()
}

/// The folders asked for from the flags, `IOHR_ASK_READ` and `[ask] read` ([`asked_folders`]).
fn folders_from_everywhere(
    flags: &[std::path::PathBuf],
    cfg: &iohr_auth::AskConfig,
) -> Vec<(access::Source, std::path::PathBuf)> {
    asked_folders(
        flags,
        std::env::var_os("IOHR_ASK_READ").as_deref(),
        cfg.read.as_deref().unwrap_or_default(),
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .as_deref(),
    )
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
        bin, agent, model, ..
    } = *chosen;
    let (mut ctx, root) = context_of(h, g)?;
    if let Some(r) = chosen.role {
        ctx.role = r.system_text();
    }
    let terminal = !out.json && prompt::interactive();
    let mode = chosen_mode(chosen, terminal)?;
    let pending = grant_access(chosen, &mut ctx, (mode, terminal), question)?;
    audit_grants(chosen, &ctx, out);
    let message = headless::assemble(question, &ctx).map_err(Error::Usage)?;
    let fancy = Ui::detect(out.json).fancy();
    let stdout_tty = std::io::stdout().is_terminal();
    // The live ask (PRD 0197): only where the run may read the folder (or in the home folder
    // with the person asked), and only when the person did not turn it off. A person is asked
    // only at a terminal and not for `--json`.
    let session = permission_session(chosen, &ctx, mode)?;
    let read = headless::read_line(&ctx, mode, session.is_some());
    let label = ask::label(agent);
    let started = Instant::now();
    if out.json || !(h.hook || (fancy && (h.card || stdout_tty))) {
        let label = format!("{label} {read}");
        if !out.json {
            Out::note(&label);
        }
        let ran = run_at(
            (bin, model),
            &message,
            started,
            &ctx,
            (session.as_ref(), mode),
        )
        .await;
        if !out.json {
            for line in settled_lines(session.as_ref(), 0) {
                Out::note(&line);
            }
            for line in refused_lines(session.as_ref()) {
                Out::note(&line);
            }
        }
        let (text, duration_ms) = ran.map_err(Error::Failed)?;
        print_answer(
            out,
            chosen,
            (&ctx, &label, session.as_ref()),
            (text, duration_ms),
            (question, h, pending.as_ref()),
        );
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
            session,
            mode,
            pending,
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
    /// The live permission ask, when the run may run commands; dropped when the card ends.
    session: Option<Session>,
    /// The mode this run settled on.
    mode: Mode,
    /// A folder Io needed a yes to read and did not get one for.
    pending: Option<access::Request>,
}

/// The mode this run is in: the flags, then `[ask]`, then the terminal ([`Mode::choose`]).
fn chosen_mode(chosen: &Chosen<'_>, terminal: bool) -> Result<Mode, Error> {
    let (flag, config) = chosen.modes;
    Mode::choose(flag, config, terminal).map_err(Error::Usage)
}

/// Starts the broker the `mode` calls for. `None` when the mode runs no commands (`plan` and
/// `deny`), when the folder is not readable (the run has no tools then; the home folder is the
/// exception in `manual` at a terminal: `Bash` alone, each command asked) or when this system
/// has no unix sockets. A broker that cannot start is an error the person sees, never a
/// silent fall back to something more permissive.
// Off unix there is no broker to start or to fail: the body is `Ok(None)`, and the signature stays
// the unix one the caller uses.
#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn permission_session(
    chosen: &Chosen<'_>,
    ctx: &Context,
    mode: Mode,
) -> Result<Option<Session>, Error> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let shell_at_home =
        mode == Mode::Manual && headless::is_home(Path::new(&ctx.cwd), home.as_deref());
    if !mode.runs_commands() || !(ctx.reads || !ctx.also.is_empty() || shell_at_home) {
        return Ok(None);
    }
    // No unix sockets, no live ask: off unix `iohr ask` stays read-only (see
    // `permissions::broker`), so there is no session to start.
    #[cfg(unix)]
    return start_broker(mode, chosen, ctx).map(Some);
    #[cfg(not(unix))]
    {
        let _ = chosen;
        Ok(None)
    }
}

/// The words that name a mode: the flag layer (`--mode`, `--permissions`, and their environment)
/// and the `[ask]` layer.
fn words<'a>(args: &'a AskArgs, cfg: &'a iohr_auth::AskConfig) -> (Words<'a>, Words<'a>) {
    (
        Words {
            mode: args.mode.as_deref(),
            permissions: args.permissions.as_deref(),
        },
        Words {
            mode: cfg.mode.as_deref(),
            permissions: cfg.permissions.as_deref(),
        },
    )
}

/// What the mode means for the streamed run, which has no permission broker so it maps onto
/// what print mode can do: `(allow_edits, plan)`. `plan` reads and changes nothing, `auto` is
/// `--allow-edits` (edits yes, the shell still no: commands need the card's broker), and
/// `--allow-edits` alone is as before.
fn streamed(
    (flag, config): (Words<'_>, Words<'_>),
    args: &AskArgs,
    cfg: &iohr_auth::AskConfig,
    terminal: bool,
) -> Result<(bool, bool), Error> {
    let mode = Mode::choose(flag, config, terminal).map_err(Error::Usage)?;
    let plan = mode == Mode::Plan;
    let edits = args.allow_edits || cfg.allow_edits.unwrap_or(false) || mode == Mode::Auto;
    Ok((!plan && edits, plan))
}

/// Ask, never "I can't" (PRD 0187, RFC 0203). The home folder, the root and a folder the
/// question names outside the working one are too broad or too far to read unasked
/// ([`access::needed`]). At a terminal, where every command is asked anyway, Io asks on the
/// permission card whether it may read that folder; a yes for always is kept in `trusted.toml`
/// in the user's iohr folder, a yes for this time keeps nothing.
///
/// A folder is granted from five places, and they add up: `--read` (alias `--add-dir`, as
/// often as you like), `IOHR_ASK_READ`, `[ask] read` (all three a yes for this run, the way
/// without a terminal: [`Chosen::folders`]), the folders in `trusted.toml` (when the working
/// folder is itself readable; the home folder and the root are never added this way), and the
/// card's Add folder, which keeps its yes in `trusted.toml`. Every one goes through
/// [`access::allowable_at`] first, so none is a folder that is missing, cannot be listed or is
/// a secret place, and the secret file rules (`permissions::secrets`) apply inside all of them.
/// A yes gives the file tools beside the asked commands; a no, no terminal, `--json` or a mode
/// that runs nothing for the answer's sake (`plan`, `deny`, `rules`) leaves the request
/// pending: the card says what Io needs and offers Allow, and the run answers from the names.
///
/// # Errors
///
/// A folder asked for (flag, variable or config) is not one Io may read, or there are more
/// than [`MAX_FOLDERS`] of them.
fn grant_access(
    chosen: &Chosen<'_>,
    ctx: &mut Context,
    (mode, terminal): (Mode, bool),
    question: &str,
) -> Result<Option<access::Request>, Error> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let cwd = std::path::PathBuf::from(&ctx.cwd);
    if chosen.folders.len() > MAX_FOLDERS {
        return Err(Error::Usage(format!(
            "{} folders asked for to read; at most {MAX_FOLDERS}",
            chosen.folders.len()
        )));
    }
    for (source, folder) in &chosen.folders {
        let real = access::allowable_at(folder, &cwd, home.as_deref(), *source).map_err(|why| {
            Error::Usage(format!("{} {}: {why}", asked_by(*source), folder.display()))
        })?;
        apply_grant(ctx, &real, *source);
    }
    if ctx.reads {
        // The folders added with Add folder stand for every ask; each is checked again, as a
        // flag is, and the home folder, the root and anything broad are never added this way.
        for folder in trust::standing(chosen.dir) {
            if ctx.also.len() < MAX_FOLDERS
                && let Ok(real) =
                    access::allowable_at(&folder, &cwd, home.as_deref(), access::Source::Trust)
            {
                apply_grant(ctx, &real, access::Source::Trust);
            }
        }
    }
    // One folder at a time: a question can name several, and each is granted or left pending.
    loop {
        let Some(req) = access::needed(question, &cwd, home.as_deref(), (ctx.reads, &ctx.also))
        else {
            return Ok(None);
        };
        if !req.askable() {
            return Ok(Some(req));
        }
        let may_ask = terminal && matches!(mode, Mode::Manual | Mode::Auto);
        let kept = trust::is_trusted(chosen.dir, &req.folder);
        let asked = (!kept && may_ask).then(|| match trust::ask(&req.folder) {
            trust::Answer::Always => {
                if let Err(why) = trust::remember(chosen.dir, &req.folder) {
                    let _ = console::Term::stderr()
                        .write_line(&format!("Trusted for this time only: {why}"));
                }
                true
            }
            trust::Answer::Once => true,
            trust::Answer::No => false,
        });
        if !(kept || asked == Some(true)) || ctx.also.len() >= MAX_FOLDERS {
            return Ok(Some(req));
        }
        let source = if kept {
            access::Source::Trust
        } else {
            access::Source::Card
        };
        apply_grant(ctx, &req.folder, source);
    }
}

/// Gives the run the file tools for `folder`: the working folder itself, or a folder beside it
/// (once: a repeat of one already there adds nothing), and notes where the yes came from for
/// the audit line, `--json` and the label.
fn apply_grant(ctx: &mut Context, folder: &Path, source: access::Source) {
    let here = access::canonical(Path::new(&ctx.cwd));
    let new = if here.as_deref() == Some(folder) {
        !std::mem::replace(&mut ctx.reads, true)
    } else if ctx.also.iter().any(|a| a == folder) {
        false
    } else {
        ctx.also.push(folder.to_path_buf());
        true
    };
    if new {
        ctx.granted.push(access::Grant {
            source,
            folder: folder.to_path_buf(),
        });
    }
}

/// One `permissions.log` line for each folder the run was given: the source of the yes and
/// the path, never anything in it. A log that cannot be written is said, not an error.
fn audit_grants(chosen: &Chosen<'_>, ctx: &Context, out: Out) {
    let files = store::Files::find(Path::new(&ctx.cwd), chosen.dir);
    for g in &ctx.granted {
        if let Err(why) = store::audit_folder(&files, g.source.name(), &g.folder)
            && !out.json
        {
            Out::note(&format!("could not write the permission log: {why}"));
        }
    }
}

/// The model's answer split into what is shown, the questions it asked back and the next
/// steps it suggested; neither block is ever left in the text.
fn split_answer(raw: &str, cwd: &Path) -> (String, Vec<askback::Question>, Vec<Step>) {
    let (text, questions) = askback::split(raw);
    let command = card::command_of(&text);
    let (text, steps) = steps::split(&text, cwd, command.as_deref());
    (text, questions, steps)
}

/// Starts the broker. A broker that cannot start is an error, never a fall back to something
/// more permissive.
#[cfg(unix)]
fn start_broker(mode: Mode, chosen: &Chosen<'_>, ctx: &Context) -> Result<Session, Error> {
    // The commands run where the run starts, so the rules are judged from the same folder.
    let at = scope_dir(ctx);
    let place = Place::at(&at);
    let bin = crate::commands::shell::bin_name(std::env::args_os().next().as_deref()).to_owned();
    let files = Files::find(&at, chosen.dir);
    Session::start(Opts {
        mode,
        files,
        place,
        bin,
    })
    .map_err(|why| {
        Error::Failed(format!(
            "cannot start the permission ask: {why}; run with `--permissions deny` to go on without commands"
        ))
    })
}

/// One capped run where `ctx` says: in the folder when it may read it or run commands there.
async fn run_at(
    (bin, model): (&Path, &str),
    message: &str,
    started: Instant,
    ctx: &Context,
    (session, mode): (Option<&Session>, Mode),
) -> Result<(String, u64), String> {
    let folder = run_dir(ctx, session);
    headless::run(
        bin,
        message,
        model,
        started,
        folder.as_deref(),
        (ctx.reads, &ctx.also),
        (session, mode == Mode::Plan),
    )
    .await
}

/// The folder the run's file tools and its asked commands are anchored to: the working folder,
/// except when that one is too broad to read (the home folder) and the person allowed others:
/// then only the first allowed one (the rest are added by name, `headless::args`), so a yes
/// for `~/Downloads` never reads the rest of home.
fn scope_dir(ctx: &Context) -> std::path::PathBuf {
    match ctx.also.first() {
        Some(also) if !ctx.reads => also.clone(),
        _ => std::path::PathBuf::from(&ctx.cwd),
    }
}

/// Where the run starts: [`scope_dir`] when it may read there or run commands, else none
/// (the temp folder).
fn run_dir(ctx: &Context, session: Option<&Session>) -> Option<std::path::PathBuf> {
    (ctx.reads || !ctx.also.is_empty() || session.is_some()).then(|| scope_dir(ctx))
}

/// The lines after an answer for what Io wanted and could not be allowed: with a rule to
/// allow each. Empty when nothing was refused, or at a terminal where the person was asked.
fn refused_lines(session: Option<&Session>) -> Vec<String> {
    session
        .map(|s| broker::denied_lines(&s.denied(), s.bin()))
        .unwrap_or_default()
}

/// What the person answered at the terminal, from the `from`th answer on (a card drawn after
/// an earlier one shows only what is new).
fn settled_of(session: Option<&Session>, from: usize) -> Vec<Settled> {
    session
        .map(|s| s.settled().into_iter().skip(from).collect())
        .unwrap_or_default()
}

/// The rules the person saved with "always" in this run, each once: what the Undo action
/// takes back.
fn saved_rules(session: Option<&Session>) -> Vec<String> {
    let mut rules: Vec<String> = Vec::new();
    for r in settled_of(session, 0)
        .into_iter()
        .filter(|s| s.grant == Grant::Always)
        .flat_map(|s| s.rules)
    {
        if !rules.contains(&r) {
            rules.push(r);
        }
    }
    rules
}

/// The permission results as notes, for output with no card to hold them (a plain look, a
/// pipe): the sentence, then for a saved rule the rule, the file and the command that undoes
/// it, because there is no action row there to carry an Undo.
fn settled_lines(session: Option<&Session>, from: usize) -> Vec<String> {
    let bin = session.map_or("iohr", Session::bin);
    settled_of(session, from)
        .iter()
        .map(|s| {
            let mut line = s.sentence();
            if let (Some(detail), Some(undo)) = (s.detail(), s.undo_command(bin)) {
                line = format!("{line} ({detail}). Undo: {undo}");
            }
            line
        })
        .collect()
}

/// The permission results as the card's status rows. Where the hook binds the action row,
/// Undo is there; without it the row's detail names the command that undoes the rule.
fn statuses(session: Option<&Session>, from: usize, hook: bool) -> Vec<Status> {
    let bin = session.map_or("iohr", Session::bin);
    settled_of(session, from)
        .iter()
        .map(|s| {
            let detail = match (s.detail(), s.undo_command(bin)) {
                (Some(d), Some(undo)) if !hook => Some(format!("{d} \u{b7} undo: {undo}")),
                (d, _) => d,
            };
            Status {
                text: s.sentence(),
                detail,
                refused: s.grant == Grant::No,
            }
        })
        .collect()
}

/// A failed run is a card too; the exit code is the agent's.
fn draw_failed(
    shown: &Card<'_>,
    why: &str,
    started: Instant,
    (width, look): (usize, CardLook),
) -> Error {
    let failed = Card {
        elapsed: Some(started.elapsed()),
        body: Body::Failed(why),
        ..*shown
    };
    card::draw(&card::render(&failed, width, look));
    Error::Child(1)
}

/// After the answer: what the rules refused, then the one line about waiting asks.
fn closing_notes(
    session: Option<&Session>,
    waiting: Option<&asks::Waiting>,
    (h, menu_key, look, width): (&HeadlessArgs, MenuKey, CardLook, usize),
    (pending, attach): (Option<&access::Request>, Option<&str>),
) {
    // A drawn card holds the permission results itself; a plain look has none to hold them.
    if !look.fancy {
        for line in settled_lines(session, 0) {
            Out::note(&line);
        }
    }
    for line in refused_lines(session) {
        Out::note(&line);
    }
    // What Io needs a yes for: the menu's key when the hook binds it, else the flag.
    if let Some(req) = pending {
        let key = menu_key.label().filter(|_| h.hook && req.askable());
        let line = access::note(req, key);
        Out::note(&if look.fancy {
            console::style(line).dim().to_string()
        } else {
            line
        });
    }
    if let Some(line) = attach {
        Out::note(line);
    }
    if let Some(w) = waiting {
        Out::note(&waiting_line(w, h.hook, menu_key, look, width));
    }
}

/// The one line that says the last command's output was not sent and how to send it, where the
/// card's action row does not (it is drawn only by a fancy look under the hook): `None` when
/// nothing is offered (the command did not fail, the question does not ask why, or output is
/// attached already). `(key, drawn)` is the menu key and whether the row is on screen.
/// Whether Attach output is on offer for this ask, and the plain note that says so when the
/// card draws no row of its own.
fn attach_offer(
    ctx: &Context,
    question: &str,
    h: &HeadlessArgs,
    (key, fancy): (MenuKey, bool),
) -> (bool, Option<String>) {
    let offered = attach::offer(
        question,
        &ctx.command,
        ctx.exit_code,
        !ctx.output.is_empty(),
    )
    .is_some();
    (
        offered,
        attach_note(ctx, question, h, (key, h.hook && fancy)),
    )
}

fn attach_note(
    ctx: &Context,
    question: &str,
    h: &HeadlessArgs,
    (key, drawn): (MenuKey, bool),
) -> Option<String> {
    attach::offer(
        question,
        &ctx.command,
        ctx.exit_code,
        !ctx.output.is_empty(),
    )?;
    if drawn {
        return None;
    }
    Some(match key.label().filter(|_| h.hook) {
        Some(k) => format!(
            "Its output was not sent. {k} then w re-runs the command with its output captured (you are asked first), p pastes it."
        ),
        None => "Its output was not sent. To include it, save it to a file and ask again with --output-file FILE; secrets in it are redacted.".to_owned(),
    })
}

/// The line the shell hook reads back: the answer's one command and link, and the rules the
/// person saved in this run (what Undo takes back).
fn record_hook(text: &str, cwd: &Path, undo: &[String], next: &actions::Next<'_>) {
    let command = card::command_of(text);
    let link = card::link_of(text, cwd);
    Out::raw(actions::record(text, command.as_deref(), link.as_deref(), undo, next).as_bytes());
}

/// The answer of a run with no card: the JSON of `--json`, else the text once on stdout with
/// the questions, what Io needs a yes for and the next steps as notes apart from it.
fn print_answer(
    out: Out,
    chosen: &Chosen<'_>,
    (ctx, label, session): (&Context, &str, Option<&Session>),
    (raw, duration_ms): (String, u64),
    (question, h, pending): (&str, &HeadlessArgs, Option<&access::Request>),
) {
    let (text, questions, steps) = split_answer(&raw, Path::new(&ctx.cwd));
    if out.json {
        Out::print_json(&Answer {
            command: card::command_of(&text),
            text,
            questions,
            agent: chosen.agent.name(),
            via: chosen.via.name(),
            label: label.to_owned(),
            model: chosen.model.to_owned(),
            exit_code: 0,
            duration_ms,
            permissions: settled_of(session, 0).iter().map(Settled::json).collect(),
            access: pending.map(access::Request::shown),
            next: steps.iter().map(Step::shown).collect(),
            role: chosen.role.map(runtime::Applied::json),
            context: ctx.sent(question),
        });
        return;
    }
    Out::raw(format!("{text}\n").as_bytes());
    // No terminal to ask on: the questions are plain lines, apart from the answer.
    for line in askback::plain_lines(&questions) {
        Out::note(&line);
    }
    if let Some(line) = attach_note(ctx, question, h, (MenuKey::Off, false)) {
        Out::note(&line);
    }
    if let Some(req) = pending {
        Out::note(&access::note(req, None));
    }
    for step in &steps {
        Out::note(&format!("next: {}", step.label()));
    }
}

/// What the action bar offers besides the command: Allow for the folder asked about (when a
/// yes would help) and the next steps.
fn next_for<'a>(pending: Option<&'a access::Request>, steps: &'a [Step]) -> actions::Next<'a> {
    actions::Next {
        allow: pending.filter(|r| r.askable()).map(|r| r.folder.as_path()),
        steps,
    }
}

/// The footer a card shows: a plain look has none to move on (its label discloses).
fn footer_for(look: CardLook, (dir, hints): (&Path, hints::Mode), notice: &str) -> hints::Footer {
    if look.fancy {
        hints::next(dir, hints, notice, hints::now())
    } else {
        hints::Footer::Full
    }
}

/// The short hint a footer carries, none for the full one.
fn footer_hint(footer: hints::Footer) -> Option<&'static str> {
    match footer {
        hints::Footer::Short(h) => h,
        hints::Footer::Full => None,
    }
}

/// Runs the model; on a drawn card the asks waiting are looked up beside it.
async fn think(
    run: impl std::future::Future<Output = Result<(String, u64), String>>,
    (agent, look, started): (Agent, CardLook, Instant),
    (g, env): (&Global, &Env),
) -> Result<(Result<(String, u64), String>, Option<asks::Waiting>), Error> {
    if look.fancy {
        let (done, waiting) =
            tokio::join!(spin_while(run, agent, look, started), asks::waiting(g, env));
        Ok((done?, waiting))
    } else {
        Ok((run.await, None))
    }
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
        session,
        mode,
        pending,
    } = prepared;
    let look = CardLook::detect(fancy);
    let how = (session.as_ref(), mode);
    let run = run_at((bin, model), &message, started, &ctx, how);
    // The asks waiting are looked up while the model thinks, so they add no time.
    let (done, waiting) = think(run, (agent, look, started), (g, env)).await?;
    let notice = format!("An AI model {} {}", ask::PROVIDER, ask::CHECK);
    let footer = footer_for(look, (dir, hints), &notice);
    // The permission results of the run so far, drawn in the card; a second card (after the
    // model's question is answered) shows only what came after.
    let seen = settled_of(session.as_ref(), 0).len();
    let undo = saved_rules(session.as_ref());
    let status = statuses(session.as_ref(), 0, h.hook);
    let shown = Card {
        agent: agent.display(),
        model,
        mode: mode.shown(),
        elapsed: None,
        follows: h.prior_question.as_deref(),
        read: &read,
        notice: &notice,
        full: footer == hints::Footer::Full,
        label: &label,
        hint: footer_hint(footer),
        row: None,
        status: &status,
        body: Body::Failed(""),
    };
    let width = card::stderr_width();
    let (cwd, (offered, note)) = (
        Path::new(&ctx.cwd).to_path_buf(),
        attach_offer(&ctx, question, h, (menu_key, look.fancy)),
    );
    match &done {
        Ok((raw, duration_ms)) => {
            let (mut text, questions, mut steps) = split_answer(raw, &cwd);
            let took = Duration::from_millis(*duration_ms);
            let ctx_of = (cwd.as_path(), !undo.is_empty(), offered);
            draw_answer(
                &shown,
                &text,
                took,
                (h, look, menu_key),
                (ctx_of, &next_for(pending.as_ref(), &steps)),
                width,
            );
            // The model asked back: ask the person, keep the answer, and answer once more.
            let can_ask = may_ask_back(h, look, &questions);
            if let Some(root) = root.as_deref().filter(|_| can_ask)
                && let Some(again) =
                    ask_back(chosen, question, &mut ctx, (root, &questions), look, how).await?
            {
                text = again.0;
                steps = again.1;
                let how = (seen, h, look, menu_key);
                let took = Duration::from_millis(again.2);
                draw_again(
                    &shown,
                    session.as_ref(),
                    how,
                    (&text, took),
                    (&cwd, width, offered, &next_for(pending.as_ref(), &steps)),
                );
            }
            closing_notes(
                session.as_ref(),
                waiting.as_ref(),
                (h, menu_key, look, width),
                (pending.as_ref(), note.as_deref()),
            );
            let undo = saved_rules(session.as_ref());
            hand_over(
                h,
                stdout_tty,
                (&text, &cwd),
                (&undo, &next_for(pending.as_ref(), &steps)),
            );
            Ok(())
        }
        Err(why) => Err(draw_failed(&shown, why, started, (width, look))),
    }
}

/// Whether the model's questions are put to the person: a drawn card at a terminal.
fn may_ask_back(h: &HeadlessArgs, look: CardLook, questions: &[askback::Question]) -> bool {
    look.fancy
        && prompt::interactive()
        && (h.hook || h.card || !h.headless)
        && !questions.is_empty()
}

/// What goes to stdout after the card: the hook's record (for the shell's menu and `??`), else
/// the answer once when stdout is not the terminal the card is on.
fn hand_over(
    h: &HeadlessArgs,
    stdout_tty: bool,
    (text, cwd): (&str, &Path),
    (undo, next): (&[String], &actions::Next<'_>),
) {
    if h.hook {
        record_hook(text, cwd, undo, next);
    } else if !stdout_tty {
        Out::raw(format!("{text}\n").as_bytes());
    }
}

/// The card once more after the model's question was answered: the same card with only the
/// permission results that came since (`seen` is how many the first card showed).
fn draw_again(
    shown: &Card<'_>,
    session: Option<&Session>,
    (seen, h, look, menu_key): (usize, &HeadlessArgs, CardLook, MenuKey),
    (text, took): (&str, Duration),
    (cwd, width, offered, next): (&Path, usize, bool, &actions::Next<'_>),
) {
    let fresh = statuses(session, seen, h.hook);
    let again = Card {
        status: &fresh,
        ..shown.clone()
    };
    let undo = !saved_rules(session).is_empty();
    draw_answer(
        &again,
        text,
        took,
        (h, look, menu_key),
        ((cwd, undo, offered), next),
        width,
    );
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
    how: (Option<&Session>, Mode),
) -> Result<Option<(String, Vec<Step>, u64)>, Error> {
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
    let run = run_at((chosen.bin, chosen.model), &again, started, ctx, how);
    match spin_while(run, chosen.agent, look, started).await? {
        // The second answer asks nothing: a question in it is dropped.
        Ok((raw, ms)) => {
            let (text, _, steps) = split_answer(&raw, Path::new(&ctx.cwd));
            Ok(Some((text, steps, ms)))
        }
        Err(why) => {
            Out::note(&format!("could not refresh the answer: {why}"));
            Ok(None)
        }
    }
}

/// Draws one answer card: the row under it only when the hook is there to bind its key
/// (`undo` adds the action that takes back a rule saved in this run).
fn draw_answer(
    shown: &Card<'_>,
    text: &str,
    took: Duration,
    (h, look, menu_key): (&HeadlessArgs, CardLook, MenuKey),
    ((cwd, undo, attach), next): ((&Path, bool, bool), &actions::Next<'_>),
    width: usize,
) {
    let command = card::command_of(text);
    let link = card::link_of(text, cwd);
    // The row promises a key that only the hook binds, so only the hook draws it.
    let row = (h.hook && look.fancy)
        .then(|| {
            let has = actions::Extras {
                link: link.is_some(),
                undo,
                attach,
            };
            actions::row(
                menu_key,
                actions::Offer::of(command.as_deref(), cwd),
                has,
                next,
            )
        })
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
    fn an_allowed_folder_beside_the_home_folder_is_the_only_read_scope() {
        let home = Context {
            cwd: "/home/me".into(),
            reads: false,
            also: vec!["/home/me/Downloads".into()],
            ..Context::default()
        };
        // With or without a session (the shell), the run starts in the allowed folder, never home.
        assert_eq!(scope_dir(&home), Path::new("/home/me/Downloads"));
        assert_eq!(run_dir(&home, None), Some("/home/me/Downloads".into()));
        // The broker is built over `scope_dir`, so the shell is anchored there too.
        let args = headless::args("haiku", (home.reads, &home.also), None, false).unwrap();
        assert!(!args.iter().any(|a| a.starts_with("--add-dir")));
        // Home alone with a session (the shell, no file tools) stays at home; with nothing, no folder.
        let shell = Context {
            also: Vec::new(),
            ..home.clone()
        };
        assert_eq!(scope_dir(&shell), Path::new("/home/me"));
        assert_eq!(run_dir(&shell, None), None);
    }

    #[test]
    fn a_readable_working_folder_stays_the_run_folder_and_the_allowed_one_is_added() {
        let project = Context {
            cwd: "/work/p".into(),
            reads: true,
            also: vec!["/data/x".into()],
            ..Context::default()
        };
        assert_eq!(scope_dir(&project), Path::new("/work/p"));
        assert_eq!(run_dir(&project, None), Some("/work/p".into()));
        let args = headless::args("haiku", (project.reads, &project.also), None, false).unwrap();
        assert!(args.contains(&"--add-dir=/data/x".to_owned()));
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

    // ---- ask instead of "I can't" (RFC 0203)

    struct Tree {
        dir: tempfile::TempDir,
        /// The folder the question is asked in.
        work: std::path::PathBuf,
        /// A folder beside it that a question can name.
        other: std::path::PathBuf,
        /// The place `trusted.toml` lives.
        user: std::path::PathBuf,
    }

    fn tree() -> Tree {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let (work, other, user) = (root.join("work"), root.join("other"), root.join("cfg"));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(work.join("a.txt"), "x").unwrap();
        Tree {
            dir: d,
            work,
            other,
            user,
        }
    }

    fn ctx_in(cwd: &Path, reads: bool) -> Context {
        Context {
            cwd: cwd.display().to_string(),
            reads,
            ..Context::default()
        }
    }

    fn chosen<'a>(dir: &'a Path, bin: &'a Path) -> Chosen<'a> {
        Chosen {
            bin,
            agent: Agent::Claude,
            via: Via::Local,
            model: "haiku",
            hints: hints::Mode::default(),
            menu_key: MenuKey::Down,
            dir,
            role: None,
            modes: (Words::default(), Words::default()),
            folders: Vec::new(),
        }
    }

    #[test]
    fn a_named_folder_outside_is_asked_for_and_nothing_is_read_before_the_yes() {
        let t = tree();
        let bin = Path::new("claude");
        let mut ctx = ctx_in(&t.work, true);
        let q = format!("what is in {}", t.other.display());
        // No terminal, so Io cannot ask here: the request goes back, the run stays as it was.
        let pending = grant_access(&chosen(&t.user, bin), &mut ctx, (Mode::Manual, false), &q)
            .unwrap()
            .unwrap();
        assert_eq!(pending.folder, t.other);
        assert!(!pending.is_cwd && pending.askable());
        assert_eq!(ctx.also.len(), 0);
        // Deny never asks, even at a terminal: no prompt is drawn and the request is kept.
        let pending =
            grant_access(&chosen(&t.user, bin), &mut ctx, (Mode::Deny, true), &q).unwrap();
        assert!(pending.is_some());
        assert_eq!(ctx.also.len(), 0);
    }

    #[test]
    fn a_yes_on_record_gives_the_read_tools_for_that_folder() {
        let t = tree();
        let bin = Path::new("claude");
        trust::remember(&t.user, &t.other).unwrap();
        let mut ctx = ctx_in(&t.work, true);
        let q = format!("what is in {}", t.other.display());
        let pending =
            grant_access(&chosen(&t.user, bin), &mut ctx, (Mode::Manual, false), &q).unwrap();
        assert!(pending.is_none());
        assert_eq!(ctx.also, vec![t.other.clone()]);
        // The run then has tools for it, secrets still denied (see the headless tests).
        let args = headless::args("haiku", (ctx.reads, &ctx.also), None, false).unwrap();
        assert!(args.contains(&format!("--add-dir={}", t.other.display())));
        assert!(
            args.iter()
                .any(|a| a.starts_with("--disallowedTools=Read("))
        );
    }

    #[test]
    fn a_broad_folder_with_a_yes_for_it_reads_and_without_one_asks() {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            return;
        };
        let Ok(home) = home.canonicalize() else {
            return;
        };
        let t = tree();
        let bin = Path::new("claude");
        let mut ctx = ctx_in(&home, false);
        let pending = grant_access(
            &chosen(&t.user, bin),
            &mut ctx,
            (Mode::Manual, false),
            "explain this directory",
        )
        .unwrap()
        .unwrap();
        assert!(pending.is_cwd);
        assert!(!ctx.reads, "never answered with tools it was not given");
        trust::remember(&t.user, &home).unwrap();
        let mut ctx = ctx_in(&home, false);
        let pending = grant_access(
            &chosen(&t.user, bin),
            &mut ctx,
            (Mode::Manual, false),
            "explain this directory",
        )
        .unwrap();
        assert!(pending.is_none());
        assert!(ctx.reads);
    }

    #[test]
    fn read_is_the_yes_for_one_run_and_refuses_what_is_not_a_readable_folder() {
        let t = tree();
        let bin = Path::new("claude");
        let mut ctx = ctx_in(&t.work, true);
        let q = format!("what is in {}", t.other.display());
        let mut yes = chosen(&t.user, bin);
        yes.folders = vec![(FROM_FLAG, t.other.clone())];
        let pending = grant_access(&yes, &mut ctx, (Mode::Manual, false), &q).unwrap();
        assert!(pending.is_none());
        assert_eq!(ctx.also, vec![t.other.clone()]);
        assert!(!trust::is_trusted(&t.user, &t.other), "one run only");
        for bad in [t.work.join("a.txt"), t.work.join("nope")] {
            let mut ctx = ctx_in(&t.work, true);
            let mut no = chosen(&t.user, bin);
            no.folders = vec![(FROM_FLAG, bad.clone())];
            let err = grant_access(&no, &mut ctx, (Mode::Manual, false), "q").unwrap_err();
            assert!(matches!(err, Error::Usage(_)), "{bad:?}");
        }
    }

    /// Runs one ask over `folders` as `chosen` says, in `work` with the file tools.
    fn granted(
        t: &Tree,
        folders: Vec<(access::Source, std::path::PathBuf)>,
        question: &str,
    ) -> Result<Context, Error> {
        let mut ctx = ctx_in(&t.work, true);
        let mut c = chosen(&t.user, Path::new("claude"));
        c.folders = folders;
        grant_access(&c, &mut ctx, (Mode::Manual, false), question)?;
        Ok(ctx)
    }

    fn third(t: &Tree) -> std::path::PathBuf {
        let p = t.dir.path().canonicalize().unwrap().join("third");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn the_four_sources_name_folders_and_each_grants_one_beside_the_working_folder() {
        let t = tree();
        let home = t.dir.path().canonicalize().unwrap();
        let (other, third) = (t.other.clone(), third(&t));
        // Flags, then IOHR_ASK_READ (a path list), then [ask] read, each tagged with its source.
        let env = std::env::join_paths([&other, &third]).unwrap();
        let all = asked_folders(
            std::slice::from_ref(&other),
            Some(&env),
            &["~/third".to_owned(), "  ".to_owned()],
            Some(&home),
        );
        assert_eq!(
            all,
            [
                (FROM_FLAG, other.clone()),
                (FROM_ENV, other.clone()),
                (FROM_ENV, third.clone()),
                (FROM_CONFIG, third.clone()),
            ]
        );
        // Each source alone reaches the run, with no question naming the folder.
        for source in [FROM_FLAG, FROM_ENV, FROM_CONFIG] {
            let ctx = granted(&t, vec![(source, other.clone())], "what is this").unwrap();
            assert_eq!(ctx.also, vec![other.clone()], "{source:?}");
        }
        // Several at once, none twice.
        let ctx = granted(&t, all, "what is this").unwrap();
        assert_eq!(ctx.also, [other.clone(), third]);
        // The trust store is the fourth: a folder added there as standing is granted on every ask.
        trust::add_standing(&t.user, &other).unwrap();
        let ctx = granted(&t, Vec::new(), "what is this").unwrap();
        assert_eq!(ctx.also, [other]);
    }

    #[test]
    fn an_empty_env_and_config_add_nothing() {
        let all = asked_folders(&[], Some(std::ffi::OsStr::new("")), &[], None);
        assert_eq!(all.len(), 0);
        assert_eq!(asked_folders(&[], None, &[String::new()], None).len(), 0);
    }

    #[test]
    fn a_trusted_folder_is_not_granted_without_the_file_tools_or_when_it_is_gone() {
        let t = tree();
        trust::add_standing(&t.user, &t.other).unwrap();
        // The working folder itself cannot be read (no tools): nothing is added behind that.
        let mut ctx = ctx_in(&t.work, false);
        grant_access(
            &chosen(&t.user, Path::new("claude")),
            &mut ctx,
            (Mode::Manual, false),
            "what is this",
        )
        .unwrap();
        assert_eq!(ctx.also.len(), 0);
        // A trusted folder that was removed since is skipped, not an error.
        std::fs::remove_dir(&t.other).unwrap();
        let ctx = granted(&t, Vec::new(), "what is this").unwrap();
        assert_eq!(ctx.also.len(), 0);
    }

    #[test]
    fn a_secret_place_is_refused_from_every_source_and_never_trusted_in() {
        let t = tree();
        let secret = t.dir.path().canonicalize().unwrap().join(".ssh");
        std::fs::create_dir_all(&secret).unwrap();
        for source in [FROM_FLAG, FROM_ENV, FROM_CONFIG] {
            let err = granted(&t, vec![(source, secret.clone())], "q").unwrap_err();
            let Error::Usage(why) = err else {
                panic!("{source:?}: not a usage error")
            };
            assert!(why.starts_with(asked_by(source)), "{why}");
        }
        // The card's own check says the same, and a folder in the trust store that is secret is skipped.
        assert!(folder_to_add_is_refused(&t, &secret));
        trust::add_standing(&t.user, &secret).unwrap();
        let ctx = granted(&t, Vec::new(), "what is this").unwrap();
        assert_eq!(ctx.also.len(), 0);
    }

    fn folder_to_add_is_refused(t: &Tree, typed: &Path) -> bool {
        crate::actions::folder_to_add(&typed.display().to_string(), &t.work, None).is_err()
    }

    #[test]
    fn more_folders_than_the_cap_are_refused_whole() {
        let t = tree();
        let many = (0..=MAX_FOLDERS)
            .map(|_| (FROM_ENV, t.other.clone()))
            .collect();
        let err = granted(&t, many, "q").unwrap_err();
        assert!(matches!(err, Error::Usage(_)));
        let ok = (0..MAX_FOLDERS)
            .map(|_| (FROM_ENV, t.other.clone()))
            .collect();
        assert!(granted(&t, ok, "q").is_ok());
    }

    #[test]
    fn a_question_naming_two_folders_needs_both_and_the_asked_one_covers_only_itself() {
        let t = tree();
        let third = third(&t);
        let q = format!("compare {} and {}", t.other.display(), third.display());
        // One folder given: the other is still asked for.
        let mut ctx = ctx_in(&t.work, true);
        let mut c = chosen(&t.user, Path::new("claude"));
        c.folders = vec![(FROM_FLAG, t.other.clone())];
        let pending = grant_access(&c, &mut ctx, (Mode::Manual, false), &q)
            .unwrap()
            .unwrap();
        assert_eq!(pending.folder, third);
        // Both given: nothing is asked.
        let ctx = granted(
            &t,
            vec![(FROM_FLAG, t.other.clone()), (FROM_ENV, third.clone())],
            &q,
        )
        .unwrap();
        assert_eq!(ctx.also, [t.other.clone(), third]);
    }

    #[test]
    fn the_answer_is_split_into_text_questions_and_steps_with_no_block_left() {
        let t = tree();
        std::fs::create_dir_all(t.work.join("api")).unwrap();
        let raw = "A service.\n\n```next\nread api/\nrun ls\n```\n\n```sh\nls\n```";
        let (text, questions, steps) = split_answer(raw, &t.work);
        assert!(!text.contains("next"), "{text}");
        assert_eq!(questions.len(), 0);
        // The answer's own command is not repeated as a step.
        assert_eq!(steps, vec![Step::Read("api".into())]);
        assert_eq!(card::command_of(&text).as_deref(), Some("ls"));
    }

    #[test]
    fn plain_look_carries_no_escape_codes() {
        let line = waiting_line(&one("Merge #12"), true, MenuKey::CtrlG, PLAIN, 80);
        assert!(!line.contains('\u{1b}'), "{line:?}");
    }

    #[test]
    fn every_folder_granted_is_audited_with_its_source_and_listed_in_json() {
        let t = tree();
        let third = third(&t);
        let ctx = granted(
            &t,
            vec![(FROM_ENV, t.other.clone()), (FROM_CONFIG, third.clone())],
            "what is this",
        )
        .unwrap();
        let sent = ctx.sent("what is this");
        let json = serde_json::to_value(&sent).unwrap();
        assert_eq!(json["folders"][0]["source"], "env");
        assert_eq!(json["folders"][0]["path"], t.other.display().to_string());
        assert_eq!(json["folders"][1]["source"], "config");
        // One log line per folder, in the project's log.
        let c = chosen(&t.user, Path::new("claude"));
        audit_grants(&c, &ctx, Out { json: true });
        let log = std::fs::read_to_string(store::Files::find(&t.work, &t.user).log_path()).unwrap();
        let lines: Vec<serde_json::Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{log}");
        assert_eq!(lines[0]["source"], "env");
        assert_eq!(lines[1]["source"], "config");
        assert_eq!(lines[1]["path"], third.display().to_string());
        assert!(!log.contains("a.txt"), "never contents or names inside");
    }

    #[test]
    fn a_standing_folder_is_granted_on_every_ask_and_one_yes_for_a_question_is_not() {
        let t = tree();
        // A yes for one question (Always on the card) grants nothing on a later ask.
        trust::remember(&t.user, &t.other).unwrap();
        let ctx = granted(&t, Vec::new(), "what is this").unwrap();
        assert_eq!(ctx.also.len(), 0, "per-question only");
        // Add folder is standing: granted with no question naming it, tagged `trust`.
        trust::add_standing(&t.user, &t.other).unwrap();
        let ctx = granted(&t, Vec::new(), "what is this").unwrap();
        assert_eq!(ctx.also, std::slice::from_ref(&t.other));
        assert_eq!(ctx.granted[0].source, access::Source::Trust);
    }

    #[test]
    fn a_standing_folder_that_became_broad_is_skipped_and_an_env_home_is_refused() {
        let t = tree();
        let home = t.dir.path().canonicalize().unwrap();
        // A hand-edited standing list naming the root is not granted.
        trust::add_standing(&t.user, Path::new("/")).unwrap();
        let ctx = granted(&t, Vec::new(), "what is this").unwrap();
        assert_eq!(ctx.also.len(), 0);
        // Only the flag and the card may name home; env and config never.
        assert!(access::allowable_at(&home, &t.work, Some(&home), FROM_ENV).is_err());
        assert!(access::allowable_at(&home, &t.work, Some(&home), FROM_CONFIG).is_err());
        assert!(access::allowable_at(&home, &t.work, Some(&home), FROM_FLAG).is_ok());
    }
}
