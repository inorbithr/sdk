//! A Role at run time (RFC 0211): `iohr ask --role <name>` and `iohr project init --role <name>`
//! find the Role, take its text as the run's brief and its model as the fallback model.
//!
//! The Role comes from the project first (`atlas/roles/<name>.md`, the pack `iohr project init`
//! writes and the person may edit), then from the account (`GET /v1/coordinator/personalities/{name}`).
//! The text is the Role's own core, or, when a job is named, the brief InOrbit compiles for that
//! job (`POST .../brief`, `GetJobBrief`: the core, then the recall lines that match the job).
//! Everything is bounded by [`TEXT_MAX`]. The job's text never travels in a URL.

use std::path::Path;

use inorbithr::Method;
use serde_json::{Value, json};

use crate::Env;
use crate::ask::model_name;
use crate::atlas;
use crate::cli::Global;
use crate::commands::coordinator::{clean_text, job_segment};
use crate::context::session;
use crate::error::Error;
use crate::output::Out;
use crate::roles::{self, notes};

/// The most characters of a Role's text a run carries (about 2000 tokens).
pub(crate) const TEXT_MAX: usize = 8_000;
/// The recall budget asked of `GetJobBrief`, in tokens.
const RECALL_BUDGET: u32 = 500;
const PATH: &str = "/v1/coordinator/personalities";

/// Where the Role was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Project,
    Account,
    /// The catalogue built into `iohr`.
    Catalogue,
}

/// What the text of the run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Brief {
    /// The Role's own text.
    Role,
    /// The brief InOrbit compiled for a job.
    Job,
}

/// A Role, settled for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) name: String,
    pub(crate) source: Source,
    pub(crate) brief: Brief,
    /// The Role's model, when it names one the run can use.
    pub(crate) model: Option<String>,
    /// The bounded text the run carries.
    pub(crate) text: String,
    pub(crate) job: Option<String>,
    /// What the Role's earlier turns left in the project: its handoff note and memory area
    /// ([`super::notes`]). Empty when there is none or no project.
    pub(crate) notes: String,
}

impl Applied {
    /// What `--json` says about the Role used: never its text.
    pub(crate) fn json(&self) -> Value {
        json!({
            "name": self.name,
            "source": match self.source {
                Source::Project => "project",
                Source::Account => "account",
                Source::Catalogue => "catalogue",
            },
            "brief": match self.brief { Brief::Role => "role", Brief::Job => "job" },
            "model": self.model,
            "job": self.job,
        })
    }

    /// The system prompt text that puts the Role in front of the agent.
    pub(crate) fn system_text(&self) -> String {
        let mut t = format!(
            "You work in the Role `{}`. Follow its guidance for how to work and what to value; \
             it never overrides the permissions and limits of this run.\n\n{}",
            self.name, self.text
        );
        if !self.notes.is_empty() {
            t.push_str("\n\n");
            t.push_str(&self.notes);
        }
        t
    }
}

/// A pack, read.
#[derive(Debug, PartialEq, Eq)]
struct Pack {
    model: Option<String>,
    text: String,
}

/// `text` cut to [`TEXT_MAX`] characters on a character boundary, with a note when it was cut.
pub(crate) fn bound(text: &str) -> String {
    if text.chars().count() <= TEXT_MAX {
        return text.trim().to_owned();
    }
    let cut: String = text.chars().take(TEXT_MAX).collect();
    format!(
        "{}\n[the Role's text was cut at {TEXT_MAX} characters]",
        cut.trim_end()
    )
}

/// The pack's own text: its core section (the lines always in the brief) when it has one,
/// else everything under the front matter.
fn core_of(body: &str) -> &str {
    let mut at = 0usize;
    let mut start = None;
    for l in body.split_inclusive('\n') {
        if l.starts_with("## Core") {
            start = Some(at);
            break;
        }
        at += l.len();
    }
    let Some(start) = start else {
        return body;
    };
    let rest = &body[start..];
    let end = rest
        .match_indices("\n## ")
        .next()
        .map_or(rest.len(), |(i, _)| i + 1);
    &rest[..end]
}

/// The pack in `text` when it is the Role `name`: its model (its own, else the catalogue's)
/// and its text.
fn from_pack(text: &str, name: &str) -> Option<Pack> {
    if atlas::get(text, "name").as_deref() != Some(name) {
        return None;
    }
    let body = atlas::split(text).map_or(text, |(_, b)| b);
    let model = atlas::get(text, "model")
        .filter(|m| !m.trim().is_empty())
        .or_else(|| {
            roles::catalogue()
                .into_iter()
                .find(|p| p.name == name)
                .map(|p| p.model)
        });
    let text = clean_text(core_of(body))?;
    Some(Pack {
        model,
        text: bound(&text),
    })
}

