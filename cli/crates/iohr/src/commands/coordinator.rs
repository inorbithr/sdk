//! `iohr coordinator`: the coordinator's jobs and queued asks over the public API
//! (RFC 0120, RFC 0138). The coordinator serves signed-in people, so these need a
//! person's profile (`iohr login`), not an API key.

use std::io::Read as _;

use inorbithr::Method;
use serde_json::{Value, json};

use crate::Env;
use crate::cli::{CoordinatorAsksCommand, CoordinatorCommand, CoordinatorJobsCommand, Global};
use crate::context::{Session, session};
use crate::error::Error;
use crate::output::Out;

use super::{day, str_of};

pub(crate) async fn run(
    g: &Global,
    env: &Env,
    cmd: CoordinatorCommand,
    out: Out,
) -> Result<(), Error> {
    let s = session(g, env).await?;
    match cmd {
        CoordinatorCommand::Jobs(CoordinatorJobsCommand::List { state, mine, limit }) => {
            list_jobs(&s, state, mine, limit, out).await
        }
        CoordinatorCommand::Jobs(CoordinatorJobsCommand::Create {
            kind,
            brief,
            document,
            repository,
            model,
            model_override_reason,
            tags,
            priority,
            supersedes,
            security,
            compliance_pr,
            ask,
        }) => {
            let brief = read_brief(&brief)?;
            let body = job_body(&JobArgs {
                kind: &kind,
                brief: &brief,
                document,
                repository,
                model,
                model_override_reason,
                tags,
                priority,
                supersedes,
                security,
                compliance_pr,
                ask,
            })?;
            create_job(&s, &body, out).await
        }
        CoordinatorCommand::Asks(CoordinatorAsksCommand::List {
            state,
            source,
            limit,
        }) => list_asks(&s, state, source, limit, out).await,
    }
}

fn scoped(e: inorbithr::Error) -> Error {
    if matches!(e.status(), Some(401 | 403)) {
        Error::with_hint(
            e,
            "The coordinator serves signed-in people: run `iohr login`; an API key cannot use it.",
        )
    } else {
        e.into()
    }
}

/// The brief: the argument, or all of standard input for `-`. Never empty.
fn read_brief(arg: &str) -> Result<String, Error> {
    let text = if arg == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            Error::Usage(format!("could not read the brief from standard input: {e}"))
        })?;
        buf
    } else {
        arg.to_owned()
    };
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err(Error::Usage("the brief is empty".into()));
    }
    Ok(text)
}

struct JobArgs<'a> {
    kind: &'a str,
    brief: &'a str,
    document: Option<String>,
    repository: Option<String>,
    model: Option<String>,
    model_override_reason: Option<String>,
    tags: Vec<String>,
    priority: Option<u32>,
    supersedes: Option<String>,
    security: bool,
    compliance_pr: Option<String>,
    ask: Option<String>,
}

/// The `CreateJobRequest` body: only what was given, so the kind's defaults apply.
fn job_body(a: &JobArgs<'_>) -> Result<Value, Error> {
    if a.model_override_reason.is_some() && a.model.is_none() {
        return Err(Error::Usage("--model-override-reason needs --model".into()));
    }
    let mut b = serde_json::Map::new();
    b.insert("kind".into(), json!(a.kind));
    b.insert("brief".into(), json!(a.brief));
    let optional = [
        ("document_id", &a.document),
        ("repository", &a.repository),
        ("model", &a.model),
        ("model_override_reason", &a.model_override_reason),
        ("supersedes", &a.supersedes),
        ("compliance_pr", &a.compliance_pr),
        ("queued_ask_id", &a.ask),
    ];
    for (k, v) in optional {
        if let Some(v) = v {
            b.insert(k.into(), json!(v));
        }
    }
    if !a.tags.is_empty() {
        b.insert("tags".into(), json!(a.tags));
    }
    if let Some(p) = a.priority {
        b.insert("priority".into(), json!(p));
    }
    if a.security {
        b.insert("security".into(), json!(true));
    }
    Ok(Value::Object(b))
}

fn headline(brief: &str) -> String {
    let line = brief.lines().next().unwrap_or_default();
    if line.chars().count() > 70 {
        let cut: String = line.chars().take(69).collect();
        format!("{cut}...")
    } else {
        line.to_owned()
    }
}

