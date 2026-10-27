//! `iohr runner` against a mock coordinator and a fake `claude` on PATH: what is sent,
//! that the child never sees a credential, and that nothing starts without the
//! subscription.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use wiremock::matchers::{body_string_contains, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SUB: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#;
const TURN: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"{\"say\":\"hello\",\"state\":\"s\",\"actions\":[{\"kind\":\"ask\",\"payload\":{\"q\":1}}],\"evidence\":[\"e1\"],\"left\":[]}","session_id":"s1","num_turns":2,"duration_ms":5,"total_cost_usd":0.5,"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":30,"cache_creation_input_tokens":40}}"#;
const API_KEY: &str = r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#;

fn token() -> String {
    token_with(&["coordinator:runner", "coordinator:read"])
}

fn token_with(scopes: &[&str]) -> String {
    let enc = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(json!({"alg": "RS256"})),
        enc(
            json!({"sub": "usr_me", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
                   "scp": scopes, "org": "acc_1", "plan": "free", "key": "key_1"})
        )
    )
}

/// A `claude` that answers `auth status` and `--version`, and records the environment of
/// every call.
fn fake_claude(dir: &Path, auth: &str) {
    let p = dir.join("claude");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nd=$(dirname \"$0\")\nenv >> \"$d/env.txt\"\ncase \"$1\" in\n  auth) echo '{auth}';;\n  --version) echo '9.9.9 (fake)';;\n  *) cat > /dev/null; printf '%s\n' '{TURN}';;\nesac\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn iohr(server: &MockServer, dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.args(args)
        .env_clear()
        .env("IOHR_BASE_URL", server.uri())
        .env("IOHR_CONFIG_DIR", dir.join("config"))
        .env("IOHR_PROFILE", "p")
        .env("HOME", dir)
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        // What must never reach claude or the coordinator.
        .env("ANTHROPIC_API_KEY", "sk-ant-SECRETKEY")
        .env("ANTHROPIC_BASE_URL", "https://proxy.example")
        .env("AWS_SECRET_ACCESS_KEY", "AWSSECRET")
        .env("GH_TOKEN", "ghp_GHSECRET")
        .env("GITHUB_TOKEN", "ghs_GHSECRET")
        .env("AZURE_API_KEY", "AZURESECRET")
        .env("FOO_SECRET", "FOOSECRET")
        .stdin(Stdio::null());
    c
}

/// Signs `iohr` in as profile `p` with `tok`, the token kept in an owner-only file (the
/// test machine has no Keychain to use; the runner's key goes to the same store).
async fn login(server: &MockServer, dir: &Path, tok: &str) {
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subject": "usr_me", "kind": "person", "org": "acc_1", "scopes": ["identity:read"]
        })))
        .mount(server)
        .await;
    let mut c = iohr(
        server,
        dir,
        &[
            "login",
            "--with-token",
            "--insecure-storage",
            "--profile",
            "p",
        ],
    );
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{tok}\n").as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
}

fn sha(s: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(s.as_bytes())
        .iter()
        .fold(String::new(), |mut o, b| {
            let _ = write!(o, "{b:02x}");
            o
        })
}

const SALT: &str = "5a17000000000000000000000000000000000000000000000000000000005a17";
const CHALLENGE: &str = "c0de000000000000000000000000000000000000000000000000000000000c0d";

fn run(c: &mut Command) -> Output {
    c.output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

async fn bodies(server: &MockServer, p: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == p)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

/// The account every coordinator request named, as `(METHOD path, account_id)`: from the
/// JSON body of a POST, from the query string of a GET. `None` when it named none.
async fn accounts_named(server: &MockServer) -> Vec<(String, Option<String>)> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().starts_with("/v1/coordinator/"))
        .map(|r| {
            let account = if r.method.as_str() == "GET" {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == "account_id")
                    .map(|(_, v)| v.into_owned())
            } else {
                serde_json::from_slice::<Value>(&r.body)
                    .ok()
                    .and_then(|b| b["account_id"].as_str().map(str::to_owned))
            };
            (format!("{} {}", r.method, r.url.path()), account)
        })
        .collect()
}

fn runner_json() -> Value {
    json!({"runner_id": "run_1", "user": "usr_me", "name": "mac", "status": "online",
           "last_seen_at": "2026-10-12T08:00:00Z", "providers": []})
}

/// What every claude call of the test saw in its environment must hold none of this.
fn assert_no_secrets(dir: &Path) {
    let env = std::fs::read_to_string(dir.join("env.txt")).unwrap();
    for bad in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "AWS_SECRET",
        "IOHR_TOKEN",
        "SECRETKEY",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "AZURE_API_KEY",
        "FOO_SECRET",
        "GHSECRET",
        "AZURESECRET",
        "FOOSECRET",
        "IOHR_PROFILE",
    ] {
        assert!(!env.contains(bad), "{bad} reached claude");
    }
    assert!(env.contains("HOME="));
}