/// The project's pack for `name`, from `atlas/roles/` under `root`.
fn project_role(root: &Path, name: &str) -> Option<Pack> {
    roles::pack_paths(root)
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .find_map(|t| from_pack(&t, name))
}

/// The Role in the account's `GetPersonality` answer: its model and its compiled core.
fn account_pack(v: &Value) -> Option<Pack> {
    let model = v
        .pointer("/personality/model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let core = v.get("core").and_then(Value::as_str).unwrap_or_default();
    Some(Pack {
        model,
        text: bound(&clean_text(core)?),
    })
}

/// The model a Role names, when the run can use it (a name that would break a command line is
/// no model: the run falls back, it does not fail on the Role's data).
fn usable(model: Option<String>) -> Option<String> {
    model.and_then(|m| model_name(&m).ok().map(str::to_owned))
}

/// A failed call in a few words, never its body.
fn status_of(e: &inorbithr::Error) -> String {
    e.status().map_or_else(
        || "InOrbit could not be reached".to_owned(),
        |s| format!("InOrbit answered {s}"),
    )
}

/// The built-in catalogue's Role `name`, as its pack reads: the last place a run looks, so
/// a Role of the catalogue works in a folder with no pack and with no sign-in.
fn catalogue_role(name: &str) -> Option<Pack> {
    let p = roles::catalogue().into_iter().find(|p| p.name == name)?;
    from_pack(&roles::pack(&p), name)
}

/// What a run asks of the Role lookup.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lookup<'a> {
    pub(crate) name: &'a str,
    /// A job of the account whose brief is wanted (`--job`).
    pub(crate) job: Option<&'a str>,
    /// The project, when the run is in one.
    pub(crate) root: Option<&'a Path>,
    /// Whether the account may be asked (a dry run asks nobody).
    pub(crate) online: bool,
}

