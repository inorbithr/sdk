use iohr_auth::Kind;
use reqwest::Method;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Env;
use crate::api::ApiError;
use crate::cli::{Global, TokenCommand, TokenStatus};
use crate::commands::day;
use crate::context::{Session, account, session};
use crate::error::Error;
use crate::output::Out;

const PAGE: usize = 100;

pub(crate) async fn run(g: &Global, env: &Env, cmd: TokenCommand, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    match cmd {
        TokenCommand::Create {
            name,
            scopes,
            days,
            account: acc,
        } => create(&s, &account(&s, acc)?, &name, &scopes, days, out).await,
        TokenCommand::List {
            account: acc,
            status,
            query,
            limit,
        } => list(&s, &account(&s, acc)?, status, query.as_deref(), limit, out).await,
        TokenCommand::Revoke { id, account: acc } => revoke(&s, &account(&s, acc)?, &id, out).await,
    }
}

async fn create(
    s: &Session,
    acc: &str,
    name: &str,
    scopes: &[String],
    days: u32,
    out: Out,
) -> Result<(), Error> {
    let body =
        serde_json::json!({ "name": name.trim(), "scopes": scopes, "expires_in_days": days });
    let made: Value = s
        .api
        .send(
            Method::POST,
            &format!("/v1/accounts/orgs/{acc}/tokens"),
            &[],
            Some(&body),
        )
        .await
        .and_then(|r| r.json())
        .map_err(|e| person_only(s, e))?;
    if out.json {
        Out::print_json(&made);
        return Ok(());
    }
    let t = made.get("token").cloned().unwrap_or(Value::Null);
    Out::note(&format!(
        "Created token {} ({}), {} scope(s), expires {}. It is shown once, below; keep it in a secret store.",
        str_of(&t, "name"),
        str_of(&t, "id"),
        t.get("scopes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        day(&str_of(&made, "expires_at")),
    ));
    Out::raw(format!("{}\n", str_of(&made, "access_token")).as_bytes());
    Ok(())
}

async fn list(
    s: &Session,
    acc: &str,
    status: Option<TokenStatus>,
    query: Option<&str>,
    limit: usize,
    out: Out,
) -> Result<(), Error> {
    let mut tokens: Vec<Value> = Vec::new();
    let mut page_token = String::new();
    let page_size = PAGE.to_string();
    loop {
        let mut q: Vec<(&str, &str)> = vec![("kind", "token"), ("page_size", &page_size)];
        if let Some(st) = status {
            q.push(("status", st.as_str()));
        }
        if let Some(text) = query {
            q.push(("query", text));
        }
        if !page_token.is_empty() {
            q.push(("page_token", &page_token));
        }
        let page: Value = s
            .api
            .get(&format!("/v1/accounts/orgs/{acc}/keys"), &q)
            .await
            .map_err(|e| person_only(s, e))?;
        tokens.extend(
            page.get("keys")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        str_of(&page, "next_page_token").clone_into(&mut page_token);
        if page_token.is_empty() || tokens.len() >= limit {
            break;
        }
    }
    tokens.truncate(limit);
    if out.json {
        Out::print_json(&tokens);
        return Ok(());
    }
    let now = OffsetDateTime::now_utc();
    let rows: Vec<Vec<String>> = tokens.iter().map(|t| row(t, now)).collect();
    Out::table(
        &["ID", "NAME", "SCOPES", "EXPIRES", "LAST USED", "STATE"],
        &rows,
    );
    Ok(())
}

async fn revoke(s: &Session, acc: &str, id: &str, out: Out) -> Result<(), Error> {
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Error::Usage(
            "a token id has only letters, digits, '-' and '_'".into(),
        ));
    }
    s.api
        .send(
            Method::DELETE,
            &format!("/v1/accounts/orgs/{acc}/keys/{id}"),
            &[],
            None,
        )
        .await
        .map_err(|e| person_only(s, e))?;
    if out.json {
        Out::print_json(&serde_json::json!({ "id": id, "revoked": true }));
    } else {
        Out::note(&format!(
            "Revoked {id}. Calls with it are refused within seconds."
        ));
    }
    Ok(())
}

fn row(t: &Value, now: OffsetDateTime) -> Vec<String> {
    let scopes = t
        .get("scopes")
        .and_then(Value::as_array)
        .map_or_else(String::new, |v| {
            v.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        });
    let last = str_of(t, "last_used_at");
    vec![
        str_of(t, "id"),
        str_of(t, "name"),
        scopes,
        day(&str_of(t, "expires_at")),
        if last.is_empty() {
            "never".into()
        } else {
            day(&last)
        },
        state(t, now).into(),
    ]
}

/// Tokens are made, listed and revoked by a signed-in person, never by another token.
fn person_only(s: &Session, e: ApiError) -> Error {
    if e.status() == Some(403) && s.kind == Kind::Token {
        Error::with_hint(
            e,
            "Tokens are made, listed and revoked by a signed-in person, not by another token. \
             Use the console's API tokens page until `iohr login` in a browser arrives.",
        )
    } else {
        e.into()
    }
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn state(t: &Value, now: OffsetDateTime) -> &'static str {
    if !str_of(t, "revoked_at").is_empty() {
        return "revoked";
    }
    match OffsetDateTime::parse(&str_of(t, "expires_at"), &Rfc3339) {
        Ok(exp) if exp <= now => "expired",
        Ok(exp) if exp - now <= time::Duration::days(7) => "expiring",
        _ => "active",
    }
}
