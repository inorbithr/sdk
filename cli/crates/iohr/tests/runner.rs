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

mod pty;

const SUB: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"seat@example.com","orgId":"00000000-0000-4000-8000-00000000000a","orgName":"Example","subscriptionType":"max","loginExpiresAt":"2026-11-05T10:00:00Z","configDirectory":"/home/a/.claude","projectsDirectory":"/home/a/.claude/projects"}"#;
/// The same seat after a token refresh, on another host, with the email in another case.
const SUB_REFRESHED: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"Seat@Example.com","orgId":"00000000-0000-4000-8000-00000000000a","orgName":"Example Renamed","subscriptionType":"pro","loginExpiresAt":"2026-11-06T22:00:00Z","configDirectory":"/Users/b/.config/claude","projectsDirectory":"/srv/projects"}"#;
/// The canonical identity line of `SUB` and `SUB_REFRESHED`.
const SUB_LINE: &str = "authMethod=claude.ai apiProvider=firstParty orgId=00000000-0000-4000-8000-00000000000a email=seat@example.com";
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
        .env("IOHR_NO_BROWSER", "1")
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

fn sha(s: &(impl AsRef<[u8]> + ?Sized)) -> String {
    use std::fmt::Write as _;
    Sha256::digest(s.as_ref())
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
    // A person's own runner names no holder.
    assert!(post.get("holder_subject").is_none());
    // The identity hash is sha256(salt + "\n" + the canonical identity line).
    assert_eq!(
        post["identity_sha256"]["claude"],
        sha(&format!("{SALT}\n{SUB_LINE}"))
    );
    // A standard, padded base64 key; the challenge; a signature over the registration lines.
    let key = post["public_key"].as_str().unwrap();
    assert_eq!(key.len(), 44);
    assert_eq!(post["challenge"], CHALLENGE);
    let message = format!("iohr.coordinator.v1.RegisterRunner\n{CHALLENGE}\n{key}");
    assert!(verify(key, &message, post["signature"].as_str().unwrap()));
    let all = serde_json::to_string(&sent).unwrap();
    assert!(!all.contains("SECRETKEY") && !all.contains("firstParty"));
    assert!(!all.contains("example.com") && !all.contains("00000000-0000"));
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
async fn register_with_a_holder_names_that_person() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token()).await;
    mount_register(&server).await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &[
            "runner",
            "register",
            "--name",
            "mac",
            "--holder",
            " person-1 ",
        ],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let sent = bodies(&server, "/v1/coordinator/runners").await;
    let post = sent.iter().find(|b| b.get("name").is_some()).unwrap();
    assert_eq!(post["holder_subject"], "person-1");
    // The holder is checked against the sign-in the runner holds, so it goes with it.
    assert_eq!(
        post["identity_sha256"]["claude"],
        sha(&format!("{SALT}\n{SUB_LINE}"))
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
            "{path} was not called"
        );
    }
    for (_, a) in &named {
        assert_eq!(
            a.as_deref(),
            Some("acc_1"),
            "a coordinator call named no account"
        );
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
    for (_, a) in &named[before..] {
        assert_eq!(
            a.as_deref(),
            Some("team_9"),
            "a coordinator call named the wrong account"
        );
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
            "{path} was not called"
        );
    }
    for (_, a) in &named {
        assert_eq!(
            a.as_deref(),
            Some("acc_1"),
            "a coordinator call named no account"
        );
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
    // The first beat tried the new body twice (with the wait, then without it, RFC 0193);
    // every beat after it is the old one.
    assert_eq!(beats[0]["capacity"], 4, "{beats:?}");
    assert_eq!(beats[0]["wait_seconds"], 4, "{beats:?}");
    assert!(beats[1].get("wait_seconds").is_none(), "{beats:?}");
    let old: Vec<&Value> = beats.iter().skip(2).collect();
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
async fn a_coordinator_that_knows_capacity_but_not_the_wait_is_beaten_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    // A coordinator from before RFC 0193: it takes `capacity` and `version` but its REST
    // binding refuses `wait_seconds`.
    Mock::given(method("POST"))
        .and(path(BEAT))
        .and(body_string_contains("\"wait_seconds\""))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "bad_request",
            "error": "body unrecognized field `wait_seconds`, expected one of runner_id, providers, capacity",
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
            "3",
        ],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let beats = bodies(&server, BEAT).await;
    // One try with the wait; every beat after it keeps what it declares, without the wait.
    assert_eq!(beats[0]["wait_seconds"], 4, "{beats:?}");
    let rest: Vec<&Value> = beats.iter().skip(1).collect();
    assert!(rest.len() >= 3, "{beats:?}");
    for b in rest {
        assert!(b.get("wait_seconds").is_none(), "{b}");
        assert_eq!(b["capacity"], 4, "{b}");
    }
    assert!(text(&o).contains("0 failed"), "{}", text(&o));
}

