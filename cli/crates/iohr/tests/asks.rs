//! `iohr asks` against a mock API: the list and the ask once, the two decisions a terminal
//! may make, approving that only points at the phone, the denied path, and that the text of
//! an ask never reaches stderr (RFC 0204, ADR 0215).

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::Path;
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ACCOUNT: &str = "acc_test1";
const ID: &str = "apr_01M6NZ452CGYK04BSGM9BNJD4Q";
const SECRET_TEXT: &str = "PRIVATE-ASK-TEXT";

fn token() -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(json!({"alg": "RS256"})),
        enc(json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["approvals:read"], "org": ACCOUNT, "plan": "free", "key": "key_1"
        }))
    )
}

fn iohr(server: &MockServer, config: &Path, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.args(args)
        .env_clear()
        .env("IOHR_BASE_URL", server.uri())
        .env("IOHR_CONFIG_DIR", config)
        .env("IOHR_TOKEN", token());
    for k in ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    c.stdin(Stdio::null()).output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn ask(status: &str) -> serde_json::Value {
    json!({
        "approval_id": ID, "kind": "pull_request", "title": format!("Merge {SECRET_TEXT}"),
        "status": status, "agent": "builder", "created_at": "2026-11-05T09:00:00Z",
        "summary": "Fixes it.", "approve_means": "Merge the pull request.",
        "payload": {"pull_request": {"repo": "inorbithr/core", "number": 12}},
        "timeline": [{"step": "asked", "by": "builder", "at": "2026-11-05T09:00:00Z"}]
    })
}

async fn list_mock(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/approvals"))
        .and(query_param("account_id", ACCOUNT))
        .and(query_param("status", "open"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"approvals": [ask("pending")]})),
        )
        .mount(server)
        .await;
}

async fn get_mock(server: &MockServer, status: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/approvals/{ID}")))
        .and(query_param("account_id", ACCOUNT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"approval": ask(status)})))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_prints_the_table_once_in_a_pipe_and_json_on_request() {
    let server = MockServer::start().await;
    list_mock(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "list"]);
    assert!(o.status.success(), "{}", err(&o));
    let t = out(&o);
    assert!(
        t.starts_with("ASK") && t.contains(ID) && t.contains("pending"),
        "{t}"
    );
    assert!(t.contains(SECRET_TEXT) && !t.contains('\u{1b}'));

    let o = iohr(&server, dir.path(), &["asks", "list", "--json", "--once"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["approvals"][0]["approval_id"], ID);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_rejects_an_unknown_status_before_any_call() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "list", "--status", "bogus"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn show_prints_the_ask_with_its_timeline_and_never_logs_its_text() {
    let server = MockServer::start().await;
    get_mock(&server, "pending").await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(
        &server,
        dir.path(),
        &["--verbose", "asks", "show", ID, "--once"],
    );
    assert!(o.status.success(), "{}", err(&o));
    let t = out(&o);
    for want in [ID, "pending", "inorbithr/core", "Approve means", "Timeline"] {
        assert!(t.contains(want), "missing {want}: {t}");
    }
    assert!(!err(&o).contains(SECRET_TEXT), "ask text on stderr");
    assert!(!out(&o).contains("TOKENSIGNATUREMARKER") && !err(&o).contains("TOKENSIGNATUREMARKER"));
}

#[tokio::test(flavor = "multi_thread")]
async fn show_refuses_a_malformed_id_without_calling() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "show", "../x"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_sends_the_note_and_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/approvals/{ID}/decision")))
        .and(body_json(
            json!({"account_id": ACCOUNT, "decision": "deny", "note": "not yet"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"approval": ask("denied")})))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(
        &server,
        dir.path(),
        &["asks", "deny", ID, "--note", "not yet"],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("Denied") && out(&o).contains("denied"),
        "{}",
        out(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn question_needs_its_text_and_then_keeps_the_ask_open() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "question", ID]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(server.received_requests().await.unwrap().is_empty());

    Mock::given(method("POST"))
        .and(path(format!("/v1/approvals/{ID}/decision")))
        .and(body_json(
            json!({"account_id": ACCOUNT, "decision": "question", "note": "which repo?"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"approval": ask("question")})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let o = iohr(
        &server,
        dir.path(),
        &["asks", "question", ID, "--note", "which repo?"],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("stays open"), "{}", out(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn approve_points_at_the_phone_and_never_decides() {
    let server = MockServer::start().await;
    get_mock(&server, "pending").await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "approve", ID, "--once"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("Approve on your phone"), "{}", err(&o));
    let posts = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 0, "the terminal never approves");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_decision_says_to_sign_in() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/approvals/{ID}/decision")))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"message": "a person must decide"})),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "deny", ID]);
    assert!(!o.status.success());
    assert!(err(&o).contains("iohr login"), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn decide_needs_a_terminal() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let o = iohr(&server, dir.path(), &["asks", "decide", ID]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
}
