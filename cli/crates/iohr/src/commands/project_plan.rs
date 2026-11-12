//! The Planner step of `iohr project init` (RFC 0211, ADR 0221): a project built from the
//! `coordinated` template belongs to a Planner plan, so init makes the plan (or finds the
//! one it made before) through the SDK and writes its id into `coordination.toml`.
//!
//! Signed out, offline or refused, init still works: the id stays empty and the step says
//! how to set it later. The plan is found by `external_id` `project:<name>`, so a second
//! init of the same name makes no second plan.

use std::path::Path;

use toml_edit::{DocumentMut, value};

use crate::context::{Api, Ctx, run_profile, shared_session};
use crate::planner::{Plans, plan_segment};
use crate::project::{Step, step};
use crate::{Env, cli::Global};

/// The manifest the coordinated template writes.
pub(crate) const MANIFEST: &str = "coordination.toml";

/// The id `[plan] id` holds, when it is set.
pub(crate) fn plan_id_in(text: &str) -> Option<String> {
    let doc: DocumentMut = text.parse().ok()?;
    doc.get("plan")?
        .get("id")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `text` with `[plan] id` set to `id`, every comment and the rest as they were.
pub(crate) fn with_plan_id(text: &str, id: &str) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{MANIFEST} is not valid TOML: {e}"))?;
    if !doc.contains_table("plan") {
        doc["plan"] = toml_edit::table();
    }
    doc["plan"]["id"] = value(id);
    Ok(doc.to_string())
}

/// How to set the id later, said whenever the step could not.
fn later(name: &str) -> String {
    format!(
        "set it later: run `iohr login`, then `iohr coordinator plan create {name} --external-id project:{name}` and put the id under [plan] in {MANIFEST}"
    )
}

/// What the step can call the Planner with: a session, or the reason there is none.
pub(super) enum Access<'a> {
    In(&'a Api),
    /// No session, and why, in words fit to follow "not checked:" or begin a sentence.
    Without(String),
}

/// The session init would call with. Only a machine with no profile and no token is "not
/// signed in"; any other refusal (a token for another audience, an expired one, a store
/// that cannot be read) says what it is.
pub(super) async fn access(
    g: &Global,
    env: &Env,
) -> Result<std::sync::Arc<crate::context::Session>, String> {
    let ctx = Ctx::load(g).map_err(|e| first_line(&e.to_string()))?;
    if env.token.is_none() && run_profile(g, env, &ctx).is_none() {
        return Err("you are not signed in".to_owned());
    }
    shared_session(g, env).await.map_err(|e| {
        format!(
            "your session cannot be used ({})",
            first_line(&e.to_string())
        )
    })
}

/// What the step settled: its report, and the plan the project's items go in (when there is
/// one), so a later step files into it without looking the plan up again.
pub(crate) struct Planned {
    pub step: Step,
    pub id: Option<String>,
}

/// The step: make or find the plan, and write its id when the project has a
/// `coordination.toml`. A project without one (the blank template) still gets the plan,
/// because the project's shape item is filed in it; the id is then only passed on.
pub(crate) async fn run(
    g: &Global,
    env: &Env,
    dir: &Path,
    name: &str,
    chosen: Option<&str>,
    skip: bool,
    local: bool,
) -> Planned {
    let file = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&file).ok();
    let title = "The project's Planner plan";
    // `--no-sync` keeps the project on this machine: nothing is made in the account, so no plan
    // is made or looked up. A project with a `coordination.toml` (every template but blank) keeps
    // its `[plan] id` empty and is told how to set it; one without would only have had the plan
    // for the shape item, so it says nothing is filed in it.
    // A plan id you chose with `--plan-id` is still written (and checked when signed in).
    if local && !skip && (text.is_none() || chosen.is_none()) {
        let said = if text.is_some() {
            format!(
                "left empty (--no-sync keeps this project on this machine); {}",
                later(name)
            )
        } else {
            "not made (--no-sync keeps this project on this machine), so nothing is filed in it"
                .to_owned()
        };
        return Planned {
            step: step("plan", title, "skipped", said, ""),
            id: None,
        };
    }
    if skip {
        let said = if text.is_some() {
            format!("left empty (--no-plan); {}", later(name))
        } else {
            "not made (--no-plan), so nothing is filed in it".to_owned()
        };
        return Planned {
            step: step("plan", title, "skipped", said, ""),
            id: None,
        };
    }
    let session = access(g, env).await;
    let api = match &session {
        Ok(s) => Access::In(&s.api),
        Err(why) => Access::Without(why.clone()),
    };
    match text {
        Some(text) => {
            let step = settle(&api, &file, &text, name, chosen, title).await;
            let id = std::fs::read_to_string(&file)
                .ok()
                .and_then(|t| plan_id_in(&t));
            Planned { step, id }
        }
        None => settle_bare(&api, name, chosen, title).await,
    }
}

