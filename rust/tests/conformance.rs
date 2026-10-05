#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::single_match_else
)]

//! The driver for `conformance/cases`: starts the replay server, loads every case,
//! runs its action through the public API of this crate, and compares the result and
//! the server's verdict with `expect` (`conformance/README.md`).
//!
//! The server binary is built by `mise run conformance:server:build`. Without it the
//! test is skipped with a note, unless `IOHR_TEST_REQUIRE_REPLAY=1` (set by
//! `mise run conformance:rust` and CI), which makes a missing server a failure.
//!
//! The pure-function vectors of `conformance/vectors/` run here too, without a server
//! (`docs/config.md` section 9.2), through the crate's public `config` functions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use inorbithr::config::{LoadOptions, Os};
use inorbithr::middleware::{BoxFuture, CallOptions, Middleware, Next, Request, Response};
use inorbithr::{
    Client, ClientBuilder, Code, ConfigError, Error, Headers, LogLevel, Profile, Public,
    RateLimitMode, RawResponse, Streams,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Deserialize)]
struct Loaded {
    case: Case,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    proxy_url: Option<String>,
    #[serde(default)]
    ca_file: Option<String>,
    #[serde(default)]
    client_cert_file: Option<String>,
    #[serde(default)]
    client_key_file: Option<String>,
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

#[derive(Deserialize, Clone)]
struct Action {
    op: String,
    #[serde(default)]
    args: BTreeMap<String, Value>,
    #[serde(default)]
    repeat: Option<u32>,
    #[serde(default)]
    concurrent: Option<u32>,
    #[serde(default)]
    take: Option<usize>,
    #[serde(default)]
    options: Option<ActionOptions>,
    #[serde(default)]
    rewrite: Option<Rewrite>,
}

#[derive(Deserialize, Clone, Default)]
struct ActionOptions {
    idempotency_key: Option<String>,
    traceparent: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize, Clone)]
struct Rewrite {
    after: u32,
    files: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct ClientOptions {
    max_retries: Option<u32>,
    timeout_ms: Option<u64>,
    key_id: Option<String>,
    key_secret: Option<String>,
    scopes: Option<Vec<String>>,
    streams: Option<String>,
    stream_idle_timeout_ms: Option<u64>,
    #[serde(default)]
    load: bool,
    #[serde(default)]
    env: BTreeMap<String, String>,
    config_file: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, String>,
    profile: Option<String>,
    credential_sources: Option<Vec<String>>,
    #[serde(default)]
    cli: bool,
    pipeline: Option<PipelineEdit>,
    log: Option<String>,
    log_headers: Option<bool>,
    log_allow_headers: Option<Vec<String>>,
    rate_limit: Option<String>,
    total_timeout_ms: Option<u64>,
    retry_budget_capacity: Option<u32>,
    tracing: Option<bool>,
    transport: Option<String>,
    ca_bundle: Option<bool>,
    no_proxy: Option<String>,
}

#[derive(Deserialize, Default)]
struct PipelineEdit {
    #[serde(default)]
    add: Vec<ProbeSpec>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Deserialize, Clone)]
struct ProbeSpec {
    name: String,
    stage: String,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Deserialize, Default)]
struct Expect {
    ok: Option<Value>,
    error: Option<ExpectError>,
    items: Option<Vec<Value>>,
    attempts: Option<u32>,
    token_exchanges: Option<u32>,
    probes: Option<BTreeMap<String, ExpectProbe>>,
    logs: Option<ExpectLogs>,
    spans: Option<Vec<ExpectSpan>>,
    #[serde(default, deserialize_with = "present")]
    rate_limit: Option<Value>,
    idempotency_key: Option<String>,
    config: Option<Value>,
}

/// A key that is present, even as `null`, is `Some`.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

#[derive(Deserialize)]
struct ExpectProbe {
    count: Option<usize>,
    #[serde(default)]
    seen: Vec<BTreeMap<String, String>>,
}

#[derive(Deserialize, Default)]
struct ExpectLogs {
    #[serde(default)]
    contains: Vec<Value>,
    #[serde(default)]
    excludes: Vec<String>,
}

#[derive(Deserialize)]
#[cfg_attr(not(feature = "otel"), allow(dead_code))]
struct ExpectSpan {
    name: String,
    kind: Option<String>,
    #[serde(default)]
    attributes: serde_json::Map<String, Value>,
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
    /// The server's certificate directory, removed with it whatever happens.
    _pki: tempfile::TempDir,
}

