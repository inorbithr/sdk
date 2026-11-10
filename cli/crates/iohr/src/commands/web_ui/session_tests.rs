//! The sign-in and the proxy behind `iohr ui serve` (RFC 0217, round 4): the page gate,
//! the cookie exchange, the session state and the forwarded reads.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iohr_auth::Redacted;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::tests::{
    built_ui, get, launch_of, panels_with, post, send, send_bare, start, start_panels, token_of,
};
use crate::Env;
use crate::ext::install::Store;

const SECRET_MARK: &str = "SIGNATUREMARKER";

fn jwt() -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.{SECRET_MARK}",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_ui", "org": "org_1", "aud": ["iohr-api"],
            "exp": 4_102_444_800_i64, "scp": ["iohr.api"]
        }))
    )
}

fn body_of(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

fn first_line(answer: &str) -> &str {
    answer.lines().next().unwrap_or_default()
}

/// A browser's own call of the page: same origin, with the token header.
fn api(path: &str, host: &str, token: Option<&str>) -> String {
    let token = token.map_or_else(String::new, |t| format!("x-iohr-token: {t}\r\n"));
    format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nSec-Fetch-Site: same-origin\r\n\
         Origin: http://{host}\r\n{token}\r\n"
    )
}

/// A serve acting as the holder of `IOHR_TOKEN`, against `api_url`, with an empty config.
async fn signed_in(
    api_url: &str,
    config: &std::path::Path,
) -> (
    tempfile::TempDir,
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    String,
) {
    let dir = built_ui();
    let env = Env {
        token: Some(Redacted::new(jwt())),
        ..Env::default()
    };
    let panels = panels_with(
        &[
            "--base-url",
            api_url,
            "--config-dir",
            config.to_str().expect("utf8"),
        ],
        env,
    );
    let store = Store::new(dir.path().join("store"));
    let (addr, stop) = start_panels(dir.path(), store, panels).await;
    let page = send(addr, &get("/", &super::host_of(addr))).await;
    let token = token_of(&page);
    (dir, addr, stop, token)
}

#[tokio::test]
async fn a_page_without_the_launch_secret_or_the_cookie_is_refused() {
    let dir = built_ui();
    let (addr, _stop) = start(dir.path()).await;
    let host = super::host_of(addr);

    for target in [
        "/",
        "/index.html",
        "/assets/x.js",
        "/?launch=nope",
        "/?launch=",
    ] {
        let r = send_bare(addr, &get(target, &host)).await;
        assert!(first_line(&r).contains("401"), "{target}: {r}");
        assert!(!r.contains("iohr-token"), "{target} leaks the start token");
    }
    let r = send_bare(addr, &api("/api/extensions", &host, None)).await;
    assert!(first_line(&r).contains("401"), "{r}");
    assert!(body_of(&r).contains("\"error\""), "{r}");

    // A cookie of another serve (another port's name) is not this one's.
    let other = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nCookie: iohr_ui_1={}\r\n\r\n",
        "0".repeat(64)
    );
    let r = send_bare(addr, &other).await;
    assert!(first_line(&r).contains("401"), "{r}");
}

#[tokio::test]
async fn the_launch_secret_is_exchanged_for_a_cookie_and_leaves_the_address() {
    let dir = built_ui();
    let (addr, _stop) = start(dir.path()).await;
    let host = super::host_of(addr);
    let launch = launch_of(addr);

    let r = send_bare(addr, &get(&format!("/?launch={launch}"), &host)).await;
    assert!(first_line(&r).contains("303"), "{r}");
    let head = r.split("\r\n\r\n").next().unwrap_or_default();
    assert!(
        head.contains("Location: /\r\n") || head.ends_with("Location: /"),
        "{head}"
    );
    let cookie = head
        .lines()
        .find(|l| l.starts_with("Set-Cookie: "))
        .expect("a cookie");
    for part in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(cookie.contains(part), "{cookie}");
    }
    assert!(
        cookie.contains(&format!("iohr_ui_{}=", addr.port())),
        "{cookie}"
    );
    assert!(
        !cookie.contains(&launch),
        "the cookie is not the launch secret"
    );
    assert!(
        !r.contains(&launch[..]) || head.contains("Set-Cookie"),
        "no secret in the body"
    );

    // The cookie, and only the cookie, opens the page.
    let value = cookie
        .trim_start_matches("Set-Cookie: ")
        .split(';')
        .next()
        .expect("pair");
    let with = format!("GET / HTTP/1.1\r\nHost: {host}\r\nCookie: {value}\r\n\r\n");
    let r = send_bare(addr, &with).await;
    assert!(first_line(&r).contains("200"), "{r}");
    assert!(r.contains("iohr-token"), "the page carries its token");
}