#[tokio::test]
async fn a_service_that_holds_the_beat_is_beaten_again_without_waiting_for_the_interval() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    // The service says it held each call (RFC 0193), so the loop does not wait 30 s.
    Mock::given(method("POST"))
        .and(path(BEAT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "runner": runner_json(), "heartbeat_interval_seconds": 30, "waited_seconds": 4
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    let started = std::time::Instant::now();
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &[
            "runner",
            "run",
            "--name",
            "mac",
            "--interval",
            "30",
            "--max-beats",
            "3",
        ],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    let beats = bodies(&server, BEAT).await;
    assert!(beats.len() >= 3, "{beats:?}");
    assert!(beats.iter().all(|b| b["wait_seconds"] == 4), "{beats:?}");
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
async fn config_show_names_the_upgrade_settings_and_the_flag_turns_it_off() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    login(&server, dir.path(), &token()).await;
    let show = |extra: &[&str]| {
        let mut args = vec!["runner", "config", "show", "--json"];
        args.extend_from_slice(extra);
        let o = run(&mut iohr(&server, dir.path(), &args));
        assert_eq!(o.status.code(), Some(0), "{}", text(&o));
        serde_json::from_slice::<Value>(&o.stdout).unwrap()
    };
    let v = show(&[]);
    assert_eq!(
        v["auto_upgrade"],
        json!({"value": true, "source": "default"})
    );
    assert_eq!(
        v["upgrade_interval_minutes"],
        json!({"value": 60, "source": "default"})
    );
    assert_eq!(
        v["upgrade_restart"],
        json!({"value": "exec", "source": "default"})
    );
    assert_eq!(
        v["upgrade_sign_identity"],
        json!({"value": "off", "source": "default"})
    );
    add_to_config(
        dir.path(),
        "\n[runner]\nauto_upgrade = true\nupgrade_interval_minutes = 15\nupgrade_restart = \"exit\"\nupgrade_sign_identity = \"auto\"\n",
    );
    let v = show(&[]);
    assert_eq!(
        v["auto_upgrade"],
        json!({"value": true, "source": "[runner]"})
    );
    assert_eq!(
        v["upgrade_interval_minutes"],
        json!({"value": 15, "source": "[runner]"})
    );
    assert_eq!(
        v["upgrade_restart"],
        json!({"value": "exit", "source": "[runner]"})
    );
    assert_eq!(
        v["upgrade_sign_identity"],
        json!({"value": "auto", "source": "[runner]"})
    );
    let v = show(&["--no-auto-upgrade"]);
    assert_eq!(v["auto_upgrade"], json!({"value": false, "source": "flag"}));
    // The variable beats the file; `-` is off.
    let o = run(
        iohr(&server, dir.path(), &["runner", "config", "show", "--json"])
            .env("IOHR_SIGN_IDENTITY", "-"),
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let v = serde_json::from_slice::<Value>(&o.stdout).unwrap();
    assert_eq!(
        v["upgrade_sign_identity"],
        json!({"value": "off", "source": "IOHR_SIGN_IDENTITY"})
    );
    // A value out of range stops the start, before anything is sent.
    let p = dir.path().join("config").join("config.toml");
    let t = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, t.replace("= 15", "= 2")).unwrap();
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("upgrade_interval_minutes"),
        "{}",
        text(&o)
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

/// The identity hash one `claude auth status` answer registers with.
async fn registered_identity(auth: &str) -> Value {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), auth);
    login(&server, dir.path(), &token()).await;
    mount_register(&server).await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "register", "--name", "mac"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    bodies(&server, "/v1/coordinator/runners")
        .await
        .into_iter()
        .find_map(|b| b.get("identity_sha256").cloned())
        .unwrap()
}