/// The plan to use: the chosen one (checked when signed in), or the one made or found by its
/// `external_id`. `Err` is the finished step, `Ok` the id and what to say about it.
async fn pick(
    api: &Access<'_>,
    name: &str,
    chosen: Option<&str>,
    title: &str,
    has_manifest: bool,
) -> Result<(String, String), Step> {
    if let Some(id) = chosen
        && let Err(e) = plan_segment(id)
    {
        return Err(step("plan", title, "failed", e.to_string(), ""));
    }
    match (chosen, api) {
        (Some(id), Access::In(api)) => match Plans::new(api, None).get(id).await {
            Ok(_) => Ok((id.to_owned(), format!("chosen: {id}"))),
            Err(e) => {
                // The files are already written when this runs: say so, and how to attach it.
                let after = if has_manifest {
                    format!(
                        "{MANIFEST} has no plan yet: put a plan id you can see under [plan] id, or {}",
                        later(name)
                    )
                } else {
                    later(name)
                };
                Err(step(
                    "plan",
                    title,
                    "failed",
                    format!(
                        "the plan {id} was not found or cannot be seen ({}); the project's files are written, and {after}",
                        first_line(&e.to_string()),
                    ),
                    "",
                ))
            }
        },
        (Some(id), Access::Without(why)) => {
            Ok((id.to_owned(), format!("chosen: {id} (not checked: {why})")))
        }
        (None, Access::In(api)) => match Plans::new(api, None)
            .create(name, Some(&format!("project:{name}")), None)
            .await
        {
            Ok((plan, created)) => {
                let id = plan
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let how = if created { "made" } else { "found" };
                Ok((id.clone(), format!("{how}: {id}")))
            }
            Err(e) => Err(step(
                "plan",
                title,
                "skipped",
                format!(
                    "the Planner did not answer ({}); {}",
                    first_line(&e.to_string()),
                    later(name)
                ),
                "",
            )),
        },
        (None, Access::Without(why)) => Err(step(
            "plan",
            title,
            "skipped",
            format!("{why}; {}", later(name)),
            "",
        )),
    }
}

/// [`run`] on a project with no manifest: the plan is made or found, and nothing is written.
pub(super) async fn settle_bare(
    api: &Access<'_>,
    name: &str,
    chosen: Option<&str>,
    title: &str,
) -> Planned {
    match pick(api, name, chosen, title, false).await {
        Ok((id, said)) => Planned {
            step: step(
                "plan",
                title,
                "done",
                format!("{said} (this project has no {MANIFEST}, so the id is not written down)"),
                "",
            ),
            id: Some(id),
        },
        Err(step) => Planned { step, id: None },
    }
}

/// [`run`] on a manifest already read: the choice, the write and the report.
async fn settle(
    api: &Access<'_>,
    file: &Path,
    text: &str,
    name: &str,
    chosen: Option<&str>,
    title: &str,
) -> Step {
    // A plan already named in the file stays: init may run again on a folder it made.
    if chosen.is_none()
        && let Some(id) = plan_id_in(text)
    {
        return step("plan", title, "done", format!("already set: {id}"), "");
    }
    let (id, said) = match pick(api, name, chosen, title, true).await {
        Ok(x) => x,
        Err(step) => return step,
    };
    let written =
        with_plan_id(text, &id).and_then(|t| std::fs::write(file, t).map_err(|e| e.to_string()));
    match written {
        Ok(()) => step(
            "plan",
            title,
            "done",
            format!("{said}, written to {MANIFEST}"),
            "",
        ),
        Err(e) => step(
            "plan",
            title,
            "failed",
            format!("{said}, but {MANIFEST} could not be written: {e}"),
            "",
        ),
    }
}