#[tokio::test]
async fn the_launch_secret_is_spent_by_its_first_use_and_a_second_use_is_refused() {
    let dir = built_ui();
    let (addr, _stop) = start(dir.path()).await;
    let host = super::host_of(addr);
    let launch = launch_of(addr);

    let first = send_bare(addr, &get(&format!("/?launch={launch}"), &host)).await;
    assert!(first_line(&first).contains("303"), "{first}");

    // The same address again, as a copied or leaked link would be: refused, no cookie.
    let again = send_bare(addr, &get(&format!("/?launch={launch}"), &host)).await;
    assert!(first_line(&again).contains("401"), "{again}");
    assert!(!again.contains("Set-Cookie"), "{again}");
}

#[tokio::test]
async fn a_launch_secret_is_never_spent_or_accepted_by_a_post() {
    let dir = built_ui();
    let (addr, _stop) = start(dir.path()).await;
    let host = super::host_of(addr);
    let launch = launch_of(addr);

    let raw =
        format!("POST /?launch={launch} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\n\r\n");
    let r = send_bare(addr, &raw).await;
    assert!(first_line(&r).contains("401"), "{r}");

    // The refused POST did not spend it: the first GET still gets in.
    let r = send_bare(addr, &get(&format!("/?launch={launch}"), &host)).await;
    assert!(first_line(&r).contains("303"), "{r}");
}

#[tokio::test]
async fn a_spent_secret_is_replaced_in_the_record_so_iohr_ui_open_keeps_working() {
    let dir = built_ui();
    let folder = tempfile::tempdir().expect("folder");
    let server = super::Server::bind(
        dir.path(),
        0,
        Store::new(dir.path().join("no-extensions")),
        panels_with(&[], Env::default()),
    )
    .await
    .expect("bind");
    let (addr, url) = (server.addr, server.url());
    super::remember(folder.path(), &url, dir.path(), &server.launch()).expect("remember");
    server.keep_record(folder.path().to_owned(), url, dir.path().to_owned());
    let first = server.launch();
    let access = server.site.access.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(server.serve(async {
        let _ = rx.await;
    }));

    let host = super::host_of(addr);
    let r = send_bare(addr, &get(&format!("/?launch={first}"), &host)).await;
    assert!(first_line(&r).contains("303"), "{r}");

    let next = access.launch();
    assert_ne!(next, first, "the spent secret is replaced");
    let text = std::fs::read_to_string(folder.path().join(super::SERVING)).expect("record");
    assert_eq!(super::recorded_launch(&text), Some(next.clone()), "{text}");
    assert!(!text.contains(&first), "the spent secret is gone: {text}");

    // The replacement opens once, like the first did.
    let r = send_bare(addr, &get(&format!("/?launch={next}"), &host)).await;
    assert!(first_line(&r).contains("303"), "{r}");
    let _ = tx.send(());
}

#[tokio::test]
async fn the_session_state_says_not_signed_in_and_never_carries_a_credential() {
    let config = tempfile::tempdir().expect("config");
    let mock = MockServer::start().await;
    let dir = built_ui();
    let panels = panels_with(
        &[
            "--base-url",
            &mock.uri(),
            "--config-dir",
            config.path().to_str().expect("utf8"),
        ],
        Env::default(),
    );
    let (addr, _stop) =
        start_panels(dir.path(), Store::new(dir.path().join("store")), panels).await;
    let host = super::host_of(addr);
    let token = token_of(&send(addr, &get("/", &host)).await);

    // Without the page's token header, or with a wrong one: no.
    for t in [None, Some("0".repeat(64))] {
        let r = send(addr, &api("/api/session", &host, t.as_deref())).await;
        assert!(first_line(&r).contains("403"), "{r}");
    }
    let r = send(addr, &api("/api/session", &host, Some(&token))).await;
    assert!(first_line(&r).contains("200"), "{r}");
    let v: serde_json::Value = serde_json::from_str(body_of(&r)).expect("json");
    assert_eq!(v["signed_in"], false, "{v}");
    assert_eq!(v["login"], "idle", "{v}");
}

