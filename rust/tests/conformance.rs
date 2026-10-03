#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stderr)]

//! The driver for `conformance/cases`: starts the replay server, loads every case,
//! runs its action through the public API of this crate, and compares the result and
//! the server's verdict with `expect` (`conformance/README.md`).
//!
//! The server binary is built by `mise run conformance:server:build`. Without it the
//! test is skipped with a note, unless `IOHR_TEST_REQUIRE_REPLAY=1` (set by
//! `mise run conformance:rust` and CI), which makes a missing server a failure.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use inorbithr::{Client, Code, Error, Method, Operation, Public, RawResponse};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Deserialize)]
struct Loaded {
    case: Case,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    area: String,
    #[serde(default)]
    pending: Vec<String>,
    action: Action,
    #[serde(default)]
    client: ClientOptions,
    #[serde(default)]
    expect: Expect,
}

#[derive(Deserialize)]
struct Action {
    op: String,
    #[serde(default)]
    args: BTreeMap<String, Value>,
    #[serde(default)]
    repeat: Option<u32>,
    #[serde(default)]
    concurrent: Option<u32>,
}

#[derive(Deserialize, Default)]
struct ClientOptions {
    max_retries: Option<u32>,
    timeout_ms: Option<u64>,
    key_id: Option<String>,
    key_secret: Option<String>,
    scopes: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct Expect {
    ok: Option<Value>,
    error: Option<ExpectError>,
    attempts: Option<u32>,
    token_exchanges: Option<u32>,
}

#[derive(Deserialize)]
struct ExpectError {
    kind: Option<String>,
    code: Option<String>,
    status: Option<u16>,
    message_contains: Option<String>,
    message_excludes: Option<String>,
}

#[derive(Deserialize)]
struct Verdict {
    status: String,
    #[serde(default)]
    token_exchanges: u32,
    #[serde(default)]
    attempts: u32,
    #[serde(default)]
    mismatch: Option<Value>,
    #[serde(default)]
    next: Option<Value>,
}

struct Replay {
    child: Child,
    url: String,
}

impl Drop for Replay {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn start_replay() -> Option<Replay> {
    let root = repo_root();
    let bin = root.join("conformance/server/bin/replay");
    if !bin.is_file() {
        let note = "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`";
        assert!(
            std::env::var_os("IOHR_TEST_REQUIRE_REPLAY").is_none(),
            "{note}"
        );
        eprintln!("{note}; skipping");
        return None;
    }
    let mut child = Command::new(bin)
        .arg("--addr")
        .arg("127.0.0.1:0")
        .arg("--cases")
        .arg(root.join("conformance/cases"))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("start the replay server");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    let first = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
        .await
        .expect("the replay server announces itself within 10 s")
        .expect("read the replay server's first line")
        .expect("the replay server prints its address");
    let url = first
        .strip_prefix("replay: listening on ")
        .unwrap_or_else(|| panic!("unexpected first line: {first}"))
        .trim()
        .to_owned();
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Some(Replay { child, url })
}

fn control() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("the control client starts")
}

async fn list_cases(http: &reqwest::Client, url: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct List {
        cases: Vec<String>,
    }
    http.get(format!("{url}/_cases"))
        .send()
        .await
        .expect("GET /_cases")
        .json::<List>()
        .await
        .expect("a case list")
        .cases
}

async fn load_case(http: &reqwest::Client, url: &str, name: &str) -> Result<Case, String> {
    let resp = http
        .post(format!("{url}/_case"))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .expect("POST /_case");
    if resp.status() == 501 {
        return Err("not implemented by the replay server yet".into());
    }
    assert!(
        resp.status().is_success(),
        "{name}: loading answered {}",
        resp.status()
    );
    Ok(resp.json::<Loaded>().await.expect("the loaded case").case)
}

async fn verdict(http: &reqwest::Client, url: &str) -> Verdict {
    http.get(format!("{url}/_result"))
        .send()
        .await
        .expect("GET /_result")
        .json()
        .await
        .expect("a verdict")
}

fn build_client(url: &str, options: &ClientOptions) -> Client<Public> {
    let mut b = Client::<Public>::builder()
        .base_url(url)
        .token_url(format!("{url}/oauth2/token"))
        .key(
            options.key_id.clone().unwrap_or_else(|| "ak_test".into()),
            options
                .key_secret
                .clone()
                .unwrap_or_else(|| "s3cr3t".into()),
        )
        .scopes(
            options
                .scopes
                .clone()
                .unwrap_or_else(|| vec!["identity:read".into()]),
        )
        .max_retries(options.max_retries.unwrap_or(2));
    if let Some(ms) = options.timeout_ms {
        b = b.timeout(Duration::from_millis(ms));
    }
    b.build().expect("the client builds")
}

/// The operations the cases name, called through the raw path until the generated
/// public surface replaces this table.
fn operation(action: &Action) -> Operation<'static> {
    let arg = |k: &str| {
        action
            .args
            .get(k)
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
    };
    match action.op.as_str() {
        "me" => Operation::new(Method::Get, "/v1/me").named("me"),
        "accounts.get_me" => {
            Operation::new(Method::Get, "/v1/accounts/me").named("accounts.get_me")
        }
        "accounts.get_usage" => {
            let org = arg("org_id").unwrap_or_default();
            Operation::new(Method::Get, format!("/v1/accounts/orgs/{org}/usage"))
                .named("accounts.get_usage")
                .query_opt("from", arg("from"))
                .query_opt("to", arg("to"))
        }
        other => panic!("the conformance schema names an op this driver does not know: {other}"),
    }
}

