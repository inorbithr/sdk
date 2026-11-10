//! The project's shape (PRD 0210): monolith, microservices or combined.
//!
//! Init asks once and files the question as an item in the project's Planner plan
//! (`project-shape`), through the same client every other Planner step uses
//! (`crate::planner`). Asked at a terminal, the answer is recorded and the item ends done.
//! Headless, the item stays open (`proposed`) for the Mac app or the phone to answer.
//! The answer goes to `inorbit.toml` (`[project] shape`) and to the knowledge base as a fact,
//! so `?` and `??` know it.

use std::path::Path;

use crate::cli::Global;
use crate::error::Error;
use crate::memory::store::{self, Kind as MemKind};
use crate::planner::{Outcome, Plans, Step as Item, default_holder, find_item};
use crate::project::place;
use crate::project::{Step, step};
use crate::ui::{Choice, Ui};
use crate::{Env, commands::project_plan};

/// The item's slug in the plan.
pub(super) const SLUG: &str = "project-shape";
const TITLE: &str = "Project shape: monolith, microservices or combined?";
const STEP: &str = "Project shape";
/// The key under `[project]` in `inorbit.toml`.
const KEY: &str = "shape";
const SHAPES: [(&str, &str, &str); 3] = [
    (
        "monolith",
        "Monolith",
        "one codebase, built and shipped as one",
    ),
    (
        "microservices",
        "Microservices",
        "many services, each built and shipped on its own",
    ),
    (
        "combined",
        "Combined",
        "a core plus separate services around it",
    ),
];

/// The sentence the knowledge base keeps, so a question about the project is answered in its
/// shape.
fn fact(shape: &str) -> String {
    let what = match shape {
        "monolith" => "a monolith: one codebase, built and shipped as one",
        "microservices" => "microservices: many services, each built and shipped on its own",
        _ => "combined: a core plus separate services around it",
    };
    format!("This project is {what}.")
}

/// What init does, for a dry run.
pub(super) fn would(flag: Option<&str>) -> String {
    match flag {
        Some(s) => format!(
            "record the project shape ({s}) in inorbit.toml and the knowledge base, and file the {SLUG} item in the Planner plan as done"
        ),
        None => format!(
            "ask the project's shape (monolith, microservices or combined) and file the {SLUG} item in the Planner plan"
        ),
    }
}

/// The answer: the flag, the one already recorded, or the question at a terminal. `None` when
/// nobody answered (the item stays open).
pub(super) fn pick(
    flag: Option<&str>,
    interactive: bool,
    ui: Ui,
    dir: &Path,
) -> Result<Option<String>, Error> {
    if let Some(s) = flag {
        return Ok(Some(s.to_owned()));
    }
    if let Some(s) = place::project_key(dir, KEY) {
        return Ok(Some(s));
    }
    if !interactive {
        return Ok(None);
    }
    let choices: Vec<Choice> = SHAPES
        .iter()
        .map(|(value, label, hint)| Choice {
            value: (*value).into(),
            label: (*label).into(),
            hint: (*hint).into(),
            available: true,
            key: None,
        })
        .collect();
    ui.select("What shape is this project?", &choices, "monolith")
        .map(Some)
}

/// Records the answer where the CLI reads it: `[project] shape`, and one knowledge entry.
fn record(dir: &Path, shape: &str) -> Result<(), String> {
    place::write_project_key(dir, KEY, shape)?;
    let store = store::Store::project(dir);
    store::remember(
        &store,
        &fact(shape),
        MemKind::Fact,
        "answered in iohr project init",
        &[],
        false,
    )
    .map_err(|e| format!("cannot write the memory entry: {e}"))?;
    Ok(())
}

/// The step: record the answer (when there is one) and file the item. Signed out or offline,
/// the answer is still kept locally and the step says the item was not filed.
pub(super) async fn run(g: &Global, env: &Env, dir: &Path, answer: Option<&str>) -> Step {
    let plan = std::fs::read_to_string(dir.join(project_plan::MANIFEST))
        .ok()
        .and_then(|t| project_plan::plan_id_in(&t));
    let already = place::project_key(dir, KEY);
    let mut kept = String::new();
    if let Some(shape) = answer {
        if already.as_deref() == Some(shape) {
            kept = format!("{shape}, already in inorbit.toml");
        } else {
            if let Err(e) = record(dir, shape) {
                return step("shape", STEP, "failed", e, "");
            }
            kept = format!("{shape}, written to inorbit.toml and the knowledge base");
        }
    }
    let Some(plan) = plan else {
        return unfiled(answer, &kept, "the project has no Planner plan yet");
    };
    let session = project_plan::access(g, env).await;
    let api = match &session {
        Ok(s) => project_plan::Access::In(&s.api),
        Err(why) => project_plan::Access::Without(why.clone()),
    };
    settle(&api, &plan, answer, &kept).await
}

fn unfiled(answer: Option<&str>, kept: &str, why: &str) -> Step {
    match answer {
        Some(_) => step(
            "shape",
            STEP,
            "skipped",
            format!("{kept}; the {SLUG} item was not filed ({why})"),
            "",
        ),
        None => step(
            "shape",
            STEP,
            "skipped",
            format!(
                "not asked here and {why}; run `iohr init --shape monolith|microservices|combined` later"
            ),
            "",
        ),
    }
}

