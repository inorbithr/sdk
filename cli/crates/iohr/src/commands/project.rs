//! `iohr project init` (alias `create`) and `template`: set a project of your own up (PRD 0088).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::Env;
use crate::ask::stream::{Look, Thinking};
use crate::ask::{self, Agent};
use crate::atlas;
use crate::cli::{Global, InitArgs, ProjectCommand, TemplateCommand};
use crate::commands::ask::Run;
use crate::context::{Ctx, session};
use crate::error::Error;
use crate::handoff::{self, Stop};
use crate::output::Out;
use std::fmt::Write as _;

use super::project_template;
use crate::project::folder::{self, Kind as Place, Proposal, Where};
use crate::project::wizard::{self, Draft, Stage, Walk, Went};
use crate::project::{self, Report, Smoke, check_name, choose_template, lines};
use crate::template::{self, Answer, Answers, Kind, LANGUAGE, Question, Spec, StepName, Template};
use crate::toolchain::{self, Lang, State};
use crate::ui::{Choice, Mark, Nav, Ui, cancel};

/// The project's own state: `~/.inorbit/projects/<name>/`, never inside the repository.
fn state_root() -> Result<PathBuf, Error> {
    etcetera::home_dir()
        .map(|h| h.join(".inorbit").join("projects"))
        .map_err(|e| Error::Failed(format!("no home folder: {e}")))
}

/// The publisher half of the id: `--publisher`, else the account's slug when signed in,
/// else `local`. The private listing takes it when that step lands.
fn publisher_of(asked: Option<&str>, account: Option<&str>) -> String {
    let pick = asked.or(account).unwrap_or("").trim().to_ascii_lowercase();
    let ok = pick.len() >= 2
        && pick.len() <= 39
        && pick.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && pick
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok { pick } else { "local".to_owned() }
}

pub(super) fn now() -> String {
    crate::front::now()
}

/// The menu's first entry: the agent builds it from a description.
const DESCRIBE: &str = "@describe";
/// The template a described project is made from, unless one is named.
const DESCRIBE_TEMPLATE: &str = "blank";

pub(crate) async fn run(g: &Global, env: &Env, cmd: ProjectCommand, out: Out) -> Result<(), Error> {
    match cmd {
        ProjectCommand::Init(a) => init(g, env, *a, out).await,
        // The old name of `template list`: the same handler, so the output is identical.
        ProjectCommand::Templates => template_command(g, TemplateCommand::List, out),
        ProjectCommand::Template { cmd } => template_command(g, cmd, out),
    }
}

/// The template `init` uses without `--template`: `[project] template`, else the built-in
/// default.
fn default_template(g: &Global) -> Result<String, Error> {
    let configured = Ctx::load(g)?.config.project.template;
    Ok(choose_template(None, configured.as_deref()))
}

fn template_command(g: &Global, cmd: TemplateCommand, out: Out) -> Result<(), Error> {
    match cmd {
        TemplateCommand::List => project_template::list(&default_template(g)?, out),
        TemplateCommand::Show { id } => project_template::show(&id, &default_template(g)?, out),
        TemplateCommand::Path { id } => project_template::path(&id, out),
        TemplateCommand::New { id, extends } => {
            let dir = template::scaffold(&template::user_root()?, &id, extends.as_deref())
                .map_err(Error::Usage)?;
            if out.json {
                Out::print_json(&serde_json::json!({ "id": id, "dir": dir }));
            } else {
                Out::note(&format!(
                    "Made {}\nEdit its template.toml, then `iohr project template lint {}` and `iohr project init NAME --template {id} --dry-run`.",
                    dir.display(),
                    dir.display()
                ));
            }
            Ok(())
        }
        TemplateCommand::Lint { path } => {
            let r = template::lint_dir(&path, &template::user_root()?);
            if out.json {
                Out::print_json(&r);
            } else if r.ok {
                Out::note(&format!("{}: ok", path.display()));
            } else {
                for f in &r.findings {
                    Out::raw(format!("{}: {f}\n", path.display()).as_bytes());
                }
            }
            if r.ok {
                Ok(())
            } else {
                Err(Error::Failed(format!(
                    "{} finding{}",
                    r.findings.len(),
                    if r.findings.len() == 1 { "" } else { "s" }
                )))
            }
        }
        TemplateCommand::Schema => {
            Out::print_json(&template::schema());
            Ok(())
        }
    }
}

/// The one gate every action passes through. A dry run records what it would do and does
/// nothing; no step decides that on its own.
struct Flow {
    dry: bool,
    ui: Ui,
    would: Vec<(String, String)>,
}

impl Flow {
    /// True when the action may really happen. In a dry run it says what it would do, and
    /// returns false.
    fn gate(&mut self, id: &str, what: &str) -> bool {
        if self.dry {
            self.would.push((id.to_owned(), what.to_owned()));
            // Drawn, the plan is one numbered box at the end of the dry run; plain, each
            // action is a `would:` line as it is planned.
            if !self.ui.fancy() {
                self.ui.info(&format!("would: {what}"));
            }
            false
        } else {
            true
        }
    }

    /// Runs `f` unless this is a dry run.
    fn act<T>(&mut self, id: &str, what: &str, f: impl FnOnce() -> T) -> Option<T> {
        self.gate(id, what).then(f)
    }

    /// The same, for an action that waits.
    async fn act_async<T, Fut: std::future::Future<Output = T>>(
        &mut self,
        id: &str,
        what: &str,
        f: impl FnOnce() -> Fut,
    ) -> Option<T> {
        if self.gate(id, what) {
            Some(f().await)
        } else {
            None
        }
    }
}

fn no_agent_help() -> String {
    "no agent found: put `claude` (Claude Code) or `codex` on your PATH, or name one in iohr's config.toml with `[ask]` `agent = \"claude\"` (or `\"codex\"`)".to_owned()
}

fn nonempty(v: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        Err("say what you want".into())
    } else {
        Ok(())
    }
}

