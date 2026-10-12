//! `iohr runner` against a mock coordinator and a fake `claude` on PATH: what is sent,
//! that the child never sees a credential, and that nothing starts without the
//! subscription.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SUB: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#;
const API_KEY: &str = r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#;

fn token() -> String {
    let enc = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(json!({"alg": "RS256"})),
        enc(
            json!({"sub": "usr_me", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
                   "scp": ["coordinator:runner"], "org": "acc_1", "plan": "free", "key": "key_1"})
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
            "#!/bin/sh\nd=$(dirname \"$0\")\nenv >> \"$d/env.txt\"\ncase \"$1\" in\n  auth) echo '{auth}';;\n  --version) echo '9.9.9 (fake)';;\nesac\n"
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
        .env("IOHR_TOKEN", token())
        .env("HOME", dir)
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        // What must never reach claude or the coordinator.
        .env("ANTHROPIC_API_KEY", "sk-ant-SECRETKEY")
        .env("ANTHROPIC_BASE_URL", "https://proxy.example")
        .env("AWS_SECRET_ACCESS_KEY", "AWSSECRET")
        .stdin(Stdio::null());
    c
}

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

fn runner_json() -> Value {
    json!({"runner_id": "run_1", "user": "usr_me", "name": "mac", "status": "online",
           "last_seen_at": "2026-10-12T08:00:00Z", "providers": []})
}

#[tokio::test]
async fn register_sends_this_machines_facts_and_never_a_credential() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": []})))
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
    assert!(text(&o).contains("run_1"));

    let sent = bodies(&server, "/v1/coordinator/runners").await;
    let post = sent.iter().find(|b| b.get("name").is_some()).unwrap();
    assert_eq!(post["name"], "mac");
    assert_eq!(post["kinds"], json!(["coordinate"]));
    assert_eq!(post["providers"][0]["method"], "subscription");
    let hash = post["identity_sha256"]["claude"].as_str().unwrap();
    assert!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    let all = serde_json::to_string(&sent).unwrap();
    assert!(!all.contains("SECRETKEY") && !all.contains("firstParty"));

    // Every claude call ran without the credentials the runner itself was given.
    let env = std::fs::read_to_string(dir.path().join("env.txt")).unwrap();
    for bad in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "AWS_SECRET",
        "IOHR_TOKEN",
        "SECRETKEY",
    ] {
        assert!(!env.contains(bad), "{bad} reached claude");
    }
    assert!(env.contains("HOME="));
}

#[tokio::test]
async fn register_refuses_an_api_key_sign_in_and_calls_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), API_KEY);
    let o = run(&mut iohr(&server, dir.path(), &["runner", "register"]));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("subscription only"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn run_heartbeats_accepts_the_offered_lease_and_says_the_stubs_are_stubs() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": [runner_json()]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/run_1/heartbeat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "runner": runner_json(),
            "offered": [{"lease_id": "lease_1", "runner_id": "run_1", "job_id": "job_1",
                         "turn_id": "turn_1", "fencing_token": "5"}]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/runners/run_1/heartbeat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runner": runner_json()})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/coordinator/leases/lease_1/accept"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"lease": {"lease_id": "lease_1"}})),
        )
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
    let beats = bodies(&server, "/v1/coordinator/runners/run_1/heartbeat").await;
    assert_eq!(beats.len(), 3);
    assert_eq!(beats[0]["providers"][0]["signed_in"], true);
    let accepts = bodies(&server, "/v1/coordinator/leases/lease_1/accept").await;
    assert_eq!(accepts.len(), 1);
    assert_eq!(accepts[0]["fencing_token"], "5");
    assert_eq!(accepts[0]["env_scrubbed"], true);
    assert_eq!(accepts[0]["caps_set"], true);
    assert_eq!(accepts[0]["worktree_is_main_checkout"], false);
    let t = text(&o);
    assert!(
        t.contains("not implemented") && t.contains("GetTurnPack"),
        "{t}"
    );
    assert!(t.contains("3 heartbeats"), "{t}");
}

#[tokio::test]
async fn run_needs_a_registered_runner() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": []})))
        .mount(&server)
        .await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o).contains("iohr runner register"));
}
