//! Sending the board to the InOrbit Planner (RFC 0003, study 0006).
//!
//! Sync reconciles; it does not replay events. It reads the plan, compares it with the board
//! and sends only the difference: missing or changed items, claims for cards in doing or
//! review, the "in review" note, `done` with the card's PR or evidence, and the status of
//! ready and dropped cards. Then it writes the plan and item ids back into each card's
//! `planner`. A second run finds nothing to send.
//!
//! It calls [`Plans`] in this process with the session's own credential, so no token is read
//! or printed here. The dry run plans the same steps without reading the plan and sends
//! nothing. The Planner holds execution state (claims, leases, approvals, evidence); the board
//! holds the spec. A dropped item is final in the Planner: moving its card out of dropped is
//! refused there and reported here.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::board::{Board, Card, Planner};
use super::view::{fmt_h, planner_status};
use super::Error;
use crate::planner::{Plans, default_holder, find_item, item_fields, note_text};

/// The plan's external id, so `plan create` returns the same plan every run.
pub(crate) fn external_id(b: &Board, repo: &str) -> String {
    format!("scrum:{repo}/{}", b.board)
}

/// One shell word in single quotes.
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn body(c: &Card) -> String {
    let section = c
        .x
        .get("section")
        .and_then(Value::as_str)
        .map(|s| format!("section {s}, "))
        .unwrap_or_default();
    format!(
        "{section}size {}, {} h, {} to {}",
        c.size,
        fmt_h(c.h()),
        c.start,
        c.due
    )
}

/// The note a review card carries in the Planner, which has no review status of its own.
pub(crate) const IN_REVIEW: &str = "in review";

fn done_note(c: &Card) -> String {
    c.pr.as_deref().map_or_else(
        || "done on the board".to_owned(),
        |p| format!("done: {p}"),
    )
}

/// One write to the Planner, for one card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Add {
        card: String,
        title: String,
        body: String,
    },
    Claim {
        card: String,
    },
    Note {
        card: String,
        text: String,
    },
    /// Put the item into a status (ready, dropped, done).
    Status {
        card: String,
        status: String,
    },
    Done {
        card: String,
        note: String,
    },
}

impl Step {
    pub(crate) fn card(&self) -> &str {
        match self {
            Step::Add { card, .. }
            | Step::Claim { card }
            | Step::Note { card, .. }
            | Step::Status { card, .. }
            | Step::Done { card, .. } => card,
        }
    }

    /// The command as a person would type it; `plan` is a shell word such as `"$PLAN"`.
    pub(crate) fn line(&self, plan: &str) -> String {
        match self {
            Step::Add { card, title, body } => format!(
                "iohr coordinator item add {plan} {} --title {} --body {}",
                q(card),
                q(title),
                q(body)
            ),
            Step::Claim { card } => format!("iohr coordinator item claim {plan} {}", q(card)),
            Step::Note { card, text } => {
                format!("iohr coordinator item note {plan} {} {}", q(card), q(text))
            }
            Step::Status { card, status } => {
                format!("# set {plan} {} to {status} through the Planner API", q(card))
            }
            Step::Done { card, note } => format!(
                "iohr coordinator item done {plan} {} --note {}",
                q(card),
                q(note)
            ),
        }
    }
}

/// One item as the Planner holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) status: String,
    pub(crate) title: String,
    pub(crate) body: String,
}

