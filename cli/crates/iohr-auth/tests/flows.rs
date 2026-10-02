//! Sign-in flows against a mock provider: discovery checks, the loopback redirect,
//! device polling, and a session refreshing with rotation.

#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iohr_auth::{
    AuthError, Authorization, Browser, Credential, Device, EntryKey, MemoryStore, Provider,
    Redacted, Session, Store,
};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT: &str = "iohr-cli";

fn jwt(claims: &serde_json::Value) -> String {
    let enc = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.c2ln",
        enc(&serde_json::json!({"alg": "RS256"})),
        enc(claims)
    )
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn access(n: u32, lifetime: i64) -> String {
    jwt(&serde_json::json!({
        "sub": "person-1", "aud": ["iohr-api"], "exp": now() + lifetime,
        "scp": ["openid", "offline_access", "iohr.api"], "org": "acc_1", "plan": "free", "n": n
    }))
}

fn tokens(issuer: &str, n: u32, lifetime: i64) -> serde_json::Value {
    serde_json::json!({
        "access_token": access(n, lifetime),
        "token_type": "bearer",
        "expires_in": lifetime,
        "refresh_token": format!("rt-{n}"),
        "id_token": jwt(&serde_json::json!({"iss": issuer, "aud": [CLIENT], "sub": "person-1"})),
    })
}

