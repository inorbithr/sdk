//! The writes and the streams of `iohr ui serve` (RFC 0217, the live slice): a real server on a free port, a
//! wiremock API for the writes and a raw socket as the API for the streams (so a test can hold an event back and
//! prove the page gets the one before it).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iohr_auth::Redacted;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use wiremock::matchers::{body_json, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::forward::Limits;
use super::tests::{built_ui, get, panels_with, send, start_limited, token_of};
use crate::Env;

fn jwt() -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.sig",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_ui", "org": "org_1", "aud": ["iohr-api"],
            "exp": 4_102_444_800_i64, "scp": ["iohr.api"]
        }))
    )
}

struct Rig {
    _dir: tempfile::TempDir,
    _cfg: tempfile::TempDir,
    addr: std::net::SocketAddr,
    host: String,
    token: String,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn rig(api_url: &str, limits: Limits) -> Rig {
    let dir = built_ui();
    let cfg = tempfile::tempdir().expect("cfg");
    let env = Env {
        token: Some(Redacted::new(jwt())),
        ..Env::default()
    };
    let panels = panels_with(
        &[
            "--base-url",
            api_url,
            "--config-dir",
            cfg.path().to_str().expect("utf8"),
        ],
        env,
    );
    let (addr, stop) = start_limited(dir.path(), panels, limits).await;
    let host = super::host_of(addr);
    let token = token_of(&send(addr, &get("/", &host)).await);
    Rig {
        _dir: dir,
        _cfg: cfg,
        addr,
        host,
        token,
        _stop: stop,
    }
}

impl Rig {
    /// A write as the page sends it.
    fn write(&self, verb: &str, target: &str, body: &str, extra: &str) -> String {
        format!(
            "{verb} /api/proxy{target} HTTP/1.1\r\nHost: {h}\r\nSec-Fetch-Site: same-origin\r\n\
             Origin: http://{h}\r\nx-iohr-token: {t}\r\nContent-Type: application/json\r\n\
             Content-Length: {n}\r\n{extra}\r\n{body}",
            h = self.host,
            t = self.token,
            n = body.len()
        )
    }

    /// An event stream as the page opens it (the write side stays open: a half close means "page gone").
    async fn open(&self, target: &str) -> TcpStream {
        let mut s = TcpStream::connect(self.addr).await.expect("connect");
        let raw = format!(
            "GET /api/proxy{target} HTTP/1.1\r\nHost: {h}\r\nSec-Fetch-Site: same-origin\r\n\
             Origin: http://{h}\r\nx-iohr-token: {t}\r\nAccept: text/event-stream\r\n\r\n",
            h = self.host,
            t = self.token
        );
        let raw = with_cookie(self.addr, &raw);
        s.write_all(raw.as_bytes()).await.expect("write");
        s
    }
}

fn with_cookie(addr: std::net::SocketAddr, raw: &str) -> String {
    let probe = super::tests::cookie_of(addr);
    raw.replacen("\r\n", &format!("\r\nCookie: {probe}\r\n"), 1)
}

fn status(answer: &str) -> &str {
    answer.lines().next().unwrap_or_default()
}

async fn read_until(s: &mut TcpStream, needle: &str) -> String {
    let mut got = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut chunk))
            .await
            .expect("the stream stalled")
            .expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&chunk[..n]);
        if String::from_utf8_lossy(&got).contains(needle) {
            break;
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

#[tokio::test]
async fn a_write_is_forwarded_with_the_credential_and_the_body() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/accounts/org_1/members"))
        .and(header_exists("authorization"))
        .and(body_json(serde_json::json!({"email": "a@b.c"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "m1"})))
        .expect(1)
        .mount(&api)
        .await;
    let rig = rig(&api.uri(), Limits::default()).await;

    let r = send(
        rig.addr,
        &rig.write(
            "POST",
            "/v1/accounts/org_1/members",
            r#"{"email":"a@b.c"}"#,
            "",
        ),
    )
    .await;
    assert!(status(&r).contains("201"), "{r}");
    assert!(r.contains(r#""id":"m1""#), "{r}");
    assert!(!r.contains(&rig.token), "the start token is never echoed");
    assert!(!r.contains(&jwt()), "the credential never reaches the page");
}

#[tokio::test]
async fn a_write_is_refused_without_the_token_the_origin_the_type_or_a_known_path() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    let rig = rig(
        &api.uri(),
        Limits {
            max_body: 16,
            ..Limits::default()
        },
    )
    .await;
    let ok = "/v1/accounts/org_1/members";
    let body = r#"{"a":1}"#;

    let no_token = rig
        .write("POST", ok, body, "")
        .replace(&format!("x-iohr-token: {}\r\n", rig.token), "");
    let bad_token = rig
        .write("POST", ok, body, "")
        .replace(&rig.token, &"0".repeat(64));
    let bad_origin = rig.write("POST", ok, body, "").replace(
        &format!("Origin: http://{}", rig.host),
        "Origin: http://evil.example",
    );
    let cross_site = rig
        .write("POST", ok, body, "")
        .replace("same-origin", "cross-site");
    let bad_host = rig.write("POST", ok, body, "").replacen(
        &format!("Host: {}", rig.host),
        "Host: evil.example",
        1,
    );
    for (name, raw) in [
        ("no token", no_token),
        ("bad token", bad_token),
        ("bad origin", bad_origin),
        ("cross-site", cross_site),
        ("bad host", bad_host),
    ] {
        let r = send(rig.addr, &raw).await;
        assert!(status(&r).contains("403"), "{name}: {r}");
    }

    let cases = [
        (
            "text/plain",
            rig.write("POST", ok, body, "")
                .replace("application/json", "text/plain"),
            "415",
        ),
        (
            "too large",
            rig.write("POST", ok, &format!("{{\"a\":\"{}\"}}", "x".repeat(40)), ""),
            "413",
        ),
        ("not json", rig.write("POST", ok, "{nope", ""), "400"),
        (
            "chunked",
            rig.write("POST", ok, body, "Transfer-Encoding: chunked\r\n"),
            "4",
        ),
        (
            "unknown area",
            rig.write("POST", "/v1/billing/charge", body, ""),
            "403",
        ),
        ("the socket", rig.write("POST", "/v1/ws", body, ""), "403"),
        (
            "traversal",
            rig.write("POST", "/v1/accounts/../x", body, ""),
            "400",
        ),
        ("v2", rig.write("POST", "/v2/accounts/x", body, ""), "400"),
        ("delete with body", rig.write("DELETE", ok, body, ""), "400"),
        ("method", rig.write("TRACE", ok, "", ""), "405"),
    ];
    for (name, raw, want) in cases {
        let r = send(rig.addr, &raw).await;
        assert!(status(&r).contains(want), "{name}: {r}");
    }
}

#[tokio::test]
async fn a_delete_without_a_body_is_forwarded() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/accounts/org_1/members/m1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&api)
        .await;
    let rig = rig(&api.uri(), Limits::default()).await;
    let raw = rig
        .write("DELETE", "/v1/accounts/org_1/members/m1", "", "")
        .replace("Content-Type: application/json\r\n", "");
    let r = send(rig.addr, &raw).await;
    assert!(status(&r).contains("204"), "{r}");
}