pub(super) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use iohr_auth::{Redacted, StaticToken};
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::context::AnyCredential;

    const FILE: &str = "# keep me\nversion = 1\n\n[plan]\n# the plan\nid = \"\"\nitem = \"\"\n";

    fn api(server: &MockServer) -> Api {
        let cred = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "t".to_owned(),
        ))));
        Api::new(&server.uri(), cred, false).unwrap()
    }

    fn manifest(dir: &Path) -> std::path::PathBuf {
        let f = dir.join(MANIFEST);
        std::fs::write(&f, FILE).unwrap();
        f
    }

    #[test]
    fn the_id_is_written_into_the_plan_table_and_the_comments_stay() {
        assert_eq!(plan_id_in(FILE), None);
        let out = with_plan_id(FILE, "plan_01A").unwrap();
        assert_eq!(plan_id_in(&out).as_deref(), Some("plan_01A"));
        assert!(out.contains("# keep me") && out.contains("# the plan"));
        assert!(out.contains("item = \"\""));
        assert!(with_plan_id("x = [", "plan_1").is_err());
        let bare = with_plan_id("version = 1\n", "plan_2").unwrap();
        assert_eq!(plan_id_in(&bare).as_deref(), Some("plan_2"));
    }

    #[tokio::test]
    async fn a_signed_in_init_makes_the_plan_once_and_writes_its_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/coordinator/plans"))
            .and(body_partial_json(
                json!({"title": "my-shop", "external_id": "project:my-shop"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"plan": {"id": "plan_01SHOP", "title": "my-shop"}, "created": true}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let d = tempfile::tempdir().unwrap();
        let f = manifest(d.path());
        let a = api(&server);
        let st = settle(&Access::In(&a), &f, FILE, "my-shop", None, "t").await;
        assert_eq!(st.state, "done", "{}", st.said);
        assert!(st.said.starts_with("made: plan_01SHOP"), "{}", st.said);
        let written = std::fs::read_to_string(&f).unwrap();
        assert_eq!(plan_id_in(&written).as_deref(), Some("plan_01SHOP"));

        // Run again on the written file: it keeps the id and makes no second plan (the
        // mock allows one call).
        let again = settle(&Access::In(&a), &f, &written, "my-shop", None, "t").await;
        assert_eq!(again.state, "done");
        assert_eq!(again.said, "already set: plan_01SHOP");
    }

    #[tokio::test]
    async fn signed_out_or_refused_leaves_the_id_empty_and_says_how_to_set_it() {
        let d = tempfile::tempdir().unwrap();
        let f = manifest(d.path());
        let st = settle(
            &Access::Without("you are not signed in".to_owned()),
            &f,
            FILE,
            "my-shop",
            None,
            "t",
        )
        .await;
        assert_eq!(st.state, "skipped");
        assert!(
            st.said
                .contains("iohr coordinator plan create my-shop --external-id project:my-shop"),
            "{}",
            st.said
        );
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            FILE,
            "the file is untouched"
        );

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_raw(
                r#"{"code":"forbidden","error":"no","details":[]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let a = api(&server);
        let st = settle(&Access::In(&a), &f, FILE, "my-shop", None, "t").await;
        assert_eq!(st.state, "skipped", "{}", st.said);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), FILE);
    }

    #[tokio::test]
    async fn a_chosen_plan_is_checked_when_signed_in_and_refused_when_unknown() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/coordinator/plans/plan_01OK"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"plan": {"id": "plan_01OK"}, "items": [], "notes": []})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/coordinator/plans/plan_01GONE"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"code":"not_found","error":"no such plan","details":[]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let d = tempfile::tempdir().unwrap();
        let f = manifest(d.path());
        let a = api(&server);
        let ok = settle(&Access::In(&a), &f, FILE, "p", Some("plan_01OK"), "t").await;
        assert_eq!(ok.state, "done", "{}", ok.said);
        assert_eq!(
            plan_id_in(&std::fs::read_to_string(&f).unwrap()).as_deref(),
            Some("plan_01OK")
        );
        let inner = d.path().join("p");
        std::fs::create_dir(&inner).unwrap();
        let f2 = manifest(&inner);
        let gone = settle(&Access::In(&a), &f2, FILE, "p", Some("plan_01GONE"), "t").await;
        assert_eq!(gone.state, "failed");
        assert!(
            gone.said.contains("files are written") && gone.said.contains("[plan] id"),
            "{}",
            gone.said
        );
        assert_eq!(std::fs::read_to_string(&f2).unwrap(), FILE);
        let bad = settle(&Access::In(&a), &f2, FILE, "p", Some("../x"), "t").await;
        assert_eq!(bad.state, "failed");
        // Signed out, a chosen id is written as given and said to be unchecked.
        let off = settle(
            &Access::Without("you are not signed in".to_owned()),
            &f2,
            FILE,
            "p",
            Some("plan_01OFF"),
            "t",
        )
        .await;
        assert_eq!(off.state, "done");
        assert!(off.said.contains("not checked"), "{}", off.said);
        // A session that cannot be used says why, not "not signed in".
        let bad = settle(
            &Access::Without("your session cannot be used (wrong audience)".to_owned()),
            &f2,
            FILE,
            "p",
            None,
            "t",
        )
        .await;
        assert_eq!(bad.state, "skipped");
        assert!(
            bad.said.contains("wrong audience") && !bad.said.contains("not signed in"),
            "{}",
            bad.said
        );
    }
}
