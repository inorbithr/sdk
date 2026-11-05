//! `iohr project init` (alias `create`) and `template`: set a project of your own up (PRD 0088).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::Env;
use crate::ask::stream::{Look, Thinking};
use crate::ask::{self, Agent};
use crate::atlas;
use crate::cli::{Global, ProjectCommand, TemplateCommand};
use crate::commands::ask::Run;
use crate::context::{Ctx, session};
use crate::error::Error;
use crate::output::Out;
use std::fmt::Write as _;

use super::project_template;
use crate::project::folder::{self, Kind as Place, Proposal, Where};
use crate::project::{self, Report, Smoke, check_name, choose_template, lines};
use crate::template::{self, Answer, Answers, Kind, LANGUAGE, Question, Spec, StepName, Template};
use crate::ui::{Choice, Ui};

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
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The menu's first entry: the agent builds it from a description.
const DESCRIBE: &str = "@describe";
/// The template a described project is made from, unless one is named.
const DESCRIBE_TEMPLATE: &str = "blank";

/// What the person said to `init`, as the flags carry it.
struct InitArgs {
    name: Option<String>,
    template: Option<String>,
    set: Vec<String>,
    answers: Option<PathBuf>,
    describe: Option<String>,
    agent_turns: Option<u32>,
    agent_timeout: Option<u64>,
    dry_run: bool,
    yes: bool,
    dir: Option<PathBuf>,
    lang: Option<String>,
    publisher: Option<String>,
    no_smoke: bool,
}