#[tokio::test]
async fn identity_hash_is_stable_across_token_refresh() {
    let want = json!({"claude": sha(&format!("{SALT}\n{SUB_LINE}"))});
    assert_eq!(registered_identity(SUB).await, want);
    // Another expiry, other directories, another plan and organisation name, the
    // email's case: the same seat, the same hash.
    assert_eq!(registered_identity(SUB_REFRESHED).await, want);
    // Another email, or the same email in another organisation: another seat.
    let other_email = SUB.replace("seat@example.com", "other@example.com");
    let other_org = SUB.replace("00000000000a", "00000000000b");
    for other in [other_email, other_org] {
        assert_ne!(registered_identity(&other).await, want, "{other}");
    }
}

#[tokio::test]
async fn register_refuses_a_sign_in_that_names_no_seat() {
    for auth in [
        r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#,
        r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"seat@example.com"}"#,
        r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","orgId":"00000000-0000-4000-8000-00000000000a"}"#,
        "Logged in",
    ] {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        fake_claude(dir.path(), auth);
        login(&server, dir.path(), &token()).await;
        mount_register(&server).await;
        let o = run(&mut iohr(
            &server,
            dir.path(),
            &["runner", "register", "--name", "mac"],
        ));
        assert_eq!(o.status.code(), Some(1), "{auth}: {}", text(&o));
        let sent = bodies(&server, "/v1/coordinator/runners").await;
        assert!(
            sent.iter().all(|b| b.get("public_key").is_none()),
            "{auth}: nothing registered"
        );
    }
}

#[tokio::test]
async fn register_refresh_sends_the_held_key_and_the_current_sign_in() {
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

    // The sign-in changed seat since: the refresh sends the new seat's hash.
    let moved = SUB.replace("seat@example.com", "moved@example.com");
    fake_claude(dir.path(), &moved);
    let moved_line = SUB_LINE.replace("seat@example.com", "moved@example.com");
    let mut mine = runner_json();
    mine["public_key"] = json!(key);
    for (answer, says) in [
        (runner_json(), "refreshed"),
        (
            json!({"runner_id": "run_2", "user": "usr_me", "name": "mac", "status": "online",
                   "last_seen_at": "2026-10-12T08:00:00Z", "providers": []}),
            "revoke run_1",
        ),
    ] {
        let second = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/coordinator/runners"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"runners": [mine.clone()]})),
            )
            .mount(&second)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/coordinator/runners/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"challenge": CHALLENGE, "identity_salt": SALT, "account_id": "acc_1"}),
            ))
            .mount(&second)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/coordinator/runners"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runner": answer})))
            .mount(&second)
            .await;
        let o = run(&mut iohr(
            &second,
            dir.path(),
            &["runner", "register", "--name", "mac", "--refresh"],
        ));
        assert_eq!(o.status.code(), Some(0), "{}", text(&o));
        assert!(text(&o).contains(says), "{}", text(&o));
        assert!(!text(&o).contains("already registered"), "{}", text(&o));
        let sent = bodies(&second, "/v1/coordinator/runners").await;
        let post = sent.iter().find(|b| b.get("public_key").is_some()).unwrap();
        // The key this machine holds, a fresh signature over the new challenge.
        assert_eq!(post["public_key"], json!(key));
        let message = format!("iohr.coordinator.v1.RegisterRunner\n{CHALLENGE}\n{key}");
        assert!(verify(&key, &message, post["signature"].as_str().unwrap()));
        assert_eq!(
            post["identity_sha256"]["claude"],
            sha(&format!("{SALT}\n{moved_line}"))
        );
    }
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

