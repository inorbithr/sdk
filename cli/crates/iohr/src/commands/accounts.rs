use serde_json::Value;

use crate::Env;
use crate::cli::Global;
use crate::context::session;
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn list(g: &Global, env: &Env, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    let me: Value = s.api.get("/v1/accounts/me", &[]).await.map_err(|e| {
        if e.status() == Some(403) {
            Error::with_hint(
                e,
                "An API token reads its accounts only with the account:read scope.",
            )
        } else {
            e.into()
        }
    })?;
    let mut accounts: Vec<&Value> = me.get("account").into_iter().collect();
    if let Some(teams) = me.get("teams").and_then(Value::as_array) {
        accounts.extend(teams);
    }
    if out.json {
        Out::print_json(&accounts);
        return Ok(());
    }
    let field = |a: &Value, k: &str| {
        a.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let rows: Vec<Vec<String>> = accounts
        .iter()
        .map(|a| {
            let id = field(a, "id");
            let mark = if id == s.account { "*" } else { "" };
            vec![
                format!("{id}{mark}"),
                field(a, "kind"),
                field(a, "name"),
                field(a, "plan"),
                field(a, "role"),
            ]
        })
        .collect();
    Out::table(&["ACCOUNT", "KIND", "NAME", "PLAN", "ROLE"], &rows);
    Ok(())
}
