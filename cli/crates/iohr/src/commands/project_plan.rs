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
enum Access<'a> {
    In(&'a Api),
    /// No session, and why, in words fit to follow "not checked:" or begin a sentence.
    Without(String),
}

/// The session init would call with. Only a machine with no profile and no token is "not
/// signed in"; any other refusal (a token for another audience, an expired one, a store
/// that cannot be read) says what it is.
async fn access(g: &Global, env: &Env) -> Result<std::sync::Arc<crate::context::Session>, String> {
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

/// The step: make or find the plan and write its id. `None` when the project has no
/// `coordination.toml`, so there is nowhere to write one.
pub(crate) async fn run(
    g: &Global,
    env: &Env,
    dir: &Path,
    name: &str,
    chosen: Option<&str>,
    skip: bool,
) -> Option<Step> {
    let file = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&file).ok()?;
    let title = "The project's Planner plan";
    if skip {
        return Some(step(
            "plan",
            title,
            "skipped",
            format!("left empty (--no-plan); {}", later(name)),
            "",
        ));
    }
    let session = access(g, env).await;
    let api = match &session {
        Ok(s) => Access::In(&s.api),
        Err(why) => Access::Without(why.clone()),
    };
    Some(settle(&api, &file, &text, name, chosen, title).await)
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
    let fail = |said: String| step("plan", title, "failed", said, "");
    if let Some(id) = chosen
        && let Err(e) = plan_segment(id)
    {
        return fail(e.to_string());
    }
    // A plan already named in the file stays: init may run again on a folder it made.
    if chosen.is_none()
        && let Some(id) = plan_id_in(text)
    {
        return step("plan", title, "done", format!("already set: {id}"), "");
    }
    let (id, said) = match (chosen, api) {
        (Some(id), Access::In(api)) => match Plans::new(api, None).get(id).await {
            Ok(_) => (id.to_owned(), format!("chosen: {id}")),
            Err(e) => {
                // The files are already written when this runs: say so, and how to attach it.
                return fail(format!(
                    "the plan {id} was not found or cannot be seen ({}); the project's files are written, and {MANIFEST} has no plan yet: put a plan id you can see under [plan] id, or {}",
                    first_line(&e.to_string()),
                    later(name)
                ));
            }
        },
        (Some(id), Access::Without(why)) => {
            (id.to_owned(), format!("chosen: {id} (not checked: {why})"))
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
                (id.clone(), format!("{how}: {id}"))
            }
            Err(e) => {
                return step(
                    "plan",
                    title,
                    "skipped",
                    format!(
                        "the Planner did not answer ({}); {}",
                        first_line(&e.to_string()),
                        later(name)
                    ),
                    "",
                );
            }
        },
        (None, Access::Without(why)) => {
            return step(
                "plan",
                title,
                "skipped",
                format!("{why}; {}", later(name)),
                "",
            );
        }
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
        Err(e) => fail(format!("{said}, but {MANIFEST} could not be written: {e}")),
    }
}

fn first_line(s: &str) -> String {
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