async fn provider_server() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": uri,
            "authorization_endpoint": format!("{uri}/oauth2/auth"),
            "token_endpoint": format!("{uri}/oauth2/token"),
            "device_authorization_endpoint": format!("{uri}/oauth2/device/auth"),
            "revocation_endpoint": format!("{uri}/oauth2/revoke"),
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn discovery_refuses_another_issuer_and_foreign_endpoints() {
    for doc in [
        serde_json::json!({"issuer": "https://evil.example", "authorization_endpoint": "x", "token_endpoint": "y"}),
        serde_json::json!({"issuer": "SELF", "authorization_endpoint": "https://evil.example/auth", "token_endpoint": "SELF/t"}),
    ] {
        let server = MockServer::start().await;
        let body = doc.to_string().replace("SELF", &server.uri());
        Mock::given(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;
        let e = Provider::discover(&server.uri(), CLIENT).await.unwrap_err();
        assert!(matches!(e, AuthError::Discovery(_)), "{e}");
    }
    assert!(
        Provider::discover("http://auth.example.com", CLIENT)
            .await
            .is_err(),
        "plain http to another host"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_sign_in_ignores_other_paths_and_checks_the_state() {
    let server = provider_server().await;
    let issuer = server.uri();
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier="))
        .respond_with(ResponseTemplate::new(200).set_body_json(tokens(&issuer, 1, 900)))
        .mount(&server)
        .await;

    let provider = Provider::discover(&issuer, CLIENT).await.unwrap();
    let auth = Authorization::<Browser>::start(provider).await.unwrap();
    let url = auth.url().clone();
    let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["audience"], "iohr-api");
    assert_eq!(q["client_id"], CLIENT);
    assert_eq!(q["code_challenge"].len(), 43);
    let redirect = q["redirect_uri"].clone();
    assert!(redirect.starts_with("http://127.0.0.1:") || redirect.starts_with("http://[::1]:"));
    let state = q["state"].clone();

    let browser = tokio::spawn(async move {
        let http = reqwest::Client::new();
        let base = redirect.trim_end_matches("/callback").to_owned();
        let favicon = http
            .get(format!("{base}/favicon.ico"))
            .send()
            .await
            .unwrap();
        assert_eq!(favicon.status(), 404);
        let page = http
            .get(format!("{redirect}?code=the-code&state={state}"))
            .send()
            .await
            .unwrap();
        let csp = page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(csp.contains("default-src 'none'"));
        page.text().await.unwrap()
    });
    let granted = auth.finish().await.unwrap();
    assert!(browser.await.unwrap().contains("Signed in"));
    assert_eq!(granted.account(), Some("acc_1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_callback_with_another_state_ends_the_sign_in() {
    let server = provider_server().await;
    let provider = Provider::discover(&server.uri(), CLIENT).await.unwrap();
    let auth = Authorization::<Browser>::start(provider).await.unwrap();
    let redirect = auth
        .url()
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .unwrap()
        .1
        .into_owned();
    let browser = tokio::spawn(async move {
        reqwest::get(format!("{redirect}?code=c&state=forged"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });
    let e = auth.finish().await.unwrap_err();
    assert!(matches!(e, AuthError::StateMismatch), "{e}");
    assert!(browser.await.unwrap().contains("did not finish"));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/oauth2/token")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_browser_window_is_bounded() {
    let server = provider_server().await;
    let provider = Provider::discover(&server.uri(), CLIENT).await.unwrap();
    let auth = Authorization::<Browser>::start(provider).await.unwrap();
    let e = auth
        .finish_within(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert!(matches!(e, AuthError::TimedOut), "{e}");
}

/// A provider whose device endpoint hands out code `dc`, and whose token endpoint
/// gives `answers(issuer)` in order, repeating the last one.
async fn device_server(answers: impl FnOnce(&str) -> Vec<ResponseTemplate>) -> MockServer {
    let server = provider_server().await;
    let uri = server.uri();
    Mock::given(method("POST"))
        .and(path("/oauth2/device/auth"))
        .and(body_string_contains("audience=iohr-api"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dc", "user_code": "ABCD2345", "verification_uri": format!("{uri}/oauth2/device/verify"),
            "verification_uri_complete": format!("{uri}/oauth2/device/verify?user_code=ABCD2345"),
            "expires_in": 600, "interval": 1
        })))
        .mount(&server)
        .await;
    let answers = answers(&uri);
    let n = answers.len();
    for (i, answer) in answers.into_iter().enumerate() {
        let mock = Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .and(body_string_contains("device_code=dc"))
            .respond_with(answer);
        if i + 1 < n {
            mock.up_to_n_times(1).mount(&server).await;
        } else {
            mock.mount(&server).await;
        }
    }
    server
}

fn oauth_error(code: &str) -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": code}))
}

#[tokio::test]
async fn device_polling_waits_slows_down_and_finishes() {
    let server = device_server(|issuer| {
        vec![
            oauth_error("authorization_pending"),
            oauth_error("slow_down"),
            ResponseTemplate::new(200).set_body_json(tokens(issuer, 7, 900)),
        ]
    })
    .await;
    let provider = Provider::discover(&server.uri(), CLIENT).await.unwrap();
    let auth = Authorization::<Device>::start(provider).await.unwrap();
    assert_eq!(auth.user_code(), "ABCD2345");
    assert!(auth.verification_uri_complete().is_some());
    let started = std::time::Instant::now();
    let granted = auth.finish().await.unwrap();
    // 1 s, 1 s, then 1 + 5 s after slow_down.
    assert!(
        started.elapsed() >= Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(granted.account(), Some("acc_1"));
}

#[tokio::test]
async fn device_polling_stops_on_denial_and_expiry() {
    for (answer, want_denied) in [("access_denied", true), ("expired_token", false)] {
        let server = device_server(|_| vec![oauth_error(answer)]).await;
        let provider = Provider::discover(&server.uri(), CLIENT).await.unwrap();
        let e = Authorization::<Device>::start(provider)
            .await
            .unwrap()
            .finish()
            .await
            .unwrap_err();
        if want_denied {
            assert!(matches!(e, AuthError::Denied), "{e}");
        } else {
            assert!(matches!(e, AuthError::TimedOut), "{e}");
        }
    }
}

#[tokio::test]
async fn a_device_link_to_another_site_is_refused() {
    let server = provider_server().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/device/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dc", "user_code": "X", "verification_uri": "https://evil.example/device", "expires_in": 600
        })))
        .mount(&server)
        .await;
    let provider = Provider::discover(&server.uri(), CLIENT).await.unwrap();
    assert!(Authorization::<Device>::start(provider).await.is_err());
}

fn stored(refresh: &str, access_token: &str, expires_at: i64) -> Redacted<String> {
    Redacted::new(
        serde_json::json!({"refresh_token": refresh, "access_token": access_token, "expires_at": expires_at}).to_string(),
    )
}

fn key() -> EntryKey {
    EntryKey::new("me".parse().unwrap(), "acc_1").unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_refreshes_and_writes_the_rotated_token_back() {
    let server = provider_server().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("refresh_token=rt-0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tokens(&server.uri(), 1, 900)))
        .expect(1)
        .mount(&server)
        .await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    store
        .set(&key(), &stored("rt-0", &access(0, -10), now() - 10))
        .unwrap();

    let session = Session::load(Arc::clone(&store), key(), &server.uri(), CLIENT)
        .await
        .unwrap();
    let first = session.bearer().await.unwrap();
    let second = session.bearer().await.unwrap();
    assert_eq!(first.expose(), second.expose(), "the fresh token is reused");
    let kept = store.get(&key()).unwrap().unwrap();
    assert!(
        kept.expose().contains("\"rt-1\""),
        "the rotated refresh token is stored"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_rotated_by_another_process_is_taken_instead_of_ending_the_session() {
    let server = provider_server().await;
    // rt-0 was rotated by another process: refusing it is not the end of the session.
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("refresh_token=rt-0"))
        .respond_with(oauth_error("invalid_grant"))
        .mount(&server)
        .await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    store
        .set(&key(), &stored("rt-0", &access(0, -10), now() - 10))
        .unwrap();
    let session = Session::load(Arc::clone(&store), key(), &server.uri(), CLIENT)
        .await
        .unwrap();
    // The other process writes its fresh pair after this one read the store.
    let other = Arc::clone(&store);
    let fresh = access(5, 900);
    let fresh_for_store = fresh.clone();
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("refresh_token=rt-0"))
        .respond_with(move |_: &wiremock::Request| {
            other
                .set(&key(), &stored("rt-5", &fresh_for_store, now() + 900))
                .unwrap();
            oauth_error("invalid_grant")
        })
        .with_priority(1)
        .mount(&server)
        .await;
    let bearer = session.bearer().await.unwrap();
    assert_eq!(bearer.expose(), fresh);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_refresh_with_nothing_newer_ends_the_session() {
    let server = provider_server().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(oauth_error("invalid_grant"))
        .mount(&server)
        .await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    store
        .set(&key(), &stored("rt-0", &access(0, -10), now() - 10))
        .unwrap();
    let session = Session::load(store, key(), &server.uri(), CLIENT)
        .await
        .unwrap();
    assert!(matches!(
        session.bearer().await.unwrap_err(),
        AuthError::SessionEnded { .. }
    ));
}

#[tokio::test]
async fn revoking_sends_the_refresh_token_to_the_revocation_endpoint() {
    let server = provider_server().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/revoke"))
        .and(body_string_contains("token=rt-0"))
        .and(body_string_contains("token_type_hint=refresh_token"))
        .and(body_string_contains("client_id=iohr-cli"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    store
        .set(&key(), &stored("rt-0", &access(0, 900), now() + 900))
        .unwrap();
    let session = Session::load(store, key(), &server.uri(), CLIENT)
        .await
        .unwrap();
    session.revoke().await.unwrap();
}

#[tokio::test]
async fn debug_output_of_a_session_holds_no_token() {
    let server = provider_server().await;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    store
        .set(&key(), &stored("rt-secret", &access(0, 900), now() + 900))
        .unwrap();
    let session = Session::load(store, key(), &server.uri(), CLIENT)
        .await
        .unwrap();
    let shown = format!("{session:?} {:?}", session.bearer().await.unwrap());
    assert!(
        !shown.contains("rt-secret") && !shown.contains("eyJ"),
        "{shown}"
    );
}