/// `runner run-key --show` only reads: with no key it fails and makes none, with one it
/// prints the public key and the runner id the service enrolled it under, filing nothing.
#[tokio::test]
async fn run_key_show_reads_the_key_and_makes_and_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    login(&server, dir.path(), &token_with(&["identity:read"])).await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run-key", "--show"],
    ));
    assert_eq!(o.status.code(), Some(1), "no key yet: {}", text(&o));
    assert!(text(&o).contains("no run key"), "{}", text(&o));

    // Make the key (an ask is filed, which this test then counts).
    let ask = "apr_01M5Z1KKYQ16SMN4XFYS5K5RFA";
    Mock::given(method("GET"))
        .and(path("/v1/approvals/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/approvals"))
        .respond_with(move |r: &wiremock::Request| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            ResponseTemplate::new(200).set_body_json(json!({"approval": {
                "approval_id": ask, "status": "pending", "payload": b["payload"],
            }}))
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/approvals/{ask}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"approval": {"approval_id": ask, "status": "pending"}})),
        )
        .mount(&server)
        .await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run-key", "--json", "--wait", "0"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let made: Value = serde_json::from_slice(&o.stdout).unwrap();
    let public = made["public_key"].as_str().unwrap().to_owned();
    let asks = bodies(&server, "/v1/approvals").await.len();

    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run-key", "--show", "--json"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let shown: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(shown["public_key"], public.as_str(), "the same key");
    assert_eq!(shown["runner_id"], Value::Null, "not enrolled");

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v1/approvals/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "runners": [{"runner_id": "arn_1", "public_key": public, "label": "mac"}]
        })))
        .mount(&server)
        .await;
    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run-key", "--show", "--json"],
    ));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let shown: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(shown["runner_id"], "arn_1");
    assert_eq!(shown["label"], "mac");
    assert_eq!(asks, 1, "only the making filed an ask");
    assert!(
        bodies(&server, "/v1/approvals").await.is_empty(),
        "--show files nothing"
    );

    let o = run(&mut iohr(
        &server,
        dir.path(),
        &["runner", "run-key", "--show", "--label", "x"],
    ));
    assert_eq!(
        o.status.code(),
        Some(2),
        "--show conflicts with --label: {}",
        text(&o)
    );
}

/// `runner run-key` makes the approvals key once, keeps it in the profile's store, needs
/// no coordinator scope, and enrols it with a device's proof plus a P-256 signature of
/// possession over the enrolment message.
#[tokio::test]
#[allow(clippy::too_many_lines)] // one story: ask, then the proof file, read top to bottom
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
    // Not enrolled, so it files a `runner_enrollment` ask (RFC 0172, amendment 1); the
    // service answers with the ask as filed, which is still pending.
    let ask = "apr_01M5Z1KKYQ16SMN4XFYS5K5RFA";
    Mock::given(method("POST"))
        .and(path("/v1/approvals"))
        .respond_with(move |r: &wiremock::Request| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            ResponseTemplate::new(200).set_body_json(json!({"approval": {
                "approval_id": ask, "status": "pending", "payload": b["payload"],
            }}))
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/approvals/{ask}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"approval": {
                "approval_id": ask, "status": "pending",
            }})),
        )
        .mount(&server)
        .await;
    let args = ["runner", "run-key", "--json", "--verbose", "--wait", "0"];
    let o = run(&mut iohr(&server, dir.path(), &args));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let first: Value = serde_json::from_slice(&o.stdout).unwrap();
    let public = first["public_key"].as_str().unwrap().to_owned();
    let point = URL_SAFE_NO_PAD.decode(&public).unwrap();
    assert_eq!(point.len(), 65, "a P-256 point");
    assert_eq!(first["runner_id"], Value::Null, "not enrolled yet");
    assert_eq!(first["approval_id"], ask);
    let fingerprint = sha(&point);
    assert_eq!(first["fingerprint"], fingerprint.as_str());
    let filed = bodies(&server, "/v1/approvals").await;
    assert_eq!(filed.len(), 1, "one ask");
    assert_eq!(filed[0]["kind"], "runner_enrollment");
    assert_eq!(filed[0]["account_id"], "acc_1");
    let e = &filed[0]["payload"]["runner_enrollment"];
    assert_eq!(e["public_key"], public.as_str());
    assert_eq!(e["fingerprint"], fingerprint.as_str());
    let said = text(&o);
    assert!(said.contains(&fingerprint[..4]), "the short form: {said}");
    assert!(said.contains(ask), "{said}");
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