async fn mount_register(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": []})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/challenge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"challenge": CHALLENGE, "identity_salt": SALT, "account_id": "acc_1"}),
        ))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runner": runner_json()})))
        .mount(server)
        .await;
}

fn verify(public_key: &str, message: &str, signature: &str) -> bool {
    let key = STANDARD.decode(public_key).unwrap();
    let sig = STANDARD.decode(signature).unwrap();
    UnparsedPublicKey::new(&ED25519, &key)
        .verify(message.as_bytes(), &sig)
        .is_ok()
}

#[tokio::test]
async fn register_signs_the_challenge_and_never_sends_a_credential() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token()).await;
    mount_register(&server).await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("run_1"));

    let sent = bodies(&server, "/v1/coordinator/runners").await;
    let post = sent.iter().find(|b| b.get("name").is_some()).unwrap();
    assert_eq!(post["name"], "mac");
    // Every class unless `[runner] kinds` says otherwise; capacity 1 unless set.
    assert_eq!(post["kinds"], json!(["coordinate", "build"]));
    assert_eq!(post["capacity"], 1);
    assert_eq!(post["providers"][0]["method"], "subscription");
    // The identity hash is sha256(salt + "\n" + line).
    assert_eq!(
        post["identity_sha256"]["claude"],
        sha(&format!("{SALT}\n{SUB}"))
    );
    // A standard, padded base64 key; the challenge; a signature over the registration lines.
    let key = post["public_key"].as_str().unwrap();
    assert_eq!(key.len(), 44);
    assert_eq!(post["challenge"], CHALLENGE);
    let message = format!("iohr.coordinator.v1.RegisterRunner\n{CHALLENGE}\n{key}");
    assert!(verify(key, &message, post["signature"].as_str().unwrap()));
    let all = serde_json::to_string(&sent).unwrap();
    assert!(!all.contains("SECRETKEY") && !all.contains("firstParty"));
    assert_no_secrets(dir.path());
    // The key is in the profile's store, not a file of the runner's own.
    assert!(
        dir.path()
            .join("config/secrets")
            .read_dir()
            .unwrap()
            .count()
            >= 2
    );
}

#[tokio::test]
async fn register_and_status_act_on_the_profiles_account_or_the_flags() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token()).await;
    mount_register(&server).await;
    // The profile's account (acc_1, the token's) when no flag is given: challenge,
    // the list that checks for an existing runner, and the registration itself.
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let named = accounts_named(&server).await;
    for path in [
        "GET /v1/coordinator/runners",
        "POST /v1/coordinator/runners/challenge",
        "POST /v1/coordinator/runners",
    ] {
        assert!(
            named.iter().any(|(p, _)| p == path),
            "{path} was not called: {named:?}"
        );
    }
    for (p, a) in &named {
        assert_eq!(a.as_deref(), Some("acc_1"), "{p} named no account");
    }

    // `--account` wins over the profile's, on register and on status.
    let before = named.len();
    for args in [
        &["runner", "register", "--name", "mac", "--account", "team_9"][..],
        &["runner", "status", "--account", "team_9"][..],
    ] {
        let o = run(&mut iohr(&server, dir.path(), args));
        assert_eq!(o.status.code(), Some(0), "{args:?}: {}", text(&o));
    }
    let named = accounts_named(&server).await;
    assert!(named.len() > before);
    for (p, a) in &named[before..] {
        assert_eq!(a.as_deref(), Some("team_9"), "{p}");
    }
}

#[tokio::test]
async fn register_refuses_an_api_key_sign_in_and_calls_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), API_KEY);
    login(&server, dir.path(), &token()).await;
    let before = server.received_requests().await.unwrap().len();
    let o = run(&mut iohr(&server, dir.path(), &["runner", "register"]));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("subscription only"));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn the_runner_needs_the_runner_and_read_scopes() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token_with(&["coordinator:runner"])).await;
    let before = server.received_requests().await.unwrap().len();
    let o = run(&mut iohr(&server, dir.path(), &["runner", "register"]));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("coordinator:read") && text(&o).contains("--scope"),
        "{}",
        text(&o)
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