fn live_names(rows: &[template::Row]) -> String {
    rows.iter()
        .filter(|r| r.status == "live")
        .map(|r| format!("`{}`", r.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The language menu: every language of the template, the live ones first, each marked with
/// whether its toolchain is on the PATH (`have`); the cursor starts on the first live one that
/// has its toolchain (else on the template's default). The ones still coming are listed, dim,
/// and refused when picked. `behind` says, for a toolchain that is found, where it stands
/// against the latest stable (`rustc 1.90.0, 1.93.0 is out`), or nothing.
fn language_choices(
    tpl: &Template,
    have: impl Fn(&str) -> bool,
    behind: impl Fn(&template::VariantDef) -> Option<String>,
) -> (Vec<Choice>, String) {
    let found = |v: &template::VariantDef| v.needs.iter().all(|n| have(n));
    let live = tpl.live_variants();
    let recommended = live
        .iter()
        .find(|v| found(v))
        .copied()
        .or_else(|| tpl.default_variant());
    let mut out: Vec<Choice> = live
        .iter()
        .map(|v| {
            let what = if v.needs.is_empty() {
                "nothing to install".to_owned()
            } else if found(v) {
                match behind(v) {
                    Some(note) => format!("{} found \u{b7} {note}", v.needs.join(", ")),
                    None => format!("{} found", v.needs.join(", ")),
                }
            } else {
                format!("{} not on your PATH", v.needs.join(", "))
            };
            let star = recommended.is_some_and(|r| r.id == v.id);
            Choice {
                value: v.id.clone(),
                label: v.label.clone(),
                hint: if star {
                    format!("{what}, recommended")
                } else {
                    what
                },
                available: true,
                key: None,
            }
        })
        .collect();
    out.extend(
        tpl.meta
            .variants
            .iter()
            .filter(|v| v.status != template::Status::Live)
            .map(|v| Choice {
                value: v.id.clone(),
                label: v.label.clone(),
                hint: String::new(),
                available: false,
                key: None,
            }),
    );
    let initial = recommended.map(|v| v.id.clone()).unwrap_or_default();
    (out, initial)
}

/// The folder menu: every proposal that is free, the recommended first, each with its reason.
fn folder_choices(list: &[Proposal]) -> Vec<Choice> {
    list.iter()
        .filter(|p| !p.busy)
        .enumerate()
        .map(|(i, p)| Choice {
            value: i.to_string(),
            label: match p.kind {
                Place::ThisFolder => "this folder (.)".to_owned(),
                _ => p.shown.clone(),
            },
            hint: p.reason.clone(),
            available: true,
            key: None,
        })
        .collect()
}

/// The folder menu at a terminal: the free proposals, then "Somewhere else..." asks for a path.
/// `prev` is the folder picked before (going back, or changing the step): the cursor starts there.
fn pick_folder(
    ui: Ui,
    w: &Where,
    list: &[Proposal],
    prev: Option<&Proposal>,
    back: bool,
) -> Result<Nav<Proposal>, Error> {
    let free: Vec<&Proposal> = list.iter().filter(|p| !p.busy).collect();
    for p in list.iter().filter(|p| p.busy) {
        ui.info(&format!("{} is not empty, so it is not offered", p.shown));
    }
    let choices = folder_choices(list);
    // A typed path is not a row of the menu: it was the "Somewhere else" row.
    let typed_before = prev.filter(|p| p.kind == Place::Given);
    let initial = match prev {
        _ if typed_before.is_some() => free
            .iter()
            .position(|p| p.kind == Place::Elsewhere)
            .map_or_else(|| "0".to_owned(), |i| i.to_string()),
        Some(p) => free
            .iter()
            .position(|f| f.kind == p.kind && f.dir == p.dir)
            .map_or_else(|| "0".to_owned(), |i| i.to_string()),
        None => "0".to_owned(),
    };
    loop {
        let picked = match ui.select_step("Where should it go?", &choices, &initial, back)? {
            Nav::Back => return Ok(Nav::Back),
            Nav::Next(v) => v,
        };
        let idx: usize = picked.parse().unwrap_or(0);
        let p = free
            .get(idx)
            .map_or_else(|| list[0].clone(), |p| (*p).clone());
        if p.kind != Place::Elsewhere {
            return Ok(Nav::Next(p));
        }
        let before = typed_before
            .map(|p| p.dir.display().to_string())
            .unwrap_or_default();
        // The menu is the step before the typed path: back from it returns there.
        match ui.input_step(
            "Which folder?",
            &before,
            |s| {
                if s.trim().is_empty() {
                    Err("type a folder".to_owned())
                } else {
                    Ok(())
                }
            },
            true,
        )? {
            Nav::Back => {}
            Nav::Next(typed) => {
                return Ok(Nav::Next(folder::given(
                    w,
                    std::path::Path::new(typed.trim()),
                    "the folder you typed",
                )));
            }
        }
    }
}

/// What the user chose, for the dry run's report.
struct Chosen<'a> {
    place: &'a Proposal,
    lang: Option<&'a str>,
    warnings: &'a [String],
    agent: Option<&'a AgentRun>,
    answers: &'a Answers,
}

/// `iohr ext dev` runs Rust, Python and shell folders (`[dev] lang`); the other languages
/// run with their own tool, named in the README.
fn runs_in_ext_dev(lang: Option<&str>) -> bool {
    lang.is_none_or(|l| matches!(l, "python" | "rust" | "shell"))
}

/// The next commands: an extension folder (it has an `extension.toml`) is linted and run by
/// `iohr ext`, by whether `iohr ext dev` runs this language; a plain project (`blank`) is not
/// an extension, so it is read and run as its README says.
fn next_lines(extension: bool, dev: bool) -> Vec<(&'static str, &'static str)> {
    if !extension {
        return vec![(
            "README.md",
            "how to build and test it, with the language's own tools",
        )];
    }
    let mut v = vec![("iohr ext lint --preview", "read what it may do")];
    if dev {
        v.push(("iohr ext dev", "run it with the real token channel"));
    } else {
        v.push((
            "README.md",
            "how to run it; `iohr ext dev` runs only Rust, Python and shell folders",
        ));
    }
    v
}

/// Whether the project made is an extension: it has a manifest.
fn is_extension(report: &project::Report) -> bool {
    report.files.iter().any(|f| f == "extension.toml")
}

/// The template's questions at a terminal, in order, each with its earlier answer (`kept`) as
/// the default. A question after the first can go back to the one before; the first goes back
/// out of the step when `back` says there is a step before it. Returns the answers and whether
/// anything was asked.
fn ask_questions(
    ui: Ui,
    tpl: &Template,
    given: &Answers,
    kept: &Answers,
    back: bool,
) -> Result<Nav<(Answers, bool)>, Error> {
    use std::cell::{Cell, RefCell};
    let mut defaults = kept.clone();
    // The answers fixed before the question being asked again, in the order they were asked.
    let mut keep: Vec<(String, Answer)> = Vec::new();
    loop {
        let mut pre = given.clone();
        pre.extend(keep.iter().cloned());
        let base = keep.len();
        let asked: RefCell<Vec<(String, Answer)>> = RefCell::new(Vec::new());
        let went_back = Cell::new(false);
        let done = {
            let mut put = |q: &Question| {
                let at = base + asked.borrow().len();
                match ask_question(ui, tpl, q, defaults.get(&q.key), back || at > 0)? {
                    Nav::Next(a) => {
                        asked.borrow_mut().push((q.key.clone(), a.clone()));
                        Ok(a)
                    }
                    Nav::Back => {
                        went_back.set(true);
                        Err(cancel())
                    }
                }
            };
            template::answer(tpl, &pre, Some(&mut put))
        };
        let asked = asked.into_inner();
        match done {
            Ok(answers) => return Ok(Nav::Next((answers, base + asked.len() > 0))),
            Err(_) if went_back.get() => {
                // The question that said "back" was never answered: it is index `at`.
                let at = base + asked.len();
                keep.extend(asked);
                defaults.extend(keep.iter().cloned());
                let Some(n) = wizard::back_in_questions(at) else {
                    return Ok(Nav::Back);
                };
                keep.truncate(n);
            }
            Err(e) => return Err(e),
        }
    }
}

/// One question at the terminal, drawn from the schema: the kind picks the control. `prev` is
/// the answer given before, the default when the question is asked again.
fn ask_question(
    ui: Ui,
    tpl: &Template,
    q: &Question,
    prev: Option<&Answer>,
    back: bool,
) -> Result<Nav<Answer>, Error> {
    let default = prev.cloned().or_else(|| q.default.clone());
    if q.key == LANGUAGE && !tpl.meta.variants.is_empty() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let (choices, mut initial) = language_choices(
            tpl,
            |n| ask::find_on_path(n, &path).is_some(),
            |v| {
                let lang = Lang::of_toolchain(&v.toolchain)?;
                toolchain::state(lang).map(|s| s.hint(lang))
            },
        );
        if let Some(p) = prev {
            initial = p.text();
        }
        return Ok(ui
            .select_step(&q.prompt, &choices, &initial, back)?
            .map(Answer::Text));
    }
    if let Some(h) = &q.help {
        ui.info(h);
    }
    match q.kind {
        Kind::Bool => Ok(ui
            .confirm_step(&q.prompt, matches!(default, Some(Answer::Flag(true))), back)?
            .map(Answer::Flag)),
        Kind::Choice => {
            let choices: Vec<Choice> = q
                .choices
                .iter()
                .map(|c| Choice {
                    value: c.clone(),
                    label: c.clone(),
                    hint: String::new(),
                    available: true,
                    key: None,
                })
                .collect();
            let initial = default
                .map(|d| d.text())
                .or_else(|| q.choices.first().cloned())
                .unwrap_or_default();
            Ok(ui
                .select_step(&q.prompt, &choices, &initial, back)?
                .map(Answer::Text))
        }
        Kind::Multi => {
            let initial = match default {
                Some(Answer::List(l)) => l,
                _ => vec![],
            };
            Ok(ui
                .multiselect_step(&q.prompt, &q.choices, &initial, back)?
                .map(Answer::List))
        }
        Kind::String => {
            let check = q.clone();
            Ok(ui
                .input_step(
                    &q.prompt,
                    &default.map(|d| d.text()).unwrap_or_default(),
                    move |v| template::coerce(&check, &Answer::Text(v.to_owned())).map(|_| ()),
                    back,
                )?
                .map(Answer::Text))
        }
    }
}

/// The iohr commands an `init` agent may run, by the name the binary runs under: new documents,
/// the dry-run listing of what `sync` would send, and memory. Each is a prefix, so `sync` itself
/// (which writes to Decisions) is not among them: the person runs that.
fn init_allow(bin: &str) -> Vec<String> {
    [
        "atlas new",
        "atlas sync --dry-run",
        "memory remember",
        "memory recall",
        "memory list",
        "memory check",
    ]
    .iter()
    .map(|c| format!("{bin} {c}"))
    .chain(["git status", "git log", "git diff", "ls"].map(str::to_owned))
    .collect()
}

/// The agent and the prompt it would get, when one is to run.
struct AgentRun {
    agent: Agent,
    bin: PathBuf,
    prompt: String,
    /// The arguments of the print-mode run ([`ask::argv`]), the prompt last.
    args: Vec<String>,
    model: Option<String>,
    max_turns: u32,
    timeout: Duration,
    thinking: Thinking,
}

/// Starts the latest-stable lookups: every language at a terminal (the menu shows each one's
/// state), else only the language `--lang` names (the lookup of any other starts if it is
/// asked for). Not under `--json`.
fn start_toolchain_check(a: &InitArgs, interactive: bool, out: Out) {
    if out.json {
        return;
    }
    let langs: Vec<Lang> = if interactive {
        Lang::ALL.to_vec()
    } else {
        a.lang
            .as_deref()
            .and_then(lang_of_variant)
            .into_iter()
            .collect()
    };
    toolchain::start(&langs);
}

/// The language of a built-in variant id (`typescript` is Node).
fn lang_of_variant(id: &str) -> Option<Lang> {
    match id {
        "rust" => Some(Lang::Rust),
        "go" => Some(Lang::Go),
        "typescript" => Some(Lang::Node),
        "python" => Some(Lang::Python),
        _ => None,
    }
}

/// What the toolchain check did, for the report.
struct ToolchainCheck {
    step: project::Step,
    /// An upgrade ran and succeeded: read the version again.
    upgraded: bool,
}

/// Puts the check's result in the plan (a dry run says what it would do), and, when the
/// toolchain is behind, offers the upgrade: at a terminal a yes/no question that defaults to
/// no, and the command runs only on a yes; without a terminal, with `--yes` or in a dry run
/// the command is printed and never run. Nothing here fails `init`.
///
/// # Errors
/// The person cancelled the question.
fn check_toolchain(
    flow: &mut Flow,
    interactive: bool,
    lang: Lang,
    state: &State,
) -> Result<ToolchainCheck, Error> {
    let ui = flow.ui;
    let said = state.said(lang);
    let step = |st: &str, text: String| project::step("toolchain", "Toolchain", st, text, "");
    let State::Behind { latest, .. } = state else {
        flow.gate("toolchain", &format!("report the toolchain check: {said}"));
        let (st, text) = if matches!(state, State::Unchecked { .. }) {
            ("skipped", said)
        } else {
            ("done", said)
        };
        return Ok(ToolchainCheck {
            step: step(st, text),
            upgraded: false,
        });
    };
    let up = toolchain::upgrade_here(lang, latest);
    let how = if up.run.is_some() {
        "asked first, never run by default"
    } else {
        "for you to run"
    };
    if !flow.gate(
        "toolchain",
        &format!(
            "report the toolchain check: {said}; upgrade with `{}` ({how})",
            up.shown
        ),
    ) {
        return Ok(ToolchainCheck {
            step: step("ask", format!("{said}; upgrade with `{}`", up.shown)),
            upgraded: false,
        });
    }
    let behind = format!("{said}; upgrade with `{}`", up.shown);
    if up.run.is_none() || !interactive {
        ui.info(&format!("{behind} (iohr did not run it)"));
        return Ok(ToolchainCheck {
            step: step("ask", behind),
            upgraded: false,
        });
    }
    ui.info(&said);
    let yes = ui.confirm(
        &format!("Upgrade {} now with `{}`?", lang.label(), up.shown),
        false,
    )?;
    if !yes {
        return Ok(ToolchainCheck {
            step: step("ask", behind),
            upgraded: false,
        });
    }
    match toolchain::run_upgrade(&up) {
        Ok(()) => Ok(ToolchainCheck {
            step: step(
                "done",
                format!("upgraded {} with `{}`", lang.label(), up.shown),
            ),
            upgraded: true,
        }),
        Err(why) => {
            ui.warn(&format!("the upgrade did not finish: {why}"));
            Ok(ToolchainCheck {
                step: step("skipped", format!("{behind}; it did not finish: {why}")),
                upgraded: false,
            })
        }
    }
}

/// The most turns the agent gets in `init` when neither a flag nor `[project] agent_turns` says:
/// init's own default, because `[ask]` in config.toml has no bound. Enough to fetch a site and
/// write a knowledge base; a wrong request still cannot run away.
const AGENT_TURNS: u32 = 150;
/// The most seconds the agent gets in `init` when neither a flag nor `[project] agent_timeout`
/// says (init's own default).
const AGENT_SECONDS: u64 = 1800;
/// The Claude model an `init` agent run uses when neither the flag (`--agent-model`,
/// `IOHR_INIT_AGENT_MODEL`) nor `[project] agent_model` names one: a build of many files needs
/// more than the small fast tier `iohr ask` answers with.
const AGENT_MODEL: &str = "sonnet";

#[allow(
    clippy::too_many_lines,
    reason = "one flow, top to bottom, every action through the gate"
)]
#[allow(
    clippy::many_single_char_names,
    reason = "g, a, out are the repo's names for globals, args and output"
)]
async fn init(g: &Global, env: &Env, a: InitArgs, out: Out) -> Result<(), Error> {
    let cfg = Ctx::load(g)?.config;
    let configured = cfg.project.template;
    let ui = Ui::detect(out.json);
    let interactive = ui.fancy() && !a.yes;
    let dry = a.dry_run;
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let agent = ask::detect_agent(cfg.ask.agent.as_deref(), &path_env).map_err(Error::Usage)?;
    if a.describe.as_deref().is_some_and(|d| d.trim().is_empty()) {
        return Err(Error::Usage(
            "--describe needs words: what do you want built?".into(),
        ));
    }
    if a.agent_turns.or(cfg.project.agent_turns) == Some(0)
        || a.agent_timeout.or(cfg.project.agent_timeout) == Some(0)
    {
        return Err(Error::Usage(
            "--agent-turns and --agent-timeout need a number above 0".into(),
        ));
    }
    if a.describe.is_some() && agent.is_none() {
        return Err(Error::Failed(no_agent_help()));
    }
    let root = template::user_root()?;
    // The latest stable of each installed language is looked up on threads of its own while
    // the person answers; never under --json, and never a reason for init to fail.
    start_toolchain_check(&a, interactive, out);
    if ui.fancy() {
        ui.intro(if dry {
            "iohr project init (dry run)"
        } else {
            "iohr project init"
        });
    }

    let here = Where::here(cfg.project.root.clone());
    let cx = Exec {
        g,
        env,
        a: &a,
        ui,
        interactive,
        agent: &agent,
        agent_model: a.agent_model.clone().or(cfg.project.agent_model.clone()),
        agent_turns: a
            .agent_turns
            .or(cfg.project.agent_turns)
            .unwrap_or(AGENT_TURNS),
        agent_timeout: a
            .agent_timeout
            .or(cfg.project.agent_timeout)
            .unwrap_or(AGENT_SECONDS),
        ask_thinking: cfg.ask.thinking.clone(),
        out,
    };
    let mut wiz = Wizard {
        ui,
        a: &a,
        interactive,
        agent: &agent,
        root,
        configured,
        here,
        draft: Draft::from_flags(a.name.clone(), None, a.describe.clone()),
        tpl: None,
        given: None,
        at: Walk::new(),
    };
    loop {
        wiz.run_to_the_end()?;
        // Without a terminal, with `--yes` and in a dry run there is no review: the dry run is
        // the plan, and the flags are the answers.
        if !interactive || dry {
            return execute(&cx, &wiz.resolved()?, false).await;
        }
        let r = wiz.resolved()?;
        let planned = execute(&cx, &r, true).await;
        wiz.draft.agent_ok = r.agent_ok.get();
        match review(ui, &wiz, planned.err())? {
            Review::Create => return execute(&cx, &wiz.resolved()?, false).await,
            Review::Change(stage) => wiz.at.jump(stage),
        }
    }
}

