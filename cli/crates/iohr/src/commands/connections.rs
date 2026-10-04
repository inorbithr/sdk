//! `iohr connections`: an account's connections to outside systems, made from the
//! catalogue with a key or by signing in at the provider (platform RFC 0044).
//!
//! A secret field is read without echo, from a file or from standard input, never from
//! an argument; it is held as [`Redacted`] until the one request that carries it and is
//! never printed. Signing in opens the provider in a browser and waits on the
//! platform's connect session.

use std::collections::BTreeMap;
use std::time::Duration;

use inorbithr::Method;
use iohr_auth::{Kind, Redacted};
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Env;
use crate::browser;
use crate::cli::{
    AccountArg, ConnectionStatus, ConnectionsAdd, ConnectionsCommand, ConnectionsReconnect, Global,
    SecretInput, UseStatus,
};
use crate::commands::connectors::{self, is_ai, scoped};
use crate::commands::{day, pages, str_of};
use crate::context::{Session, account, session};
use crate::error::Error;
use crate::output::Out;
use crate::prompt;

/// How often a connect session is asked whether the person has finished.
const POLL: Duration = Duration::from_secs(2);
/// How long to wait when the session names no expiry, and at most in any case.
const DEFAULT_WAIT: Duration = Duration::from_mins(10);
const MAX_WAIT: Duration = Duration::from_mins(15);

pub(crate) async fn run(
    g: &Global,
    env: &Env,
    cmd: ConnectionsCommand,
    out: Out,
) -> Result<(), Error> {
    let s = session(g, env).await?;
    match cmd {
        ConnectionsCommand::List { status, account } => {
            list(&s, &acc(&s, account)?, status, out).await
        }
        ConnectionsCommand::Show {
            connection,
            account,
        } => {
            let c = resolve(&s, &acc(&s, account)?, &connection).await?;
            if out.json {
                Out::print_json(&c);
            } else {
                print_connection(&c);
            }
            Ok(())
        }
        ConnectionsCommand::Add(args) => add(&s, args, out).await,
        ConnectionsCommand::Test {
            connection,
            account,
        } => test(&s, &acc(&s, account)?, &connection, out).await,
        ConnectionsCommand::History {
            connection,
            action,
            status,
            consumer,
            limit,
            account,
        } => {
            let filters = Filters {
                action,
                status,
                consumer,
                limit,
            };
            history(&s, &acc(&s, account)?, connection.as_deref(), filters, out).await
        }
        ConnectionsCommand::Pause {
            connection,
            account,
        } => pause(&s, &acc(&s, account)?, &connection, true, out).await,
        ConnectionsCommand::Resume {
            connection,
            account,
        } => pause(&s, &acc(&s, account)?, &connection, false, out).await,
        ConnectionsCommand::Rename {
            connection,
            new_name,
            account,
        } => rename(&s, &acc(&s, account)?, &connection, &new_name, out).await,
        ConnectionsCommand::Delete {
            connection,
            yes,
            account,
        } => delete(&s, &acc(&s, account)?, &connection, yes, out).await,
        ConnectionsCommand::Reconnect(args) => reconnect(&s, args, out).await,
        ConnectionsCommand::Grant {
            connection,
            to,
            actions,
            expires,
            account,
        } => {
            let a = acc(&s, account)?;
            grant(&s, &a, &connection, &to, &actions, expires.as_deref(), out).await
        }
        ConnectionsCommand::Grants {
            connection,
            account,
        } => grants(&s, &acc(&s, account)?, &connection, out).await,
        ConnectionsCommand::RevokeGrant {
            connection,
            grant,
            account,
        } => revoke_grant(&s, &acc(&s, account)?, &connection, &grant, out).await,
    }
}

fn acc(s: &Session, a: AccountArg) -> Result<String, Error> {
    account(s, a.account)
}

fn base(acc: &str) -> String {
    format!("/v1/accounts/orgs/{acc}/connections")
}

/// An id the API made (`con_...`, `gnt_...`) or a name, safe as one path segment.
fn segment<'a>(what: &str, v: &'a str) -> Result<&'a str, Error> {
    if !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(v)
    } else {
        Err(Error::Usage(format!(
            "{what} has only letters, digits, '-' and '_'"
        )))
    }
}

/// A connection's name: lowercase letters, digits and '-'.
fn slug(v: &str) -> Result<String, Error> {
    let v = v.trim();
    if !v.is_empty()
        && v.len() <= 63
        && !v.starts_with('-')
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Ok(v.to_owned())
    } else {
        Err(Error::Usage(
            "a connection's name has only lowercase letters, digits and '-', such as pagerduty-eu"
                .into(),
        ))
    }
}

