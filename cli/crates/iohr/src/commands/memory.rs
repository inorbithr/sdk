//! `iohr memory remember|recall|list|forget`: the signed-in person's memory in InOrbit
//! (`iohr.memory.v1.MemoryService`, RFC 0002), scoped to the project the command runs in.
//!
//! Nothing is kept on this machine. The service does not filter by source, so the project's
//! entries are picked out of the answer here. Entry text goes in request bodies, never in a
//! URL, a log or an error.

use inorbithr::Method;
use serde_json::Value;

use crate::Env;
use crate::cli::{Global, MemoryCommand};
use crate::context::{Session, session};
use crate::error::Error;
use crate::memory::{self, SOURCE_CLI};
use crate::output::Out;

use super::{day, pages, str_of};

pub(crate) async fn run(g: &Global, env: &Env, cmd: MemoryCommand, out: Out) -> Result<(), Error> {
    let s = session(g, env).await.map_err(|e| match e {
        Error::NotSignedIn(_) => Error::NotSignedIn(
            "memory needs a sign-in, and nothing is kept on this machine instead: run \
             `iohr login`"
                .into(),
        ),
        other => other,
    })?;
    match cmd {
        MemoryCommand::Remember {
            text,
            preference,
            source,
        } => remember(&s, &text, preference, source, out).await,
        MemoryCommand::Recall {
            query,
            limit,
            all,
            source,
        } => recall(&s, &query, limit, scope(all, source).as_deref(), out).await,
        MemoryCommand::List { limit, all, source } => {
            list(&s, limit, scope(all, source).as_deref(), out).await
        }
        MemoryCommand::Forget { id } => forget(&s, &id, out).await,
    }
}

/// The project the command runs in, as its source tag, when it runs inside one.
fn project_source() -> Option<String> {
    let here = std::env::current_dir().ok()?;
    let root = crate::atlas::find_root(&here)?;
    let (name, _) = super::atlas::project_meta(&root);
    Some(memory::source_tag(&crate::atlas::slug(&name)))
}

/// The source entries are shown for: `--source`, else this project's, and none with `--all`.
fn scope(all: bool, source: Option<String>) -> Option<String> {
    if all {
        return None;
    }
    source.or_else(project_source)
}

fn scoped(e: inorbithr::Error) -> Error {
    match e.status() {
        Some(401 | 403) => Error::with_hint(
            e,
            "Memory belongs to a signed-in person: run `iohr login`; an API key cannot use it.",
        ),
        Some(429) => Error::with_hint(
            e,
            "Your memory is full: `iohr memory list` and `iohr memory forget <id>` make room.",
        ),
        _ => e.into(),
    }
}

fn cells(e: &Value) -> Vec<String> {
    let score = e
        .get("score")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)
        .map_or_else(String::new, |s| format!("{s:.2}"));
    vec![
        str_of(e, "id").to_owned(),
        memory::kind_name(str_of(e, "kind")).to_owned(),
        str_of(e, "source").to_owned(),
        day(str_of(e, "created_at")),
        score,
        str_of(e, "text").to_owned(),
    ]
}

fn show(entries: &[Value]) {
    let rows: Vec<Vec<String>> = entries.iter().map(cells).collect();
    Out::table(&["id", "kind", "source", "date", "score", "text"], &rows);
}

async fn remember(
    s: &Session,
    text: &str,
    preference: bool,
    source: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Usage(
            "there is nothing to remember: the text is empty".into(),
        ));
    }
    if text.chars().count() > memory::MAX_TEXT {
        return Err(Error::Usage(format!(
            "an entry is a short fact, at most {} characters",
            memory::MAX_TEXT
        )));
    }
    let source = source
        .or_else(project_source)
        .unwrap_or_else(|| SOURCE_CLI.to_owned());
    let body = memory::remember_body(text, preference, &source);
    let v: Value = s
        .api
        .send(Method::Post, "/v1/memory/entries", &[], Some(&body))
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    if out.json {
        Out::print_json(&v);
        return Ok(());
    }
    let e = v.get("entry").cloned().unwrap_or(Value::Null);
    Out::pairs(&[
        ("id", str_of(&e, "id").to_owned()),
        ("source", str_of(&e, "source").to_owned()),
    ]);
    Ok(())
}

async fn recall(
    s: &Session,
    query: &str,
    limit: u32,
    source: Option<&str>,
    out: Out,
) -> Result<(), Error> {
    if query.trim().is_empty() {
        return Err(Error::Usage(
            "say what to recall: the topic is empty".into(),
        ));
    }
    let want = limit.max(1);
    let body = memory::recall_body(query.trim(), memory::ask_for(want, source.is_some()));
    let v: Value = s
        .api
        .send(Method::Post, "/v1/memory/recall", &[], Some(&body))
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    let mut entries = memory::keep(
        v.get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        source,
    );
    entries.truncate(want as usize);
    if out.json {
        Out::print_json(&serde_json::json!({ "source": source, "entries": entries }));
    } else if entries.is_empty() {
        Out::note("nothing recalled");
    } else {
        show(&entries);
    }
    Ok(())
}

async fn list(s: &Session, limit: usize, source: Option<&str>, out: Out) -> Result<(), Error> {
    // The service cannot filter by source, so a scoped list reads every page, then picks.
    let read = if source.is_some() {
        10_000
    } else {
        limit.max(1)
    };
    let all = pages(&s.api, "/v1/memory/entries", &[], "entries", read)
        .await
        .map_err(scoped)?;
    let mut entries = memory::keep(all, source);
    entries.truncate(limit.max(1));
    if out.json {
        Out::print_json(&serde_json::json!({ "source": source, "entries": entries }));
    } else if entries.is_empty() {
        Out::note("nothing kept");
    } else {
        show(&entries);
    }
    Ok(())
}

async fn forget(s: &Session, id: &str, out: Out) -> Result<(), Error> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::Usage(
            "an entry id is letters, digits, _ and -".into(),
        ));
    }
    s.api
        .send(
            Method::Delete,
            &format!("/v1/memory/entries/{id}"),
            &[],
            None,
        )
        .await
        .map_err(scoped)?;
    if out.json {
        Out::print_json(&serde_json::json!({ "forgotten": id }));
    } else {
        Out::pairs(&[("forgotten", id.to_owned())]);
    }
    Ok(())
}
