//! `iohr connectors`: the catalogue of apps a connection can be made from (platform
//! RFC 0044).

use serde_json::Value;

use crate::Env;
use crate::cli::{ConnectorsCommand, Global};
use crate::commands::{pages, str_of};
use crate::context::{Session, session};
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn run(
    g: &Global,
    env: &Env,
    cmd: ConnectorsCommand,
    out: Out,
) -> Result<(), Error> {
    let s = session(g, env).await?;
    match cmd {
        ConnectorsCommand::List { category } => {
            let mut all = pages(&s.api, "/v1/connectors", &[], "connectors", usize::MAX)
                .await
                .map_err(scoped)?;
            if let Some(c) = category {
                all.retain(|x| str_of(x, "category") == c.as_str());
            }
            if out.json {
                Out::print_json(&all);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = all
                .iter()
                .map(|c| {
                    vec![
                        str_of(c, "id").to_owned(),
                        str_of(c, "name").to_owned(),
                        str_of(c, "category").to_owned(),
                        modes(c).join(" "),
                        state(c),
                    ]
                })
                .collect();
            Out::table(&["ID", "NAME", "CATEGORY", "MODES", "STATE"], &rows);
            Ok(())
        }
        ConnectorsCommand::Show { id } => {
            let c = fetch(&s, &id).await?;
            if out.json {
                Out::print_json(&c);
            } else {
                print(&c);
            }
            Ok(())
        }
    }
}

/// One connector, by id.
pub(crate) async fn fetch(s: &Session, id: &str) -> Result<Value, Error> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Error::Usage(
            "a connector's id has only lowercase letters, digits and '-': `iohr connectors list`"
                .into(),
        ));
    }
    let v: Value = s
        .api
        .get(&format!("/v1/connectors/{id}"), &[])
        .await
        .map_err(|e| {
            if e.status() == Some(404) {
                Error::with_hint(e, "`iohr connectors list` shows the catalogue.")
            } else {
                scoped(e)
            }
        })?;
    Ok(v.get("connector").cloned().unwrap_or(Value::Null))
}

pub(crate) fn scoped(e: inorbithr::Error) -> Error {
    if e.status() == Some(403) {
        Error::with_hint(
            e,
            "Connections need the connections:read scope to list and connections:write to \
             change, and an owner or admin of the account to connect, delete or grant.",
        )
    } else {
        e.into()
    }
}

/// Whether the connector is an AI model: its prompts go to the provider.
pub(crate) fn is_ai(c: &Value) -> bool {
    c.get("ai").and_then(Value::as_bool).unwrap_or(false)
}

/// The sign-in modes, in the catalogue's order.
pub(crate) fn modes(c: &Value) -> Vec<String> {
    c.get("auth_modes")
        .and_then(Value::as_array)
        .map(|ms| ms.iter().map(|m| str_of(m, "mode").to_owned()).collect())
        .unwrap_or_default()
}

fn state(c: &Value) -> String {
    let mut st = match str_of(c, "status") {
        "" | "available" => "available".to_owned(),
        "coming_soon" => "coming soon".to_owned(),
        "needs_app" => "not set up on this platform".to_owned(),
        other => other.to_owned(),
    };
    if str_of(c, "kind") == "client" {
        st.push_str(", assistant (setup sheet in the console)");
    } else if is_ai(c) {
        st.push_str(", AI model");
    }
    st
}

fn list_of(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn field_text(f: &Value) -> String {
    let mut t = str_of(f, "name").to_owned();
    if f.get("secret").and_then(Value::as_bool).unwrap_or(false) {
        t.push_str(" (secret)");
    }
    let choices = list_of(f, "choices");
    if !choices.is_empty() {
        t.push_str(" [");
        t.push_str(&choices.join("|"));
        t.push(']');
    }
    t
}

fn fields_text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_array)
        .map(|fs| fs.iter().map(field_text).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

fn print(c: &Value) {
    let mut pairs = vec![
        ("id", str_of(c, "id").to_owned()),
        ("name", str_of(c, "name").to_owned()),
        ("category", str_of(c, "category").to_owned()),
        ("state", state(c)),
        ("description", str_of(c, "description").to_owned()),
        ("docs", str_of(c, "docs").to_owned()),
        ("hosts", list_of(c, "hosts").join(" ")),
    ];
    let settings = fields_text(c, "config_fields");
    if !settings.is_empty() {
        pairs.push(("settings", settings));
    }
    Out::pairs(&pairs);
    if is_ai(c) {
        Out::note(&format!(
            "AI model: data you send in prompts goes to {} under your own agreement with them.",
            str_of(c, "name")
        ));
    }
    let modes: Vec<Vec<String>> = c
        .get("auth_modes")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    vec![
                        str_of(m, "mode").to_owned(),
                        str_of(m, "label").to_owned(),
                        fields_text(m, "fields"),
                        list_of(m, "scopes").join(" "),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    if !modes.is_empty() {
        Out::raw(b"\n");
        Out::table(&["MODE", "LABEL", "FIELDS", "SCOPES"], &modes);
    }
    let actions: Vec<Vec<String>> = c
        .get("actions")
        .and_then(Value::as_array)
        .map(|acts| {
            acts.iter()
                .map(|a| {
                    vec![
                        str_of(a, "name").to_owned(),
                        str_of(a, "class").to_owned(),
                        str_of(a, "model").to_owned(),
                        list_of(a, "auth_modes").join(" "),
                        fields_text(a, "params"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    if !actions.is_empty() {
        Out::raw(b"\n");
        Out::table(&["ACTION", "CLASS", "MODEL", "MODES", "PARAMS"], &actions);
    }
}
