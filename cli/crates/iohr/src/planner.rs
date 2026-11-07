//! The Planner through the public API (RFC 0166, RFC 0211): plans, their items, claims
//! and notes, and the one helper every build step runs through, [`Plans::run_step`].
//!
//! `iohr coordinator plan|item` is the command line over this, `iohr project init` makes
//! and finds a project's plan with it, and the `iohr auto` slices (ADR 0221) call
//! [`Plans::run_step`] so each build step is a Planner item with a live state.
//!
//! The coordinator stores the plan; every call here is a REST route of its
//! `CreatePlan`, `ListPlans`, `GetPlan`, `UpsertPlanItem`, `ClaimPlanItem`,
//! `ReleasePlanItem` and `NotePlanItem`, under the caller's own rights (a plan the caller
//! may not see is not found).
//!
//! # A step's states
//!
//! The Planner's words for an item are `proposed`, `building`, `ready`, `done` and
//! `dropped`; it has no failed. A step moves like this:
//!
//! ```text
//! (missing) --add--> proposed --claim--> proposed+claimed --status--> building
//! building --work ok--> note "done: ..." --status--> done   (the claim clears)
//! building --work fails--> note "failed: ..." --status--> proposed --release--> proposed
//! done --run again--> skipped            dropped --run again--> refused
//! building, another holder's live claim --run--> refused, naming the holder
//! ```
//!
//! A failed step goes back to `proposed`, never `dropped`: dropped is final and counts as
//! a failure of the plan, while proposed with a `failed:` note is claimable again.

use std::future::Future;

use inorbithr::Method;
use serde_json::{Map, Value, json};

use crate::context::Api;
use crate::error::Error;

/// The longest note the coordinator takes.
const NOTE_MAX: usize = 2000;

/// A claim's holder is 1 to 120 characters.
const HOLDER_MAX: usize = 120;

/// What a claimed step holds its item for, in seconds, unless the caller says.
const LEASE_SECONDS: u32 = 3600;

/// The coordinator's refusal of a signed-out or API-key caller, with what to do.
pub(crate) fn scoped(e: inorbithr::Error) -> Error {
    if matches!(e.status(), Some(401 | 403)) {
        Error::with_hint(
            e,
            "The coordinator serves signed-in people: run `iohr login`; an API key cannot use it.",
        )
    } else {
        e.into()
    }
}

/// A plan id as one path segment: letters, digits, `-` and `_`.
pub(crate) fn plan_segment(v: &str) -> Result<&str, Error> {
    if !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(v)
    } else {
        Err(Error::Usage(
            "a plan id has only letters, digits, '-' and '_' (it looks like plan_...)".into(),
        ))
    }
}

/// An item slug: 1 to 80 of letters, digits, `_`, `.` and `-` (the coordinator's rule),
/// and not `.` or `..`, so it stays one path segment.
pub(crate) fn item_slug(v: &str) -> Result<&str, Error> {
    if (1..=80).contains(&v.len())
        && v != "."
        && v != ".."
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        Ok(v)
    } else {
        Err(Error::Usage(
            "an item slug is 1 to 80 of A-Z, a-z, 0-9, '_', '.' and '-'".into(),
        ))
    }
}

/// Who holds a claim when the caller names nobody: `iohr:` and the profile's label,
/// within the coordinator's 120 characters.
pub(crate) fn default_holder(label: &str) -> String {
    let mut h = format!("iohr:{label}");
    if h.len() > HOLDER_MAX {
        let mut end = HOLDER_MAX;
        while !h.is_char_boundary(end) {
            end -= 1;
        }
        h.truncate(end);
    }
    h
}

/// A note's text within the coordinator's 2000 characters, one line of reason cut short:
/// control characters other than newlines gone, and a `…` where it was cut.
pub(crate) fn note_text(prefix: &str, text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    let mut out = format!("{prefix}{}", clean.trim());
    if out.chars().count() > NOTE_MAX {
        out = out.chars().take(NOTE_MAX - 1).collect();
        out.push('…');
    }
    out
}

