//! `iohr project init` (alias `create`) and `templates`: set a project of your own up (PRD 0088).

use std::path::{Path, PathBuf};

use crate::Env;
use crate::cli::{Global, ProjectCommand};
use crate::context::{Ctx, session};
use crate::error::Error;
use crate::output::Out;
use crate::project::{self, Report, Smoke, TEMPLATES, check_name, choose_template, lines};
use crate::ui::{Choice, Ui};

/// Where projects go by default: `~/development/<name>`.
fn dev_root() -> Result<PathBuf, Error> {
    etcetera::home_dir()
        .map(|h| h.join("development"))
        .map_err(|e| Error::Failed(format!("no home folder: {e}")))
}

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

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

pub(crate) async fn run(g: &Global, env: &Env, cmd: ProjectCommand, out: Out) -> Result<(), Error> {
    match cmd {
        ProjectCommand::Init {
            name,
            template,
            yes,
            dir,
            publisher,
            no_smoke,
        } => {
            let configured = Ctx::load(g)?.config.project.template;
            let ui = Ui::detect(out.json);
            let (name, template, dir) = if ui.fancy() {
                ui.intro("iohr project init");
                match ask(
                    ui,
                    name,
                    template,
                    configured.as_deref(),
                    yes,
                    dir.as_deref(),
                )? {
                    Some(a) => (a.name, a.template, a.dir),
                    // The person declined at the confirmation: nothing was written.
                    None => return Ok(()),
                }
            } else {
                (
                    name.ok_or_else(|| {
                        Error::Usage(
                            "a project name is needed: iohr project init NAME (there is no terminal to ask on)"
                                .into(),
                        )
                    })?,
                    choose_template(template.as_deref(), configured.as_deref()),
                    dir,
                )
            };
            init(
                g,
                env,
                &name,
                &template,
                dir.as_deref(),
                publisher.as_deref(),
                no_smoke,
                out,
                ui,
            )
            .await
        }
        ProjectCommand::Templates => {
            let configured = Ctx::load(g)?.config.project.template;
            templates(configured.as_deref(), out);
            Ok(())
        }
    }
}

/// The templates table, or the same as a JSON list; `default` marks the one `init` uses.
fn templates(configured: Option<&str>, out: Out) {
    let default = choose_template(None, configured);
    let list = project::listing(&default);
    if out.json {
        Out::print_json(&serde_json::Value::Array(list));
        return;
    }
    let width = TEMPLATES
        .iter()
        .map(|(id, _, _)| id.len())
        .max()
        .unwrap_or(0);
    Out::raw(format!("{:<width$}  {:<6}  DESCRIPTION\n", "NAME", "STATUS").as_bytes());
    for t in &list {
        let name = t["name"].as_str().unwrap_or_default();
        let status = t["status"].as_str().unwrap_or_default();
        let star = if t["default"] == true {
            " (default)"
        } else {
            ""
        };
        Out::raw(
            format!(
                "{name:<width$}  {status:<6}  {}{star}\n",
                t["description"].as_str().unwrap_or_default()
            )
            .as_bytes(),
        );
    }
}

/// What the guided questions settled.
struct Asked {
    name: String,
    template: String,
    dir: Option<PathBuf>,
}