/// The bug: `iohr login` never asked for the runner's scopes, so `iohr runner register`
/// could not be satisfied by any sign-in. The scopes the runner demands are read from its
/// own refusal and must reach the sign-in service's request, device and browser alike.
#[tokio::test]
async fn login_asks_the_sign_in_service_for_the_scopes_the_runner_demands() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token_with(&[])).await;
    let o = run(&mut iohr(&server, dir.path(), &["runner", "register"]));
    let hint = text(&o);
    let wanted = hint
        .split("iohr login --scope ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .unwrap_or_else(|| panic!("the refusal names no scopes: {hint}"))
        .trim_matches('`')
        .to_owned();
    assert!(wanted.contains("coordinator:runner"), "{wanted}");

    let uri = server.uri();
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": uri,
            "authorization_endpoint": format!("{uri}/oauth2/auth"),
            "token_endpoint": format!("{uri}/oauth2/token"),
            "device_authorization_endpoint": format!("{uri}/oauth2/device/auth"),
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/device/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dc", "user_code": "ABCD2345",
            "verification_uri": format!("{uri}/oauth2/device/verify"),
            "expires_in": 600, "interval": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": token(), "token_type": "bearer", "expires_in": 900,
            "refresh_token": "REFRESHMARKER",
            "id_token": format!("e30.{}.x", URL_SAFE_NO_PAD.encode(
                json!({"iss": uri, "aud": ["iohr-cli"], "sub": "usr_me"}).to_string())),
        })))
        .mount(&server)
        .await;
    let o = run(iohr(
        &server,
        dir.path(),
        &[
            "login",
            "--device",
            "--insecure-storage",
            "--profile",
            "q",
            "--scope",
            &wanted,
        ],
    )
    .env("IOHR_ISSUER", &uri));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let sent = server.received_requests().await.unwrap();
    let body = sent
        .iter()
        .find(|r| r.url.path() == "/oauth2/device/auth")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .unwrap();
    let scope: String = url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == "scope")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let asked: Vec<&str> = scope.split(' ').collect();
    for w in wanted.split(',') {
        assert!(asked.contains(&w), "{w} missing from {scope}");
    }
    for d in ["openid", "offline_access", "iohr.api"] {
        assert!(asked.contains(&d), "default {d} missing from {scope}");
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    reason = "one scenario, the signature rebuilt line by line"
)]
async fn run_takes_the_pack_keeps_the_lease_alive_and_reports_a_signed_turn() {
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    // First machine session: register, which makes and stores the key.
    let first = MockServer::start().await;
    login(&first, dir.path(), &token()).await;
    mount_register(&first).await;
    let o = run(&mut iohr(
        &first,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let reg = bodies(&first, "/v1/coordinator/runners").await;
    let public_key = reg
        .iter()
        .find_map(|b| b["public_key"].as_str())
        .unwrap()
        .to_owned();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": [runner_json()]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/run_1/heartbeat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "runner": runner_json(), "heartbeat_interval_seconds": 1,
            "offered": [{"lease_id": "lease_1", "runner_id": "run_1", "job_id": "job_1",
                         "turn_id": "turn_1", "fencing_token": "5"}]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/run_1/heartbeat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"runner": runner_json(), "heartbeat_interval_seconds": 1})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/leases/lease_1/accept"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"lease": {"lease_id": "lease_1"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/leases/lease_1/pack"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"pack": {
            "turn_id": "turn_1", "user": "usr_me", "kind": "coordinate",
            "pack_sha256": "ab".repeat(32), "system_prompt": "SYS", "prompt": "hi",
            "model": "sonnet", "max_turns": 5, "allowed_tools": ["Read"],
            "wall_clock_secs": 60, "thread_id": "th_1", "job_id": "job_1", "attempt": 2,
            "lease_id": "lease_1", "fencing_token": "5", "token_budget": 4000,
            "reply_shape": "{}"
        }})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/leases/lease_1/heartbeat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"stop": false})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("^/v1/coordinator/turns/turn_1/report$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"turn": {}})))
        .mount(&server)
        .await;

    let o = run(&mut iohr(
        &server,
        dir.path(),
        &[
            "runner",
            "run",
            "--name",
            "mac",
            "--interval",
            "1",
            "--max-beats",
            "3",
        ],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let accepts = bodies(&server, "/v1/coordinator/leases/lease_1/accept").await;
    assert_eq!(accepts.len(), 1);
    assert_eq!(accepts[0]["fencing_token"], "5");
    assert_eq!(accepts[0]["env_scrubbed"], true);
    assert_eq!(accepts[0]["worktree_is_main_checkout"], false);
    // The pack was asked for with the lease's fencing token.
    let pack_req = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == "/v1/coordinator/leases/lease_1/pack")
        .unwrap();
    assert_eq!(
        pack_req.url.query(),
        Some("fencing_token=5&account_id=acc_1")
    );
    // Every call of the loop names the profile's account: the token's own would be the
    // person's personal one.
    let named = accounts_named(&server).await;
    for path in [
        "GET /v1/coordinator/runners",
        "POST /v1/coordinator/runners/run_1/heartbeat",
        "POST /v1/coordinator/leases/lease_1/accept",
        "GET /v1/coordinator/leases/lease_1/pack",
        "POST /v1/coordinator/leases/lease_1/heartbeat",
        "POST /v1/coordinator/turns/turn_1/report",
    ] {
        assert!(
            named.iter().any(|(p, _)| p == path),
            "{path} was not called: {named:?}"
        );
    }
    for (p, a) in &named {
        assert_eq!(a.as_deref(), Some("acc_1"), "{p} named no account");
    }
    let lease_beats = bodies(&server, "/v1/coordinator/leases/lease_1/heartbeat").await;
    assert_ne!(lease_beats.len(), 0, "no lease heartbeat during the turn");

    let reports = bodies(&server, "/v1/coordinator/turns/turn_1/report").await;
    assert_eq!(reports.len(), 1, "{}", text(&o));
    let r = &reports[0];
    assert_eq!(
        (r["outcome"].as_str(), r["attempt"].as_u64()),
        (Some("replied"), Some(2))
    );
    assert_eq!(r["fencing_token"], "5");
    assert_eq!(r["reply"]["actions"][0]["kind"], "ask");
    assert_eq!(r["reply"]["actions"][0]["payload_json"], r#"{"q":1}"#);
    assert_eq!(r["reply"]["evidence"], json!(["e1"]));
    assert_eq!(r["usage"]["cache_write"], "40");

    // The signature verifies against the registered key over the README's 27 lines,
    // rebuilt here from the report as sent.
    let h = |v: &Value| sha(v.as_str().unwrap_or_default());
    let list = |lines: Vec<String>| sha(&lines.join("\n"));
    let strs = |v: &Value| -> Vec<String> { v.as_array().unwrap().iter().map(h).collect() };
    let actions: Vec<String> = r["reply"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| list(vec![h(&a["kind"]), h(&a["payload_json"])]))
        .collect();
    let bits = format!("{:016x}", r["total_cost_usd"].as_f64().unwrap().to_bits());
    let n = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    let lines = [
        "iohr.coordinator.v1.ReportTurn".to_owned(),
        h(&r["turn_id"]),
        h(&r["lease_id"]),
        n(&r["fencing_token"]),
        h(&r["runner_id"]),
        h(&r["pack_sha256"]),
        h(&r["outcome"]),
        h(&r["reply"]["say"]),
        h(&r["reply"]["state"]),
        list(actions),
        list(strs(&r["reply"]["evidence"])),
        list(strs(&r["reply"]["left"])),
        h(&r["provider"]),
        h(&r["provider_session_id"]),
        r["resumed"].to_string(),
        h(&r["provider_version"]),
        n(&r["num_turns"]),
        n(&r["duration_ms"]),
        bits,
        n(&r["usage"]["input"]),
        n(&r["usage"]["output"]),
        n(&r["usage"]["cache_read"]),
        n(&r["usage"]["cache_write"]),
        list(strs(&r["denied_tools"])),
        h(&r["detail"]),
        n(&r["attempt"]),
        h(&r["resets_at"]),
    ];
    assert_eq!(lines.len(), 27);
    assert!(
        verify(
            &public_key,
            &lines.join("\n"),
            r["signature"].as_str().unwrap()
        ),
        "the report's signature does not verify"
    );
    assert_no_secrets(dir.path());
    let t = text(&o);
    assert!(t.contains("1 replies delivered"), "{t}");
}

/// Registers runner `mac` on its own server (which makes this machine's key), then
/// mounts a coordinator that lists it and answers every heartbeat with no offer.
async fn registered_and_idle(dir: &Path) -> MockServer {
    fake_claude(dir, SUB);
    let first = MockServer::start().await;
    login(&first, dir, &token()).await;
    mount_register(&first).await;
    let o = run(&mut iohr(
        &first,
        dir,
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": [runner_json()]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/run_1/heartbeat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"runner": runner_json(), "heartbeat_interval_seconds": 1})),
        )
        .mount(&server)
        .await;
    server
}

const BEAT: &str = "/v1/coordinator/runners/run_1/heartbeat";

fn add_to_config(dir: &Path, toml: &str) {
    let p = dir.join("config").join("config.toml");
    let mut text = std::fs::read_to_string(&p).unwrap();
    text.push_str(toml);
    std::fs::write(&p, text).unwrap();
}

/// Waits up to 15 s for a heartbeat body that `want` accepts.
async fn beat_where(server: &MockServer, want: impl Fn(&Value) -> bool) -> bool {
    for _ in 0..150 {
        if bodies(server, BEAT).await.iter().any(&want) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// Waits up to 15 s for `n` heartbeats; the count seen.
async fn beats_at_least(server: &MockServer, n: usize) -> usize {
    let mut seen = 0;
    for _ in 0..150 {
        seen = bodies(server, BEAT).await.len();
        if seen >= n {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    seen
}

fn signal(pid: u32, sig: &str) {
    let ok = Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .unwrap()
        .success();
    assert!(ok, "kill {sig} {pid}");
}

#[tokio::test]
async fn run_declares_the_configured_capacity_and_a_sighup_changes_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    add_to_config(
        dir.path(),
        "\n[runner]\ncapacity = 3\nkinds = [\"coordinate\"]\n",
    );
    let log = std::fs::File::create(dir.path().join("run.log")).unwrap();
    let mut child = iohr(
        &server,
        dir.path(),
        &["runner", "run", "--name", "mac", "--interval", "1"],
    )
    .stdout(Stdio::null())
    .stderr(log)
    .spawn()
    .unwrap();
    let first = beat_where(&server, |b| b["capacity"] == 3).await;
    // Two beats in: the SIGHUP handler is surely installed.
    let seen = beats_at_least(&server, 2).await;
    let p = dir.path().join("config").join("config.toml");
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, text.replace("capacity = 3", "capacity = 8")).unwrap();
    signal(child.id(), "-HUP");
    let raised = beat_where(&server, |b| b["capacity"] == 8).await;
    signal(child.id(), "-INT");
    let status = child.wait().unwrap();
    let log = std::fs::read_to_string(dir.path().join("run.log")).unwrap();
    assert!(first, "no heartbeat declared capacity 3: {log}");
    assert!(seen >= 2, "fewer than two heartbeats: {log}");
    assert!(raised, "the SIGHUP did not change the capacity sent: {log}");
    assert!(status.success(), "{log}");
    let beats = bodies(&server, BEAT).await;
    for b in &beats {
        assert_eq!(b["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(b["kinds"], json!(["coordinate"]));
    }
    assert!(log.contains("config.toml read again: capacity 8"), "{log}");
}

#[tokio::test]
async fn a_bad_capacity_on_reload_keeps_the_one_in_use() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    add_to_config(dir.path(), "\n[runner]\ncapacity = 2\n");
    let log = std::fs::File::create(dir.path().join("run.log")).unwrap();
    let mut child = iohr(
        &server,
        dir.path(),
        &["runner", "run", "--name", "mac", "--interval", "1"],
    )
    .stdout(Stdio::null())
    .stderr(log)
    .spawn()
    .unwrap();
    let seen = beats_at_least(&server, 2).await;
    let p = dir.path().join("config").join("config.toml");
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, text.replace("capacity = 2", "capacity = 99")).unwrap();
    signal(child.id(), "-HUP");
    let before = bodies(&server, BEAT).await.len();
    // Two beats after the reload.
    let _ = beats_at_least(&server, before + 2).await;
    signal(child.id(), "-INT");
    let _ = child.wait().unwrap();
    let log = std::fs::read_to_string(dir.path().join("run.log")).unwrap();
    assert!(seen >= 2, "{log}");
    let beats = bodies(&server, BEAT).await;
    assert!(beats.len() > before, "no heartbeat after the reload: {log}");
    assert!(beats.iter().all(|b| b["capacity"] == 2), "{beats:?}");
    assert!(log.contains("was not taken"), "{log}");
}

#[tokio::test]
async fn an_older_coordinator_that_refuses_the_new_fields_gets_the_old_heartbeat() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    // What a coordinator from before these fields answers: its REST binding refuses a
    // field it does not know.
    Mock::given(method("POST"))
        .and(path(BEAT))
        .and(body_string_contains("\"version\""))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "bad_request",
            "error": "body unrecognized field `capacity`, expected one of runner_id, providers, account_id",
            "details": []
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    add_to_config(dir.path(), "\n[runner]\ncapacity = 4\n");
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &[
            "runner",
            "run",
            "--name",
            "mac",
            "--interval",
            "1",
            "--max-beats",
            "2",
        ],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let beats = bodies(&server, BEAT).await;
    // The first beat tried the new body once; every beat after it is the old one.
    assert_eq!(beats[0]["capacity"], 4, "{beats:?}");
    let old: Vec<&Value> = beats.iter().skip(1).collect();
    assert!(old.len() >= 2, "{beats:?}");
    for b in old {
        assert!(
            b.get("version").is_none() && b.get("capacity").is_none(),
            "{b}"
        );
        assert_eq!(b["runner_id"], "run_1");
    }
    assert!(text(&o).contains("0 failed"), "{}", text(&o));
}

#[tokio::test]
async fn config_show_names_each_value_and_its_source_without_a_sign_in() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    login(&server, dir.path(), &token()).await;
    add_to_config(
        dir.path(),
        "\n[runner]\ncapacity = 4\nalways_on = true\n\n[profiles.p.runner]\ncapacity = 8\n",
    );
    let before = server.received_requests().await.unwrap().len();
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "config", "show", "--json"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        v["capacity"],
        json!({"value": 8, "source": "[profiles.p.runner]"})
    );
    assert_eq!(v["always_on"], json!({"value": true, "source": "[runner]"}));
    assert_eq!(
        v["kinds"],
        json!({"value": ["coordinate", "build"], "source": "default"})
    );
    assert_eq!(v["build_floor"]["floor_bytes"], 100_000_000_000_u64);
    // A flag wins over both tables.
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &[
            "runner",
            "config",
            "show",
            "--json",
            "--capacity",
            "2",
            "--kinds",
            "build",
        ],
    ));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["capacity"], json!({"value": 2, "source": "flag"}));
    assert_eq!(v["kinds"], json!({"value": ["build"], "source": "flag"}));
    // A bad value is refused, naming where it came from.
    add_to_config(dir.path(), "");
    let p = dir.path().join("config").join("config.toml");
    let t = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, t.replace("capacity = 8", "capacity = 40")).unwrap();
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "config", "show"],
    ));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("[profiles.p.runner]"), "{}", text(&o));
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        before,
        "config show called the API"
    );
}