/// The Planner half: file the item as done when there is an answer, else leave it open.
async fn settle(
    api: &project_plan::Access<'_>,
    plan: &str,
    answer: Option<&str>,
    kept: &str,
) -> Step {
    let api = match api {
        project_plan::Access::In(api) => api,
        project_plan::Access::Without(why) => return unfiled(answer, kept, why),
    };
    let plans = Plans::new(api, None);
    let holder = default_holder("init");
    let item = Item {
        plan,
        item: SLUG,
        title: TITLE,
        kind: "question",
        holder: &holder,
    };
    let said = if let Some(shape) = answer {
        let done = plans
            .run_step(&item, || async { Ok(format!("shape: {shape}")) })
            .await;
        match done {
            Ok(Outcome::Done(_)) => format!("{kept}; the {SLUG} item is done"),
            Ok(Outcome::Skipped) => format!("{kept}; the {SLUG} item was already done"),
            Err(e) => {
                return step(
                    "shape",
                    STEP,
                    "skipped",
                    format!(
                        "{kept}; the {SLUG} item could not be filed ({})",
                        project_plan::first_line(&e.to_string())
                    ),
                    "",
                );
            }
        }
    } else {
        let filed = async {
            let got = plans.get(plan).await?;
            if find_item(&got, SLUG).is_some() {
                Ok::<_, Error>("is already in the plan")
            } else {
                plans.add_item(&item).await?;
                Ok("was added to the plan")
            }
        }
        .await;
        match filed {
            Ok(what) => format!(
                "not answered here: the {SLUG} item {what}, open for the Mac app or the phone"
            ),
            Err(e) => {
                return step(
                    "shape",
                    STEP,
                    "skipped",
                    format!(
                        "the Planner did not answer ({}); the {SLUG} item was not filed",
                        project_plan::first_line(&e.to_string())
                    ),
                    "",
                );
            }
        }
    };
    step("shape", STEP, "done", said, "")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use iohr_auth::{Redacted, StaticToken};
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::context::{AnyCredential, Api};

    fn api(server: &MockServer) -> Api {
        let cred = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "t".to_owned(),
        ))));
        Api::new(&server.uri(), cred, false).unwrap()
    }

    async fn plan_with(server: &MockServer, items: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/v1/coordinator/plans/plan_01A"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": items})))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn headless_the_item_is_filed_open_and_not_claimed_or_done() {
        let server = MockServer::start().await;
        plan_with(&server, json!([])).await;
        Mock::given(method("PUT"))
            .and(path("/v1/coordinator/plans/plan_01A/items/project-shape"))
            .and(body_partial_json(
                json!({"item": {"kind": "question", "status": "proposed"}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"item": {}})))
            .expect(1)
            .mount(&server)
            .await;
        let a = api(&server);
        let st = settle(&project_plan::Access::In(&a), "plan_01A", None, "").await;
        assert_eq!(st.state, "done", "{}", st.said);
        assert!(
            st.said.contains("open for the Mac app or the phone"),
            "{}",
            st.said
        );
        // No claim and no status change: the mock server has no route for them and the
        // single PUT above is all it saw.
    }

    #[tokio::test]
    async fn an_item_already_in_the_plan_is_not_added_twice() {
        let server = MockServer::start().await;
        plan_with(
            &server,
            json!([{"item_id": "project-shape", "status": "proposed"}]),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path_regex("/items/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .mount(&server)
            .await;
        let a = api(&server);
        let st = settle(&project_plan::Access::In(&a), "plan_01A", None, "").await;
        assert_eq!(st.state, "done", "{}", st.said);
        assert!(st.said.contains("already in the plan"), "{}", st.said);
    }

    #[tokio::test]
    async fn an_answer_ends_the_item_done_with_a_note() {
        let server = MockServer::start().await;
        plan_with(
            &server,
            json!([{"item_id": "project-shape", "status": "proposed"}]),
        )
        .await;
        Mock::given(method("POST"))
            .and(path_regex("/items/project-shape/claim$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex("/items/project-shape/notes$"))
            .and(body_partial_json(json!({"body": "done: shape: combined"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex("/items/project-shape$"))
            .and(body_partial_json(json!({"item": {"status": "done"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"item": {}})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex("/items/project-shape$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"item": {}})))
            .mount(&server)
            .await;
        let a = api(&server);
        let st = settle(
            &project_plan::Access::In(&a),
            "plan_01A",
            Some("combined"),
            "combined, written",
        )
        .await;
        assert_eq!(st.state, "done", "{}", st.said);
        assert!(st.said.contains("item is done"), "{}", st.said);
    }

    #[tokio::test]
    async fn signed_out_the_answer_is_kept_and_the_step_says_the_item_was_not_filed() {
        let st = settle(
            &project_plan::Access::Without("you are not signed in".into()),
            "plan_01A",
            Some("monolith"),
            "monolith, written",
        )
        .await;
        assert_eq!(st.state, "skipped");
        assert!(st.said.contains("not filed") && st.said.contains("not signed in"));
    }

    #[test]
    fn the_answer_is_recorded_once_and_the_fact_says_it() {
        let d = tempfile::tempdir().unwrap();
        record(d.path(), "monolith").unwrap();
        assert_eq!(
            place::project_key(d.path(), KEY).as_deref(),
            Some("monolith")
        );
        assert!(fact("combined").contains("combined"));
    }
}
