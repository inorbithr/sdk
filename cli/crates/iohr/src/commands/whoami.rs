use serde_json::Value;

use crate::Env;
use crate::api::ApiError;
use crate::cli::Global;
use crate::context::session;
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn run(g: &Global, env: &Env, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    // Either may be outside a token's scopes; what the token says about itself fills in.
    let me = optional(s.api.get::<Value>("/v1/me", &[]).await)?;
    let account = optional(s.api.get::<Value>("/v1/accounts/me", &[]).await)?;
    let expires = s.claims.expires_at().and_then(|t| {
        t.format(time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute] UTC"
        ))
        .ok()
    });

    if out.json {
        Out::print_json(&serde_json::json!({
            "profile": s.label, "account": s.account, "scopes": s.claims.scp,
            "expires_at": expires, "me": me, "account_details": account,
        }));
        return Ok(());
    }
    let str_at = |v: &Option<Value>, p: &str| {
        v.as_ref()
            .and_then(|v| v.pointer(p))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let mut pairs = vec![
        ("profile", s.label.clone()),
        (
            "subject",
            str_at(&me, "/subject").unwrap_or_else(|| s.claims.sub.clone()),
        ),
        (
            "kind",
            str_at(&me, "/kind").unwrap_or_else(|| "client".into()),
        ),
        ("account", s.account.clone()),
    ];
    if let Some(name) = str_at(&account, "/account/name") {
        pairs.push(("name", name));
    }
    if let Some(plan) = str_at(&account, "/account/plan").or_else(|| s.claims.plan.clone()) {
        pairs.push(("plan", plan));
    }
    pairs.push(("scopes", s.claims.scp.join(" ")));
    pairs.push(("expires", expires.unwrap_or_else(|| "never".into())));
    Out::pairs(&pairs);
    if me.is_none() {
        Out::note("(subject and kind from the token itself: it does not hold identity:read)");
    }
    Ok(())
}

/// A 403 means "not in this token's scopes": the field is left out, not an error.
fn optional(r: Result<Value, ApiError>) -> Result<Option<Value>, Error> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.status() == Some(403) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