#[tokio::test]
async fn run_needs_a_registered_runner() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token()).await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o).contains("iohr runner register"));
}

#[tokio::test]
async fn register_repairs_a_name_whose_key_this_machine_does_not_hold() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token()).await;
    mount_register(&server).await;
    // Same name, registered from another machine: its key is not ours.
    let mut other = runner_json();
    other["public_key"] = json!("A".repeat(43) + "=");
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": [other]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/challenge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"challenge": CHALLENGE, "identity_salt": SALT, "account_id": "acc_1"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runner": runner_json()})))
        .mount(&server)
        .await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(!text(&o).contains("already registered"), "{}", text(&o));
    let sent = bodies(&server, "/v1/coordinator/runners").await;
    assert!(
        sent.iter().any(|b| b.get("public_key").is_some()),
        "a fresh registration was sent"
    );
}

#[tokio::test]
async fn register_with_the_matching_key_is_already_registered_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    let first = MockServer::start().await;
    login(&first, dir.path(), &token()).await;
    mount_register(&first).await;
    let o = run(&mut iohr(
        &first,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let key = bodies(&first, "/v1/coordinator/runners")
        .await
        .iter()
        .find_map(|b| b["public_key"].as_str().map(str::to_owned))
        .unwrap();

    let second = MockServer::start().await;
    let mut mine = runner_json();
    mine["public_key"] = json!(key);
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": [mine]})))
        .mount(&second)
        .await;
    let o = run(&mut iohr(
        &second,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("already registered"), "{}", text(&o));
    let sent = bodies(&second, "/v1/coordinator/runners").await;
    assert!(sent.iter().all(|b| b.get("public_key").is_none()));
}

// ---- `iohr runner gate-report` (platform RFC 0120, the Checked gate) ----

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", text(&out));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A bare origin with one commit on main, and a clone on a branch one commit ahead.
fn checkout(root: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(root.join("origin")).unwrap();
    git(&root.join("origin"), &["init", "--bare", "--quiet"]);
    git(root, &["clone", "--quiet", "origin", "work"]);
    let work = root.join("work");
    std::fs::write(work.join("a.txt"), "a\n").unwrap();
    git(&work, &["add", "a.txt"]);
    git(&work, &["commit", "--quiet", "-m", "main"]);
    git(&work, &["push", "--quiet", "origin", "HEAD:main"]);
    git(&work, &["checkout", "--quiet", "-b", "feature"]);
    std::fs::write(work.join("b.txt"), "b\n").unwrap();
    git(&work, &["add", "b.txt"]);
    git(&work, &["commit", "--quiet", "-m", "feature"]);
    work
}

/// Registers (which makes and stores the key), then a second service that lists the
/// runner with that key and takes a gate report with `answer`.
async fn gate_server(dir: &Path, answer: ResponseTemplate) -> (MockServer, String) {
    let first = MockServer::start().await;
    login(&first, dir, &token()).await;
    mount_register(&first).await;
    let o = run(&mut iohr(
        &first,
        dir,
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let key = bodies(&first, "/v1/coordinator/runners")
        .await
        .iter()
        .find_map(|b| b["public_key"].as_str().map(str::to_owned))
        .unwrap();
    let server = MockServer::start().await;
    let mut mine = runner_json();
    mine["public_key"] = json!(key.clone());
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": [mine]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/jobs/job_1/gate-report"))
        .respond_with(answer)
        .mount(&server)
        .await;
    (server, key)
}

fn gate_args<'a>(work: &'a str, command: &'a str) -> Vec<&'a str> {
    vec![
        "runner",
        "gate-report",
        "--job",
        "job_1",
        "--lease",
        "lse_1",
        "--token",
        "5",
        "--repo",
        work,
        "--command",
        command,
        "--verbose",
    ]
}

#[tokio::test]
async fn gate_report_runs_the_check_and_sends_a_report_signed_by_the_runner_key() {
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    let work = checkout(dir.path());
    let (server, key) = gate_server(
        dir.path(),
        ResponseTemplate::new(200).set_body_json(json!({
            "evidence_id": "evd_1", "duplicate": false, "job": {"stage": "merged"}
        })),
    )
    .await;
    // The check also proves it ran with a scrubbed environment.
    let command = "echo ok; test -z \"$GH_TOKEN$FOO_SECRET$ANTHROPIC_API_KEY$IOHR_PROFILE\"";
    let work_s = work.to_str().unwrap();
    let o = run(&mut iohr(&server, dir.path(), &gate_args(work_s, command)));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let said = text(&o);
    assert!(said.contains("evd_1") && said.contains("merged"), "{said}");
    assert!(!said.contains("TOKENSIGNATUREMARKER"), "{said}");

    let sent = bodies(&server, "/v1/coordinator/jobs/job_1/gate-report").await;
    assert_eq!(sent.len(), 1, "one report");
    let b = &sent[0];
    assert_eq!(b["job_id"], "job_1");
    assert_eq!(b["lease_id"], "lse_1");
    assert_eq!(b["fencing_token"], "5");
    assert_eq!(b["runner_id"], "run_1");
    assert_eq!(b["command"], command);
    assert_eq!(b["exit_code"], 0, "the scrubbed check passed");
    assert_eq!(b["head"], git(&work, &["rev-parse", "HEAD"]));
    assert_eq!(b["main_commit"], git(&work, &["rev-parse", "origin/main"]));
    assert_eq!(
        b["output_sha256"],
        sha("ok\n"),
        "the hash of the output, not the output"
    );
    let all = serde_json::to_string(&sent).unwrap();
    // The command text names FOO_SECRET (it tests the scrub), so the body check is on its
    // shape instead: exactly the contract's fields, no field that could carry the output.
    let mut keys: Vec<&str> = b.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "account_id",
            "command",
            "exit_code",
            "fencing_token",
            "head",
            "job_id",
            "lease_id",
            "main_commit",
            "output_sha256",
            "runner_id",
            "signature"
        ],
        "{all}"
    );
    assert!(!all.contains("ok\\n"), "{all}");
    // Signed over the ten lines of the contract by the key the runner registered.
    let h = |s: &str| sha(s);
    let message = [
        "iohr.coordinator.v1.SubmitGateReport".to_owned(),
        h("job_1"),
        h("lse_1"),
        "5".to_owned(),
        h("run_1"),
        h(command),
        "0".to_owned(),
        h(b["head"].as_str().unwrap()),
        h(b["main_commit"].as_str().unwrap()),
        h(b["output_sha256"].as_str().unwrap()),
    ]
    .join("\n");
    assert!(
        verify(&key, &message, b["signature"].as_str().unwrap()),
        "the signature verifies under the enrolled key"
    );
}