async fn list_jobs(
    s: &Session,
    state: Option<String>,
    mine: bool,
    limit: u32,
    out: Out,
) -> Result<(), Error> {
    let size = limit.to_string();
    let mut q: Vec<(&str, &str)> = vec![("page_size", &size)];
    if let Some(st) = state.as_deref() {
        q.push(("state", st));
    }
    if mine {
        q.push(("mine", "true"));
    }
    let v: Value = s
        .api
        .get("/v1/coordinator/jobs", &q)
        .await
        .map_err(scoped)?;
    let jobs = v
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if out.json {
        Out::print_json(&v);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = jobs
        .iter()
        .map(|j| {
            vec![
                str_of(j, "job_id").to_owned(),
                str_of(j, "state").to_owned(),
                str_of(j, "kind").to_owned(),
                str_of(j, "model").to_owned(),
                day(str_of(j, "created_at")),
                headline(str_of(j, "brief")),
            ]
        })
        .collect();
    Out::table(
        &["JOB", "STATE", "KIND", "MODEL", "CREATED", "BRIEF"],
        &rows,
    );
    Ok(())
}

async fn create_job(s: &Session, body: &Value, out: Out) -> Result<(), Error> {
    let v: Value = s
        .api
        .send(Method::Post, "/v1/coordinator/jobs", &[], Some(body))
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    if out.json {
        Out::print_json(&v);
        return Ok(());
    }
    let j = v.get("job").cloned().unwrap_or(Value::Null);
    Out::pairs(&[
        ("job", str_of(&j, "job_id").to_owned()),
        ("state", str_of(&j, "state").to_owned()),
        ("kind", str_of(&j, "kind").to_owned()),
        ("model", str_of(&j, "model").to_owned()),
    ]);
    Ok(())
}

async fn list_asks(
    s: &Session,
    state: Option<String>,
    source: Option<String>,
    limit: u32,
    out: Out,
) -> Result<(), Error> {
    let size = limit.to_string();
    let mut q: Vec<(&str, &str)> = vec![("page_size", &size)];
    if let Some(st) = state.as_deref() {
        q.push(("state", st));
    }
    if let Some(src) = source.as_deref() {
        q.push(("source", src));
    }
    let v: Value = s
        .api
        .get("/v1/coordinator/asks", &q)
        .await
        .map_err(scoped)?;
    let asks = v
        .get("asks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if out.json {
        Out::print_json(&v);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = asks
        .iter()
        .map(|a| {
            vec![
                str_of(a, "id").to_owned(),
                str_of(a, "state").to_owned(),
                str_of(a, "source").to_owned(),
                day(str_of(a, "created_at")),
                headline(str_of(a, "title")),
            ]
        })
        .collect();
    Out::table(&["ASK", "STATE", "SOURCE", "CREATED", "TITLE"], &rows);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(brief: &str) -> JobArgs<'_> {
        JobArgs {
            kind: "chore",
            brief,
            document: None,
            repository: None,
            model: None,
            model_override_reason: None,
            tags: vec![],
            priority: None,
            supersedes: None,
            security: false,
            compliance_pr: None,
            ask: None,
        }
    }

    #[test]
    fn body_carries_only_what_was_given() {
        let b = job_body(&args("fix it")).unwrap();
        assert_eq!(b, json!({"kind": "chore", "brief": "fix it"}));
    }

    #[test]
    fn body_maps_every_field() {
        let mut a = args("x");
        a.tags = vec!["a".into()];
        a.priority = Some(3);
        a.security = true;
        a.ask = Some("ask_1".into());
        a.model = Some("sonnet".into());
        a.model_override_reason = Some("cheap".into());
        let b = job_body(&a).unwrap();
        assert_eq!(b["queued_ask_id"], "ask_1");
        assert_eq!(b["security"], true);
        assert_eq!(b["priority"], 3);
        assert_eq!(b["tags"], json!(["a"]));
    }

    #[test]
    fn override_reason_needs_a_model() {
        let mut a = args("x");
        a.model_override_reason = Some("why".into());
        assert!(job_body(&a).is_err());
    }

    #[test]
    fn empty_brief_is_refused() {
        assert!(read_brief("   ").is_err());
    }
}