impl Drop for Replay {
    /// SIGTERM, so the server removes its certificate directory (`/tmp/replay-pki-*`);
    /// a kill is the fallback.
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            let stopped = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status()
                .is_ok_and(|s| s.success());
            if stopped {
                for _ in 0..50 {
                    if matches!(self.child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        let _ = self.child.start_kill();
    }
}

fn replay_bin() -> PathBuf {
    repo_root().join(format!(
        "conformance/server/bin/replay{}",
        std::env::consts::EXE_SUFFIX
    ))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn start_replay() -> Option<Replay> {
    let root = repo_root();
    let bin = root.join(format!(
        "conformance/server/bin/replay{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !bin.is_file() {
        let note = "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`";
        assert!(
            std::env::var_os("IOHR_TEST_REQUIRE_REPLAY").is_none(),
            "{note}"
        );
        eprintln!("{note}; skipping");
        return None;
    }
    let pki = tempfile::Builder::new()
        .prefix("replay-pki-rust-")
        .tempdir()
        .expect("a temporary directory");
    let mut child = Command::new(bin)
        .arg("--dir")
        .arg(pki.path())
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
    Some(Replay {
        child,
        url,
        _pki: pki,
    })
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

async fn load_case(http: &reqwest::Client, url: &str, name: &str) -> Result<Loaded, String> {
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
    Ok(resp.json::<Loaded>().await.expect("the loaded case"))
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

/// What a case's client recorded: probes, log records, spans.
#[derive(Default, Clone)]
struct Captured {
    probes: Arc<Mutex<BTreeMap<String, Vec<Headers>>>>,
    logs: Arc<Mutex<Vec<Value>>>,
    #[cfg(feature = "otel")]
    spans: Option<opentelemetry_sdk::trace::InMemorySpanExporter>,
    #[cfg(feature = "otel")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

/// A probe middleware: records the headers of every request it sees, and passes it on.
struct Probe {
    name: &'static str,
    seen: Arc<Mutex<BTreeMap<String, Vec<Headers>>>>,
}

impl Middleware for Probe {
    fn name(&self) -> &'static str {
        self.name
    }

    fn handle<'a>(
        &'a self,
        req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        self.seen
            .lock()
            .unwrap()
            .entry(self.name.to_owned())
            .or_default()
            .push(req.headers().clone());
        next.run(req)
    }
}

/// A case's temporary directory, with `{dir}` and `{replay}` substitution.
struct Scratch {
    dir: tempfile::TempDir,
    replay: String,
}

impl Scratch {
    fn new(replay: &str) -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            replay: replay.to_owned(),
        }
    }

    fn path(&self) -> String {
        self.dir.path().to_string_lossy().into_owned()
    }

    fn sub(&self, s: &str) -> String {
        s.replace("{replay}", &self.replay)
            .replace("{dir}", &self.path())
    }

    fn write(&self, files: &BTreeMap<String, String>) {
        for (name, content) in files {
            let p = self.dir.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, self.sub(content)).unwrap();
        }
    }
}

fn build_client(
    loaded: &Loaded,
    url: &str,
    scratch: &Scratch,
    captured: &mut Captured,
) -> Client<Public> {
    let options = &loaded.case.client;
    let base = loaded.base_url.clone().unwrap_or_else(|| url.to_owned());
    scratch.write(&options.files);
    let mut b = Client::<Public>::builder();
    if !options.load {
        b = b
            .base_url(&base)
            .token_url(format!("{base}/oauth2/token"))
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
    } else if let Some(n) = options.max_retries {
        b = b.max_retries(n);
    }
    if let Some(ms) = options.timeout_ms {
        b = b.timeout(Duration::from_millis(ms));
    }
    if options.streams.as_deref() == Some("socket") {
        b = b.streams(Streams::Socket);
    }
    if let Some(ms) = options.stream_idle_timeout_ms {
        b = b.stream_idle_timeout(Duration::from_millis(ms));
    }
    if let Some(ms) = options.total_timeout_ms {
        b = b.total_timeout(Duration::from_millis(ms));
    }
    if let Some(n) = options.retry_budget_capacity {
        b = b.retry_budget_capacity(n);
    }
    if let Some(m) = &options.rate_limit {
        b = b.rate_limit(match m.as_str() {
            "wait" => RateLimitMode::Wait,
            "off" => RateLimitMode::Off,
            _ => RateLimitMode::Observe,
        });
    }
    if let Some(level) = &options.log {
        b = b.log(match level.as_str() {
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Off,
        });
        let logs = Arc::clone(&captured.logs);
        b = b.logger(move |r| {
            logs.lock().unwrap().push(Value::Object(r.fields.clone()));
        });
    }
    if let Some(on) = options.log_headers {
        b = b.log_headers(on);
    }
    if let Some(names) = &options.log_allow_headers {
        b = b.log_allow_headers(names.clone());
    }
    if let Some(on) = options.tracing {
        b = b.tracing(on);
        #[cfg(feature = "otel")]
        if on {
            let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
            let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            b = b.tracer_provider(&provider);
            captured.spans = Some(exporter);
            captured.provider = Some(provider);
        }
    }
    if let Some("https" | "proxy" | "mtls") = options.transport.as_deref() {
        {
            if options.ca_bundle != Some(false)
                && let Some(ca) = &loaded.ca_file
            {
                b = b.ca_bundle(ca);
            }
            if options.transport.as_deref() == Some("mtls")
                && let (Some(c), Some(k)) = (&loaded.client_cert_file, &loaded.client_key_file)
            {
                b = b.client_cert(c, k);
            }
            if options.transport.as_deref() == Some("proxy")
                && let Some(p) = &loaded.proxy_url
            {
                b = b.proxy(p.as_str());
            }
        }
    }
    if let Some(n) = &options.no_proxy {
        b = b.no_proxy(
            n.split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_owned),
        );
    }
    if let Some(edit) = &options.pipeline {
        let seen = Arc::clone(&captured.probes);
        let add = edit.add.clone();
        let remove = edit.remove.clone();
        b = b.pipeline(move |p| {
            for spec in add {
                let probe = Probe {
                    name: Box::leak(spec.name.clone().into_boxed_str()),
                    seen: Arc::clone(&seen),
                };
                if let Some(before) = &spec.before {
                    p.insert_before(before, probe);
                } else if let Some(after) = &spec.after {
                    p.insert_after(after, probe);
                } else if spec.stage == "per_call" {
                    p.add_per_call(probe);
                } else {
                    p.add_per_retry(probe);
                }
            }
            for name in remove {
                p.remove(&name);
            }
            p
        });
    }
    if let Some(p) = &options.profile {
        b = b.profile(p);
    }
    if let Some(s) = &options.credential_sources {
        b = b.credential_sources(s.clone());
    }
    if options.cli {
        b = b.cli_path(replay_bin());
    }
    if options.load {
        if let Some(text) = &options.config_file {
            let file = scratch.dir.path().join("config.toml");
            std::fs::write(&file, scratch.sub(text)).unwrap();
            b = b.config_file(file);
        }
        let env: BTreeMap<String, String> = options
            .env
            .iter()
            .map(|(k, v)| (k.clone(), scratch.sub(v)))
            .collect();
        let home = scratch.dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        b = b.load_options(
            LoadOptions::new()
                .env(env)
                .home(Some(home))
                .cwd(scratch.dir.path()),
        );
        return b.load().expect("the client loads");
    }
    b.build().expect("the client builds")
}

/// The operations the cases name, called through the generated public surface
/// (`inorbithr::public`), so a passing case proves the generated code.
async fn call(client: &Client<Public>, action: &Action) -> Result<RawResponse, Error> {
    // A case's answer holds the fields its behaviour needs, not every field the model
    // requires: the raw answer is the result when only the typed decoding failed.
    match call_typed(client, action).await {
        Err(Error::Decode { raw, .. }) => Ok(*raw),
        other => other,
    }
}

async fn call_typed(client: &Client<Public>, action: &Action) -> Result<RawResponse, Error> {
    use inorbithr::public::{
        AccountsGetUsageParams, CreateEndpointRequest, Surface as _, UpdateEndpointRequest,
    };
    let with;
    let client = match &action.options {
        Some(o) => {
            let mut c = CallOptions::new();
            if let Some(k) = &o.idempotency_key {
                c = c.idempotency_key(k);
            }
            if let Some(t) = &o.traceparent {
                c = c.traceparent(t);
            }
            if let Some(ms) = o.timeout_ms {
                c = c.timeout(Duration::from_millis(ms));
            }
            with = client.with_options(c);
            &with
        }
        None => client,
    };
    let arg = |k: &str| {
        action
            .args
            .get(k)
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
    };
    match action.op.as_str() {
        "me" => client.me().await.map(|r| r.raw),
        "accounts.get_me" => client.accounts().get_me().await.map(|r| r.raw),
        "accounts.get_usage" => {
            let org = arg("org_id").unwrap_or_default();
            let params = AccountsGetUsageParams {
                from: arg("from"),
                to: arg("to"),
            };
            client
                .accounts()
                .get_usage(&org, &params)
                .await
                .map(|r| r.raw)
        }
        "radar.get_digest" => {
            let id = arg("id").unwrap_or_default();
            client.radar().get_digest(&id).await.map(|r| r.raw)
        }
        "events.create_endpoint" => {
            let body: CreateEndpointRequest =
                serde_json::from_value(Value::Object(action.args.clone().into_iter().collect()))
                    .expect("the case's args are a CreateEndpointRequest");
            client.events().create_endpoint(&body).await.map(|r| r.raw)
        }
        "events.update_endpoint" => {
            let id = arg("endpoint_id").unwrap_or_default();
            let mut fields = action.args.clone();
            fields.remove("endpoint_id"); // the path carries it
            let body: UpdateEndpointRequest =
                serde_json::from_value(Value::Object(fields.into_iter().collect()))
                    .expect("the case's args are an UpdateEndpointRequest");
            client
                .events()
                .update_endpoint(&id, &body)
                .await
                .map(|r| r.raw)
        }
        "events.delete_endpoint" => {
            let id = arg("endpoint_id").unwrap_or_default();
            client.events().delete_endpoint(&id).await.map(|r| r.raw)
        }
        other => panic!("the conformance schema names an op this driver does not know: {other}"),
    }
}

/// A stream's action: every item it yielded, as wire JSON, and the error it ended with.
async fn stream(client: &Client<Public>, action: &Action) -> (Vec<Value>, Option<Error>) {
    use inorbithr::public::{EventsStreamEventsParams, Surface as _};
    assert_eq!(action.op, "events.stream_events", "an unknown stream op");
    let arg = |k: &str| {
        action
            .args
            .get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let params = EventsStreamEventsParams {
        types: arg("types"),
        account_id: arg("account_id"),
    };
    let mut items = Vec::new();
    let mut events = match client.events().stream_events(&params).await {
        Ok(s) => s,
        Err(e) => return (items, Some(e)),
    };
    while let Some(event) = events.next().await {
        match event {
            Ok(ev) => items.push(serde_json::to_value(ev).expect("a model serialises")),
            Err(e) => return (items, Some(e)),
        }
        if action.take.is_some_and(|n| items.len() >= n) {
            break;
        }
    }
    (items, None)
}

fn check_stream(case: &Case, items: &[Value], error: Option<&Error>) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(want) = &case.expect.items {
        if want.len() == items.len() {
            for (i, (w, g)) in want.iter().zip(items).enumerate() {
                if !subset(w, g) {
                    problems.push(format!("item {i}: want a superset of {w}, got {g}"));
                }
            }
        } else {
            problems.push(format!(
                "items: want {}, got {}: {items:?}",
                want.len(),
                items.len()
            ));
        }
    }
    match (error, &case.expect.error) {
        (Some(e), Some(want)) => problems.extend(check_error(e, want)),
        (Some(e), None) => problems.push(format!("want a clean end, got {e}")),
        (None, Some(_)) => problems.push("want an error, got a clean end".into()),
        (None, None) => {}
    }
    problems
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

/// A header value matcher: a literal, `*`, `$name` (the same value every time), or
/// `~regex` over the whole value.
fn header_matches(want: &str, got: Option<&str>, captures: &mut BTreeMap<String, String>) -> bool {
    let Some(got) = got else {
        return false;
    };
    if want == "*" {
        return true;
    }
    if let Some(name) = want.strip_prefix('$') {
        return captures
            .entry(name.to_owned())
            .or_insert_with(|| got.to_owned())
            == got;
    }
    if let Some(re) = want.strip_prefix('~') {
        return regex::Regex::new(&format!("^(?:{re})$")).is_ok_and(|r| r.is_match(got));
    }
    want == got
}

/// The M6 expectations: probes, logs, spans, the rate-limit snapshot, the idempotency
/// key and the configuration.
fn check_m6(
    case: &Case,
    client: &Client<Public>,
    results: &[Result<RawResponse, Error>],
    captured: &Captured,
) -> Vec<String> {
    let mut problems = Vec::new();
    let e = &case.expect;
    if let Some(probes) = &e.probes {
        let seen = captured.probes.lock().unwrap();
        let mut captures = BTreeMap::new();
        for (name, want) in probes {
            let got = seen.get(name).cloned().unwrap_or_default();
            if let Some(n) = want.count
                && n != got.len()
            {
                problems.push(format!(
                    "probe {name}: want {n} requests, got {}",
                    got.len()
                ));
            }
            for (i, headers) in want.seen.iter().enumerate() {
                let Some(have) = got.get(i) else {
                    problems.push(format!("probe {name}: no request {i}"));
                    continue;
                };
                for (k, v) in headers {
                    if !header_matches(v, have.get(k), &mut captures) {
                        problems.push(format!(
                            "probe {name}: request {i} header {k}: want {v}, got {:?}",
                            have.get(k)
                        ));
                    }
                }
            }
        }
    }
    if let Some(logs) = &e.logs {
        let records = captured.logs.lock().unwrap();
        for want in &logs.contains {
            if !records.iter().any(|r| subset(want, r)) {
                problems.push(format!("logs: no record has {want}; records: {records:?}"));
            }
        }
        for x in &logs.excludes {
            for r in records.iter() {
                if r.to_string().contains(x.as_str()) {
                    problems.push(format!("logs: {x:?} appears in {r}"));
                }
            }
        }
    }
    #[cfg(feature = "otel")]
    if let Some(want) = &e.spans {
        let mut spans = captured
            .spans
            .as_ref()
            .map(|x| x.get_finished_spans().unwrap())
            .unwrap_or_default();
        spans.sort_by_key(|s| s.start_time);
        if spans.len() == want.len() {
            for (w, got) in want.iter().zip(&spans) {
                if w.name != got.name {
                    problems.push(format!("span: want {}, got {}", w.name, got.name));
                }
                let kind = match got.span_kind {
                    opentelemetry::trace::SpanKind::Client => "client",
                    opentelemetry::trace::SpanKind::Internal => "internal",
                    _ => "other",
                };
                if let Some(k) = &w.kind
                    && k != kind
                {
                    problems.push(format!("span {}: want kind {k}, got {kind}", w.name));
                }
                for (k, v) in &w.attributes {
                    let have = got
                        .attributes
                        .iter()
                        .find(|kv| kv.key.as_str() == k)
                        .map(|kv| match &kv.value {
                            opentelemetry::Value::I64(n) => json!(n),
                            opentelemetry::Value::Bool(b) => json!(b),
                            opentelemetry::Value::F64(f) => json!(f),
                            other => json!(other.as_str()),
                        });
                    if have.as_ref() != Some(v) {
                        problems.push(format!(
                            "span {}: attribute {k} want {v}, got {have:?}",
                            w.name
                        ));
                    }
                }
            }
        } else {
            problems.push(format!(
                "spans: want {}, got {}: {:?}",
                want.len(),
                spans.len(),
                spans.iter().map(|s| s.name.to_string()).collect::<Vec<_>>()
            ));
        }
    }
    let first = results.iter().find_map(|r| r.as_ref().ok());
    if let Some(want) = &e.rate_limit {
        let got = first
            .and_then(|r| r.rate_limit.as_ref())
            .map_or(Value::Null, |s| serde_json::to_value(s).unwrap());
        if !subset(want, &got) {
            problems.push(format!("rate_limit: want {want}, got {got}"));
        }
    }
    if let Some(want) = &e.idempotency_key {
        let got = first.and_then(|r| r.idempotency_key.clone());
        let ok = match (want.as_str(), &got) {
            ("*", Some(k)) => !k.is_empty(),
            (w, Some(k)) => w == k,
            _ => false,
        };
        if !ok {
            problems.push(format!("idempotency_key: want {want}, got {got:?}"));
        }
        if want != "*" && !first.is_some_and(|r| r.idempotency_replayed) {
            problems.push("idempotency_replayed: want true".into());
        }
    }
    if let Some(want) = &e.config {
        let got = client.config().describe();
        if !subset(want, &got) {
            problems.push(format!("config: want a subset {want}, got {got}"));
        }
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
        let loaded = match load_case(&http, &replay.url, name).await {
            Ok(c) => c,
            Err(why) => {
                eprintln!("skip {name}: {why}");
                continue;
            }
        };
        let case = &loaded.case;
        if case.pending.iter().any(|l| l == "rust") {
            eprintln!("skip {name}: pending for rust");
            continue;
        }
        if cfg!(not(feature = "otel")) && case.expect.spans.is_some() {
            eprintln!("skip {name}: spans need the `otel` feature (mise run conformance:rust)");
            continue;
        }
        let scratch = Scratch::new(loaded.base_url.as_deref().unwrap_or(&replay.url));
        let mut captured = Captured::default();
        let client = build_client(&loaded, &replay.url, &scratch, &mut captured);
        if matches!(case.area.as_str(), "sse" | "socket") {
            let (items, error) = stream(&client, &case.action).await;
            drop(client);
            // A socket's cancel and close go out after the caller stopped reading.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let v = verdict(&http, &replay.url).await;
            let mut problems = check(case, &[], &v);
            problems.extend(check_stream(case, &items, error.as_ref()));
            ran += 1;
            if problems.is_empty() {
                eprintln!("pass {name}");
            } else {
                eprintln!("FAIL {name}:\n  {}", problems.join("\n  "));
                failed.push(case.name.clone());
            }
            continue;
        }
        let results: Vec<Result<RawResponse, Error>> = if let Some(n) = case.action.concurrent {
            let action = std::sync::Arc::new(case.action.clone());
            let calls = (0..n).map(|_| {
                let client = client.clone();
                let action = std::sync::Arc::clone(&action);
                async move { call(&client, &action).await }
            });
            futures_join_all(calls).await
        } else {
            let mut out = Vec::new();
            for i in 0..case.action.repeat.unwrap_or(1) {
                out.push(call(&client, &case.action).await);
                if let Some(r) = &case.action.rewrite
                    && r.after == i + 1
                {
                    scratch.write(&r.files);
                }
            }
            out
        };
        #[cfg(feature = "otel")]
        if let Some(p) = &captured.provider {
            let _ = p.force_flush();
        }
        let v = verdict(&http, &replay.url).await;
        let mut problems = check(case, &results, &v);
        problems.extend(check_m6(case, &client, &results, &captured));
        drop(client);
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

// The vectors (`conformance/vectors/`, docs/config.md section 9.2): pure functions, no
// server, through the crate's public `config` functions and `load` with injected inputs.

/// The typed profile the vectors name.
struct AcmeCi;

impl Profile for AcmeCi {
    const NAME: &'static str = "acme-ci";
}

fn vectors(kind: &str) -> Vec<(PathBuf, Value)> {
    let dir = repo_root().join("conformance/vectors").join(kind);
    let mut out: Vec<(PathBuf, Value)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            let v: Value = serde_yaml_ng::from_str(&text).unwrap();
            (p, v)
        })
        .filter(|(_, v)| {
            !v["pending"]
                .as_array()
                .is_some_and(|a| a.iter().any(|l| l == "rust"))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no vectors in {}", dir.display());
    out
}

fn os_of(v: &Value) -> Os {
    match v.as_str() {
        Some("macos") => Os::Macos,
        Some("windows") => Os::Windows,
        _ => Os::Linux,
    }
}

fn env_of(v: &Value, dir: &str) -> BTreeMap<String, String> {
    v.as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().unwrap_or_default().replace("{dir}", dir),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `expected` is a subset of `actual`; paths compare after `\` becomes `/`.
fn vsubset(expected: &Value, actual: &Value, at: &str) -> Result<(), String> {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                let av = a
                    .get(k)
                    .ok_or_else(|| format!("{at}.{k}: missing in {actual}"))?;
                vsubset(ev, av, &format!("{at}.{k}"))?;
            }
            Ok(())
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Err(format!("{at}: expected {expected}, got {actual}"));
            }
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                vsubset(ev, av, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (e, a) if e == a => Ok(()),
        (Value::String(e), Value::String(a)) if e.replace('\\', "/") == a.replace('\\', "/") => {
            Ok(())
        }
        (e, a) => Err(format!("{at}: expected {e}, got {a}")),
    }
}

fn substitute(v: &Value, dir: &str, file: &str) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace("{file}", file).replace("{dir}", dir)),
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, dir, file)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), substitute(x, dir, file)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Code options by catalogue name, onto a builder.
fn with_code<P: Profile>(
    mut b: ClientBuilder<P>,
    code: &serde_json::Map<String, Value>,
) -> ClientBuilder<P> {
    for (k, v) in code {
        let text = v.as_str().unwrap_or_default();
        let dur = || inorbithr::config::parse_duration(text).expect("a duration in code");
        b = match k.as_str() {
            "timeout" => b.timeout(dur()),
            "connect_timeout" => b.connect_timeout(dur()),
            "total_timeout" => b.total_timeout(dur()),
            "proxy" => b.proxy(text),
            "config_file" => b.config_file(text),
            "profile" => b.profile(text),
            "http_client" => b.http_client(reqwest::Client::new()),
            "max_retries" => b.max_retries(u32::try_from(v.as_u64().unwrap()).unwrap()),
            other => panic!("a vector sets {other} in code, which this driver does not know"),
        };
    }
    b
}

fn load_for<P: Profile>(
    code: &serde_json::Map<String, Value>,
    options: LoadOptions,
    cli: Option<&Path>,
) -> Result<Value, Error> {
    let mut b = with_code(Client::<P>::builder(), code).load_options(options);
    if let Some(c) = cli {
        b = b.cli_path(c);
    }
    b.load().map(|c| c.config().describe())
}

#[allow(clippy::too_many_lines)]
fn run_config_vector(path: &Path, v: &Value) -> Result<(), String> {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap().to_owned();
    let input = v.get("input").cloned().unwrap_or_else(|| json!({}));
    let os = os_of(&input["os"]);
    let env = env_of(&input["env"], &dir);
    let home_flag = input["home"].as_bool().unwrap_or(false);
    let home = home_flag.then(|| format!("{dir}/home"));
    if let Some(h) = &home {
        std::fs::create_dir_all(h).unwrap();
    }
    if let Some(files) = input["files"].as_object() {
        for (name, content) in files {
            let p = tmp.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content.as_str().unwrap_or_default()).unwrap();
        }
    }
    let mut code = input["code"].as_object().cloned().unwrap_or_default();
    let mut file = format!("{dir}/config.toml");
    let options = |env: &BTreeMap<String, String>| {
        LoadOptions::new()
            .env(env.clone())
            .os(os)
            .home(home.clone())
            .cwd(&dir)
    };
    if let Some(text) = input["config_file"].as_str() {
        if home_flag {
            // Where the file would be read from, INORBIT_CONFIG_FILE aside: `off` still
            // finds a file there and must not read it.
            let mut located = env.clone();
            located.remove("INORBIT_CONFIG_FILE");
            let p = inorbithr::config::config_file_path(&options(&located), None)
                .ok_or("the vector writes a file at a default location that does not exist")?;
            std::fs::create_dir_all(Path::new(&p).parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            file = p;
        } else {
            std::fs::write(&file, text).unwrap();
            code.insert("config_file".into(), json!(file));
        }
    }
    // `present`: any program that exists stands for `iohr`; it is never run here.
    let cli = (input["cli"].as_str() == Some("present")).then(replay_bin_or_self);
    let got = match input["profile_type"].as_str() {
        Some("acme-ci") => load_for::<AcmeCi>(&code, options(&env), cli.as_deref()),
        Some(other) => panic!("a vector names a typed profile this driver lacks: {other}"),
        None => load_for::<Public>(&code, options(&env), cli.as_deref()),
    };
    let expect = substitute(&v["expect"], &dir, &file);
    let name = path.file_name().unwrap().to_string_lossy();
    let shown = match &got {
        Ok(d) => d.to_string(),
        Err(e) => format!("{e} {e:?}"),
    };
    if let Some(ex) = expect["excludes"].as_array() {
        for x in ex {
            let x = x.as_str().unwrap_or_default();
            if shown.contains(x) {
                return Err(format!("{name}: {x:?} appears in {shown}"));
            }
        }
    }
    if let Some(err) = expect.get("error") {
        let e = match got {
            Ok(d) => return Err(format!("{name}: expected an error, got {d}")),
            Err(e) => e,
        };
        let text = e.to_string();
        let problems = match &e {
            Error::Config(ConfigError::Invalid { problems }) => problems.clone(),
            other => {
                return Err(format!(
                    "{name}: expected ConfigError::Invalid, got {other:?}"
                ));
            }
        };
        if let Some(want) = err["problems"].as_array() {
            if want.len() != problems.len() {
                return Err(format!(
                    "{name}: expected {} problems, got {problems:?}",
                    want.len()
                ));
            }
            for (w, have) in want.iter().zip(&problems) {
                if let Some(s) = w["setting"].as_str()
                    && s != have.setting
                {
                    return Err(format!("{name}: setting {s} != {have:?}"));
                }
                if let Some(s) = w["source"].as_str()
                    && s != have.source
                {
                    return Err(format!("{name}: source {s:?} != {have:?}"));
                }
                if let Some(s) = w["message_contains"].as_str()
                    && !have.message.contains(s)
                {
                    return Err(format!("{name}: message lacks {s:?}: {have:?}"));
                }
            }
        }
        if let Some(parts) = err["message_contains"].as_array() {
            for p in parts {
                let p = p.as_str().unwrap_or_default();
                if !text.contains(p) {
                    return Err(format!("{name}: error lacks {p:?}:\n{text}"));
                }
            }
        }
        return Ok(());
    }
    let d = got.map_err(|e| format!("{name}: unexpected error:\n{e}"))?;
    for key in ["profile", "settings", "credential", "pipeline"] {
        if let Some(want) = expect.get(key) {
            vsubset(want, &d[key], &format!("{name}: {key}"))?;
        }
    }
    if let Some(want) = expect.get("config_file") {
        let have = d["config_file"].as_str().map(|s| s.replace('\\', "/"));
        let want = want.as_str().map(|s| s.replace('\\', "/"));
        if want != have {
            return Err(format!("{name}: config_file {want:?} != {have:?}"));
        }
    }
    if let Some(absent) = expect["settings_absent"].as_array() {
        for a in absent {
            let a = a.as_str().unwrap_or_default();
            if d["settings"].get(a).is_some() {
                return Err(format!("{name}: {a} should not be in settings"));
            }
        }
    }
    if let Some(want) = expect["ignored"].as_array() {
        let have = d["ignored"].as_array().cloned().unwrap_or_default();
        for w in want {
            if !have.iter().any(|h| vsubset(w, h, "").is_ok()) {
                return Err(format!("{name}: ignored lacks {w}: {have:?}"));
            }
        }
    }
    Ok(())
}

/// A program that exists, to stand for an installed `iohr`.
fn replay_bin_or_self() -> PathBuf {
    let replay = replay_bin();
    if replay.is_file() {
        replay
    } else {
        std::env::current_exe().unwrap()
    }
}

#[test]
fn config_vectors() {
    let failures: Vec<String> = vectors("config")
        .iter()
        .filter_map(|(p, v)| run_config_vector(p, v).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn config_path_vectors() {
    for (_, v) in vectors("config-path") {
        for c in v["checks"].as_array().unwrap() {
            let options = LoadOptions::new()
                .env(env_of(&c["env"], ""))
                .os(os_of(&c["os"]))
                .home(c["home"].as_str());
            let got =
                inorbithr::config::config_file_path(&options, c["code"]["config_file"].as_str());
            assert_eq!(got.as_deref(), c["expect"].as_str(), "{}", c["summary"]);
        }
    }
}

#[test]
fn duration_vectors() {
    for (_, v) in vectors("durations") {
        for c in v["checks"].as_array().unwrap() {
            let got = inorbithr::config::parse_duration(c["value"].as_str().unwrap())
                .map(|d| u64::try_from(d.as_millis()).unwrap());
            assert_eq!(got, c["expect"].as_u64(), "{}", c["value"]);
        }
    }
}

#[test]
fn no_proxy_vectors() {
    for (_, v) in vectors("no-proxy") {
        for c in v["checks"].as_array().unwrap() {
            let options = LoadOptions::new().env(env_of(&c["env"], ""));
            let got = inorbithr::config::proxy_for(
                c["url"].as_str().unwrap(),
                &options,
                c["code"]["proxy"].as_str(),
                None,
            );
            match (&c["expect"], got) {
                (Value::Object(o), Err(ConfigError::Invalid { .. })) if o.contains_key("error") => {
                }
                (want, Ok(got)) => assert_eq!(want.as_str(), got.as_deref(), "{}", c["summary"]),
                (want, Err(e)) => panic!("{}: want {want}, got {e}", c["summary"]),
            }
        }
    }
}

#[test]
fn rate_limit_vectors() {
    for (_, v) in vectors("rate-limit") {
        for c in v["checks"].as_array().unwrap() {
            let headers = Headers::new(
                c["headers"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned())),
            );
            let got = inorbithr::RateLimit::from_headers(&headers)
                .map_or(Value::Null, |s| serde_json::to_value(s).unwrap());
            assert_eq!(c["expect"], got, "{}", c["summary"]);
        }
    }
}

#[test]
fn the_error_lists_every_problem_and_never_a_secret() {
    let options = LoadOptions::new().env([
        ("INORBIT_TOKEN", "t"),
        ("INORBIT_TIMEOUT", "30"),
        ("INORBIT_CLIENT_KEY_PASSWORD", "pw-never-shown"),
        ("INORBIT_CONFIG_FILE", "off"),
        ("INORBIT_LOG", "loud"),
    ]);
    let e = Client::<Public>::builder()
        .load_options(options)
        .load()
        .unwrap_err();
    let text = e.to_string();
    assert!(
        text.starts_with("configuration is invalid (2 problems):"),
        "{text}"
    );
    assert!(text.contains("timeout: \"30\" is not a duration; write it with a unit, such as 30s (from env INORBIT_TIMEOUT)"), "{text}");
    assert!(text.contains("log: \"loud\""), "{text}");
    assert!(!text.contains("pw-never-shown"));
}