/// A connection by its id (`con_...`) or its name in the account.
async fn resolve(s: &Session, acc: &str, which: &str) -> Result<Value, Error> {
    let which = segment("a connection's name or id", which)?;
    if which.starts_with("con_") {
        let v: Value = s
            .api
            .get(&format!("{}/{which}", base(acc)), &[])
            .await
            .map_err(scoped)?;
        return Ok(v.get("connection").cloned().unwrap_or(Value::Null));
    }
    let all = pages(&s.api, &base(acc), &[], "connections", usize::MAX)
        .await
        .map_err(scoped)?;
    all.into_iter()
        .find(|c| str_of(c, "name") == which)
        .ok_or_else(|| {
            Error::Failed(format!(
                "there is no connection {which} in account {acc}: `iohr connections list` shows them"
            ))
        })
}

/// The path of one connection, from its answer.
fn path_of(acc: &str, c: &Value, rest: &str) -> Result<String, Error> {
    let id = segment("the connection's id", str_of(c, "id"))?;
    Ok(format!("{}/{id}{rest}", base(acc)))
}

/// The `connection` an answer carries.
fn connection_in(v: &Value) -> Value {
    v.get("connection").cloned().unwrap_or(Value::Null)
}

fn done(c: &Value, out: Out, note: &str) {
    if out.json {
        Out::print_json(c);
    } else {
        Out::note(note);
    }
}

async fn list(
    s: &Session,
    acc: &str,
    status: Option<ConnectionStatus>,
    out: Out,
) -> Result<(), Error> {
    let mut all = pages(&s.api, &base(acc), &[], "connections", usize::MAX)
        .await
        .map_err(scoped)?;
    if let Some(st) = status {
        all.retain(|c| str_of(c, "status") == st.as_str());
    }
    if out.json {
        Out::print_json(&all);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|c| {
            vec![
                str_of(c, "name").to_owned(),
                connector_of(c).to_owned(),
                str_of(c, "label").to_owned(),
                str_of(c, "status").to_owned(),
                c.get("grants")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .to_string(),
                last_test(c),
            ]
        })
        .collect();
    Out::table(
        &["NAME", "CONNECTOR", "AS", "STATUS", "GRANTS", "LAST TEST"],
        &rows,
    );
    Ok(())
}

/// The connector a connection was made from, or its kind (`endpoint`, `webhook-in`).
fn connector_of(c: &Value) -> &str {
    match str_of(c, "connector") {
        "" => str_of(c, "kind"),
        id => id,
    }
}