/// What the Planner already holds for the plan: its items by slug and the (item, note) pairs
/// the plan read returns. The read gives only the latest notes, so a note that fell off is
/// written again; that is a repeat, not a new state.
#[derive(Debug, Clone, Default)]
pub(crate) struct Remote {
    pub(crate) items: HashMap<String, Item>,
    pub(crate) notes: HashSet<(String, String)>,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl Remote {
    /// From the plan read: `{plan, items: [{item_id, status, title, body, ...}], notes:
    /// [{item_id, body, ...}]}`.
    pub(crate) fn from_plan(v: &Value) -> Remote {
        let list = |k: &str| {
            v.get(k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        Remote {
            items: list("items")
                .iter()
                .map(|i| {
                    (
                        text(i, "item_id"),
                        Item {
                            status: text(i, "status"),
                            title: text(i, "title"),
                            body: text(i, "body"),
                        },
                    )
                })
                .collect(),
            notes: list("notes")
                .iter()
                .map(|n| (text(n, "item_id"), text(n, "body")))
                .collect(),
        }
    }
}

/// The steps that bring the plan to the board. With `remote` unknown (the dry run), a card
/// with a `planner` counts as already added and every status step is listed.
pub(crate) fn steps(b: &Board, remote: Option<&Remote>) -> Vec<Step> {
    let mut out = Vec::new();
    for c in &b.cards {
        let id = c.id.clone();
        let have = match remote {
            Some(r) => r.items.get(&c.id).cloned(),
            None => c.planner.as_ref().map(|_| Item {
                status: String::new(),
                title: c.title.clone(),
                body: body(c),
            }),
        };
        if have
            .as_ref()
            .is_none_or(|i| i.title != c.title || i.body != body(c))
        {
            out.push(Step::Add {
                card: id.clone(),
                title: c.title.clone(),
                body: body(c),
            });
        }
        let status = have.map_or_else(|| "proposed".to_owned(), |i| i.status);
        let noted = |t: &str| {
            remote.is_some_and(|r| r.notes.contains(&(c.id.clone(), t.to_owned())))
        };
        match c.list.as_str() {
            "doing" | "review" => {
                if status != "building" {
                    out.push(Step::Claim { card: id.clone() });
                }
                if c.list == "review" && !noted(IN_REVIEW) {
                    out.push(Step::Note {
                        card: id,
                        text: IN_REVIEW.into(),
                    });
                }
            }
            "done" if status != "done" => out.push(Step::Done {
                card: id,
                note: done_note(c),
            }),
            "ready" | "dropped" if status != planner_status(&c.list) => out.push(Step::Status {
                card: id,
                status: planner_status(&c.list).into(),
            }),
            _ => {}
        }
    }
    out
}

/// The plan id the board already knows, from the first synced card.
pub(crate) fn known_plan(b: &Board) -> Option<String> {
    b.cards
        .iter()
        .find_map(|c| c.planner.as_ref().map(|p| p.plan.clone()))
}

/// The commands a sync would run, in order. `$PLAN` is the plan id `plan create` prints.
pub(crate) fn sync_dry(b: &Board, repo: &str) -> Vec<String> {
    let mut out = vec![format!(
        "# sync of {} to the InOrbit Planner; nothing is sent in a dry run",
        b.board
    )];
    out.push(match known_plan(b) {
        Some(p) => format!("PLAN={}  # from the board", q(&p)),
        None => format!(
            "PLAN=$(iohr coordinator plan create {} --external-id {} --json | jq -r .plan.id)",
            q(&b.board),
            q(&external_id(b, repo))
        ),
    });
    let all = steps(b, None);
    for c in &b.cards {
        if let Some(p) = &c.planner {
            out.push(format!("# {}: already synced as {}/{}", c.id, p.plan, p.item));
        }
        out.extend(
            all.iter()
                .filter(|s| s.card() == c.id)
                .map(|s| s.line("\"$PLAN\"")),
        );
    }
    out
}

/// What a real sync did.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) plan: String,
    /// True when `plan create` made the plan in this run.
    pub(crate) created: bool,
    /// Each command sent, as typed.
    pub(crate) sent: Vec<String>,
    /// Each command the Planner refused, with its first error line.
    pub(crate) failed: Vec<String>,
    /// Cards whose `planner` field was written in this run.
    pub(crate) written: Vec<String>,
}

fn first_line(e: &crate::error::Error) -> String {
    let s = e.to_string();
    let l = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no reason given");
    l.chars().take(200).collect()
}

/// The item's fields for an Add: what the Planner holds is kept, the title and body replaced.
fn add_fields(step: &Step, got: &Value) -> (Map<String, Value>, i64) {
    let Step::Add { card, title, body } = step else {
        return (Map::new(), 0);
    };
    let (mut fields, version) = match find_item(got, card) {
        Some(i) => (
            item_fields(i),
            i.get("version").and_then(Value::as_i64).unwrap_or(0),
        ),
        None => {
            let mut m = Map::new();
            m.insert("status".into(), json!("proposed"));
            m.insert("kind".into(), json!("task"));
            (m, 0)
        }
    };
    fields.insert("title".into(), json!(title));
    fields.insert("body".into(), json!(body));
    (fields, version)
}

/// A real sync: find or create the plan, read it, send the difference, then write the ids
/// back into `b`. The caller saves the board when `written` is not empty. A refused step is
/// reported and the card's later steps are skipped; the other cards still run.
pub(crate) async fn sync(
    b: &mut Board,
    repo: &str,
    plans: &Plans<'_>,
    label: &str,
) -> Result<Report, Error> {
    let fail = |e: &crate::error::Error| Error::Refused(vec![first_line(e)]);
    let holder = default_holder(label);
    let mut r = Report::default();
    r.plan = if let Some(p) = known_plan(b) {
        p
    } else {
        let (plan, created) = plans
            .create(&b.board, Some(&external_id(b, repo)), None)
            .await
            .map_err(|e| fail(&e))?;
        r.created = created;
        plan.get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Refused(vec!["the plan has no id".into()]))?
            .to_owned()
    };
    let got = plans.get(&r.plan).await.map_err(|e| fail(&e))?;
    let remote = Remote::from_plan(&got);
    let mut have: HashSet<String> = remote.items.keys().cloned().collect();
    let mut refused: HashSet<String> = HashSet::new();
    for s in steps(b, Some(&remote)) {
        if refused.contains(s.card()) {
            continue;
        }
        let line = s.line(&q(&r.plan));
        let item = s.card().to_owned();
        let done = match &s {
            Step::Add { .. } => {
                let (fields, version) = add_fields(&s, &got);
                plans
                    .upsert(&r.plan, &item, fields, version, &holder)
                    .await
            }
            Step::Claim { .. } => plans.claim(&r.plan, &item, &holder, None, false).await,
            Step::Note { text, .. } => plans.note(&r.plan, &item, &note_text("", text)).await,
            Step::Status { status, .. } => plans.set_status(&r.plan, &item, status, &holder).await,
            Step::Done { note, .. } => {
                match plans.note(&r.plan, &item, &note_text("", note)).await {
                    Ok(_) => plans.set_status(&r.plan, &item, "done", &holder).await,
                    Err(e) => Err(e),
                }
            }
        };
        match done {
            Ok(_) => {
                if matches!(s, Step::Add { .. }) {
                    have.insert(item);
                }
                r.sent.push(line);
            }
            Err(e) => {
                refused.insert(item);
                r.failed.push(format!("{line}: {}", first_line(&e)));
            }
        }
    }
    for c in &mut b.cards {
        let want = Planner {
            plan: r.plan.clone(),
            item: c.id.clone(),
        };
        if have.contains(&c.id)
            && c.planner
                .as_ref()
                .is_none_or(|p| p.plan != want.plan || p.item != want.item)
        {
            c.planner = Some(want);
            r.written.push(c.id.clone());
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::Arc;

    use inorbithr::Secret;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::board::tests::{board, card};
    use super::*;
    use crate::context::Api;
    use crate::auth::StaticToken;
    use crate::context::AnyCredential;

    const PLAN: &str = "plan_01TEST";

    fn api(server: &MockServer) -> Api {
        let cred = Arc::new(AnyCredential::Token(StaticToken::new(Secret::new(
            "test-token".to_owned(),
        ))));
        Api::new(&server.uri(), cred, false).unwrap()
    }

    fn five() -> Board {
        let mut d = card("dd", "done", &[]);
        d.pr = Some("inorbithr/eye#7".into());
        board(vec![
            card("bb", "backlog", &[]),
            card("rr", "ready", &[]),
            card("go", "doing", &[]),
            card("rv", "review", &[]),
            d,
        ])
    }

    fn item_of(c: &Card, status: &str) -> Value {
        json!({"item_id": c.id, "kind": "task", "title": c.title, "status": status,
            "body": body(c), "version": 1})
    }

    #[test]
    fn dry_run_maps_lists_to_planner_verbs() {
        let mut r = card("rr", "review", &[]);
        r.title = "it's here".into();
        let mut d = card("dd", "done", &[]);
        d.pr = Some("inorbithr/eye#7".into());
        let b = board(vec![r, d, card("aa", "ready", &[])]);
        let out = sync_dry(&b, "inorbithr/eye").join("\n");
        assert!(out.contains("--external-id 'scrum:inorbithr/eye/test' --json | jq -r .plan.id"), "{out}");
        assert!(out.contains(r"--title 'it'\''s here'"), "{out}");
        assert!(out.contains("item claim \"$PLAN\" 'rr'\niohr coordinator item note \"$PLAN\" 'rr' 'in review'"), "{out}");
        assert!(out.contains("item done \"$PLAN\" 'dd' --note 'done: inorbithr/eye#7'"), "{out}");
        assert!(out.contains("# set \"$PLAN\" 'aa' to ready"), "{out}");

        let mut b2 = b.clone();
        b2.cards[1].planner = Some(Planner { plan: "plan_1".into(), item: "dd".into() });
        let out = sync_dry(&b2, "inorbithr/eye").join("\n");
        assert!(out.contains("PLAN='plan_1'  # from the board"), "{out}");
        assert!(out.contains("# dd: already synced as plan_1/dd"), "{out}");
        assert!(!out.contains("item add \"$PLAN\" 'dd'"), "{out}");
    }

    #[test]
    fn a_plan_that_matches_the_board_needs_no_step() {
        let b = five();
        let items: Vec<Value> = b
            .cards
            .iter()
            .map(|c| item_of(c, planner_status(&c.list)))
            .collect();
        let mut got = json!({"plan": {"id": PLAN}, "items": items, "notes": []});
        got["notes"] = json!([{"item_id": "rv", "body": IN_REVIEW}]);
        let r = Remote::from_plan(&got);
        assert_eq!(steps(&b, Some(&r)), []);
        // A card moved on the board sends only its change.
        let mut moved = b.clone();
        moved.cards[2].list = "review".into();
        assert_eq!(
            steps(&moved, Some(&r)),
            [Step::Note { card: "go".into(), text: IN_REVIEW.into() }]
        );
    }

    async fn mount(server: &MockServer, got: &Value, refuse_claim: bool) {
        let base = format!("/v1/coordinator/plans/{PLAN}");
        Mock::given(method("POST"))
            .and(path("/v1/coordinator/plans"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"plan": {"id": PLAN, "title": "test"}, "created": true}),
            ))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(base.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(got))
            .mount(server)
            .await;
        for c in ["bb", "rr", "go", "rv", "dd"] {
            let p = format!("{base}/items/{c}");
            Mock::given(method("PUT"))
                .and(path(p.clone()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"item": {}})))
                .mount(server)
                .await;
            let claim = if refuse_claim && c == "go" {
                ResponseTemplate::new(409).set_body_json(json!({"error": "held by someone else"}))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"item": {}}))
            };
            Mock::given(method("POST"))
                .and(path(format!("{p}/claim")))
                .respond_with(claim)
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("{p}/notes")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"note": {}})))
                .mount(server)
                .await;
        }
    }

    async fn requests(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect()
    }

    #[tokio::test]
    async fn a_first_run_creates_the_plan_adds_every_card_and_writes_ids_back() {
        let server = MockServer::start().await;
        let b0 = five();
        // The plan holds the done card and the ready card as the Planner would after the
        // adds, so the status writes find them.
        let items: Vec<Value> = b0.cards.iter().map(|c| item_of(c, "proposed")).collect();
        mount(&server, &json!({"plan": {"id": PLAN}, "items": items, "notes": []}), false).await;
        let a = api(&server);
        let mut b = five();
        let r = sync(&mut b, "inorbithr/eye", &Plans::new(&a, None), "test").await.unwrap();
        assert!(r.created);
        assert_eq!(r.plan, PLAN);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(r.written, ["bb", "rr", "go", "rv", "dd"]);
        assert!(b.cards.iter().all(|c| c.planner.as_ref().is_some_and(|p| p.plan == PLAN && p.item == c.id)));
        let seen = requests(&server).await;
        assert_eq!(seen[0], "POST /v1/coordinator/plans");
        assert!(seen.contains(&format!("POST /v1/coordinator/plans/{PLAN}/items/go/claim")));
        assert!(seen.contains(&format!("POST /v1/coordinator/plans/{PLAN}/items/rv/notes")));
        assert!(r.sent.iter().any(|s| s.contains("item done 'plan_01TEST' 'dd' --note 'done: inorbithr/eye#7'")));
        assert!(r.sent.iter().any(|s| s.contains("'rr' to ready")));
    }

    #[tokio::test]
    async fn a_second_run_reads_the_plan_and_sends_nothing() {
        let server = MockServer::start().await;
        let mut b = five();
        for c in &mut b.cards {
            c.planner = Some(Planner { plan: PLAN.into(), item: c.id.clone() });
        }
        let items: Vec<Value> = b.cards.iter().map(|c| item_of(c, planner_status(&c.list))).collect();
        let got = json!({"plan": {"id": PLAN}, "items": items,
            "notes": [{"item_id": "rv", "body": IN_REVIEW}]});
        mount(&server, &got, false).await;
        let a = api(&server);
        let r = sync(&mut b, "inorbithr/eye", &Plans::new(&a, None), "test").await.unwrap();
        assert_eq!((r.created, r.sent.len(), r.failed.len(), r.written.len()), (false, 0, 0, 0), "{r:?}");
        assert_eq!(requests(&server).await, [format!("GET /v1/coordinator/plans/{PLAN}")]);
    }

    #[tokio::test]
    async fn a_refused_step_is_reported_and_the_rest_still_go() {
        let server = MockServer::start().await;
        let b0 = five();
        let items: Vec<Value> = b0.cards.iter().map(|c| item_of(c, "proposed")).collect();
        mount(&server, &json!({"plan": {"id": PLAN}, "items": items, "notes": []}), true).await;
        let a = api(&server);
        let mut b = five();
        let r = sync(&mut b, "inorbithr/eye", &Plans::new(&a, None), "test").await.unwrap();
        assert_eq!(r.failed.len(), 1, "{:?}", r.failed);
        assert!(r.failed[0].starts_with("iohr coordinator item claim 'plan_01TEST' 'go':"), "{:?}", r.failed);
        assert!(r.written.contains(&"go".to_owned()), "the item exists, so its id is still written back");
        assert!(r.sent.iter().any(|s| s.contains("item done 'plan_01TEST' 'dd'")));
    }
}