const IO_SECRET: &str = "IOSECRETMARKER";

/// The Io sign-in (RFC 0065.2): `--sign-in io` mints a token for `iohr-io` from a secret
/// file, calls the coordinator with it and not with the profile's token, and a secret file
/// that others can read stops the run before any call. The default sign-in is untouched.
#[tokio::test]
async fn sign_in_io_calls_the_coordinator_with_a_minted_token_and_guards_the_secret_file() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    fake_claude(dir.path(), SUB);
    login(&server, dir.path(), &token()).await;
    let io_token = token_with(&[
        "coordinator:runner",
        "coordinator:read",
        "coordinator:write",
    ]);
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": io_token, "token_type": "bearer", "expires_in": 900}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/coordinator/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"runners": []})))
        .mount(&server)
        .await;
    let secret = dir.path().join("io-secret");
    std::fs::write(&secret, format!("{IO_SECRET}\n")).unwrap();
    let io = |server: &MockServer, args: &[&str]| {
        let mut c = iohr(server, dir.path(), args);
        c.env("IOHR_ISSUER", server.uri())
            .env("IOHR_CLIENT_SECRET_FILE", &secret);
        c
    };

    // A secret file group and world can read is refused before any sign-in call.
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    let before = server.received_requests().await.unwrap().len();
    let o = run(&mut io(&server, &["runner", "status", "--sign-in", "io"]));
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains("chmod 600"), "{}", text(&o));
    assert!(!text(&o).contains(IO_SECRET));
    assert_eq!(server.received_requests().await.unwrap().len(), before);

    // Mode 0600: the token is minted for the client and the coordinator is called with it.
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    let o = run(&mut io(&server, &["runner", "status", "--sign-in", "io"]));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(!text(&o).contains(IO_SECRET) && !text(&o).contains(&io_token));
    let seen = server.received_requests().await.unwrap();
    let mint = seen
        .iter()
        .rfind(|r| r.url.path() == "/oauth2/token")
        .expect("a token was minted");
    let basic = mint.headers.get("authorization").unwrap().to_str().unwrap();
    assert_eq!(
        basic,
        format!("Basic {}", STANDARD.encode(format!("iohr-io:{IO_SECRET}")))
    );
    let call = seen
        .iter()
        .rfind(|r| r.url.path() == "/v1/coordinator/runners")
        .unwrap();
    assert_eq!(
        call.headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {io_token}")
    );

    // The environment names it too, and no other command takes it.
    let o = run(&mut io(&server, &["whoami", "--sign-in", "io"]));
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));

    // Without the flag the profile's own token is used, as before.
    let o = run(&mut iohr(&server, dir.path(), &["runner", "status"]));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let seen = server.received_requests().await.unwrap();
    let last = seen
        .iter()
        .rfind(|r| r.url.path() == "/v1/coordinator/runners")
        .unwrap();
    assert_eq!(
        last.headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {}", token())
    );
}

// ---- the session host (RFC 0213)

/// The one socket a host made below `data`, if it is there.
fn session_socket(data: &Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(data.join("session"))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join("sock"))
        .find(|p| p.exists())
}