/// The wizard of `init` at a terminal: the steps of [`Stage`], each asked with its earlier
/// answer as the default, so going back (or changing a step from the review) loses nothing.
/// Without a terminal each step takes its answer from the flags, as `init` always did.
struct Wizard<'a> {
    ui: Ui,
    a: &'a InitArgs,
    interactive: bool,
    agent: &'a Option<(Agent, PathBuf)>,
    root: PathBuf,
    configured: Option<String>,
    here: Where,
    draft: Draft,
    /// The template the pick (or the flag) names, once loaded.
    tpl: Option<Template>,
    /// What `--set`, `--answers` and `--lang` already answered, once read.
    given: Option<Answers>,
    at: Walk,
}

impl Wizard<'_> {
    /// Whether a step puts a question to the person: the walk goes back only to those.
    fn asks(&self, stage: Stage) -> bool {
        self.interactive
            && match stage {
                Stage::Name => self.a.name.is_none(),
                Stage::Template => self.a.template.is_none(),
                Stage::Describe => self.draft.pick.as_deref() == Some(DESCRIBE),
                Stage::Questions => self.tpl.as_ref().is_some_and(|t| {
                    self.given
                        .as_ref()
                        .is_none_or(|g| t.all_questions().iter().any(|q| !g.contains_key(&q.key)))
                }),
                Stage::Folder => self.a.dir.is_none(),
            }
    }

    /// Runs the steps from where the walk stands to the end.
    fn run_to_the_end(&mut self) -> Result<(), Error> {
        while let Some(stage) = self.at.at() {
            let back = self.at.can_back(&|s| self.asks(s));
            let went = match stage {
                Stage::Name => self.name_step(back)?,
                Stage::Template => self.template_step(back)?,
                Stage::Describe => self.describe_step(back)?,
                Stage::Questions => self.questions_step(back)?,
                Stage::Folder => self.folder_step(back)?,
            };
            let mut walk = self.at;
            walk.apply(went, &|s| self.asks(s));
            self.at = walk;
        }
        Ok(())
    }

    /// The settled answers, once every step is done.
    fn resolved(&self) -> Result<Resolved, Error> {
        let missing = |what: &str| Error::Failed(format!("the {what} step has no answer"));
        Ok(Resolved {
            name: self.draft.name.clone().ok_or_else(|| missing("name"))?,
            tpl: self.tpl.clone().ok_or_else(|| missing("template"))?,
            answers: self.draft.answers.clone(),
            describe: self.draft.describe.clone(),
            place: self.draft.folder.clone().ok_or_else(|| missing("folder"))?,
            here: self.here.clone(),
            agent_ok: std::cell::Cell::new(self.draft.agent_ok),
        })
    }

    fn name_step(&mut self, back: bool) -> Result<Went, Error> {
        const PROMPT: &str = "What is the project called?";
        if let Some(n) = &self.a.name {
            check_name(n).map_err(Error::Usage)?;
            self.draft.set_name(n.clone());
            return Ok(Went::Skip);
        }
        if !self.interactive {
            return Err(Error::Usage(
                "a project name is needed: iohr project init NAME (there is no terminal to ask on)"
                    .into(),
            ));
        }
        let ui = self.ui;
        let before = self.draft.name.clone().unwrap_or_default();
        let cwd = std::env::current_dir().unwrap_or_default();
        let props = folder::name_proposals(&cwd);
        let typed = if props.is_empty() {
            ui.input_step(PROMPT, &before, check_name, back)?
        } else {
            const OTHER: &str = "@other";
            let mut choices: Vec<Choice> = props
                .iter()
                .map(|(n, why)| Choice {
                    value: n.clone(),
                    label: n.clone(),
                    hint: why.clone(),
                    available: true,
                    key: None,
                })
                .collect();
            choices.push(Choice {
                value: OTHER.into(),
                label: "Another name...".into(),
                hint: "type it".into(),
                available: true,
                key: None,
            });
            let initial = if before.is_empty() {
                props[0].0.clone()
            } else if props.iter().any(|(n, _)| *n == before) {
                before.clone()
            } else {
                OTHER.to_owned()
            };
            loop {
                match ui.select_step(PROMPT, &choices, &initial, back)? {
                    Nav::Back => break Nav::Back,
                    Nav::Next(p) if p != OTHER => break Nav::Next(p),
                    // The menu is the step before the typed name: back from it returns there.
                    Nav::Next(_) => match ui.input_step(PROMPT, &before, check_name, true)? {
                        Nav::Back => {}
                        n @ Nav::Next(_) => break n,
                    },
                }
            }
        };
        match typed {
            Nav::Back => Ok(Went::Back),
            Nav::Next(name) => {
                check_name(&name).map_err(Error::Usage)?;
                self.draft.set_name(name);
                Ok(Went::Next)
            }
        }
    }

    /// The text of the template spec: the flag, else the menu's pick, else a described
    /// project's `blank`; `None` leaves it to `[project] template`.
    fn spec_text(&self) -> Option<String> {
        if let Some(t) = &self.a.template {
            return Some(t.clone());
        }
        match self.draft.pick.as_deref() {
            Some(DESCRIBE) => Some(DESCRIBE_TEMPLATE.to_owned()),
            Some(p) => Some(p.to_owned()),
            None => self
                .draft
                .describe
                .as_ref()
                .map(|_| DESCRIBE_TEMPLATE.to_owned()),
        }
    }

    fn template_step(&mut self, back: bool) -> Result<Went, Error> {
        let mut went = Went::Skip;
        if self.a.template.is_none() && self.interactive {
            let default = choose_template(None, self.configured.as_deref());
            let rows = template::list(&self.root, &default);
            let mut choices = vec![Choice {
                value: DESCRIBE.into(),
                label: "Describe it, your agent builds it".into(),
                hint: match self.agent {
                    Some((ag, _)) => {
                        format!("starts {} in the new folder with your words", ag.name())
                    }
                    None => no_agent_help(),
                },
                available: true,
                key: None,
            }];
            choices.extend(rows.iter().map(|r| Choice {
                value: r.name.clone(),
                label: r.name.clone(),
                hint: format!(
                    "{} [{}]",
                    r.description.replace("; coming after the demo", ""),
                    r.origin
                ),
                available: r.status == "live",
                key: None,
            }));
            let initial = self.draft.pick.clone().unwrap_or(default);
            match self
                .ui
                .select_step("Which template?", &choices, &initial, back)?
            {
                Nav::Back => return Ok(Went::Back),
                Nav::Next(picked) => {
                    let described = picked == DESCRIBE;
                    if described && self.agent.is_none() {
                        return Err(Error::Failed(no_agent_help()));
                    }
                    self.draft.set_pick(picked, described);
                    went = Went::Next;
                }
            }
        }
        self.load_template()?;
        Ok(went)
    }

    /// Loads the template the answers so far name. A template given by flag is loaded once
    /// (a git one is cloned once).
    fn load_template(&mut self) -> Result<(), Error> {
        if self.a.template.is_some() && self.tpl.is_some() {
            return Ok(());
        }
        let spec = template::parse_spec(&choose_template(
            self.spec_text().as_deref(),
            self.configured.as_deref(),
        ));
        if self.a.dry_run && matches!(spec, Spec::Git { .. }) {
            return Err(Error::Usage(
                "a dry run makes no network call, so it cannot clone that template: clone it yourself and pass its folder with --template PATH".into(),
            ));
        }
        let tpl: Template = template::load(&spec, &self.root).map_err(Error::Usage)?;
        if tpl.meta.status == template::Status::Coming {
            let rows = template::list(&self.root, "");
            return Err(Error::Usage(format!(
                "the `{}` template is coming; live today: {}",
                tpl.meta.id,
                live_names(&rows)
            )));
        }
        self.tpl = Some(tpl);
        self.given = None;
        Ok(())
    }

    fn describe_step(&mut self, back: bool) -> Result<Went, Error> {
        if !self.asks(Stage::Describe) {
            return Ok(Went::Skip);
        }
        let before = self.draft.describe.clone().unwrap_or_default();
        Ok(
            match self
                .ui
                .input_step("Describe the project", &before, nonempty, back)?
            {
                Nav::Back => Went::Back,
                Nav::Next(words) => {
                    self.draft.describe = Some(words);
                    Went::Next
                }
            },
        )
    }

    /// `--answers`, then `--set`, then `--lang`: what the flags answered.
    fn flag_answers(&self, tpl: &Template) -> Result<Answers, Error> {
        let a = self.a;
        let mut given = match &a.answers {
            Some(f) => template::read_answers(f).map_err(Error::Usage)?,
            None => Answers::new(),
        };
        given.extend(template::parse_set(&a.set).map_err(Error::Usage)?);
        if let Some(l) = a.lang.as_deref().map(str::trim) {
            if tpl.meta.variants.is_empty() {
                return Err(Error::Usage(format!(
                    "the `{}` template has one set of files, so --lang does not apply",
                    tpl.meta.id
                )));
            }
            if let Some(set) = given.get(LANGUAGE).map(Answer::text)
                && set != l
            {
                return Err(Error::Usage(format!(
                    "--lang {l} and --set language={set} disagree: --lang is the same as --set language=ID"
                )));
            }
            given.insert(LANGUAGE.into(), Answer::Text(l.to_owned()));
        }
        Ok(given)
    }

    fn questions_step(&mut self, back: bool) -> Result<Went, Error> {
        let tpl = self
            .tpl
            .clone()
            .ok_or_else(|| Error::Failed("the template step has no answer".into()))?;
        let given = self.flag_answers(&tpl)?;
        self.given = Some(given.clone());
        if !self.interactive {
            self.draft.answers = template::answer(&tpl, &given, None)?;
            return Ok(Went::Skip);
        }
        match ask_questions(self.ui, &tpl, &given, &self.draft.answers, back)? {
            Nav::Back => Ok(Went::Back),
            Nav::Next((answers, asked)) => {
                self.draft.answers = answers;
                Ok(if asked { Went::Next } else { Went::Skip })
            }
        }
    }

    /// Where it goes: `--dir`, else a menu at a terminal, else `./NAME`.
    fn folder_step(&mut self, back: bool) -> Result<Went, Error> {
        let name = self
            .draft
            .name
            .clone()
            .ok_or_else(|| Error::Failed("the name step has no answer".into()))?;
        if let Some(d) = &self.a.dir {
            self.draft.folder = Some(folder::given(&self.here, d, "you passed --dir"));
            return Ok(Went::Skip);
        }
        let list = folder::proposals(&self.here, &name);
        if !self.interactive {
            self.draft.folder = Some(folder::headless(&list));
            return Ok(Went::Skip);
        }
        Ok(
            match pick_folder(self.ui, &self.here, &list, self.draft.folder.as_ref(), back)? {
                Nav::Back => Went::Back,
                Nav::Next(p) => {
                    self.draft.folder = Some(p);
                    Went::Next
                }
            },
        )
    }

    /// What a step holds now, for the "Change a step" menu.
    fn answer_text(&self, stage: Stage) -> String {
        let short = |s: &str| {
            let mut t: String = s.chars().take(48).collect();
            if s.chars().count() > 48 {
                t.push_str("...");
            }
            t
        };
        match stage {
            Stage::Name => self.draft.name.clone().unwrap_or_default(),
            Stage::Template => match self.draft.pick.as_deref() {
                Some(DESCRIBE) => "describe it, your agent builds it".to_owned(),
                _ => self
                    .tpl
                    .as_ref()
                    .map(|t| t.meta.id.clone())
                    .unwrap_or_default(),
            },
            Stage::Describe => short(self.draft.describe.as_deref().unwrap_or_default()),
            Stage::Questions => short(
                &self
                    .draft
                    .answers
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.text()))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Stage::Folder => self
                .draft
                .folder
                .as_ref()
                .map(|f| f.shown.clone())
                .unwrap_or_default(),
        }
    }
}

