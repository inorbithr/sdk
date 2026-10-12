//! `iohr project init`: the one command that sets a project of your own up (PRD 0088).

use std::path::{Path, PathBuf};

use crate::Env;
use crate::cli::{Global, ProjectCommand};
use crate::context::session;
use crate::error::Error;
use crate::output::Out;
use crate::project::{self, Report, Smoke, TEMPLATES, check_name, lines};

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
            dir,
            publisher,
            no_smoke,
        } => {
            init(
                g,
                env,
                &name,
                &template,
                dir.as_deref(),
                publisher.as_deref(),
                no_smoke,
                out,
            )
            .await
        }
    }
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
    let signed_in = match session(g, env).await {
        Ok(s) => Some(format!("{} ({})", s.label, s.account)),
        Err(_) => None,
    };
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
    let report: Report = project::init(&dir, &state_root, name, template, &publisher, &now())
        .map_err(Error::Failed)?;
    // 5. The smoke test: the project once, under its own STATE and CACHE, no token channel.
    let smoke: Option<Smoke> = if no_smoke {
        None
    } else {
        let (d, s, n) = (
            report.dir.clone(),
            report.state.clone(),
            report.name.clone(),
        );
        Some(
            tokio::task::spawn_blocking(move || project::smoke(&d, &s, &n))
                .await
                .map_err(|e| Error::Failed(format!("the smoke test could not run: {e}")))?,
        )
    };
    // 6. The report: what you now have, never claiming what did not happen.
    if out.json {
        Out::print_json(&serde_json::json!({
            "name": report.name, "id": report.id, "dir": report.dir, "state": report.state,
            "files": report.files, "signed_in": signed_in, "steps": report.steps, "smoke": smoke,
            "lines": lines(&report, smoke.as_ref(), signed_in.as_deref()),
        }));
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

#[cfg(test)]
mod tests {
    use super::publisher_of;

    #[test]
    fn the_publisher_is_the_flag_then_the_account_then_local() {
        assert_eq!(publisher_of(Some("Acme"), Some("inorbit")), "acme");
        assert_eq!(publisher_of(None, Some("inorbit")), "inorbit");
        assert_eq!(publisher_of(None, None), "local");
        assert_eq!(publisher_of(Some("bad slug"), None), "local");
        assert_eq!(publisher_of(Some("x"), None), "local");
    }
}