/// What the shell and the host wrote, for a failure message.
fn session_diag(dir: &Path) -> String {
    let host = std::fs::read_dir(dir.join("data/session"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .find_map(|e| std::fs::read_to_string(e.path().join("host.log")).ok())
        .unwrap_or_default();
    let shell = std::fs::read_to_string(dir.join("session.err")).unwrap_or_default();
    format!("shell: {shell}\nhost: {host}")
}

fn session_cmd(server: &MockServer, dir: &Path, args: &[&str]) -> Command {
    let mut c = iohr(server, dir, args);
    // `iohr session` names no runner: the host's name is this machine's, as at `register`.
    c.env("IOHR_DATA_DIR", dir.join("data"))
        .env("HOSTNAME", "mac");
    c
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..200 {
        if ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("gave up waiting for {what}");
}

#[tokio::test]
async fn session_starts_one_host_that_beats_and_stop_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let p = project.to_str().unwrap();
    let out = dir.path().join("session.out");
    let mut shell = session_cmd(&server, dir.path(), &["session", p, "--verbose"])
        .stdout(std::fs::File::create(&out).unwrap())
        .stderr(std::fs::File::create(dir.path().join("session.err")).unwrap())
        .spawn()
        .unwrap();
    // The host is a process of its own, and it is the runner: it keeps the lease alive.
    assert!(
        beats_at_least(&server, 1).await >= 1,
        "the host sent no heartbeat: {}",
        session_diag(dir.path())
    );
    wait_until("the session note", || {
        std::fs::read_to_string(dir.path().join("session.err"))
            .is_ok_and(|t| t.contains("session open: host"))
    })
    .await;
    let note = std::fs::read_to_string(dir.path().join("session.err")).unwrap();
    let host: u32 = note
        .split("session open: host ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap();
    assert_ne!(host, shell.id(), "the host is not the shell");
    assert!(session_socket(&dir.path().join("data")).is_some());
    // A second shell joins the same host: no second process.
    let second = run(&mut session_cmd(
        &server,
        dir.path(),
        &["stop", p, "--wait", "0"],
    ));
    assert_eq!(second.status.code(), Some(0), "{}", text(&second));
    // `stop` told it; the host finishes and ends, and the shell sees it go.
    let status = tokio::task::spawn_blocking(move || shell.wait().unwrap())
        .await
        .unwrap();
    assert!(status.success(), "{status}");
    wait_until("the socket to go", || {
        session_socket(&dir.path().join("data")).is_none()
    })
    .await;
    let again = run(&mut session_cmd(&server, dir.path(), &["stop", p]));
    assert_eq!(again.status.code(), Some(0));
    assert!(
        text(&again).contains("no session host runs"),
        "{}",
        text(&again)
    );
    // Neither the shell's nor the host's output holds a credential.
    let log = std::fs::read_dir(dir.path().join("data/session"))
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|e| std::fs::read_to_string(e.path().join("host.log")).ok())
        .unwrap_or_default();
    let all = format!(
        "{}{note}{log}",
        std::fs::read_to_string(&out).unwrap_or_default()
    );
    assert!(
        !all.contains("TOKENSIGNATUREMARKER") && !all.contains("SECRETKEY"),
        "{all}"
    );
}

#[tokio::test]
async fn a_killed_shell_takes_the_host_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let p = project.to_str().unwrap();
    let mut shell = session_cmd(&server, dir.path(), &["session", p])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("the host's socket", || {
        session_socket(&dir.path().join("data")).is_some()
    })
    .await;
    assert!(beats_at_least(&server, 1).await >= 1);
    // SIGKILL: the shell cannot say goodbye; the kernel closes its end of the pipe.
    shell.kill().unwrap();
    shell.wait().unwrap();
    wait_until("the host to end", || {
        session_socket(&dir.path().join("data")).is_none()
    })
    .await;
    let o = run(&mut session_cmd(&server, dir.path(), &["stop", p]));
    assert!(text(&o).contains("no session host runs"), "{}", text(&o));
}

#[tokio::test]
async fn sigterm_ends_a_host_run_by_hand() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let p = project.to_str().unwrap();
    let mut host = session_cmd(
        &server,
        dir.path(),
        &[
            "runner",
            "run",
            "--name",
            "mac",
            "--interval",
            "1",
            "--session",
            p,
        ],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    wait_until("the host's socket", || {
        session_socket(&dir.path().join("data")).is_some()
    })
    .await;
    signal(host.id(), "-TERM");
    let status = tokio::task::spawn_blocking(move || host.wait().unwrap())
        .await
        .unwrap();
    assert!(status.success(), "SIGTERM is a clean stop: {status}");
    assert!(session_socket(&dir.path().join("data")).is_none());
}

/// Bare `iohr` on a terminal is the session: the shell with the panel over it, in the host this
/// folder's session joined (started here, since none ran). `iohr stop` ends the host, which
/// closes the panel and leaves the shell; the shell then ends with its own exit status.
#[tokio::test]
async fn bare_iohr_on_a_terminal_enters_the_session_and_stop_closes_the_panel() {
    let dir = tempfile::tempdir().unwrap();
    let server = registered_and_idle(dir.path()).await;
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let p = project.to_str().unwrap().to_owned();
    let mut c = session_cmd(&server, dir.path(), &[]);
    c.current_dir(&project)
        .env_remove("TERM_PROGRAM")
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("SHELL", "/bin/sh")
        .env("PS1", "shell> ");
    let mut s = pty::Session::spawn(c);
    s.until("the panel over the shell", |screen| {
        pty::any(screen, "⌥i menu") && pty::any(screen, "shell>")
    });
    let screen = s.screen.screen().contents();
    assert!(
        !screen.contains("Usage:"),
        "bare iohr printed help:\n{screen}"
    );
    // The host is a process of its own that this shell joined: it beats, and has a socket.
    assert!(
        beats_at_least(&server, 1).await >= 1,
        "the host sent no heartbeat: {}",
        session_diag(dir.path())
    );
    assert!(session_socket(&dir.path().join("data")).is_some());
    // `iohr stop` from another process ends the host; the panel closes, the shell stays.
    let stopped = tokio::task::spawn_blocking({
        let (server_uri, dir_path) = (server.uri(), dir.path().to_path_buf());
        move || {
            let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
            c.args(["stop", &p, "--wait", "0"])
                .env_clear()
                .env("IOHR_BASE_URL", server_uri)
                .env("IOHR_CONFIG_DIR", dir_path.join("config"))
                .env("IOHR_PROFILE", "p")
                .env("HOME", &dir_path)
                .env("IOHR_DATA_DIR", dir_path.join("data"))
                .env("PATH", "/usr/bin:/bin");
            c.output().unwrap()
        }
    })
    .await
    .unwrap();
    assert_eq!(stopped.status.code(), Some(0), "{}", text(&stopped));
    // The host ends once its Roles are idle; its closing connection is what closes the panel.
    wait_until("the host gone", || {
        session_socket(&dir.path().join("data")).is_none()
    })
    .await;
    s.until("the panel gone", |screen| !pty::any(screen, "⌥i menu"));
    assert!(
        s.child.try_wait().unwrap().is_none(),
        "the shell must stay open when the host stops"
    );
    s.send(b"exit\n");
    let status = s.exited().expect("the shell ends after exit");
    assert!(status.success(), "{status}");
    // Leaving says why the panel went: the note is written where the shell ended, not over it.
    s.until("the host's stop said", |screen| {
        pty::any(screen, "the session host stopped")
    });
}

/// Under `NO_COLOR` a terminal is not the session either: the usage report, exit 2.
#[tokio::test]
async fn bare_iohr_on_a_terminal_under_no_color_prints_the_usage_report() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let mut c = iohr(&server, dir.path(), &[]);
    c.env("TERM", "xterm-256color").env("NO_COLOR", "1");
    let mut s = pty::Session::spawn(c);
    let status = s.exited().expect("it prints and ends");
    assert_eq!(status.code(), Some(2), "{}", s.screen.screen().contents());
    assert!(
        s.screen.screen().contents().contains("Usage: iohr"),
        "{}",
        s.screen.screen().contents()
    );
}
