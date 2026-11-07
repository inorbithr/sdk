//! `iohr coordinator plan` and `iohr coordinator item`: the Planner's plans, items,
//! claims and notes over the public API (RFC 0166, RFC 0211). The calls are
//! [`crate::planner`]'s, the same ones `iohr project init` and the `iohr auto` slices use.

use std::cell::Cell;

use serde_json::{Map, Value, json};

use crate::cli::{CoordinatorItemCommand, CoordinatorPlanCommand, PlanItemRef};
use crate::context::Session;
use crate::error::Error;
use crate::output::Out;
use crate::planner::{Outcome, Plans, Step, default_holder, find_item, item_fields, note_text};

use super::{day, headline, str_of};

pub(super) async fn plan(s: &Session, cmd: CoordinatorPlanCommand, out: Out) -> Result<(), Error> {
    match cmd {
        CoordinatorPlanCommand::List {
            state,
            limit,
            account,
        } => {
            let v = Plans::new(&s.api, account.as_deref())
                .list(state.as_deref(), limit)
                .await?;
            if out.json {
                Out::print_json(&v);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = list_of(&v, "plans")
                .iter()
                .map(|p| {
                    vec![
                        str_of(p, "id").to_owned(),
                        str_of(p, "state").to_owned(),
                        p.get("item_count")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            .to_string(),
                        day(str_of(p, "updated_at")),
                        headline(str_of(p, "title"), 70),
                    ]
                })
                .collect();
            Out::table(&["PLAN", "STATE", "ITEMS", "UPDATED", "TITLE"], &rows);
            Ok(())
        }
        CoordinatorPlanCommand::Show { plan, account } => {
            let v = Plans::new(&s.api, account.as_deref()).get(&plan).await?;
            if out.json {
                Out::print_json(&v);
                return Ok(());
            }
            show(&v);
            Ok(())
        }
        CoordinatorPlanCommand::Create {
            title,
            external_id,
            access,
            account,
        } => {
            let (plan, created) = Plans::new(&s.api, account.as_deref())
                .create(&title, external_id.as_deref(), access.as_deref())
                .await?;
            if out.json {
                Out::print_json(&json!({"plan": plan, "created": created}));
                return Ok(());
            }
            Out::pairs(&[
                ("plan", str_of(&plan, "id").to_owned()),
                ("title", headline(str_of(&plan, "title"), 70)),
                ("created", created.to_string()),
            ]);
            Ok(())
        }
    }
}

pub(super) async fn item(s: &Session, cmd: CoordinatorItemCommand, out: Out) -> Result<(), Error> {
    match cmd {
        CoordinatorItemCommand::Add {
            at,
            title,
            kind,
            body,
        } => add(s, &at, &title, kind, body, out).await,
        CoordinatorItemCommand::Claim {
            at,
            holder,
            seconds,
            force,
        } => {
            let holder = holder.unwrap_or_else(|| default_holder(&s.label));
            let v = Plans::new(&s.api, at.account.as_deref())
                .claim(&at.plan, &at.item, &holder, seconds, force)
                .await?;
            print_claim(&v, out);
            Ok(())
        }
        CoordinatorItemCommand::Release { at, holder, force } => {
            let holder = holder.unwrap_or_else(|| default_holder(&s.label));
            let v = Plans::new(&s.api, at.account.as_deref())
                .release(&at.plan, &at.item, &holder, force)
                .await?;
            print_claim(&v, out);
            Ok(())
        }
        CoordinatorItemCommand::Note { at, text } => {
            let text = read_text(&text)?;
            let v = Plans::new(&s.api, at.account.as_deref())
                .note(&at.plan, &at.item, &note_text("", &text))
                .await?;
            if out.json {
                Out::print_json(&v);
            } else {
                let n = v.get("note").unwrap_or(&Value::Null);
                Out::pairs(&[
                    ("note", str_of(n, "id").to_owned()),
                    ("item", at.item.clone()),
                ]);
            }
            Ok(())
        }
        CoordinatorItemCommand::Done { at, note, holder } => {
            let holder = holder.unwrap_or_else(|| default_holder(&s.label));
            let p = Plans::new(&s.api, at.account.as_deref());
            if let Some(n) = note {
                p.note(&at.plan, &at.item, &note_text("done: ", &n)).await?;
            }
            let v = p.set_status(&at.plan, &at.item, "done", &holder).await?;
            print_item(&v, out);
            Ok(())
        }
        CoordinatorItemCommand::Fail { at, reason, holder } => {
            let holder = holder.unwrap_or_else(|| default_holder(&s.label));
            let step = Step {
                plan: &at.plan,
                item: &at.item,
                title: &at.item,
                kind: "task",
                holder: &holder,
            };
            Plans::new(&s.api, at.account.as_deref())
                .fail(&step, &reason)
                .await?;
            if out.json {
                Out::print_json(&json!({"item": at.item, "status": "proposed", "failed": true}));
            } else {
                Out::pairs(&[
                    ("item", at.item.clone()),
                    ("status", "proposed".to_owned()),
                    ("failed", "true".to_owned()),
                ]);
            }
            Ok(())
        }
        CoordinatorItemCommand::Run {
            at,
            title,
            kind,
            holder,
            command,
        } => run(s, at, title, kind, holder, command, out).await,
    }
}

/// `item add`: writing an item replaces it, so one that exists keeps what is not asked for.
async fn add(
    s: &Session,
    at: &PlanItemRef,
    title: &str,
    kind: Option<String>,
    body: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let body = body.as_deref().map(read_text).transpose()?;
    let p = Plans::new(&s.api, at.account.as_deref());
    let holder = default_holder(&s.label);
    let got = p.get(&at.plan).await?;
    let (mut fields, version) = if let Some(i) = find_item(&got, &at.item) {
        (
            item_fields(i),
            i.get("version").and_then(Value::as_i64).unwrap_or(0),
        )
    } else {
        let mut m = Map::new();
        m.insert("status".into(), json!("proposed"));
        (m, 0)
    };
    fields.insert("title".into(), json!(title));
    if let Some(k) = kind {
        fields.insert("kind".into(), json!(k));
    } else if !fields.contains_key("kind") {
        fields.insert("kind".into(), json!("task"));
    }
    if let Some(b) = body {
        fields.insert("body".into(), json!(b));
    }
    let v = p
        .upsert(&at.plan, &at.item, fields, version, &holder)
        .await?;
    print_item(&v, out);
    Ok(())
}

/// `item run`: the command is the step's work; its exit code is this command's.
async fn run(
    s: &Session,
    at: PlanItemRef,
    title: Option<String>,
    kind: Option<String>,
    holder: Option<String>,
    command: Vec<String>,
    out: Out,
) -> Result<(), Error> {
    let Some((program, args)) = command.split_first() else {
        return Err(Error::Usage("give the command to run after `--`".into()));
    };
    let holder = holder.unwrap_or_else(|| default_holder(&s.label));
    let title = title.unwrap_or_else(|| at.item.clone());
    let kind = kind.unwrap_or_else(|| "task".to_owned());
    let step = Step {
        plan: &at.plan,
        item: &at.item,
        title: &title,
        kind: &kind,
        holder: &holder,
    };
    // The note names the program, never its arguments: they may hold a secret.
    let name = std::path::Path::new(program)
        .file_name()
        .map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
    let code = Cell::new(0u8);
    let result = Plans::new(&s.api, at.account.as_deref())
        .run_step(&step, || async {
            let status = tokio::process::Command::new(program)
                .args(args)
                .status()
                .await
                .map_err(|e| Error::Failed(format!("cannot start `{name}`: {}", e.kind())))?;
            if status.success() {
                Ok(format!("`{name}` exited 0"))
            } else {
                let c = status
                    .code()
                    .map_or(1, |c| u8::try_from(c).unwrap_or(1))
                    .max(1);
                code.set(c);
                Err(Error::Failed(match status.code() {
                    Some(c) => format!("`{name}` exited with code {c}"),
                    None => format!("`{name}` was stopped by a signal"),
                }))
            }
        })
        .await;
    match result {
        Ok(outcome) => {
            let state = match outcome {
                Outcome::Skipped => "skipped (already done)",
                Outcome::Done(_) => "done",
            };
            if out.json {
                Out::print_json(&json!({"item": at.item, "outcome": state}));
            } else {
                Out::note(&format!("{}: {state}", at.item));
            }
            Ok(())
        }
        Err(Error::Failed(m)) if code.get() != 0 => {
            Out::note(&format!("{}: failed, back to proposed ({m})", at.item));
            Err(Error::Child(code.get()))
        }
        Err(e) => Err(e),
    }
}

/// A text argument: itself, or all of standard input for `-`. Never empty.
fn read_text(arg: &str) -> Result<String, Error> {
    use std::io::Read as _;
    let text = if arg == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| Error::Usage(format!("could not read standard input: {e}")))?;
        buf
    } else {
        arg.to_owned()
    };
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err(Error::Usage("the text is empty".into()));
    }
    Ok(text)
}