/// The Planner of one account, called as the session's caller.
pub(crate) struct Plans<'a> {
    api: &'a Api,
    account: Option<&'a str>,
}

/// One build step: the item it is, and who builds it.
pub(crate) struct Step<'a> {
    pub(crate) plan: &'a str,
    pub(crate) item: &'a str,
    /// The title when the item has to be added.
    pub(crate) title: &'a str,
    /// The kind when the item has to be added.
    pub(crate) kind: &'a str,
    pub(crate) holder: &'a str,
}

/// How a step ended, when it did not fail.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The item was done already: nothing ran.
    Skipped,
    /// The work ran and the item is done, with the summary the work gave.
    Done(String),
}

impl<'a> Plans<'a> {
    pub(crate) fn new(api: &'a Api, account: Option<&'a str>) -> Self {
        Self { api, account }
    }

    fn with_account(&self, mut body: Map<String, Value>) -> Value {
        if let Some(a) = self.account {
            body.insert("account_id".into(), json!(a));
        }
        Value::Object(body)
    }

    fn query<'q>(&'q self, mut q: Vec<(&'q str, &'q str)>) -> Vec<(&'q str, &'q str)> {
        if let Some(a) = self.account {
            q.push(("account_id", a));
        }
        q
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, Error> {
        self.api
            .send(Method::Post, path, &[], Some(body))
            .await
            .and_then(|r| r.json())
            .map_err(scoped)
    }

    /// Create a plan, or find the one with the same `external_id` (it comes back with
    /// `created` false). Returns the plan and whether it is new.
    pub(crate) async fn create(
        &self,
        title: &str,
        external_id: Option<&str>,
        access: Option<&str>,
    ) -> Result<(Value, bool), Error> {
        let mut b = Map::new();
        b.insert("title".into(), json!(title));
        if let Some(e) = external_id {
            b.insert("external_id".into(), json!(e));
        }
        if let Some(a) = access {
            b.insert("access".into(), json!(a));
        }
        let v = self
            .post("/v1/coordinator/plans", &self.with_account(b))
            .await?;
        let plan = v
            .get("plan")
            .filter(|p| p.get("id").is_some())
            .cloned()
            .ok_or_else(|| Error::Failed("the API answered without a plan".into()))?;
        let created = v.get("created").and_then(Value::as_bool).unwrap_or(false);
        Ok((plan, created))
    }

    /// The plans the caller may see, newest change first.
    pub(crate) async fn list(&self, state: Option<&str>, limit: u32) -> Result<Value, Error> {
        let size = limit.to_string();
        let mut q = vec![("page_size", size.as_str())];
        if let Some(s) = state {
            q.push(("state", s));
        }
        self.api
            .get("/v1/coordinator/plans", &self.query(q))
            .await
            .map_err(scoped)
    }

    /// One plan with its items and their latest notes.
    pub(crate) async fn get(&self, plan: &str) -> Result<Value, Error> {
        let path = format!("/v1/coordinator/plans/{}", plan_segment(plan)?);
        self.api
            .get(&path, &self.query(Vec::new()))
            .await
            .map_err(scoped)
    }

    fn item_path(plan: &str, item: &str, tail: &str) -> Result<String, Error> {
        Ok(format!(
            "/v1/coordinator/plans/{}/items/{}{tail}",
            plan_segment(plan)?,
            item_slug(item)?
        ))
    }

    /// Create or change one item. `fields` is the whole item (a field left out is
    /// cleared); `expected_version` 0 writes whatever is there.
    pub(crate) async fn upsert(
        &self,
        plan: &str,
        item: &str,
        fields: Map<String, Value>,
        expected_version: i64,
        holder: &str,
    ) -> Result<Value, Error> {
        let path = Self::item_path(plan, item, "")?;
        let mut b = Map::new();
        b.insert("item".into(), Value::Object(fields));
        b.insert("expected_version".into(), json!(expected_version));
        b.insert("holder".into(), json!(holder));
        let body = self.with_account(b);
        let v: Value = self
            .api
            .send(Method::Put, &path, &[], Some(&body))
            .await
            .and_then(|r| r.json())
            .map_err(scoped)?;
        Ok(v.get("item").cloned().unwrap_or(v))
    }

    /// Claim an item for `holder`. The same holder renews; another's live claim is
    /// refused (409) unless `force`.
    pub(crate) async fn claim(
        &self,
        plan: &str,
        item: &str,
        holder: &str,
        seconds: Option<u32>,
        force: bool,
    ) -> Result<Value, Error> {
        let path = Self::item_path(plan, item, "/claim")?;
        let mut b = Map::new();
        b.insert("holder".into(), json!(holder));
        if let Some(s) = seconds {
            b.insert("seconds".into(), json!(s));
        }
        if force {
            b.insert("force".into(), json!(true));
        }
        self.post(&path, &self.with_account(b)).await
    }

    /// Give a claim up.
    pub(crate) async fn release(
        &self,
        plan: &str,
        item: &str,
        holder: &str,
        force: bool,
    ) -> Result<Value, Error> {
        let path = Self::item_path(plan, item, "/release")?;
        let mut b = Map::new();
        b.insert("holder".into(), json!(holder));
        if force {
            b.insert("force".into(), json!(true));
        }
        self.post(&path, &self.with_account(b)).await
    }

    /// Write a note on an item.
    pub(crate) async fn note(&self, plan: &str, item: &str, body: &str) -> Result<Value, Error> {
        let path = Self::item_path(plan, item, "/notes")?;
        let mut b = Map::new();
        b.insert("body".into(), json!(body));
        self.post(&path, &self.with_account(b)).await
    }

    /// Put an item into a status, keeping every other field. The write names the version
    /// read, so a change made in between is a conflict; it reads and writes again once.
    pub(crate) async fn set_status(
        &self,
        plan: &str,
        item: &str,
        status: &str,
        holder: &str,
    ) -> Result<Value, Error> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let got = self.get(plan).await?;
            let current = find_item(&got, item)
                .ok_or_else(|| Error::Failed(format!("the plan has no item {item}")))?;
            let version = current.get("version").and_then(Value::as_i64).unwrap_or(0);
            let mut fields = item_fields(current);
            fields.insert("status".into(), json!(status));
            match self.upsert(plan, item, fields, version, holder).await {
                Err(Error::Api { error, .. }) if attempt < 2 && error.status() == Some(409) => {}
                other => return other,
            }
        }
    }

    /// Run one build step as a Planner item: add it when the plan lacks it, skip it when
    /// it is done, claim it, mark it building, run `work`, then write a note and the
    /// status the result earned (see the module's states).
    ///
    /// `work` returns the summary for the closing note; that text is stored with the
    /// plan, so it is never customer data. A failed `work` is reported to the plan and
    /// its own error is returned unchanged.
    ///
    /// # Errors
    ///
    /// The plan, claim or status calls failing, an item another holder is building, a
    /// dropped item, or `work`'s own error.
    pub(crate) async fn run_step<F, Fut>(&self, step: &Step<'_>, work: F) -> Result<Outcome, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, Error>>,
    {
        let got = self.get(step.plan).await?;
        match find_item(&got, step.item) {
            Some(item) => {
                match item.get("status").and_then(Value::as_str).unwrap_or("") {
                    "done" => return Ok(Outcome::Skipped),
                    "dropped" => {
                        return Err(Error::Failed(format!(
                            "item {} was dropped; add it again under another slug",
                            step.item
                        )));
                    }
                    _ => {}
                }
                let held = item.get("holder").and_then(Value::as_str).unwrap_or("");
                if !held.is_empty() && held != step.holder {
                    let until = item.get("held_until").and_then(Value::as_str).unwrap_or("");
                    return Err(Error::Failed(format!(
                        "item {} is held by {held} until {until}; wait, or claim it with --force",
                        step.item
                    )));
                }
            }
            None => {
                let mut fields = Map::new();
                fields.insert("title".into(), json!(step.title));
                fields.insert("kind".into(), json!(step.kind));
                fields.insert("status".into(), json!("proposed"));
                self.upsert(step.plan, step.item, fields, 0, step.holder)
                    .await?;
            }
        }
        self.claim(
            step.plan,
            step.item,
            step.holder,
            Some(LEASE_SECONDS),
            false,
        )
        .await
        .map_err(|e| match e {
            Error::Api { error, .. } if error.status() == Some(409) => Error::Failed(format!(
                "item {} is held by another holder; wait, or claim it with --force",
                step.item
            )),
            other => other,
        })?;
        self.set_status(step.plan, step.item, "building", step.holder)
            .await?;
        match work().await {
            Ok(summary) => {
                self.note(step.plan, step.item, &note_text("done: ", &summary))
                    .await?;
                self.set_status(step.plan, step.item, "done", step.holder)
                    .await?;
                Ok(Outcome::Done(summary))
            }
            Err(e) => {
                // Reporting the failure must not hide it: the work's error is the one
                // the caller gets, whatever happens to the notes.
                let _ = self.fail(step, &e.to_string()).await;
                Err(e)
            }
        }
    }

    /// Fail an item: a `failed:` note, status back to `proposed`, the claim released.
    pub(crate) async fn fail(&self, step: &Step<'_>, reason: &str) -> Result<(), Error> {
        let first_line = reason.lines().next().unwrap_or("");
        self.note(step.plan, step.item, &note_text("failed: ", first_line))
            .await?;
        self.set_status(step.plan, step.item, "proposed", step.holder)
            .await?;
        self.release(step.plan, step.item, step.holder, false)
            .await?;
        Ok(())
    }
}