fn last_test(c: &Value) -> String {
    let at = str_of(c, "last_test_at");
    if at.is_empty() {
        return "never".into();
    }
    let ok = c
        .get("last_test_ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!("{} {}", day(at), if ok { "passed" } else { "failed" })
}

fn print_connection(c: &Value) {
    let mut pairs = vec![
        ("name", str_of(c, "name").to_owned()),
        ("id", str_of(c, "id").to_owned()),
        ("connector", connector_of(c).to_owned()),
        ("mode", str_of(c, "auth_mode").to_owned()),
        ("as", str_of(c, "label").to_owned()),
        ("status", str_of(c, "status").to_owned()),
        (
            "grants",
            c.get("grants")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .to_string(),
        ),
    ];
    let used_by = c
        .get("used_by")
        .and_then(Value::as_array)
        .map(|u| {
            u.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    if !used_by.is_empty() {
        pairs.push(("used by", used_by));
    }
    pairs.push(("last test", last_test(c)));
    let err = str_of(c, "last_test_error");
    if !err.is_empty() {
        pairs.push(("test error", err.to_owned()));
    }
    let scopes = c
        .get("scopes")
        .and_then(Value::as_array)
        .map(|u| {
            u.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    if !scopes.is_empty() {
        pairs.push(("scopes", scopes));
    }
    let expiry = str_of(c, "token_expires_at");
    if !expiry.is_empty() {
        pairs.push(("token expires", format!("{expiry} (renews automatically)")));
    }
    let set_at = c
        .get("credential")
        .map(|cr| str_of(cr, "set_at"))
        .unwrap_or_default();
    if !set_at.is_empty() {
        pairs.push(("credential set", day(set_at)));
    }
    pairs.push(("owner", str_of(c, "owner").to_owned()));
    pairs.push(("created", day(str_of(c, "created_at"))));
    pairs.retain(|(_, v)| !v.is_empty());
    Out::pairs(&pairs);
}

// --- add and reconnect ---------------------------------------------------------------

/// The body of `CreateConnection`; the credentials are borrowed from their
/// [`Redacted`] holders, never copied into a JSON value.
#[derive(Serialize)]
struct Create<'a> {
    kind: &'a str,
    auth_mode: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    config: &'a BTreeMap<String, String>,
    credentials: BTreeMap<&'a str, &'a str>,
}

/// The body of `UpdateConnection` for a new credential.
#[derive(Serialize)]
struct Recredential<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_mode: Option<&'a str>,
    credentials: BTreeMap<&'a str, &'a str>,
}

fn exposed(secrets: &BTreeMap<String, Redacted<String>>) -> BTreeMap<&str, &str> {
    secrets
        .iter()
        .map(|(k, v)| (k.as_str(), v.expose().as_str()))
        .collect()
}

/// Whether a mode signs in at the provider instead of taking a key.
fn signs_in(mode: &Value) -> bool {
    str_of(mode, "mode").starts_with("oauth")
}

fn fields(v: &Value, k: &str) -> Vec<Value> {
    v.get(k)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn is_secret(f: &Value) -> bool {
    f.get("secret").and_then(Value::as_bool).unwrap_or(false)
}

fn label_of(f: &Value) -> &str {
    match str_of(f, "label") {
        "" => str_of(f, "name"),
        l => l,
    }
}

/// The connector's mode called `wanted`, or its first.
fn pick_mode<'a>(connector: &'a Value, wanted: Option<&str>) -> Result<&'a Value, Error> {
    let modes = connector
        .get("auth_modes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let names = || {
        modes
            .iter()
            .map(|m| str_of(m, "mode"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match wanted {
        None => modes.first().ok_or_else(|| {
            Error::Usage(format!(
                "{} has no sign-in mode to connect with",
                str_of(connector, "id")
            ))
        }),
        Some(w) => modes
            .iter()
            .find(|m| str_of(m, "mode") == w)
            .ok_or_else(|| {
                Error::Usage(format!(
                    "{} has no mode {w}; its modes: {}",
                    str_of(connector, "id"),
                    names()
                ))
            }),
    }
}

/// Refuses a connector that cannot be connected from here, saying why.
fn connectable(c: &Value) -> Result<(), Error> {
    let name = str_of(c, "name");
    if str_of(c, "kind") == "client" {
        return Err(Error::Usage(format!(
            "{name} is not a connection the platform calls: you connect {name} to InOrbit's MCP \
             server with an API token of your own. The console's catalogue has its setup sheet."
        )));
    }
    match str_of(c, "status") {
        "" | "available" => Ok(()),
        "coming_soon" => Err(Error::Usage(format!(
            "{name} is coming soon and cannot be connected yet"
        ))),
        "needs_app" => Err(Error::Failed(format!(
            "{name} is not set up on this platform yet: an administrator registers its app first"
        ))),
        other => Err(Error::Failed(format!(
            "{name} cannot be connected ({other})"
        ))),
    }
}

/// The settings: `--config` pairs checked against the connector's settings and the
/// mode's fields that are not secret; a required one missing is asked for on a
/// terminal.
fn settings(
    connector: &Value,
    mode: &Value,
    pairs: &[String],
) -> Result<BTreeMap<String, String>, Error> {
    let mut allowed = fields(connector, "config_fields");
    allowed.extend(fields(mode, "fields").into_iter().filter(|f| !is_secret(f)));
    let secrets: Vec<Value> = fields(mode, "fields")
        .into_iter()
        .filter(is_secret)
        .collect();
    let mut config = BTreeMap::new();
    for p in pairs {
        let Some((k, v)) = p.split_once('=') else {
            return Err(Error::Usage(format!("--config takes KEY=VALUE, not {p}")));
        };
        if secrets.iter().any(|f| str_of(f, "name") == k) {
            return Err(Error::Usage(format!(
                "{k} is a secret and is never an argument: leave it out to be asked for it, or use \
                 --secret-file {k}=PATH or --secret-stdin {k}. If the value is in your shell \
                 history now, replace it at the provider."
            )));
        }
        let Some(f) = allowed.iter().find(|f| str_of(f, "name") == k) else {
            let names: Vec<&str> = allowed.iter().map(|f| str_of(f, "name")).collect();
            return Err(Error::Usage(if names.is_empty() {
                format!("{k} is not a setting: this connector takes none")
            } else {
                format!("{k} is not a setting; the settings: {}", names.join(", "))
            }));
        };
        check_choice(f, v)?;
        config.insert(k.to_owned(), v.to_owned());
    }
    for f in &allowed {
        let name = str_of(f, "name");
        let required = f.get("required").and_then(Value::as_bool).unwrap_or(false);
        if !required || config.contains_key(name) || !str_of(f, "default").is_empty() {
            continue;
        }
        let choices = choices(f);
        if !(prompt::interactive() && std::io::IsTerminal::is_terminal(&std::io::stdin())) {
            let hint = if choices.is_empty() {
                String::new()
            } else {
                format!(" (one of {})", choices.join(", "))
            };
            return Err(Error::Usage(format!(
                "{} is required: pass --config {name}=VALUE{hint}",
                label_of(f)
            )));
        }
        let question = if choices.is_empty() {
            label_of(f).to_owned()
        } else {
            format!("{} ({})", label_of(f), choices.join(", "))
        };
        let v = prompt::line(&question)?;
        check_choice(f, &v)?;
        config.insert(name.to_owned(), v);
    }
    Ok(config)
}

fn choices(f: &Value) -> Vec<String> {
    f.get("choices")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn check_choice(f: &Value, v: &str) -> Result<(), Error> {
    let choices = choices(f);
    if v.is_empty() || (!choices.is_empty() && !choices.iter().any(|c| c == v)) {
        let name = str_of(f, "name");
        return Err(Error::Usage(if choices.is_empty() {
            format!("{name} needs a value")
        } else {
            format!("{name} is one of {}", choices.join(", "))
        }));
    }
    Ok(())
}

/// The mode's secret fields, each from `--secret-file`, `--secret-stdin` or a prompt
/// without echo.
fn secrets(mode: &Value, input: &SecretInput) -> Result<BTreeMap<String, Redacted<String>>, Error> {
    let wanted: Vec<Value> = fields(mode, "fields")
        .into_iter()
        .filter(is_secret)
        .collect();
    let names: Vec<&str> = wanted.iter().map(|f| str_of(f, "name")).collect();
    let known = |k: &str| -> Result<(), Error> {
        if names.contains(&k) {
            Ok(())
        } else if names.is_empty() {
            Err(Error::Usage(format!(
                "{k} is not a secret of mode {}: it takes none",
                str_of(mode, "mode")
            )))
        } else {
            Err(Error::Usage(format!(
                "{k} is not a secret of mode {}; its secrets: {}",
                str_of(mode, "mode"),
                names.join(", ")
            )))
        }
    };
    let mut got: BTreeMap<String, Redacted<String>> = BTreeMap::new();
    for p in &input.files {
        let Some((k, path)) = p.split_once('=') else {
            return Err(Error::Usage(format!(
                "--secret-file takes FIELD=PATH, not {p}"
            )));
        };
        known(k)?;
        if got.contains_key(k) {
            return Err(Error::Usage(format!("{k} is given twice")));
        }
        got.insert(
            k.to_owned(),
            prompt::secret_from_file(k, std::path::Path::new(path))?,
        );
    }
    if let Some(k) = &input.stdin {
        known(k)?;
        if got.contains_key(k.as_str()) {
            return Err(Error::Usage(format!("{k} is given twice")));
        }
        got.insert(k.clone(), prompt::secret_from_stdin(k)?);
    }
    for f in &wanted {
        let name = str_of(f, "name");
        if !got.contains_key(name) {
            got.insert(name.to_owned(), prompt::secret(label_of(f))?);
        }
    }
    Ok(got)
}

fn no_secret_flags(input: &SecretInput, mode: &Value) -> Result<(), Error> {
    if input.files.is_empty() && input.stdin.is_none() {
        Ok(())
    } else {
        Err(Error::Usage(format!(
            "mode {} signs in at the provider in a browser; it takes no secret",
            str_of(mode, "mode")
        )))
    }
}

fn disclose(connector: &Value, out: Out) {
    if is_ai(connector) && !out.json {
        Out::note(&format!(
            "{} is an AI model: data you send in prompts goes to {} under your own agreement \
             with them.",
            str_of(connector, "name"),
            str_of(connector, "name")
        ));
    }
}

async fn add(s: &Session, args: ConnectionsAdd, out: Out) -> Result<(), Error> {
    let acc = acc(s, args.account)?;
    let connector = connectors::fetch(s, args.connector.trim()).await?;
    connectable(&connector)?;
    let mode = pick_mode(&connector, args.mode.as_deref())?;
    let mode_name = str_of(mode, "mode");
    let name = args.name.as_deref().map(slug).transpose()?;
    let config = settings(&connector, mode, &args.config)?;
    disclose(&connector, out);
    let provider = str_of(&connector, "name");
    let connection = if signs_in(mode) {
        no_secret_flags(&args.secrets, mode)?;
        let mut body = serde_json::json!({
            "connector": str_of(&connector, "id"),
            "auth_mode": mode_name,
            "config": config,
        });
        if !args.scopes.is_empty() {
            body["scopes"] = serde_json::json!(args.scopes);
        }
        if let Some(n) = &name {
            body["name"] = serde_json::json!(n);
        }
        sign_in(s, &acc, &body, provider, out).await?
    } else {
        if !args.scopes.is_empty() {
            return Err(Error::Usage(format!(
                "--scope is for a mode that signs in; {mode_name} takes a key"
            )));
        }
        let secrets = secrets(mode, &args.secrets)?;
        let body = Create {
            kind: str_of(&connector, "id"),
            auth_mode: mode_name,
            name: name.as_deref(),
            config: &config,
            credentials: exposed(&secrets),
        };
        let v: Value = s
            .api
            .send_body(Method::Post, &base(&acc), &body)
            .await
            .and_then(|r| r.json())
            .map_err(refused)?;
        drop(body);
        drop(secrets);
        connection_in(&v)
    };
    report(&connection, provider, "Connected", out);
    Ok(())
}

/// A key the provider refused is a failed call with what to do next.
fn refused(e: inorbithr::Error) -> Error {
    if e.status() == Some(400) {
        Error::with_hint(
            e,
            "Nothing was stored. Check the key, its permissions and the settings, then try again.",
        )
    } else {
        scoped(e)
    }
}

fn report(c: &Value, provider: &str, verb: &str, out: Out) {
    if out.json {
        Out::print_json(c);
        return;
    }
    let label = str_of(c, "label");
    Out::note(&format!(
        "{verb} {}{} on {provider}.",
        str_of(c, "name"),
        if label.is_empty() {
            String::new()
        } else {
            format!(" as {label}")
        }
    ));
    print_connection(c);
}

async fn reconnect(s: &Session, args: ConnectionsReconnect, out: Out) -> Result<(), Error> {
    let acc = acc(s, args.account)?;
    let c = resolve(s, &acc, &args.connection).await?;
    let connector_id = str_of(&c, "connector");
    if connector_id.is_empty() {
        return Err(Error::Usage(format!(
            "{} is a {} connection, not one made from a connector; change its secret in the console",
            str_of(&c, "name"),
            str_of(&c, "kind")
        )));
    }
    let connector = connectors::fetch(s, connector_id).await?;
    let current = str_of(&c, "auth_mode");
    let wanted = args
        .mode
        .as_deref()
        .or((!current.is_empty()).then_some(current));
    let mode = pick_mode(&connector, wanted)?;
    let switching = args.mode.as_deref().filter(|m| *m != current);
    let provider = str_of(&connector, "name");
    let updated = if signs_in(mode) {
        no_secret_flags(&args.secrets, mode)?;
        let body = serde_json::json!({
            "connector": connector_id,
            "auth_mode": str_of(mode, "mode"),
            "connection_id": str_of(&c, "id"),
        });
        sign_in(s, &acc, &body, provider, out).await?
    } else {
        let secrets = secrets(mode, &args.secrets)?;
        let body = Recredential {
            auth_mode: switching,
            credentials: exposed(&secrets),
        };
        let v: Value = s
            .api
            .send_body(Method::Patch, &path_of(&acc, &c, "")?, &body)
            .await
            .and_then(|r| r.json())
            .map_err(refused)?;
        drop(body);
        drop(secrets);
        connection_in(&v)
    };
    report(&updated, provider, "Reconnected", out);
    Ok(())
}

// --- signing in at the provider ------------------------------------------------------

/// Starts a connect session, opens the provider's page and waits until the person has
/// finished on the console's callback page. Returns the connection.
async fn sign_in(
    s: &Session,
    acc: &str,
    body: &Value,
    provider: &str,
    out: Out,
) -> Result<Value, Error> {
    let started: Value = s
        .api
        .send(
            Method::Post,
            &format!("{}/connect", base(acc)),
            &[],
            Some(body),
        )
        .await
        .and_then(|r| r.json())
        .map_err(not_offered)?;
    let url = authorize_url(str_of(&started, "authorize_url"))?;
    let session_id = segment("the connect session's id", str_of(&started, "session_id"))
        .map_err(|_| {
            Error::Failed(
                "the platform started no connect session to wait on; finish in the console".into(),
            )
        })?
        .to_owned();
    let wait = wait_for(str_of(&started, "expires_at"));
    if browser::available() && browser::open(&url) {
        Out::note(&format!(
            "Opened your browser to sign in to {provider}. If it did not open, visit:\n\n    {url}\n"
        ));
    } else {
        Out::note(&format!(
            "Open this link in a browser to sign in to {provider}:\n\n    {url}\n"
        ));
    }
    Out::note("Finish there; the console page it returns to completes the connection.");
    let path = format!("{}/connect/{session_id}", base(acc));
    let deadline = tokio::time::Instant::now() + wait;
    let mut waiting = prompt::Waiting::new(out.json);
    loop {
        let v: Value = s.api.get(&path, &[]).await.map_err(scoped)?;
        match str_of(&v, "status") {
            "pending" => {}
            "completed" => {
                waiting.done();
                return Ok(connection_in(&v));
            }
            "failed" => {
                waiting.done();
                return Err(Error::Failed(format!(
                    "signing in to {provider} failed: {}",
                    session_error(&v)
                )));
            }
            "expired" => {
                waiting.done();
                return Err(expired(provider));
            }
            other => {
                waiting.done();
                return Err(Error::Failed(format!(
                    "the connect session is {other:?}, which this iohr does not know; update iohr"
                )));
            }
        }
        // Redraw the waiting line between polls.
        let next = tokio::time::Instant::now() + POLL;
        while tokio::time::Instant::now() < next {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                waiting.done();
                return Err(expired(provider));
            }
            waiting.tick(&format!(
                "Waiting for you to finish signing in to {provider} ({}:{:02} left)",
                left.as_secs() / 60,
                left.as_secs() % 60
            ));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

fn expired(provider: &str) -> Error {
    Error::Failed(format!(
        "signing in to {provider} was not finished in time; run the command again"
    ))
}

/// The provider's page, which must be HTTPS: `iohr` opens it in a browser.
fn authorize_url(raw: &str) -> Result<String, Error> {
    match url::Url::parse(raw) {
        Ok(u) if u.scheme() == "https" && u.host_str().is_some() => Ok(u.into()),
        _ => Err(Error::Failed(
            "the platform answered no HTTPS sign-in page to open".into(),
        )),
    }
}

/// How long to wait: until the session's expiry, at most 15 minutes.
fn wait_for(expires_at: &str) -> Duration {
    OffsetDateTime::parse(expires_at, &Rfc3339)
        .ok()
        .and_then(|t| Duration::try_from(t - OffsetDateTime::now_utc()).ok())
        .map_or(DEFAULT_WAIT, |d| d.min(MAX_WAIT))
}

/// The session's `error`: a sentence, or an object with one.
fn session_error(v: &Value) -> String {
    match v.get("error") {
        Some(Value::String(e)) if !e.is_empty() => e.clone(),
        Some(e @ Value::Object(_)) => ["message", "error_description", "error", "code"]
            .iter()
            .map(|k| str_of(e, k))
            .find(|m| !m.is_empty())
            .unwrap_or("no reason given")
            .to_owned(),
        _ => "no reason given".into(),
    }
}

fn not_offered(e: inorbithr::Error) -> Error {
    if matches!(e.status(), Some(404 | 501)) {
        Error::with_hint(
            e,
            "This platform does not offer signing in to a connector from the command line yet; \
             connect it in the console.",
        )
    } else {
        scoped(e)
    }
}

// --- test, history, update, delete ---------------------------------------------------

async fn test(s: &Session, acc: &str, which: &str, out: Out) -> Result<(), Error> {
    let c = resolve(s, acc, which).await?;
    let v: Value = s
        .api
        .send(Method::Post, &path_of(acc, &c, "/test")?, &[], None)
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    let r = v.get("result").cloned().unwrap_or(Value::Null);
    let ok = r.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let name = str_of(&c, "name");
    if out.json {
        Out::print_json(&r);
    } else {
        let mut pairs = vec![
            ("outcome", str_of(&r, "status").to_owned()),
            ("message", str_of(&r, "message").to_owned()),
            ("error", str_of(&r, "error_class").to_owned()),
        ];
        if let Some(code) = r
            .get("status_code")
            .and_then(Value::as_u64)
            .filter(|c| *c > 0)
        {
            pairs.push(("status code", code.to_string()));
        }
        if let Some(ms) = r.get("latency_ms").and_then(Value::as_u64) {
            pairs.push(("latency", format!("{ms} ms")));
        }
        pairs.retain(|(_, v)| !v.is_empty());
        Out::pairs(&pairs);
    }
    if ok {
        if !out.json {
            Out::note(&format!("{name}: the test passed."));
        }
        Ok(())
    } else {
        let refused = str_of(&r, "error_class") == "auth"
            || matches!(
                r.get("status_code").and_then(Value::as_u64),
                Some(401 | 403)
            );
        Err(Error::Failed(if refused {
            format!(
                "{name}: the test failed, the provider refused the credential; \
                 `iohr connections reconnect {name}` gives it a new one"
            )
        } else {
            format!(
                "{name}: the test failed; `iohr connections history {name}` shows earlier outcomes"
            )
        }))
    }
}

struct Filters {
    action: Option<String>,
    status: Option<UseStatus>,
    consumer: Option<String>,
    limit: usize,
}

async fn history(
    s: &Session,
    acc: &str,
    which: Option<&str>,
    filters: Filters,
    out: Out,
) -> Result<(), Error> {
    let id = match which {
        Some(w) => Some(str_of(&resolve(s, acc, w).await?, "id").to_owned()),
        None => None,
    };
    if let Some(c) = &filters.consumer {
        consumer(c)?;
    }
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(id) = &id {
        query.push(("connection_id", id));
    }
    if let Some(a) = &filters.action {
        query.push(("action", a));
    }
    if let Some(st) = filters.status {
        query.push(("status", st.as_str()));
    }
    if let Some(c) = &filters.consumer {
        query.push(("consumer", c));
    }
    let uses = pages(
        &s.api,
        &format!("{}/history", base(acc)),
        &query,
        "uses",
        filters.limit,
    )
    .await
    .map_err(scoped)?;
    if out.json {
        Out::print_json(&uses);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = uses
        .iter()
        .map(|u| {
            let caller = match (str_of(u, "caller_kind"), str_of(u, "caller")) {
                (kind, "") => kind.to_owned(),
                (kind, id) => format!("{kind}:{id}"),
            };
            let status = match str_of(u, "error_class") {
                "" => str_of(u, "status").to_owned(),
                e => format!("{} ({e})", str_of(u, "status")),
            };
            vec![
                str_of(u, "at").to_owned(),
                str_of(u, "connection_id").to_owned(),
                str_of(u, "action").to_owned(),
                caller,
                status,
                u.get("latency_ms")
                    .and_then(Value::as_u64)
                    .map_or_else(String::new, |ms| format!("{ms} ms")),
            ]
        })
        .collect();
    Out::table(
        &["AT", "CONNECTION", "ACTION", "CALLER", "OUTCOME", "TIME"],
        &rows,
    );
    Ok(())
}

async fn pause(s: &Session, acc: &str, which: &str, paused: bool, out: Out) -> Result<(), Error> {
    let c = update(s, acc, which, &serde_json::json!({ "paused": paused })).await?;
    let name = str_of(&c, "name");
    done(
        &c,
        out,
        &if paused {
            format!(
                "Paused {name}: every call with it is refused until `iohr connections resume {name}`."
            )
        } else {
            format!("Resumed {name}.")
        },
    );
    Ok(())
}

async fn rename(
    s: &Session,
    acc: &str,
    which: &str,
    new_name: &str,
    out: Out,
) -> Result<(), Error> {
    let new_name = slug(new_name)?;
    let c = update(s, acc, which, &serde_json::json!({ "name": new_name })).await?;
    done(
        &c,
        out,
        &format!(
            "Renamed {which} to {}. Tools made from it are named after the new name.",
            str_of(&c, "name")
        ),
    );
    Ok(())
}

async fn update(s: &Session, acc: &str, which: &str, body: &Value) -> Result<Value, Error> {
    let c = resolve(s, acc, which).await?;
    let v: Value = s
        .api
        .send(Method::Patch, &path_of(acc, &c, "")?, &[], Some(body))
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    Ok(connection_in(&v))
}

async fn delete(s: &Session, acc: &str, which: &str, yes: bool, out: Out) -> Result<(), Error> {
    let c = resolve(s, acc, which).await?;
    let name = str_of(&c, "name");
    if !yes {
        let grants = c.get("grants").and_then(Value::as_u64).unwrap_or(0);
        prompt::confirm(
            &format!(
                "Delete {name} ({}{})? Its credential is destroyed and its {grants} live grant(s) \
                 stop working. Type its name to confirm",
                connector_of(&c),
                match str_of(&c, "label") {
                    "" => String::new(),
                    l => format!(", as {l}"),
                }
            ),
            name,
            &format!("deleting {name} needs a confirmation: run it at a terminal, or pass --yes"),
        )?;
    }
    s.api
        .send(Method::Delete, &path_of(acc, &c, "")?, &[], None)
        .await
        .map_err(scoped)?;
    if out.json {
        Out::print_json(&serde_json::json!({
            "id": str_of(&c, "id"), "name": name, "deleted": true,
        }));
    } else {
        Out::note(&format!(
            "Deleted {name}: its credential is destroyed and its grants no longer work."
        ));
    }
    Ok(())
}

// --- grants --------------------------------------------------------------------------

/// `product:reliability`, `key:ak_...`, `agent:NAME`, `avatar:ID`.
fn consumer(raw: &str) -> Result<(&str, &str), Error> {
    let bad = || {
        Error::Usage(format!(
            "a consumer is KIND:ID with KIND product, key, agent or avatar (product:reliability, \
             key:ak_..., agent:NAME), not {raw}"
        ))
    };
    let (kind, id) = raw.split_once(':').ok_or_else(bad)?;
    if !matches!(kind, "product" | "key" | "agent" | "avatar")
        || id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(bad());
    }
    Ok((kind, id))
}

/// `90d` from now, or an RFC 3339 time as it is.
fn expiry(raw: &str, now: OffsetDateTime) -> Result<String, Error> {
    let bad = || {
        Error::Usage(format!(
            "--expires takes days such as 90d, or an RFC 3339 time, not {raw}"
        ))
    };
    if let Some(n) = raw.strip_suffix('d') {
        let days: i64 = n.parse().map_err(|_| bad())?;
        if !(1..=3650).contains(&days) {
            return Err(bad());
        }
        return (now + time::Duration::days(days))
            .replace_nanosecond(0)
            .ok()
            .and_then(|t| t.format(&Rfc3339).ok())
            .ok_or_else(bad);
    }
    OffsetDateTime::parse(raw, &Rfc3339)
        .map(|_| raw.to_owned())
        .map_err(|_| bad())
}

async fn grant(
    s: &Session,
    acc: &str,
    which: &str,
    to: &str,
    actions: &[String],
    expires: Option<&str>,
    out: Out,
) -> Result<(), Error> {
    let (kind, id) = consumer(to)?;
    for a in actions {
        if a.is_empty()
            || !a
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::Usage(format!(
                "an action is a name such as create_incident, not {a:?}"
            )));
        }
    }
    let expires_at = expires
        .map(|e| expiry(e, OffsetDateTime::now_utc()))
        .transpose()?;
    let c = resolve(s, acc, which).await?;
    let mut body = serde_json::json!({
        "consumer": { "kind": kind, "id": id },
        "actions": actions,
    });
    if let Some(e) = &expires_at {
        body["expires_at"] = serde_json::json!(e);
    }
    let v: Value = s
        .api
        .send(
            Method::Post,
            &path_of(acc, &c, "/grants")?,
            &[],
            Some(&body),
        )
        .await
        .and_then(|r| r.json())
        .map_err(|e| {
            if e.status() == Some(403) && s.kind == Kind::Token {
                Error::with_hint(
                    e,
                    "Grants are made by a signed-in owner or admin, not by a token: `iohr login`, \
                     then run it again.",
                )
            } else {
                scoped(e)
            }
        })?;
    let g = v.get("grant").cloned().unwrap_or(Value::Null);
    if out.json {
        Out::print_json(&g);
    } else {
        Out::note(&format!(
            "Granted {to} {} on {} until {}.",
            actions.join(", "),
            str_of(&c, "name"),
            day(str_of(&g, "expires_at"))
        ));
        Out::raw(format!("{}\n", str_of(&g, "id")).as_bytes());
    }
    Ok(())
}

async fn grants(s: &Session, acc: &str, which: &str, out: Out) -> Result<(), Error> {
    let c = resolve(s, acc, which).await?;
    let all = pages(
        &s.api,
        &path_of(acc, &c, "/grants")?,
        &[],
        "grants",
        usize::MAX,
    )
    .await
    .map_err(scoped)?;
    if out.json {
        Out::print_json(&all);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|g| {
            let who = g
                .get("consumer")
                .map(|c| format!("{}:{}", str_of(c, "kind"), str_of(c, "id")))
                .unwrap_or_default();
            let actions = g
                .get("actions")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let state = if str_of(g, "revoked_at").is_empty() {
                str_of(g, "status").to_owned()
            } else {
                "revoked".to_owned()
            };
            vec![
                str_of(g, "id").to_owned(),
                who,
                actions,
                day(str_of(g, "expires_at")),
                state,
            ]
        })
        .collect();
    Out::table(&["ID", "CONSUMER", "ACTIONS", "EXPIRES", "STATE"], &rows);
    Ok(())
}

async fn revoke_grant(
    s: &Session,
    acc: &str,
    which: &str,
    grant: &str,
    out: Out,
) -> Result<(), Error> {
    let grant = segment("a grant's id", grant)?;
    let c = resolve(s, acc, which).await?;
    let v: Value = s
        .api
        .send(
            Method::Delete,
            &path_of(acc, &c, &format!("/grants/{grant}"))?,
            &[],
            None,
        )
        .await
        .and_then(|r| {
            if r.body.is_empty() {
                Ok(Value::Null)
            } else {
                r.json()
            }
        })
        .map_err(scoped)?;
    let g = v
        .get("grant")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "id": grant, "connection_id": str_of(&c, "id") }));
    if out.json {
        Out::print_json(&g);
    } else {
        Out::note(&format!(
            "Revoked {grant} on {}. The next call it covered is refused.",
            str_of(&c, "name")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{authorize_url, consumer, expiry, slug};
    use time::macros::datetime;

    #[test]
    fn consumers_and_names() {
        assert_eq!(
            consumer("product:reliability").ok(),
            Some(("product", "reliability"))
        );
        assert!(consumer("key:ak_01J").is_ok());
        for bad in [
            "person:x",
            "product:",
            "reliability",
            "agent:a b",
            "key:a/b",
        ] {
            assert!(consumer(bad).is_err(), "{bad}");
        }
        assert!(slug("pagerduty-eu").is_ok());
        for bad in ["PagerDuty", "-x", "a_b", "", "a b"] {
            assert!(slug(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn expiries() {
        let now = datetime!(2026-10-04 10:00:00.5 UTC);
        assert_eq!(
            expiry("90d", now).ok().as_deref(),
            Some("2027-01-02T10:00:00Z")
        );
        assert!(expiry("2027-01-01T00:00:00Z", now).is_ok());
        for bad in ["0d", "90", "d", "tomorrow", "99999d"] {
            assert!(expiry(bad, now).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_an_https_page_is_opened() {
        assert!(authorize_url("https://slack.com/oauth/v2/authorize?state=x").is_ok());
        for bad in [
            "http://slack.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "",
        ] {
            assert!(authorize_url(bad).is_err(), "{bad}");
        }
    }
}