/// The API for a stream: it answers `GET` with an SSE head, then lets a test write the events when it likes.
async fn fake_stream_api() -> (String, tokio::sync::mpsc::Receiver<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut head = Vec::new();
            let mut b = [0_u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut b).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&b[..n]),
                }
            }
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n\r\n")
                .await;
            if tx.send(s).await.is_err() {
                break;
            }
        }
    });
    (url, rx)
}

#[tokio::test]
async fn events_arrive_in_order_and_unbuffered_with_their_ids() {
    let (url, mut upstream) = fake_stream_api().await;
    let rig = rig(&url, Limits::default()).await;
    let mut page = rig.open("/v1/sensors/stream").await;
    let mut api = upstream
        .recv()
        .await
        .expect("the page's stream reached the API");

    api.write_all(b"data: {\"n\":1,\"sse_id\":\"c1\"}\n\n")
        .await
        .expect("e1");
    api.flush().await.expect("flush");
    // The API now holds event 2 back: the page must already have event 1.
    let first = read_until(&mut page, "\"n\":1").await;
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    assert!(first.contains("text/event-stream"), "{first}");
    assert!(
        first.contains("Cache-Control: no-store, no-transform"),
        "{first}"
    );
    assert!(first.contains("id: c1\n"), "{first}");
    assert!(!first.contains("\"n\":2"), "{first}");

    api.write_all(b"data: {\"n\":2}\n\n").await.expect("e2");
    let second = read_until(&mut page, "\"n\":2").await;
    assert!(second.contains("data: {\"n\":2}"), "{second}");
}

#[tokio::test]
async fn a_stream_with_no_events_is_closed_after_the_idle_window() {
    let (url, mut upstream) = fake_stream_api().await;
    let limits = Limits {
        stream_idle: Duration::from_millis(300),
        keepalive: Duration::from_millis(100),
        ..Limits::default()
    };
    let rig = rig(&url, limits).await;
    let mut page = rig.open("/v1/sensors/stream").await;
    let _api = upstream.recv().await.expect("reached the API");
    let mut all = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), page.read_to_end(&mut all))
        .await
        .expect("closed in time")
        .expect("read");
    let text = String::from_utf8_lossy(&all);
    assert!(text.contains(": keepalive"), "{text}");
}

#[tokio::test]
async fn streams_past_the_cap_are_refused_and_a_closed_one_frees_its_place() {
    let (url, mut upstream) = fake_stream_api().await;
    let rig = rig(
        &url,
        Limits {
            max_streams: 1,
            ..Limits::default()
        },
    )
    .await;
    let mut one = rig.open("/v1/sensors/stream").await;
    let _api = upstream.recv().await.expect("reached the API");
    let _ = read_until(&mut one, ": open").await;

    let mut two = rig.open("/v1/sensors/stream").await;
    let mut got = Vec::new();
    two.read_to_end(&mut got).await.expect("read");
    assert!(
        status(&String::from_utf8_lossy(&got)).contains("429"),
        "{}",
        String::from_utf8_lossy(&got)
    );

    drop(one); // the page went away
    let mut again;
    let mut tries = 0;
    loop {
        again = rig.open("/v1/sensors/stream").await;
        let head = read_until(&mut again, "\r\n").await;
        if head.contains("200") || tries > 40 {
            assert!(head.contains("200"), "the place was never freed: {head}");
            break;
        }
        tries += 1;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_page_that_goes_away_closes_the_stream_to_the_api() {
    let (url, mut upstream) = fake_stream_api().await;
    let rig = rig(&url, Limits::default()).await;
    let mut page = rig.open("/v1/sensors/stream").await;
    let mut api = upstream.recv().await.expect("reached the API");
    let _ = read_until(&mut page, ": open").await;
    drop(page);
    // The proxy drops its call: the API's socket reads end of file or a reset.
    let mut buf = [0_u8; 64];
    let end = tokio::time::timeout(Duration::from_secs(5), api.read(&mut buf)).await;
    assert!(
        matches!(end, Ok(Ok(0) | Err(_))),
        "the upstream stream stayed open: {end:?}"
    );
}