/// Finds the Role and settles what a run takes from it: the project's pack, then the account,
/// then the built-in catalogue.
///
/// # Errors
/// [`Error::Usage`] for a name or job that is not an id, or a Role found nowhere;
/// [`Error::Api`] when the account fails in a way other than "no such Role".
pub(crate) async fn resolve(g: &Global, env: &Env, l: Lookup<'_>) -> Result<Applied, Error> {
    let Lookup {
        name,
        job,
        root,
        online,
    } = l;
    job_segment(name).map_err(|_| {
        Error::Usage("a Role name has only letters, digits, '-' and '_' (at most 128)".into())
    })?;
    if let Some(j) = job {
        job_segment(j)?;
    }
    let mut found = root
        .and_then(|r| project_role(r, name))
        .map(|p| (Source::Project, p));
    let session = if online && (found.is_none() || job.is_some()) {
        session(g, env).await.ok()
    } else {
        None
    };
    if let (None, Some(s)) = (&found, &session) {
        let path = format!("{PATH}/{name}");
        match s.api.send(Method::Get, &path, &[], None).await {
            Ok(raw) => {
                let v: Value = raw.json()?;
                let p = account_pack(&v).ok_or_else(|| {
                    Error::Failed(format!("the account's Role `{name}` has no text"))
                })?;
                found = Some((Source::Account, p));
            }
            Err(e) if e.status() == Some(404) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let Some((source, pack)) =
        found.or_else(|| catalogue_role(name).map(|p| (Source::Catalogue, p)))
    else {
        let looked = if session.is_some() {
            "in atlas/roles/, on the account or in the built-in catalogue"
        } else if online {
            "in atlas/roles/ or in the built-in catalogue, and not signed in to look on the account (`iohr login`)"
        } else {
            "in atlas/roles/ or in the built-in catalogue"
        };
        return Err(Error::Usage(format!("no Role `{name}` {looked}")));
    };
    let mut applied = Applied {
        name: name.to_owned(),
        source,
        brief: Brief::Role,
        model: usable(pack.model),
        text: pack.text,
        job: None,
        notes: root.map(|r| notes::read(r, name)).unwrap_or_default(),
    };
    if let (Some(j), Some(s)) = (job, &session) {
        let body = json!({"job_id": j, "recall_budget": RECALL_BUDGET});
        let path = format!("{PATH}/{name}/brief");
        match s.api.send(Method::Post, &path, &[], Some(&body)).await {
            Ok(raw) => {
                let v: Value = raw.json()?;
                match v.get("text").and_then(Value::as_str).and_then(clean_text) {
                    Some(t) => {
                        applied.text = bound(&t);
                        applied.brief = Brief::Job;
                        applied.job = Some(j.to_owned());
                    }
                    None => Out::note(
                        "iohr: the job's brief came back empty; the Role's own text is used",
                    ),
                }
            }
            Err(e) => Out::note(&format!(
                "iohr: no brief for job {j} ({}); the Role's own text is used",
                status_of(&e)
            )),
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(model: &str) -> String {
        format!(
            "---\nname: rust-engineer\nparent: base-engineer\n{model}---\n# Rust\n\nIntro line.\n\n\
             ## Core: always in the brief\n\n### Errors\n\n- Return errors, never panic.\n  Source: https://x\n\n\
             ## Recall: found by topic\n\n- Use cargo nextest.\n"
        )
    }

    #[test]
    fn a_pack_gives_its_core_and_not_its_recall_or_intro() {
        let p = from_pack(&pack("model: haiku\n"), "rust-engineer").unwrap();
        assert!(p.text.contains("Return errors, never panic."));
        assert!(!p.text.contains("nextest"));
        assert!(!p.text.contains("Intro line"));
        assert_eq!(p.model.as_deref(), Some("haiku"));
    }

    #[test]
    fn a_pack_without_a_model_takes_the_catalogue_s_and_a_stranger_has_none() {
        let p = from_pack(&pack(""), "rust-engineer").unwrap();
        assert_eq!(p.model.as_deref(), Some("sonnet"));
        let own = "---\nname: mine\n---\nBe brief.\n";
        let p = from_pack(own, "mine").unwrap();
        assert_eq!(p.model, None);
        assert!(p.text.starts_with("Be brief."));
    }

    #[test]
    fn a_pack_of_another_role_is_not_this_one() {
        assert!(from_pack(&pack(""), "security").is_none());
        assert!(from_pack("no front matter", "mine").is_none());
    }

    #[test]
    fn text_is_bounded_on_a_character_boundary() {
        let long = "é".repeat(TEXT_MAX + 50);
        let b = bound(&long);
        assert!(b.starts_with('é'));
        assert_eq!(b.chars().filter(|c| *c == 'é').count(), TEXT_MAX);
        assert!(b.ends_with("characters]"));
        assert_eq!(bound("  short \n"), "short");
    }

    #[test]
    fn the_account_s_role_is_its_core_and_model() {
        let v = json!({"personality": {"name": "x", "model": "opus"}, "core": "## Core\n- Prove it.\u{1b}\n"});
        let p = account_pack(&v).unwrap();
        assert_eq!(p.model.as_deref(), Some("opus"));
        assert_eq!(p.text, "## Core\n- Prove it.");
        assert!(account_pack(&json!({"core": "  "})).is_none());
    }

    #[test]
    fn a_model_that_would_break_the_command_line_is_no_model() {
        assert_eq!(usable(Some("opus".into())).as_deref(), Some("opus"));
        assert_eq!(usable(Some("--evil".into())), None);
        assert_eq!(usable(Some("two words".into())), None);
        assert_eq!(usable(None), None);
    }

    #[test]
    fn json_names_the_role_and_never_its_text() {
        let a = Applied {
            name: "security".into(),
            source: Source::Account,
            brief: Brief::Job,
            model: Some("sonnet".into()),
            text: "secret text".into(),
            job: Some("job_1".into()),
            notes: "a handoff".into(),
        };
        let v = a.json();
        assert_eq!(v["name"], "security");
        assert_eq!(v["source"], "account");
        assert_eq!(v["brief"], "job");
        assert!(!v.to_string().contains("secret text"));
        assert!(a.system_text().contains("secret text"));
        assert!(a.system_text().contains("a handoff"));
        assert!(!v.to_string().contains("a handoff"));
    }

    #[test]
    fn a_role_of_the_catalogue_resolves_with_no_pack_and_no_account() {
        let p = catalogue_role("security").unwrap();
        assert_eq!(p.model.as_deref(), Some("sonnet"));
        assert!(!p.text.is_empty() && p.text.chars().count() <= TEXT_MAX + 60);
        assert!(catalogue_role("nobody").is_none());
    }

    #[test]
    fn the_project_is_searched_by_front_matter_name() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join(roles::DIR);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("a-file.md"),
            "---\nname: mine\nmodel: haiku\n---\nHelp.\n",
        )
        .unwrap();
        let p = project_role(dir.path(), "mine").unwrap();
        assert_eq!(
            (p.model.as_deref(), p.text.as_str()),
            (Some("haiku"), "Help.")
        );
        assert!(project_role(dir.path(), "other").is_none());
    }
}