#[tokio::test]
async fn gate_report_on_an_older_coordinator_says_so_and_does_not_fail() {
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    let work = checkout(dir.path());
    let (server, _) = gate_server(
        dir.path(),
        ResponseTemplate::new(404)
            .set_body_json(json!({"code": "not_found", "error": "no such route", "details": []})),
    )
    .await;
    let work_s = work.to_str().unwrap();
    let o = run(&mut iohr(&server, dir.path(), &gate_args(work_s, "true")));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        text(&o).contains("does not take gate reports yet"),
        "{}",
        text(&o)
    );
}

#[tokio::test]
async fn gate_report_for_an_unknown_job_is_a_failure_not_an_old_coordinator() {
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    let work = checkout(dir.path());
    let (server, _) = gate_server(
        dir.path(),
        ResponseTemplate::new(404)
            .set_body_json(json!({"code": "not_found", "error": "no such job", "details": []})),
    )
    .await;
    let work_s = work.to_str().unwrap();
    let o = run(&mut iohr(&server, dir.path(), &gate_args(work_s, "true")));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(!text(&o).contains("does not take gate reports yet"));
}

#[tokio::test]
async fn gate_report_sends_nothing_for_a_branch_that_lacks_main_or_has_unsaved_changes() {
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    let work = checkout(dir.path());
    let (server, _) = gate_server(
        dir.path(),
        ResponseTemplate::new(200).set_body_json(json!({"evidence_id": "evd_1"})),
    )
    .await;
    let work_s = work.to_str().unwrap();
    // Main moves on origin after the branch was cut.
    git(&work, &["checkout", "--quiet", "main"]);
    std::fs::write(work.join("c.txt"), "c\n").unwrap();
    git(&work, &["add", "c.txt"]);
    git(&work, &["commit", "--quiet", "-m", "more main"]);
    git(&work, &["push", "--quiet", "origin", "HEAD:main"]);
    git(&work, &["checkout", "--quiet", "feature"]);
    let o = run(&mut iohr(&server, dir.path(), &gate_args(work_s, "true")));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("merge main into the branch"),
        "{}",
        text(&o)
    );
    // Merged, but with a file changed and not committed.
    git(&work, &["merge", "--quiet", "--no-edit", "origin/main"]);
    std::fs::write(work.join("b.txt"), "changed\n").unwrap();
    let o = run(&mut iohr(&server, dir.path(), &gate_args(work_s, "true")));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("uncommitted changes"), "{}", text(&o));
    assert!(
        bodies(&server, "/v1/coordinator/jobs/job_1/gate-report")
            .await
            .is_empty(),
        "no report was sent"
    );
}