/// The item with this slug in a `GetPlan` answer.
pub(crate) fn find_item<'v>(plan: &'v Value, slug: &str) -> Option<&'v Value> {
    plan.get("items")?
        .as_array()?
        .iter()
        .find(|i| i.get("item_id").and_then(Value::as_str) == Some(slug))
}

/// The fields of an item an `UpsertPlanItem` writes back unchanged: everything the
/// person set, none of what the coordinator owns (version, claim, who and when).
pub(crate) fn item_fields(item: &Value) -> Map<String, Value> {
    const KEPT: [&str; 11] = [
        "kind", "parent", "title", "status", "body", "doc", "diagram", "preview", "size", "ask_id",
        "prs",
    ];
    let mut m = Map::new();
    for k in KEPT {
        if let Some(v) = item.get(k).filter(|v| !v.is_null()) {
            m.insert(k.into(), v.clone());
        }
    }
    if let Some(p) = item.get("position").filter(|v| !v.is_null()) {
        m.insert("position".into(), p.clone());
    }
    m
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use iohr_auth::{Redacted, StaticToken};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::context::AnyCredential;

    const PLAN: &str = "plan_01TEST";

    fn api(server: &MockServer) -> Api {
        let cred = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "test-token".to_owned(),
        ))));
        Api::new(&server.uri(), cred, false).unwrap()
    }

    fn step() -> Step<'static> {
        Step {
            plan: PLAN,
            item: "build-cli",
            title: "Build the CLI",
            kind: "feature",
            holder: "iohr:test",
        }
    }

    fn plan_with(items: Value) -> Value {
        json!({"plan": {"id": PLAN, "title": "p", "version": 3}, "items": items, "notes": []})
    }

    fn item(status: &str, holder: &str) -> Value {
        json!({"item_id": "build-cli", "kind": "feature", "title": "Build the CLI",
            "status": status, "version": 4, "position": 2, "holder": holder,
            "held_until": if holder.is_empty() { "" } else { "2099-01-01T00:00:00Z" },
            "claim_fence": "7", "updated_by": "x", "updated_at": "2026-11-07T00:00:00Z",
            "prs": ["inorbithr/sdk#1"]})
    }

    /// Mount the plan read and every write route with a fixed answer.
    async fn mount(server: &MockServer, got: &Value) {
        let base = format!("/v1/coordinator/plans/{PLAN}");
        let item_path = format!("{base}/items/build-cli");
        Mock::given(method("GET"))
            .and(path(base))
            .respond_with(ResponseTemplate::new(200).set_body_json(got))
            .mount(server)
            .await;
        Mock::given(method("PUT"))
            .and(path(item_path.clone()))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"item": item("building", "iohr:test"), "created": false}),
                ),
            )
            .mount(server)
            .await;
        for tail in ["claim", "release", "notes"] {
            Mock::given(method("POST"))
                .and(path(format!("{item_path}/{tail}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"item": {}})))
                .mount(server)
                .await;
        }
    }

    /// Method, the path's tail after the plan, and the JSON body of every call, in order.
    async fn calls(server: &MockServer) -> Vec<(String, String, Value)> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .map(|r| {
                let p = r
                    .url
                    .path()
                    .trim_start_matches(&format!("/v1/coordinator/plans/{PLAN}"));
                (
                    r.method.to_string(),
                    p.to_owned(),
                    serde_json::from_slice(&r.body).unwrap_or(Value::Null),
                )
            })
            .collect()
    }

    fn shape(c: &[(String, String, Value)]) -> Vec<String> {
        c.iter().map(|(m, p, _)| format!("{m} {p}")).collect()
    }

    #[tokio::test]
    async fn a_step_claims_notes_and_finishes_in_that_order() {
        let server = MockServer::start().await;
        mount(&server, &plan_with(json!([item("proposed", "")]))).await;
        let a = api(&server);
        let out = Plans::new(&a, None)
            .run_step(&step(), || async { Ok("built".to_owned()) })
            .await
            .unwrap();
        assert_eq!(out, Outcome::Done("built".into()));
        let c = calls(&server).await;
        assert_eq!(
            shape(&c),
            [
                "GET ",
                "POST /items/build-cli/claim",
                "GET ",
                "PUT /items/build-cli",
                "POST /items/build-cli/notes",
                "GET ",
                "PUT /items/build-cli",
            ]
        );
        assert_eq!(c[1].2["holder"], "iohr:test");
        assert_eq!(c[3].2["item"]["status"], "building");
        assert_eq!(c[3].2["expected_version"], 4);
        assert_eq!(c[3].2["item"]["position"], 2, "other fields are kept");
        assert!(
            c[3].2["item"].get("claim_fence").is_none(),
            "the claim is not written back"
        );
        assert_eq!(c[4].2["body"], "done: built");
        assert_eq!(c[6].2["item"]["status"], "done");
    }

    #[tokio::test]
    async fn a_step_adds_the_item_the_plan_lacks() {
        let server = MockServer::start().await;
        mount(&server, &plan_with(json!([]))).await;
        let a = api(&server);
        // The status writes after the claim read the plan again, which the mock still
        // answers without the item: the step stops there, after the add and the claim.
        let r = Plans::new(&a, None)
            .run_step(&step(), || async { Ok("x".to_owned()) })
            .await;
        assert!(r.is_err());
        let c = calls(&server).await;
        assert_eq!(
            &shape(&c)[..3],
            [
                "GET ",
                "PUT /items/build-cli",
                "POST /items/build-cli/claim"
            ]
        );
        assert_eq!(c[1].2["item"]["title"], "Build the CLI");
        assert_eq!(c[1].2["item"]["status"], "proposed");
        assert_eq!(c[1].2["expected_version"], 0);
    }

    #[tokio::test]
    async fn a_failed_step_goes_back_to_proposed_with_a_note_and_no_claim() {
        let server = MockServer::start().await;
        mount(&server, &plan_with(json!([item("proposed", "")]))).await;
        let a = api(&server);
        let e = Plans::new(&a, None)
            .run_step(&step(), || async {
                Err::<String, _>(Error::Failed("tests failed\nsecond line".into()))
            })
            .await
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "tests failed\nsecond line",
            "the work's own error"
        );
        let c = calls(&server).await;
        assert_eq!(
            &shape(&c)[4..],
            [
                "POST /items/build-cli/notes",
                "GET ",
                "PUT /items/build-cli",
                "POST /items/build-cli/release",
            ]
        );
        assert_eq!(c[4].2["body"], "failed: tests failed");
        assert_eq!(c[6].2["item"]["status"], "proposed");
        assert_eq!(c[7].2["holder"], "iohr:test");
    }

    #[tokio::test]
    async fn a_done_item_is_skipped_and_nothing_runs() {
        let server = MockServer::start().await;
        mount(&server, &plan_with(json!([item("done", "")]))).await;
        let a = api(&server);
        let out = Plans::new(&a, None)
            .run_step(&step(), || async {
                Err::<String, _>(Error::Failed("work ran".into()))
            })
            .await
            .unwrap();
        assert_eq!(out, Outcome::Skipped);
        assert_eq!(shape(&calls(&server).await), ["GET "]);
    }

    #[tokio::test]
    async fn another_holders_live_claim_and_a_dropped_item_are_refused() {
        let server = MockServer::start().await;
        mount(
            &server,
            &plan_with(json!([item("building", "someone-else")])),
        )
        .await;
        let a = api(&server);
        let e = Plans::new(&a, None)
            .run_step(&step(), || async {
                Err::<String, _>(Error::Failed("work ran".into()))
            })
            .await
            .unwrap_err();
        assert!(e.to_string().contains("held by someone-else"), "{e}");
        assert_eq!(shape(&calls(&server).await), ["GET "]);

        let dropped = MockServer::start().await;
        mount(&dropped, &plan_with(json!([item("dropped", "")]))).await;
        let a = api(&dropped);
        let e = Plans::new(&a, None)
            .run_step(&step(), || async {
                Err::<String, _>(Error::Failed("work ran".into()))
            })
            .await
            .unwrap_err();
        assert!(e.to_string().contains("was dropped"), "{e}");
    }

    #[tokio::test]
    async fn the_account_rides_on_every_call() {
        let server = MockServer::start().await;
        mount(&server, &plan_with(json!([item("done", "")]))).await;
        let a = api(&server);
        Plans::new(&a, Some("acc_team"))
            .run_step(&step(), || async { Ok(String::new()) })
            .await
            .unwrap();
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs[0].url.query(), Some("account_id=acc_team"));
    }

    #[tokio::test]
    async fn a_plan_is_created_once_by_its_external_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/coordinator/plans"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"plan": {"id": PLAN, "title": "shop"}, "created": false}),
                ),
            )
            .mount(&server)
            .await;
        let a = api(&server);
        let (plan, created) = Plans::new(&a, None)
            .create("shop", Some("project:shop"), None)
            .await
            .unwrap();
        assert_eq!(plan["id"], PLAN);
        assert!(!created);
        let body = &calls(&server).await[0].2;
        assert_eq!(body["external_id"], "project:shop");
    }

    #[test]
    fn slugs_ids_holders_and_notes_stay_inside_the_api_rules() {
        assert!(item_slug("a.b_c-1").is_ok());
        for bad in ["", ".", "..", "a/b", "a b", "a?x", &"x".repeat(81)] {
            assert!(item_slug(bad).is_err(), "{bad}");
        }
        assert!(plan_segment("plan_01ABC").is_ok());
        for bad in ["", "../x", "plan/1", "plan?x=1"] {
            assert!(plan_segment(bad).is_err(), "{bad}");
        }
        assert_eq!(default_holder("work"), "iohr:work");
        let cut = default_holder(&"é".repeat(100));
        assert!(cut.len() <= 120 && cut.len() >= 118, "{}", cut.len());
        let long = note_text("done: ", &"y".repeat(3000));
        assert_eq!(long.chars().count(), NOTE_MAX);
        assert!(long.ends_with('…'));
        assert_eq!(note_text("failed: ", "a\u{1b}[31m b\n"), "failed: a[31m b");
    }
}