fn list_of(v: &Value, key: &str) -> Vec<Value> {
    v.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn print_item(v: &Value, out: Out) {
    if out.json {
        Out::print_json(v);
        return;
    }
    Out::pairs(&[
        ("item", str_of(v, "item_id").to_owned()),
        ("status", str_of(v, "status").to_owned()),
        ("title", headline(str_of(v, "title"), 70)),
        ("holder", str_of(v, "holder").to_owned()),
    ]);
}

fn print_claim(v: &Value, out: Out) {
    if out.json {
        Out::print_json(v);
        return;
    }
    let i = v.get("item").unwrap_or(&Value::Null);
    Out::pairs(&[
        ("item", str_of(i, "item_id").to_owned()),
        ("holder", str_of(i, "holder").to_owned()),
        ("until", str_of(i, "held_until").to_owned()),
        ("status", str_of(i, "status").to_owned()),
    ]);
}

/// A plan, its items and their latest notes, as a person reads them.
fn show(v: &Value) {
    let p = v.get("plan").unwrap_or(&Value::Null);
    Out::pairs(&[
        ("plan", str_of(p, "id").to_owned()),
        ("title", headline(str_of(p, "title"), 70)),
        ("state", str_of(p, "state").to_owned()),
        ("access", str_of(p, "access").to_owned()),
    ]);
    let rows: Vec<Vec<String>> = list_of(v, "items")
        .iter()
        .map(|i| {
            vec![
                str_of(i, "item_id").to_owned(),
                str_of(i, "status").to_owned(),
                str_of(i, "holder").to_owned(),
                str_of(i, "kind").to_owned(),
                headline(str_of(i, "title"), 60),
            ]
        })
        .collect();
    Out::raw(b"\n");
    Out::table(&["ITEM", "STATUS", "HOLDER", "KIND", "TITLE"], &rows);
    let notes = list_of(v, "notes");
    if !notes.is_empty() {
        Out::raw(b"\n");
        for n in notes.iter().rev().take(10).rev() {
            Out::raw(
                format!(
                    "{}  {}  {}\n",
                    day(str_of(n, "created_at")),
                    str_of(n, "item_id"),
                    headline(str_of(n, "body"), 90)
                )
                .as_bytes(),
            );
        }
    }
}