#[tokio::test]
async fn gate_report_needs_the_runner_scopes_and_a_runner_key() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token_with(&["identity:read"])).await;
    let o = run(&mut iohr(&server, dir.path(), &gate_args(".", "true")));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("coordinator:runner"), "{}", text(&o));
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    login(&server, dir.path(), &token()).await;
    let o = run(&mut iohr(&server, dir.path(), &gate_args(".", "true")));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("iohr runner register"), "{}", text(&o));
}

/// `runner run-key` makes the approvals key once, keeps it in the profile's store, needs
/// no coordinator scope, and enrols it with a device's proof plus a P-256 signature of
/// possession over the enrolment message.
#[tokio::test]
async fn run_key_is_made_once_and_enrolled_with_a_proof_of_possession() {
    use aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1;
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    // A person sign-in without the coordinator scopes: run-key does not need them.
    login(&server, dir.path(), &token_with(&["identity:read"])).await;
    Mock::given(method("GET"))
        .and(path("/v1/approvals/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": []})))
        .mount(&server)
        .await;
    let args = ["runner", "run-key", "--json", "--verbose"];
    let o = run(&mut iohr(&server, dir.path(), &args));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let first: Value = serde_json::from_slice(&o.stdout).unwrap();
    let public = first["public_key"].as_str().unwrap().to_owned();
    assert_eq!(
        URL_SAFE_NO_PAD.decode(&public).unwrap().len(),
        65,
        "a P-256 point"
    );
    assert_eq!(first["runner_id"], Value::Null, "not enrolled yet");
    let said = text(&o);
    assert!(said.contains("request id"), "{said}");
    assert!(!said.contains("TOKENSIGNATUREMARKER"), "{said}");

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/v1/approvals/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "runner": {"runner_id": "arn_1", "public_key": public}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/approvals/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "runners": [{"runner_id": "arn_1", "public_key": public, "label": "mac"}]
        })))
        .mount(&server)
        .await;
    let proof = dir.path().join("proof.json");
    std::fs::write(
        &proof,
        json!({"key_id": "dk_1", "signed_at": 1_700_000_000, "nonce": "n1", "signature": "c2ln"})
            .to_string(),
    )
    .unwrap();
    let proof_s = proof.to_str().unwrap();
    let args = [
        "runner", "run-key", "--proof", proof_s, "--label", "mac", "--json",
    ];
    let o = run(&mut iohr(&server, dir.path(), &args));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let second: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(second["public_key"], public.as_str(), "the same key, kept");
    assert_eq!(second["runner_id"], "arn_1");

    let sent: Vec<Value> = bodies(&server, "/v1/approvals/runners")
        .await
        .into_iter()
        .filter(|b| !b.is_null())
        .collect();
    assert_eq!(sent.len(), 1, "one enrolment");
    let b = &sent[0];
    assert_eq!(b["account_id"], "acc_1");
    assert_eq!(b["public_key"], public.as_str());
    assert_eq!(b["label"], "mac");
    assert_eq!(b["proof"]["key_id"], "dk_1");
    assert_eq!(b["proof"]["nonce"], "n1");
    assert_eq!(b["proof"]["signature"], "c2ln");
    let message = format!("iohr-approval-runner-v1\n{public}\nmac\n1700000000\nn1\ndk_1");
    let possession = URL_SAFE_NO_PAD
        .decode(b["possession"].as_str().unwrap())
        .unwrap();
    UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_ASN1,
        URL_SAFE_NO_PAD.decode(&public).unwrap(),
    )
    .verify(message.as_bytes(), &possession)
    .expect("the possession signature verifies with the run key");
}