pub(crate) async fn run(g: &Global, env: &Env, cmd: ProjectCommand, out: Out) -> Result<(), Error> {
    match cmd {
        ProjectCommand::Init {
            name,
            template,
            set,
            answers,
            describe,
            agent_turns,
            agent_timeout,
            dry_run,
            yes,
            dir,
            lang,
            publisher,
            no_smoke,
        } => {
            init(
                g,
                env,
                InitArgs {
                    name,
                    template,
                    set,
                    answers,
                    describe,
                    agent_turns,
                    agent_timeout,
                    dry_run,
                    yes,
                    dir,
                    lang,
                    publisher,
                    no_smoke,
                },
                out,
            )
            .await
        }
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
            self.ui.info(&format!("would: {what}"));
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
/// and refused when picked.
fn language_choices(tpl: &Template, have: impl Fn(&str) -> bool) -> (Vec<Choice>, String) {
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
                format!("{} found", v.needs.join(", "))
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
fn pick_folder(ui: Ui, w: &Where, list: &[Proposal]) -> Result<Proposal, Error> {
    let free: Vec<&Proposal> = list.iter().filter(|p| !p.busy).collect();
    for p in list.iter().filter(|p| p.busy) {
        ui.info(&format!("{} is not empty, so it is not offered", p.shown));
    }
    let choices = folder_choices(list);
    let picked = ui.select("Where should it go?", &choices, "0")?;
    let idx: usize = picked.parse().unwrap_or(0);
    let p = free
        .get(idx)
        .map_or_else(|| list[0].clone(), |p| (*p).clone());
    if p.kind != Place::Elsewhere {
        return Ok(p);
    }
    let typed = ui.input("Which folder?", "", |s| {
        if s.trim().is_empty() {
            Err("type a folder".to_owned())
        } else {
            Ok(())
        }
    })?;
    Ok(folder::given(
        w,
        std::path::Path::new(typed.trim()),
        "the folder you typed",
    ))
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

/// One question at the terminal, drawn from the schema: the kind picks the control.
fn ask_question(ui: Ui, tpl: &Template, q: &Question) -> Result<Answer, Error> {
    if q.key == LANGUAGE && !tpl.meta.variants.is_empty() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let (choices, initial) = language_choices(tpl, |n| ask::find_on_path(n, &path).is_some());
        return Ok(Answer::Text(ui.select(&q.prompt, &choices, &initial)?));
    }
    if let Some(h) = &q.help {
        ui.info(h);
    }
    let default = q.default.clone();
    match q.kind {
        Kind::Bool => Ok(Answer::Flag(
            ui.confirm(&q.prompt, matches!(default, Some(Answer::Flag(true))))?,
        )),
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
            Ok(Answer::Text(ui.select(&q.prompt, &choices, &initial)?))
        }
        Kind::Multi => {
            let initial = match default {
                Some(Answer::List(l)) => l,
                _ => vec![],
            };
            Ok(Answer::List(
                ui.multiselect(&q.prompt, &q.choices, &initial)?,
            ))
        }
        Kind::String => {
            let check = q.clone();
            Ok(Answer::Text(ui.input(
                &q.prompt,
                &default.map(|d| d.text()).unwrap_or_default(),
                move |v| template::coerce(&check, &Answer::Text(v.to_owned())).map(|_| ()),
            )?))
        }
    }
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

/// The most turns the agent gets in `init` when no flag says: init's own default, because
/// `[ask]` in config.toml has no bound. Enough to write a small project; a wrong request cannot
/// run away.
const AGENT_TURNS: u32 = 40;
/// The most seconds the agent gets in `init` when no flag says (init's own default).
const AGENT_SECONDS: u64 = 600;

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
    if a.agent_turns == Some(0) || a.agent_timeout == Some(0) {
        return Err(Error::Usage(
            "--agent-turns and --agent-timeout need a number above 0".into(),
        ));
    }
    if a.describe.is_some() && agent.is_none() {
        return Err(Error::Failed(no_agent_help()));
    }
    let root = template::user_root()?;
    if ui.fancy() {
        ui.intro(if dry {
            "iohr project init (dry run)"
        } else {
            "iohr project init"
        });
    }

    // The name, and the template (a menu at a terminal).
    let name = match a.name {
        Some(n) => n,
        None if interactive => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let props = folder::name_proposals(&cwd);
            if props.is_empty() {
                ui.input("What is the project called?", "", check_name)?
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
                let picked = ui.select("What is the project called?", &choices, &props[0].0)?;
                if picked == OTHER {
                    ui.input("What is the project called?", "", check_name)?
                } else {
                    picked
                }
            }
        }
        None => {
            return Err(Error::Usage(
                "a project name is needed: iohr project init NAME (there is no terminal to ask on)"
                    .into(),
            ));
        }
    };
    check_name(&name).map_err(Error::Usage)?;
    let mut describe = a.describe;
    let mut spec_text = a.template;
    if spec_text.is_none() && interactive {
        let default = choose_template(None, configured.as_deref());
        let rows = template::list(&root, &default);
        let mut choices = vec![Choice {
            value: DESCRIBE.into(),
            label: "Describe it, your agent builds it".into(),
            hint: match &agent {
                Some((ag, _)) => format!("starts {} in the new folder with your words", ag.name()),
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
        let picked = ui.select("Which template?", &choices, &default)?;
        if picked == DESCRIBE {
            if agent.is_none() {
                return Err(Error::Failed(no_agent_help()));
            }
            describe = Some(ui.input("Describe the project", "", nonempty)?);
            spec_text = Some(DESCRIBE_TEMPLATE.to_owned());
        } else {
            spec_text = Some(picked);
        }
    }
    // A described project is a normal new project: `blank`, unless `--template` (or the menu)
    // named another one. `[project] template` is for projects you did not describe.
    if describe.is_some() && spec_text.is_none() {
        spec_text = Some(DESCRIBE_TEMPLATE.to_owned());
    }
    let spec = template::parse_spec(&choose_template(
        spec_text.as_deref(),
        configured.as_deref(),
    ));
    if dry && matches!(spec, Spec::Git { .. }) {
        return Err(Error::Usage(
            "a dry run makes no network call, so it cannot clone that template: clone it yourself and pass its folder with --template PATH".into(),
        ));
    }
    let tpl: Template = template::load(&spec, &root).map_err(Error::Usage)?;
    if tpl.meta.status == template::Status::Coming {
        let rows = template::list(&root, "");
        return Err(Error::Usage(format!(
            "the `{}` template is coming; live today: {}",
            tpl.meta.id,
            live_names(&rows)
        )));
    }

    // The answers: --set, then --answers, then the questions at a terminal, then defaults.
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
    let mut asker = |q: &Question| ask_question(ui, &tpl, q);
    let answers = template::answer(
        &tpl,
        &given,
        if interactive { Some(&mut asker) } else { None },
    )?;

    // Where it goes: --dir, else a menu at a terminal, else `./NAME`.
    let publisher = publisher_of(a.publisher.as_deref(), None);
    let here = Where::here(cfg.project.root.clone());
    let place = if let Some(d) = &a.dir {
        folder::given(&here, d, "you passed --dir")
    } else {
        let list = folder::proposals(&here, &name);
        if interactive {
            pick_folder(ui, &here, &list)?
        } else {
            folder::headless(&list)
        }
    };
    let dir = place.dir.clone();
    let mut warnings: Vec<String> = Vec::new();
    if let Some(w) = folder::nest_warning(&here, &dir) {
        ui.warn(&w);
        warnings.push(w);
    }
    let created_at = now();
    let lang = answers.get(LANGUAGE).map(Answer::text);
    let tpl_lang = match &lang {
        Some(l) => tpl.with_variant(l).map_err(Error::Usage)?,
        None => tpl.clone(),
    };
    let mut vals = template::all_values(&tpl, &name, &publisher, &created_at, &answers);
    if !vals.iter().any(|(k, _)| k == "@@DESCRIPTION@@") {
        vals.push((
            "@@DESCRIPTION@@".to_owned(),
            project::one_line_description(describe.as_deref()),
        ));
    }
    let mut files = template::render(&tpl_lang, &vals).map_err(Error::Failed)?;
    // Every project gets its InOrbit space link, whatever the template (RFC 0103).
    project::add_space_link(&mut files, None);
    // And its atlas: starter PRD, RFC, ADR and study, a canvas and a 3D folder (RFC 0205).
    atlas::add_atlas(
        &mut files,
        &atlas::Meta {
            name: &name,
            template: &tpl.meta.id,
            lang: lang.as_deref(),
            describe: describe.as_deref(),
            created_at: &created_at,
        },
    );
    let programs = template::programs(&tpl, &vals);
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
    if a.no_smoke {
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
            ui.confirm(
                &format!(
                    "Start {} in the new folder with the template's prompt?",
                    ag.name()
                ),
                true,
            )?
        } else {
            false
        };
        if wanted {
            let prompt = project::agent_prompt(
                prompt_text.as_deref(),
                describe.as_deref(),
                &name,
                &tpl.meta.id,
                &answers,
                &vals,
            );
            // The `[ask]` model (else the small fast tier), edits to files only, never shell,
            // and init's own bounds: nothing the agent is told or says is stored here.
            let model =
                ask::choose_model(*ag, None, cfg.ask.model.as_deref()).map_err(Error::Usage)?;
            let max_turns = a.agent_turns.unwrap_or(AGENT_TURNS);
            agent_run = Some(AgentRun {
                agent: *ag,
                bin: bin.clone(),
                args: ask::argv(*ag, model.as_deref(), true, Some(max_turns), &prompt),
                prompt,
                model,
                max_turns,
                timeout: Duration::from_secs(a.agent_timeout.unwrap_or(AGENT_SECONDS)),
                thinking: ask::choose_thinking(None, cfg.ask.thinking.as_deref())
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

    // Everything below that changes anything goes through `flow`.
    let mut flow = Flow {
        dry,
        ui,
        would: Vec::new(),
    };
    let state_root = state_root()?;
    let state = state_root.join(&name);
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
    let mut smoke: Option<Smoke> = None;
    let mut agent_failed: Option<String> = None;
    for s in &steps {
        match s.name {
            StepName::Git => {
                let r = flow.act("git", "make a git repository with one commit", || {
                    project::git_step(&dir, &name, &tpl.meta.id)
                });
                if let Some(st) = r {
                    report.steps.push(st);
                }
            }
            StepName::Register => {
                let what = format!(
                    "register the project: state and cache under {}",
                    state.display()
                );
                let r = flow.act("register", &what, || {
                    project::register_step(&dir, &state_root, &name, &tpl.meta.id, &created_at)
                });
                match r {
                    Some(Ok((_, st))) => {
                        report.steps.push(st);
                        report.steps.extend(project::cloud_steps(&name));
                    }
                    Some(Err(e)) => return Err(Error::Failed(e)),
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
                        "run {} once in print mode in {} (files only, never shell; at most {} turns and {} s) with the prompt below (not started in a dry run)",
                        r.agent.name(),
                        dir.display(),
                        r.max_turns,
                        r.timeout.as_secs()
                    );
                    // The AI label is printed either way; the agent starts only past the gate.
                    Out::note(&ask::label_for(
                        "iohr project init",
                        "AI-built project",
                        r.agent,
                    ));
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
                            report.steps.push(project::step(
                                "agent",
                                "Agent build",
                                "done",
                                format!(
                                    "{} built it in {secs} s, files only, never shell; AI-built, check it",
                                    r.agent.name()
                                ),
                                "",
                            ));
                        } else {
                            let why = if ended.timed_out {
                                format!(
                                    "it did not finish in {} s and was stopped",
                                    r.timeout.as_secs()
                                )
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
                            agent_failed = Some(why);
                        }
                    }
                }
            }
        }
    }

    if dry {
        dry_report(
            &flow,
            &files,
            &report,
            &tpl,
            &Chosen {
                place: &place,
                lang: lang.as_deref(),
                warnings: &warnings,
                agent: agent_run.as_ref(),
                answers: &answers,
            },
            out,
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
            &place,
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
        return Err(Error::Failed(format!(
            "the agent did not finish: {why}. The project is kept in {}; retry the agent there with `cd {} && iohr ask --allow-edits \"what you want\"`",
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
/// command and prompt, and the outro.
fn dry_report(
    flow: &Flow,
    files: &[(String, String)],
    report: &Report,
    tpl: &Template,
    chosen: &Chosen<'_>,
    out: Out,
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
    flow.ui.note(
        &format!("{} files, none written", files.len()),
        &files
            .iter()
            .map(|(p, t)| format!("{p}  ({} bytes)", t.len()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if let Some(r) = agent {
        flow.ui.note(
            "The agent would get (AI)",
            &format!(
                "cd {}\n{}\n\n{}",
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
    flow.ui.outro("dry run: nothing was changed");
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
    use super::{later_line, publisher_of};

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