/// What the person decided on the review page.
enum Review {
    Create,
    Change(Stage),
}

/// The review page, under the plan: create it, change a step, or cancel. `problem` is why the
/// plan could not be made (the folder is taken, say): then there is nothing to create yet.
fn review(ui: Ui, wiz: &Wizard<'_>, problem: Option<Error>) -> Result<Review, Error> {
    let ready = problem.is_none();
    if let Some(e) = problem {
        ui.warn(&e.to_string());
    }
    let row = |value: &str, label: &str, hint: &str| Choice {
        value: value.into(),
        label: label.into(),
        hint: hint.into(),
        available: true,
        key: None,
    };
    loop {
        let mut choices = Vec::new();
        if ready {
            choices.push(row(
                "create",
                "Create it",
                "write the files and run what the plan says",
            ));
        }
        choices.push(row(
            "change",
            "Change a step",
            "go to one and come on from there",
        ));
        choices.push(row("cancel", "Cancel", "nothing is written"));
        let initial = if ready { "create" } else { "change" };
        match ui.select("Ready?", &choices, initial)?.as_str() {
            "create" => return Ok(Review::Create),
            "change" => {
                let steps: Vec<Choice> = Stage::ALL
                    .into_iter()
                    .filter(|s| wiz.asks(*s))
                    .map(|s| row(s.key(), s.label(), &wiz.answer_text(s)))
                    .collect();
                let first = steps.first().map(|c| c.value.clone()).unwrap_or_default();
                if let Nav::Next(key) = ui.select_step("Which step?", &steps, &first, true)?
                    && let Some(stage) = Stage::from_key(&key)
                {
                    return Ok(Review::Change(stage));
                }
            }
            _ => return Err(cancel()),
        }
    }
}