/// The questions, at a terminal only: the name (the folder's name is the default), the
/// template as a menu, and the folder to confirm. A flag skips its question, `--yes` skips
/// the menu and the confirmation. `None` when the person declines the folder.
fn ask(
    ui: Ui,
    name: Option<String>,
    template: Option<String>,
    configured: Option<&str>,
    yes: bool,
    dir: Option<&Path>,
) -> Result<Option<Asked>, Error> {
    let name = if let Some(n) = name {
        n
    } else {
        let here = std::env::current_dir()
            .ok()
            .and_then(|d| d.file_name().map(|f| f.to_string_lossy().into_owned()))
            .filter(|f| check_name(f).is_ok())
            .unwrap_or_default();
        ui.input("What is the project called?", &here, check_name)?
    };
    check_name(&name).map_err(Error::Usage)?;
    let template = match template {
        Some(t) => t,
        None if yes => choose_template(None, configured),
        None => {
            let default = choose_template(None, configured);
            let choices: Vec<Choice> = TEMPLATES
                .iter()
                .map(|(id, what, live)| Choice {
                    value: (*id).to_owned(),
                    label: (*id).to_owned(),
                    hint: what.replace("; coming after the demo", ""),
                    available: *live,
                    key: None,
                })
                .collect();
            ui.select("Which template?", &choices, &default)?
        }
    };
    let target = match dir {
        Some(d) => d.to_path_buf(),
        None => dev_root()?.join(&name),
    };
    if !yes && !ui.confirm(format!("Create it in {}?", target.display()).as_str(), true)? {
        ui.outro("Nothing was written.");
        return Ok(None);
    }
    Ok(Some(Asked {
        name,
        template,
        dir: Some(target),
    }))
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

#[allow(clippy::too_many_arguments, reason = "one argument per flag")]
async fn init(
    g: &Global,
    env: &Env,
    name: &str,
    template: &str,
    dir: Option<&Path>,
    publisher: Option<&str>,
    no_smoke: bool,
    out: Out,
    ui: Ui,
) -> Result<(), Error> {
    check_name(name).map_err(Error::Usage)?;
    if !TEMPLATES.iter().any(|(id, _, _)| *id == template) {
        return Err(Error::Usage(format!(
            "there is no `{template}` template; one of: {}",
            TEMPLATES
                .iter()
                .map(|(id, what, live)| format!(
                    "{id} ({what}{})",
                    if *live { "" } else { "; coming" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    // 1. Sign-in: checked, never required. Without it the local half still happens and the
    //    report says what to do; with it the cloud steps can land once their APIs exist.
    //    The check is one call, `GET /v1/me`, so a stale token is found here and not later.
    let signed_in = check_sign_in(g, env, ui).await;
    // The account's slug is not on the token; the private-listing step reads it from
    // /v1/accounts/me when that step lands. Until then: the flag, or "local".
    let publisher = publisher_of(publisher, None);
    let dir = match dir {
        Some(d) => d.to_path_buf(),
        None => dev_root()?.join(name),
    };
    let state_root = state_root()?;
    // 2 to 4. The scaffold, the git repository, the project's own state, the Keychain slot
    //    by name, and every cloud step as not-yet with the API it needs.
    let setup = ui.step(&format!("Setting up {name} from `{template}`"));
    let report: Report = match project::init(&dir, &state_root, name, template, &publisher, &now())
    {
        Ok(r) => r,
        Err(e) => {
            setup.fail(&e);
            return Err(Error::Failed(e));
        }
    };
    if ui.fancy() {
        // The three local steps, one ok line each; a skipped one is a warning with its reason.
        setup.ok(&format!(
            "Scaffolded {} files in {}",
            report.files.len(),
            report.dir.display()
        ));
        for s in report.steps.iter().filter(|s| s.state == "skipped") {
            ui.step(&s.title).warn(&format!("{}: {}", s.title, s.said));
        }
    }
    // 5. The smoke test: the project once, under its own STATE and CACHE, no token channel.
    let smoke: Option<Smoke> = if no_smoke {
        None
    } else {
        let run = ui.step("Running the project once (smoke test)");
        let (d, s, n) = (
            report.dir.clone(),
            report.state.clone(),
            report.name.clone(),
        );
        let result = tokio::task::spawn_blocking(move || project::smoke(&d, &s, &n))
            .await
            .map_err(|e| Error::Failed(format!("the smoke test could not run: {e}")))?;
        if result.ok {
            run.ok("Smoke test passed: one heartbeat read back");
        } else {
            run.fail(&format!("Smoke test failed: {}", result.said));
        }
        Some(result)
    };
    // 6. The report: what you now have, never claiming what did not happen.
    if out.json {
        Out::print_json(&serde_json::json!({
            "name": report.name, "id": report.id, "dir": report.dir, "state": report.state,
            "files": report.files, "signed_in": signed_in, "steps": report.steps, "smoke": smoke,
            "lines": lines(&report, smoke.as_ref(), signed_in.as_deref()),
        }));
    } else if ui.fancy() {
        finish(ui, g.verbose, &report, smoke.as_ref(), signed_in.as_deref());
    } else {
        Out::note(&format!(
            "What you now have: {} at {}",
            report.id,
            report.dir.display()
        ));
        for l in lines(&report, smoke.as_ref(), signed_in.as_deref())
            .iter()
            .skip(1)
        {
            Out::note(&format!("  {l}"));
        }
        Out::note(
            "Next: open it in InOrbit Desktop's Code (it asks you first), `iohr ext lint --preview` to read what it may do, `iohr ext dev` to run it with the real token channel.",
        );
    }
    match smoke {
        Some(s) if !s.ok => Err(Error::Failed(format!("the smoke test failed: {}", s.said))),
        _ => Ok(()),
    }
}

/// The drawn ending: what is left to do in one line (`--verbose` lists it), and an outro
/// with the next commands. Nothing here claims a step that did not happen.
fn finish(ui: Ui, verbose: bool, r: &Report, smoke: Option<&Smoke>, signed_in: Option<&str>) {
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
    ui.note(
        &format!("{} is ready", r.id),
        &format!(
            "cd {dir}\niohr ext lint --preview   read what it may do\niohr ext dev              run it with the real token channel\nor open it in InOrbit Desktop's Code (it asks you first)",
            dir = r.dir.display()
        ),
    );
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
