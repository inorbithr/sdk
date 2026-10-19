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
use wiremock::matchers::{method, path, path_regex};
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
    assert_eq!(post["kinds"], json!(["coordinate"]));
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