/// Everything `init` needs besides the answers.
struct Exec<'a> {
    g: &'a Global,
    env: &'a Env,
    a: &'a InitArgs,
    ui: Ui,
    interactive: bool,
    agent: &'a Option<(Agent, PathBuf)>,
    /// `--agent-model`, `IOHR_INIT_AGENT_MODEL`, else `[project] agent_model`.
    agent_model: Option<String>,
    /// The turn cap and the time cap: the flag, else `[project]`, else init's own default.
    agent_turns: u32,
    agent_timeout: u64,
    ask_thinking: Option<String>,
    out: Out,
}

/// What the person answered, settled: the input of the plan.
struct Resolved {
    name: String,
    tpl: Template,
    answers: Answers,
    describe: Option<String>,
    place: Proposal,
    here: Where,
    /// Whether the agent step was confirmed (asked once, however often the plan is built).
    agent_ok: std::cell::Cell<Option<bool>>,
}

/// Plans the project from the answers and, unless this is a dry run, makes it. With `preview`
/// it is the dry run the review page shows: the same plan, nothing changed, no ending.
#[allow(
    clippy::too_many_lines,
    reason = "one flow, top to bottom, every action through the gate"
)]
#[allow(
    clippy::many_single_char_names,
    reason = "g, a, out are the repo's names for globals, args and output"
)]
async fn execute(cx: &Exec<'_>, r: &Resolved, preview: bool) -> Result<(), Error> {
    let (g, env, a, ui, interactive, agent, out) =
        (cx.g, cx.env, cx.a, cx.ui, cx.interactive, cx.agent, cx.out);
    let dry = a.dry_run || preview;
    // The plan: everything below is derived from the answers, and changes nothing but through `flow`.
    let publisher = publisher_of(a.publisher.as_deref(), None);
    let Resolved {
        name,
        tpl,
        answers,
        describe,
        place,
        here,
        ..
    } = r;
    let dir = place.dir.clone();
    let mut warnings: Vec<String> = Vec::new();
    if let Some(w) = folder::nest_warning(here, &dir) {
        ui.warn(&w);
        warnings.push(w);
    }
    let created_at = now();
    let lang = answers.get(LANGUAGE).map(Answer::text);
    let tpl_lang = match &lang {
        Some(l) => tpl.with_variant(l).map_err(Error::Usage)?,
        None => tpl.clone(),
    };
    let mut vals = template::all_values(tpl, name, &publisher, &created_at, answers);
    if !vals.iter().any(|(k, _)| k == "@@DESCRIPTION@@") {
        vals.push((
            "@@DESCRIPTION@@".to_owned(),
            project::one_line_description(describe.as_deref()),
        ));
    }
    // The installed toolchain, read once: its version fills the files that name one
    // (`rust-version`) and is told to the agent. A toolchain that will not run keeps the
    // variant's own default and is not told.
    let variant = lang.as_deref().and_then(|l| tpl.variant(l));
    let docs_only = variant.is_some_and(template::VariantDef::is_docs_only);
    // What this binary is called: the agent runs it by that name, AGENTS.md names it.
    let me = super::shell::running_bin();
    // Everything that changes anything goes through `flow`.
    let mut flow = Flow {
        dry,
        ui,
        would: Vec::new(),
    };
    let mut toolchain = variant.and_then(|v| project::toolchain_line(&v.toolchain));
    // Is it behind the latest stable? (Off under --json, and when the lookup found nothing.)
    let tool_lang = variant.and_then(|v| Lang::of_toolchain(&v.toolchain));
    let mut tool_step = None;
    if let Some(l) = tool_lang
        && let Some(state) = toolchain::state(l)
    {
        // A lookup that ran out of time under load has not read the installed version; this
        // function has (the line above), so the report still names what is installed.
        let state = match state {
            State::Unchecked { installed: None } => State::Unchecked {
                installed: toolchain.as_deref().and_then(project::installed_version),
            },
            other => other,
        };
        let done = check_toolchain(&mut flow, interactive, l, &state)?;
        if done.upgraded {
            // The files and the agent are told the version that is there now.
            toolchain = variant.and_then(|v| project::toolchain_line(&v.toolchain));
        }
        tool_step = Some(done.step);
    }
    if let Some((full, minor)) = toolchain.as_deref().and_then(template::version_in) {
        for (k, v) in [
            ("@@TOOLCHAIN_VERSION@@", full),
            ("@@TOOLCHAIN_MINOR@@", minor),
        ] {
            vals.retain(|(x, _)| x != k);
            vals.push((k.to_owned(), v));
        }
    }
    let mut files = template::render(&tpl_lang, &vals).map_err(Error::Failed)?;
    // Every project gets its InOrbit space link, whatever the template (RFC 0103).
    project::add_space_link(&mut files, None);
    // And its atlas: starter PRD, RFC, ADR and study, a canvas and a 3D folder (RFC 0205).
    atlas::add_atlas(
        &mut files,
        &atlas::Meta {
            name,
            template: &tpl.meta.id,
            lang: lang.as_deref(),
            describe: describe.as_deref(),
            created_at: &created_at,
            bin: &me,
        },
    );
    let programs = template::programs(tpl, &vals);
    let runner = lang.as_deref().and_then(|l| tpl.runner(l, &vals));
    // Reads only, so a dry run finds the same problems a real one would.
    project::check_target(&dir, &files).map_err(Error::Failed)?;
    project::check_manifest(&files).map_err(Error::Failed)?;
    if !a.no_smoke
        && let Some(v) = lang.as_deref().and_then(|l| tpl.variant(l))
    {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let missing: Vec<&String> = v
            .needs
            .iter()
            .filter(|n| ask::find_on_path(n, &path).is_none())
            .collect();
        if !missing.is_empty() {
            let w = format!(
                "{} is not on your PATH, so the smoke test cannot run it: install it, pick another language with --lang, or pass --no-smoke",
                missing
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            ui.warn(&w);
            warnings.push(w);
        }
    }

    // The steps, and the agent that will run, if one will.
    let mut steps = tpl.steps();
    // A documents-only language has no program: no smoke test, the plan says nothing about one.
    if a.no_smoke || docs_only {
        steps.retain(|s| s.name != StepName::Smoke);
    }
    let step_prompt = steps
        .iter()
        .find(|s| s.name == StepName::Agent)
        .map(|s| s.prompt.clone());
    let has_agent_step = step_prompt.is_some();
    let prompt_text = step_prompt
        .flatten()
        .or_else(|| tpl.meta.agent_prompt.clone());
    let mut agent_run = None;
    if let Some((ag, bin)) = &agent {
        let wanted = if describe.is_some() {
            true
        } else if has_agent_step && interactive {
            if let Some(v) = r.agent_ok.get() {
                v
            } else {
                let v = ui.confirm(
                    &format!(
                        "Start {} in the new folder with the template's prompt?",
                        ag.name()
                    ),
                    true,
                )?;
                r.agent_ok.set(Some(v));
                v
            }
        } else {
            false
        };
        if wanted {
            let prompt = project::agent_prompt(
                prompt_text.as_deref(),
                describe.as_deref(),
                name,
                &tpl.meta.id,
                answers,
                &vals,
                project::Facts {
                    toolchain: toolchain.as_deref(),
                    today: &crate::front::today(),
                    bin: &me,
                    docs_only,
                },
            );
            // The language's own build and test commands, scoped, and iohr's own document and
            // memory commands by the name this binary runs under; nothing else of the shell.
            let mut allow: Vec<String> = variant.map(|v| v.agent_allow.clone()).unwrap_or_default();
            allow.extend(init_allow(&me));
            // Init's own model (sonnet unless named), edits to files, the commands above and
            // init's own bounds: nothing the agent is told or says is stored here.
            let model = ask::choose_model(
                *ag,
                cx.agent_model.as_deref(),
                (*ag == Agent::Claude).then_some(AGENT_MODEL),
            )
            .map_err(Error::Usage)?;
            let max_turns = cx.agent_turns;
            agent_run = Some(AgentRun {
                agent: *ag,
                bin: bin.clone(),
                args: ask::argv(
                    *ag,
                    model.as_deref(),
                    (true, false),
                    &allow,
                    Some(max_turns),
                    &prompt,
                ),
                prompt,
                model,
                max_turns,
                timeout: Duration::from_secs(cx.agent_timeout),
                thinking: ask::choose_thinking(None, cx.ask_thinking.as_deref())
                    .map_err(Error::Usage)?,
            });
        }
    } else if has_agent_step {
        ui.info(&format!(
            "The template has an agent step, skipped: {}",
            no_agent_help()
        ));
    }
    if agent_run.is_some() && !has_agent_step {
        steps.push(template::StepDef {
            name: StepName::Agent,
            prompt: None,
        });
    }
    // The agent builds first and the smoke test runs what it built, so the report is true for it.
    if let (Some(at), Some(st)) = (
        steps.iter().position(|s| s.name == StepName::Agent),
        steps.iter().position(|s| s.name == StepName::Smoke),
    ) && at > st
    {
        let agent_step = steps.remove(at);
        steps.insert(st, agent_step);
    }

    let state_root = state_root()?;
    let state = state_root.join(name);
    let signed_in = flow
        .act_async("signin", "check your sign-in (one call to /v1/me)", || {
            check_sign_in(g, env, ui)
        })
        .await
        .flatten();
    let mut report = Report {
        name: name.clone(),
        id: format!("{publisher}/{name}"),
        dir: dir.clone(),
        state: state.clone(),
        files: files.iter().map(|(p, _)| p.clone()).collect(),
        steps: Vec::new(),
    };
    report.steps.extend(tool_step);
    let setup = format!(
        "write {} files into {} ({})",
        files.len(),
        place.shown,
        place.reason
    );
    let sp = (!dry).then(|| ui.step(&format!("Setting up {name} from `{}`", tpl.meta.id)));
    match flow.act("scaffold", &setup, || {
        project::scaffold(&dir, &files, &programs, &tpl.meta.id)
    }) {
        Some(Ok((written, st))) => {
            if let Some(sp) = sp {
                sp.ok(&format!(
                    "Scaffolded {} files in {}",
                    written.len(),
                    place.shown
                ));
            }
            report.files = written;
            report.steps.push(st);
        }
        Some(Err(e)) => {
            if let Some(sp) = sp {
                sp.fail(&e);
            }
            return Err(Error::Failed(e));
        }
        None => {}
    }
    // The built-in roles (security and the language's engineer): imported when signed in.
    let roles_sp = (!dry && ui.fancy()).then(|| ui.step("Importing the built-in roles"));
    let roles = flow
        .act_async(
            "roles",
            "import the built-in roles into your InOrbit account (they stay in the project when you are signed out)",
            || super::atlas::import_for(g, env, &dir, false, false),
        )
        .await;
    match (roles.as_ref().filter(|_| !out.json), roles_sp) {
        // Drawn: one line that says how it went, the role names in it.
        (Some(r), Some(sp)) if !r.outcomes.is_empty() || r.message.is_some() => {
            let (mark, said) = super::atlas::roles_result(r);
            sp.end(mark, &said);
        }
        (Some(r), None) => super::atlas::show_report(ui, r),
        (_, Some(sp)) => sp.clear(),
        (None, None) => {}
    }
    let mut smoke: Option<Smoke> = None;
    let mut agent_failed: Option<String> = None;
    let mut agent_built = false;
    let mut stop_note: Option<String> = None;
    for s in &steps {
        match s.name {
            StepName::Git => {
                let sp = (!dry).then(|| ui.step("Making a git repository"));
                let r = flow.act("git", "make a git repository with one commit", || {
                    project::git_step(&dir, name, &tpl.meta.id)
                });
                if let Some(st) = r {
                    if let Some(sp) = sp {
                        let (mark, said) = step_line(&st);
                        sp.end(mark, &said);
                    }
                    report.steps.push(st);
                }
            }
            StepName::Register => {
                let what = format!(
                    "register the project: state and cache under {}",
                    state.display()
                );
                let sp = (!dry).then(|| ui.step("Registering the project"));
                let r = flow.act("register", &what, || {
                    project::register_step(&dir, &state_root, name, &tpl.meta.id, &created_at)
                });
                match r {
                    Some(Ok((_, st))) => {
                        if let Some(sp) = sp {
                            let (mark, said) = step_line(&st);
                            sp.end(mark, &said);
                        }
                        report.steps.push(st);
                        report.steps.extend(project::cloud_steps(name));
                    }
                    Some(Err(e)) => {
                        if let Some(sp) = sp {
                            sp.fail(&format!("Could not register the project: {e}"));
                        }
                        return Err(Error::Failed(e));
                    }
                    None => {}
                }
            }
            StepName::Smoke => {
                let (d, st, n, rn) = (dir.clone(), state.clone(), name.clone(), runner.clone());
                let with = runner.as_ref().map_or("python3", |r| r.label.as_str());
                let run = (!dry).then(|| ui.step("Running the project once (smoke test)"));
                let r = flow
                    .act_async(
                        "smoke",
                        &format!("run the project once with {with} and read its heartbeat back (smoke test)"),
                        || async move {
                            tokio::task::spawn_blocking(move || {
                                project::smoke(&d, &st, &n, rn.as_ref())
                            })
                            .await
                        },
                    )
                    .await;
                if let Some(r) = r {
                    let result =
                        r.map_err(|e| Error::Failed(format!("the smoke test could not run: {e}")))?;
                    if let Some(run) = run {
                        if result.ok {
                            run.ok("Smoke test passed: one heartbeat read back");
                        } else {
                            run.fail(&format!("Smoke test failed: {}", result.said));
                        }
                    }
                    smoke = Some(result);
                }
            }
            StepName::Agent => {
                if let Some(r) = &agent_run {
                    let what = format!(
                        "run {} ({}) once in print mode in {} (it may edit files, fetch pages and run only the language's build and test commands and iohr's atlas and memory commands; at most {} turns and {} s) with the prompt below (not started in a dry run)",
                        r.agent.name(),
                        r.model.as_deref().unwrap_or("its own model"),
                        dir.display(),
                        r.max_turns,
                        r.timeout.as_secs()
                    );
                    // The AI label is printed once: in the plan pass that a real run follows
                    // it would show twice, so only the run (or a `--dry-run`) draws it.
                    if !preview || a.dry_run {
                        ui.info(&ask::label_for(
                            "iohr project init",
                            "AI-built project",
                            r.agent,
                        ));
                    }
                    if dry {
                        flow.gate("agent", &what);
                    } else if flow.gate("agent", &what) {
                        let started = Instant::now();
                        let ended = crate::commands::ask::stream(Run {
                            agent: r.agent,
                            bin: &r.bin,
                            args: &r.args,
                            cwd: Some(&dir),
                            json: out.json,
                            look: Look {
                                fancy: ui.fancy(),
                                thinking: Some(r.thinking),
                                cwd: Some(dir.display().to_string()),
                                model: r.model.clone(),
                                rows: true,
                                made: Some("AI-built"),
                                summary: true,
                            },
                            ui,
                            waiting: "Your agent is building it",
                            timeout: Some(r.timeout),
                        })
                        .await?;
                        let secs = started.elapsed().as_secs();
                        if out.json {
                            let line = serde_json::json!({
                                "type": "iohr_summary",
                                "agent": r.agent.name(),
                                "exit_code": ended.code,
                                "duration_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                                "timed_out": ended.timed_out,
                                "max_turns": r.max_turns,
                                "timeout_seconds": r.timeout.as_secs(),
                            });
                            Out::raw(format!("{line}\n").as_bytes());
                        }
                        if ended.code == 0 {
                            ui.done(&format!("{} built it in {secs} s", r.agent.name()));
                            agent_built = true;
                            report.steps.push(project::step(
                                "agent",
                                "Agent build",
                                "done",
                                format!(
                                    "{} built it in {secs} s with edits{}; AI-built, check it",
                                    r.agent.name(),
                                    if docs_only {
                                        ", the project's atlas and memory commands and the web"
                                    } else {
                                        " and the language's build and test commands only"
                                    }
                                ),
                                "",
                            ));
                        } else {
                            let stopped = if ended.timed_out {
                                Some(Stop::Time(r.timeout.as_secs()))
                            } else {
                                ended.turn_capped.then_some(Stop::Turns)
                            };
                            let why = if let Some(st) = stopped {
                                let why = st.said(r.max_turns);
                                // The work so far is on disk: leave a short note for the run
                                // that goes on from it (`ask --allow-edits` reads it).
                                match handoff::write(
                                    &dir,
                                    describe.as_deref().unwrap_or("the template's prompt"),
                                    &ended.last_said,
                                    &why,
                                    &me,
                                ) {
                                    Ok(n) => {
                                        stop_note = Some(format!(
                                            "{n} files are in the folder; {} says what is left",
                                            handoff::FILE
                                        ));
                                    }
                                    Err(e) => ui.warn(&format!("no handoff note: {e}")),
                                }
                                why
                            } else {
                                ended.failure.clone().unwrap_or_else(|| {
                                    format!("it exited with code {}", ended.code)
                                })
                            };
                            report.steps.push(project::step(
                                "agent",
                                "Agent build",
                                "failed",
                                why.clone(),
                                "",
                            ));
                            ui.failed(&format!("{} did not finish: {why}", r.agent.name()));
                            agent_failed = Some(why);
                        }
                    }
                }
            }
        }
    }

    // The agent's work is its own commit, once the smoke test has passed it.
    if agent_built
        && !dry
        && agent_failed.is_none()
        && let Some(r) = &agent_run
    {
        let st = project::ai_commit(
            &dir,
            name,
            r.agent.name(),
            smoke.as_ref().map(|s| s.ok),
            docs_only,
        );
        if !out.json {
            if st.state == "skipped" {
                ui.warn(&st.said);
            } else {
                ui.info(&st.said);
            }
        }
        report.steps.push(st);
    }
    // Extensions that fit the project: offered at a terminal, a hint line otherwise.
    super::init_extensions::offer(
        g,
        env,
        ui,
        out,
        dry,
        interactive,
        &super::init_extensions::Signals {
            template: &tpl.meta.id,
            describe: describe.as_deref(),
        },
    )
    .await;
    if dry {
        dry_report(
            &flow,
            &files,
            &report,
            tpl,
            &Chosen {
                place,
                lang: lang.as_deref(),
                warnings: &warnings,
                agent: agent_run.as_ref(),
                answers,
            },
            out,
            !preview,
        );
        return Ok(());
    }
    // The report: what you now have, never claiming what did not happen.
    if out.json {
        let ran = report.steps.iter().any(|s| s.id == "agent");
        let doc = serde_json::json!({
            "type": ran.then_some("iohr_report"),
            "name": report.name, "id": report.id, "dir": report.dir, "state": report.state,
            "template": tpl.meta.id, "origin": tpl.origin.label(),
            "folder": place, "language": lang, "warnings": warnings,
            "answered": answers.keys().collect::<Vec<_>>(),
            "files": report.files, "signed_in": signed_in, "steps": report.steps, "smoke": smoke,
            "roles": roles,
            "lines": lines(&report, smoke.as_ref(), signed_in.as_deref()),
        });
        if ran {
            // The agent's events were JSON lines; the report ends them as one more line.
            Out::raw(format!("{doc}\n").as_bytes());
        } else {
            Out::print_json(&doc);
        }
    } else if ui.fancy() {
        finish(
            ui,
            g.verbose,
            &report,
            place,
            lang.as_deref(),
            smoke.as_ref(),
            signed_in.as_deref(),
        );
    } else {
        Out::note(&format!(
            "What you now have: {} at {}",
            report.id, place.shown
        ));
        for l in lines(&report, smoke.as_ref(), signed_in.as_deref())
            .iter()
            .skip(1)
        {
            Out::note(&format!("  {l}"));
        }
        Out::note(&format!(
            "Next: {}, or open it in InOrbit Desktop's Code (it asks you first).",
            next_lines(is_extension(&report), runs_in_ext_dev(lang.as_deref()))
                .iter()
                .map(|(c, why)| format!("`{c}` ({why})"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(why) = agent_failed {
        // The scaffold stays; one line says how to go on from it.
        let kept = stop_note.map_or_else(String::new, |n| format!(" ({n})"));
        return Err(Error::Failed(format!(
            "the agent did not finish: {why}. The project is kept in {}{kept}; continue with `cd {} && {me} ask --allow-edits \"continue the work\"` (it reads the note), or raise the caps with --agent-turns and --agent-timeout",
            place.shown, place.shown
        )));
    }
    match smoke {
        Some(s) if !s.ok => {
            let last = s
                .output
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| format!(" ({})", l.trim()))
                .unwrap_or_default();
            Err(Error::Failed(format!(
                "the smoke test failed: {}{last}",
                s.said
            )))
        }
        _ => Ok(()),
    }
}

/// The dry run's ending: the plan as data with `--json`, else the files and the agent's
/// command and prompt, and the outro (`last`: the review page's preview has none, a question
/// follows it).
fn dry_report(
    flow: &Flow,
    files: &[(String, String)],
    report: &Report,
    tpl: &Template,
    chosen: &Chosen<'_>,
    out: Out,
    last: bool,
) {
    let (agent, answers) = (chosen.agent, chosen.answers);
    if out.json {
        Out::print_json(&serde_json::json!({
            "dry_run": true,
            "name": report.name, "id": report.id, "dir": report.dir,
            "folder": chosen.place, "language": chosen.lang, "warnings": chosen.warnings,
            "template": tpl.meta.id, "origin": tpl.origin.label(),
            "answered": answers.keys().collect::<Vec<_>>(),
            "files": files.iter().map(|(p, t)| serde_json::json!({"path": p, "bytes": t.len()})).collect::<Vec<_>>(),
            "steps": flow.would.iter().map(|(id, what)| serde_json::json!({"id": id, "would": what})).collect::<Vec<_>>(),
            "agent": agent.as_ref().map(|r| serde_json::json!({
                "name": r.agent.name(),
                "command": std::iter::once(r.bin.to_string_lossy().into_owned())
                    .chain(r.args.iter().cloned())
                    .collect::<Vec<_>>(),
                "cwd": report.dir,
                "prompt": r.prompt,
                "model": r.model,
                "max_turns": r.max_turns,
                "timeout_seconds": r.timeout.as_secs(),
            })),
        }));
        return;
    }
    flow.ui.plan(
        "What init will do",
        &flow
            .would
            .iter()
            .map(|(_, w)| w.clone())
            .collect::<Vec<_>>(),
    );
    flow.ui.note(
        &format!("{} files, none written", files.len()),
        &plan_lines(files.iter().map(|(p, t)| (p.as_str(), t.len()))).join("\n"),
    );
    if let Some(r) = agent {
        flow.ui.note(
            "The agent would get (AI)",
            &format!(
                "model {}, at most {} turns and {} s\n\ncd {}\n{}\n\n{}",
                r.model.as_deref().unwrap_or("its own"),
                r.max_turns,
                r.timeout.as_secs(),
                report.dir.display(),
                ask::shell_line(
                    &r.bin.display().to_string(),
                    &r.args
                        .iter()
                        .take(r.args.len().saturating_sub(1))
                        .cloned()
                        .chain(["<prompt>".to_owned()])
                        .collect::<Vec<_>>()
                ),
                r.prompt
            ),
        );
    }
    if last {
        flow.ui.outro("dry run: nothing was changed");
    }
}

/// The sign-in check: checked, never required (one call, `GET /v1/me`, so a stale token is
/// found here and not later). `Some(who)` when signed in.
async fn check_sign_in(g: &Global, env: &Env, ui: Ui) -> Option<String> {
    let sign_in = ui.step("Checking your sign-in");
    let signed_in = match session(g, env).await {
        Ok(s) => match s.api.get::<serde_json::Value>("/v1/me", &[]).await {
            Ok(me) => Some(format!(
                "{} ({}, account {})",
                s.label,
                me.pointer("/subject")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&s.claims.sub),
                s.account
            )),
            Err(_) => None,
        },
        Err(_) => None,
    };
    match &signed_in {
        Some(who) => sign_in.ok(&format!("Signed in as {who}")),
        None => {
            sign_in.warn("Not signed in: run `iohr login` so init can make the project in InOrbit");
        }
    }
    signed_in
}

/// How a finished report step reads on the terminal: its mark and one line. Only `done` is a
/// check; a step that was skipped or waits on a decision says so in its line.
fn step_line(st: &project::Step) -> (Mark, String) {
    let mark = Mark::of(&st.state);
    let said = match st.state.as_str() {
        "done" => format!("{}: {}", st.title, st.said),
        "skipped" => format!("{} skipped: {}", st.title, st.said),
        "failed" => format!("{} failed: {}", st.title, st.said),
        _ => format!("{} waits: {}", st.title, st.said),
    };
    (mark, said)
}

/// The drawn ending: what is left to do in one line (`--verbose` lists it), and an outro
/// with the next commands. Nothing here claims a step that did not happen.
fn finish(
    ui: Ui,
    verbose: bool,
    r: &Report,
    place: &Proposal,
    lang: Option<&str>,
    smoke: Option<&Smoke>,
    signed_in: Option<&str>,
) {
    let later: Vec<String> = lines(r, smoke, signed_in)
        .into_iter()
        .filter(|l| l.starts_with("not yet:"))
        .collect();
    let asked: Vec<String> = lines(r, smoke, signed_in)
        .into_iter()
        .filter(|l| l.starts_with("asked:"))
        .collect();
    for a in &asked {
        ui.info(a);
    }
    if verbose {
        ui.note("Comes later", &later.join("\n"));
    } else if !later.is_empty() {
        ui.info(&later_line(later.len()));
    }
    let next = next_lines(is_extension(r), runs_in_ext_dev(lang));
    let width = next.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    let mut body = format!("cd {}", place.shown);
    for (c, why) in &next {
        let _ = write!(body, "\n{c:<width$}   {why}");
    }
    body.push_str("\nor open it in InOrbit Desktop's Code (it asks you first)");
    ui.note(&format!("{} is ready", r.id), &body);
    ui.outro("Done.");
}

/// The one line that replaces the list of what waits on an API that does not exist yet.
fn later_line(n: usize) -> String {
    let things = if n == 1 { "thing comes" } else { "things come" };
    format!("{n} {things} later (RFC 0127.x): run with --verbose to list them")
}

#[cfg(test)]
mod tests {
    use super::{Lang, lang_of_variant, language_choices, later_line, publisher_of};
    use crate::template::{self, Spec};
    use std::path::Path;

    #[test]
    fn the_picker_hint_says_where_the_installed_toolchain_stands() {
        let tpl =
            template::load(&Spec::Id("blank".into()), Path::new("/no-such-iohr-root")).unwrap();
        let hint_of = |id: &str, behind: Option<&str>| {
            let (choices, _) = language_choices(
                &tpl,
                |_| true,
                |v| {
                    (v.id == "rust")
                        .then(|| behind.map(str::to_owned))
                        .flatten()
                },
            );
            choices.into_iter().find(|c| c.value == id).unwrap().hint
        };
        assert!(
            hint_of("rust", Some("rustc 1.90.0, 1.93.0 is out"))
                .starts_with("cargo found \u{b7} rustc 1.90.0, 1.93.0 is out"),
            "{}",
            hint_of("rust", Some("rustc 1.90.0, 1.93.0 is out"))
        );
        // No lookup (offline off, --json, a language with no source): the hint is as before.
        assert!(hint_of("rust", None).starts_with("cargo found"));
        assert!(!hint_of("rust", None).contains('\u{b7}'));
    }

    #[test]
    fn a_variant_id_is_its_language() {
        assert_eq!(lang_of_variant("typescript"), Some(Lang::Node));
        assert_eq!(lang_of_variant("rust"), Some(Lang::Rust));
        assert_eq!(lang_of_variant("go"), Some(Lang::Go));
        assert_eq!(lang_of_variant("python"), Some(Lang::Python));
        assert_eq!(lang_of_variant("cobol"), None);
    }

    #[test]
    fn the_later_line_counts_and_points_at_verbose() {
        assert_eq!(
            later_line(5),
            "5 things come later (RFC 0127.x): run with --verbose to list them"
        );
        assert!(later_line(1).starts_with("1 thing comes later"));
    }

    #[test]
    fn the_publisher_is_the_flag_then_the_account_then_local() {
        assert_eq!(publisher_of(Some("Acme"), Some("inorbit")), "acme");
        assert_eq!(publisher_of(None, Some("inorbit")), "inorbit");
        assert_eq!(publisher_of(None, None), "local");
        assert_eq!(publisher_of(Some("bad slug"), None), "local");
        assert_eq!(publisher_of(Some("x"), None), "local");
    }
}

/// The dry run's file list: one line per file, except that a folder holding more than
/// [`FOLDED`] files (the memory facts) is one line with its count and size.
fn plan_lines<'a>(files: impl Iterator<Item = (&'a str, usize)> + Clone) -> Vec<String> {
    const FOLDED: usize = 5;
    let folder = |p: &'a str| p.rsplit_once('/').map_or("", |(d, _)| d);
    let mut counts: std::collections::BTreeMap<&str, (usize, usize)> =
        std::collections::BTreeMap::new();
    for (p, n) in files.clone() {
        let c = counts.entry(folder(p)).or_default();
        c.0 += 1;
        c.1 += n;
    }
    let mut shown = std::collections::BTreeSet::new();
    let mut lines = Vec::new();
    for (p, n) in files {
        let d = folder(p);
        match counts.get(d) {
            Some(&(count, bytes)) if !d.is_empty() && count > FOLDED => {
                if shown.insert(d) {
                    lines.push(format!("{d}/  ({count} files, {bytes} bytes)"));
                }
            }
            _ => lines.push(format!("{p}  ({n} bytes)")),
        }
    }
    lines
}

#[cfg(test)]
mod plan_lines_tests {
    #[test]
    fn a_folder_of_many_files_is_one_line_and_the_rest_stay_one_per_file() {
        let mut files = vec![("README.md", 10), ("src/main.rs", 20)];
        let facts: Vec<String> = (0..7).map(|i| format!(".inorbit/memory/f{i}.md")).collect();
        files.extend(facts.iter().map(|p| (p.as_str(), 3)));
        let lines = super::plan_lines(files.iter().copied());
        assert_eq!(
            lines,
            [
                "README.md  (10 bytes)",
                "src/main.rs  (20 bytes)",
                ".inorbit/memory/  (7 files, 21 bytes)"
            ]
        );
    }
}