#[tokio::test]
async fn the_session_state_names_who_is_signed_in_without_the_credential() {
    let mock = MockServer::start().await;
    let config = tempfile::tempdir().expect("config");
    let (_dir, addr, _stop, token) = signed_in(&mock.uri(), config.path()).await;
    let host = super::host_of(addr);

    let r = send(addr, &api("/api/session", &host, Some(&token))).await;
    let v: serde_json::Value = serde_json::from_str(body_of(&r)).expect("json");
    assert_eq!(v["signed_in"], true, "{v}");
    assert_eq!(v["kind"], "token", "{v}");
    assert_eq!(v["subject"], "ak_ui", "{v}");
    assert_eq!(v["account"], "org_1", "{v}");
    assert!(!r.contains(SECRET_MARK), "no credential in the answer: {r}");
    assert!(!r.contains(&jwt()), "no credential in the answer");
}

#[tokio::test]
async fn the_proxy_forwards_one_read_with_the_credential_and_hides_it() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/orgs/org_1/monitors"))
        .and(query_param("limit", "5 x"))
        .and(header(
            "authorization",
            format!("Bearer {}", jwt()).as_str(),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"monitors":[{"id":"m1"}]}"#),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/nothing"))
        .respond_with(
            ResponseTemplate::new(404)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":"gone"}"#),
        )
        .mount(&mock)
        .await;
    let config = tempfile::tempdir().expect("config");
    let (_dir, addr, _stop, token) = signed_in(&mock.uri(), config.path()).await;
    let host = super::host_of(addr);

    let r = send(
        addr,
        &api(
            "/api/proxy/v1/orgs/org_1/monitors?limit=5+x",
            &host,
            Some(&token),
        ),
    )
    .await;
    assert!(first_line(&r).contains("200"), "{r}");
    assert!(body_of(&r).contains(r#""id":"m1""#), "{r}");
    assert!(!r.contains(SECRET_MARK), "the credential is not echoed");

    let r = send(addr, &api("/api/proxy/v1/nothing", &host, Some(&token))).await;
    assert!(first_line(&r).contains("404"), "{r}");
    assert!(body_of(&r).contains("gone"), "{r}");

    // Only a plain read under /v1/ goes through; the token header is needed.
    for bad in [
        "/api/proxy/v2/x",
        "/api/proxy/",
        "/api/proxy/v1/../v2/x",
        "/api/proxy/v1/%2e%2e/x",
        "/api/proxy/v1//x",
        "/api/proxy/v1/a%2Fb/%00",
    ] {
        let r = send(addr, &api(bad, &host, Some(&token))).await;
        assert!(first_line(&r).contains("400"), "{bad}: {r}");
    }
    let r = send(addr, &api("/api/proxy/v1/nothing", &host, None)).await;
    assert!(first_line(&r).contains("403"), "{r}");
    let r = send_bare(addr, &api("/api/proxy/v1/nothing", &host, Some(&token))).await;
    assert!(first_line(&r).contains("401"), "no cookie: {r}");
}

#[tokio::test]
async fn the_proxy_answers_401_when_nobody_is_signed_in() {
    let mock = MockServer::start().await;
    let config = tempfile::tempdir().expect("config");
    let dir = built_ui();
    let panels = panels_with(
        &[
            "--base-url",
            &mock.uri(),
            "--config-dir",
            config.path().to_str().expect("utf8"),
        ],
        Env::default(),
    );
    let (addr, _stop) =
        start_panels(dir.path(), Store::new(dir.path().join("store")), panels).await;
    let host = super::host_of(addr);
    let token = token_of(&send(addr, &get("/", &host)).await);

    let r = send(addr, &api("/api/proxy/v1/accounts/me", &host, Some(&token))).await;
    assert!(first_line(&r).contains("401"), "{r}");
    assert!(body_of(&r).contains("not signed in"), "{r}");
    assert!(
        mock.received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn starting_a_sign_in_needs_the_token_and_reports_a_failure() {
    let config = tempfile::tempdir().expect("config");
    let dir = built_ui();
    let panels = panels_with(
        &[
            "--base-url",
            "http://127.0.0.1:1",
            "--issuer",
            "http://127.0.0.1:1",
            "--config-dir",
            config.path().to_str().expect("utf8"),
        ],
        Env::default(),
    );
    let (addr, _stop) =
        start_panels(dir.path(), Store::new(dir.path().join("store")), panels).await;
    let host = super::host_of(addr);
    let token = token_of(&send(addr, &get("/", &host)).await);

    let r = send(
        addr,
        &post("/api/session/login", &host, &"0".repeat(64), ""),
    )
    .await;
    assert!(first_line(&r).contains("403"), "{r}");
    let r = send_bare(addr, &post("/api/session/login", &host, &token, "")).await;
    assert!(first_line(&r).contains("401"), "no cookie: {r}");

    let r = send(addr, &post("/api/session/login", &host, &token, "")).await;
    assert!(first_line(&r).contains("200"), "{r}");
    assert!(body_of(&r).contains("waiting"), "{r}");

    // The issuer is unreachable: the sign-in settles as failed, with a reason.
    let mut state = serde_json::Value::Null;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let r = send(addr, &api("/api/session", &host, Some(&token))).await;
        state = serde_json::from_str(body_of(&r)).expect("json");
        if state["login"] != "waiting" {
            break;
        }
    }
    assert_eq!(state["login"], "failed", "{state}");
    assert!(
        state["reason"].as_str().is_some_and(|s| !s.is_empty()),
        "{state}"
    );
    assert_eq!(state["signed_in"], false);
}

/// A failure whose text quotes a credential, as a transport error quoting a request
/// would.
struct Quoting(String);

impl std::fmt::Display for Quoting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[test]
fn error_text_for_the_page_never_carries_anything_shaped_like_a_token() {
    let token = jwt();
    for said in [
        format!("request failed: Authorization: Bearer {token}"),
        format!("rejected the credential {token} for this call"),
        "sent header authorization=Bearer fakebearer0123456789abcdefghij".to_owned(),
        "body was {\"access_token\": \"fakebearer0123456789abcdefghij\"}".to_owned(),
        "used ghp_0123456789abcdefghijklmnopqrstuvwxyz1234 to call".to_owned(),
        "password=hunter2hunter2 refused".to_owned(),
    ] {
        let seen = super::session::safe(&Quoting(said.clone()));
        for marker in [SECRET_MARK, "fakebearer", "ghp_0123", "hunter2"] {
            assert!(!seen.contains(marker), "{said} -> {seen}");
        }
        assert!(seen.contains("[redacted]"), "{said} -> {seen}");
    }
    // Plain words stay readable.
    assert_eq!(
        super::session::safe(&Quoting("the sign-in was cancelled".into())),
        "the sign-in was cancelled"
    );
}

#[tokio::test]
async fn a_failed_proxy_call_never_shows_the_page_the_credential() {
    // Nothing listens at the base address, and it carries a credential in its user part,
    // so the transport error has something to quote.
    let config = tempfile::tempdir().expect("config");
    let base = format!("http://user:{SECRET_MARK}pw@127.0.0.1:9");
    let (_dir, addr, _stop, token) = signed_in(&base, config.path()).await;
    let host = super::host_of(addr);

    let r = send(
        addr,
        &api("/api/proxy/v1/orgs/org_1/monitors", &host, Some(&token)),
    )
    .await;
    assert!(!first_line(&r).contains("200"), "{r}");
    assert!(!r.contains(SECRET_MARK), "{r}");
    assert!(!r.contains(&jwt()), "{r}");
    let r = send(addr, &api("/api/session", &host, Some(&token))).await;
    assert!(!r.contains(SECRET_MARK), "{r}");
}