/// `want` is a subset of `got`: objects by key, arrays element by element, scalars equal.
fn subset(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => w
            .iter()
            .all(|(k, v)| g.get(k).is_some_and(|gv| subset(v, gv))),
        (Value::Array(w), Value::Array(g)) => {
            w.len() == g.len() && w.iter().zip(g).all(|(a, b)| subset(a, b))
        }
        _ => want == got,
    }
}

fn error_kind(e: &Error) -> &'static str {
    match e {
        Error::Api(_) => "api",
        Error::Connection { .. } => "connection",
        Error::Timeout { .. } => "timeout",
        Error::Auth(_) => "auth",
        Error::Config(_) => "config",
        _ => "other",
    }
}

fn check(case: &Case, results: &[Result<RawResponse, Error>], verdict: &Verdict) -> Vec<String> {
    let mut problems = Vec::new();
    if verdict.status != "pass" {
        problems.push(format!(
            "server: {} mismatch={} next={}",
            verdict.status,
            verdict
                .mismatch
                .as_ref()
                .map_or("none".into(), Value::to_string),
            verdict
                .next
                .as_ref()
                .map_or("none".into(), Value::to_string)
        ));
    }
    if let Some(want) = case.expect.attempts
        && verdict.attempts != want
    {
        problems.push(format!("attempts: want {want}, got {}", verdict.attempts));
    }
    if let Some(want) = case.expect.token_exchanges
        && verdict.token_exchanges != want
    {
        problems.push(format!(
            "token_exchanges: want {want}, got {}",
            verdict.token_exchanges
        ));
    }
    for result in results {
        match (result, &case.expect.ok, &case.expect.error) {
            (Ok(raw), Some(want), _) => {
                let got: Value = serde_json::from_slice(&raw.body).unwrap_or(Value::Null);
                if !subset(want, &got) {
                    problems.push(format!("ok: want a superset of {want}, got {got}"));
                }
            }
            (Ok(raw), None, Some(_)) => {
                problems.push(format!("want an error, got HTTP {}", raw.status));
            }
            (Err(e), _, Some(want)) => problems.extend(check_error(e, want)),
            (Err(e), Some(_), None) => problems.push(format!("want ok, got {e}")),
            _ => {}
        }
    }
    problems
}

fn check_error(e: &Error, want: &ExpectError) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(kind) = &want.kind
        && error_kind(e) != kind
    {
        problems.push(format!(
            "error kind: want {kind}, got {} ({e})",
            error_kind(e)
        ));
    }
    if let Error::Api(api) = e {
        if let Some(code) = &want.code
            && api.code != Code::from(code.as_str())
        {
            problems.push(format!("error code: want {code}, got {}", api.code));
        }
        if let Some(status) = want.status
            && api.status != status
        {
            problems.push(format!("error status: want {status}, got {}", api.status));
        }
    } else if want.code.is_some() || want.status.is_some() {
        problems.push(format!("error: want an API error, got {e}"));
    }
    let message = e.to_string();
    if let Some(s) = &want.message_contains
        && !message.contains(s)
    {
        problems.push(format!(
            "message: want it to contain {s:?}, got {message:?}"
        ));
    }
    if let Some(s) = &want.message_excludes
        && message.contains(s)
    {
        problems.push(format!("message: must not contain {s:?}, got {message:?}"));
    }
    problems
}

#[tokio::test(flavor = "multi_thread")]
async fn every_case_passes() {
    let Some(replay) = start_replay().await else {
        return;
    };
    let http = control();
    let names = list_cases(&http, &replay.url).await;
    assert!(!names.is_empty(), "no cases listed");
    let mut failed = Vec::new();
    let mut ran = 0;
    for name in &names {
        let case = match load_case(&http, &replay.url, name).await {
            Ok(c) => c,
            Err(why) => {
                eprintln!("skip {name}: {why}");
                continue;
            }
        };
        if case.pending.iter().any(|l| l == "rust") {
            eprintln!("skip {name}: pending for rust");
            continue;
        }
        if matches!(case.area.as_str(), "sse" | "socket") {
            eprintln!("skip {name}: streaming comes with the streaming milestone");
            continue;
        }
        let client = build_client(&replay.url, &case.client);
        let results: Vec<Result<RawResponse, Error>> = if let Some(n) = case.action.concurrent {
            let calls = (0..n).map(|_| {
                let client = client.clone();
                let op = operation(&case.action);
                async move { client.send(op).await }
            });
            futures_join_all(calls).await
        } else {
            let mut out = Vec::new();
            for _ in 0..case.action.repeat.unwrap_or(1) {
                out.push(client.send(operation(&case.action)).await);
            }
            out
        };
        let v = verdict(&http, &replay.url).await;
        let problems = check(&case, &results, &v);
        ran += 1;
        if problems.is_empty() {
            eprintln!("pass {name}");
        } else {
            eprintln!("FAIL {name}:\n  {}", problems.join("\n  "));
            failed.push(case.name.clone());
        }
    }
    assert!(ran > 0, "no case ran");
    assert!(failed.is_empty(), "failed cases: {failed:?}");
}

/// Runs the futures at once and keeps their order, without another dependency.
async fn futures_join_all<F, T>(futures: impl IntoIterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.expect("a call task finishes"));
    }
    out
}
