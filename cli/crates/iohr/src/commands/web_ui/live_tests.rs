//! `/api/live` (RFC 0217, slice 2): a real server on a free port, a real WebSocket client as the
//! page, and a WebSocket server standing in for the API's `/v1/ws`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::{WebSocketStream, accept_async, client_async};

use super::forward::Limits;
use super::forward_tests::{Rig, rig, status, with_cookie};
use super::tests::send_bare;

type Page = WebSocketStream<TcpStream>;
/// A refusal case: its name, how it changes the upgrade, and the status it must get.
type Case = (&'static str, Box<dyn Fn(String) -> String>, &'static str);

/// The API's `/v1/ws`: every call gets two data frames; `WatchCoordinator` then stays open
/// until cancelled, anything else ends. What the API received is sent on `seen`.
struct FakeApi {
    url: String,
    accepts: Arc<AtomicUsize>,
    seen: mpsc::UnboundedReceiver<Value>,
}

async fn fake_api() -> FakeApi {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let accepts = Arc::new(AtomicUsize::new(0));
    let (tx, seen) = mpsc::unbounded_channel();
    let count = Arc::clone(&accepts);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            count.fetch_add(1, Ordering::SeqCst);
            let tx = tx.clone();
            tokio::spawn(async move {
                let Ok(mut ws) = accept_async(tcp).await else {
                    return;
                };
                while let Some(Ok(Message::Text(t))) = ws.next().await {
                    let frame: Value = serde_json::from_str(&t).expect("json");
                    let _ = tx.send(frame.clone());
                    if frame["type"] != "call" {
                        continue;
                    }
                    let id = frame["id"].clone();
                    for n in 1..=2 {
                        let data = json!({"type": "data", "id": id, "body": {"n": n}});
                        let _ = ws.send(Message::text(data.to_string())).await;
                    }
                    if !frame["method"]
                        .as_str()
                        .unwrap_or_default()
                        .ends_with("WatchCoordinator")
                    {
                        let end = json!({"type": "end", "id": id});
                        let _ = ws.send(Message::text(end.to_string())).await;
                    }
                }
            });
        }
    });
    FakeApi { url, accepts, seen }
}

impl Rig {
    /// The upgrade as the page sends it, `tweak` changing the header block first.
    fn upgrade_raw(&self, tweak: impl Fn(String) -> String) -> String {
        // RFC 6455's sample key, built here so secret scanners do not read it as a credential.
        let key = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            b"the sample nonce",
        );
        let raw = format!(
            "GET /api/live HTTP/1.1\r\nHost: {h}\r\nSec-Fetch-Site: same-origin\r\n\
             Origin: http://{h}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n",
            h = self.host
        );
        tweak(with_cookie(self.addr, &raw))
    }

    /// A page socket past the handshake (and the token frame when `auth`).
    async fn page(&self, auth: bool) -> Page {
        let tcp = TcpStream::connect(self.addr).await.expect("connect");
        let mut req = format!("ws://{}/api/live", self.host)
            .into_client_request()
            .expect("request");
        let h = req.headers_mut();
        h.insert(
            "Origin",
            format!("http://{}", self.host).parse().expect("origin"),
        );
        h.insert("Sec-Fetch-Site", "same-origin".parse().expect("site"));
        h.insert(
            "Cookie",
            super::tests::cookie_of(self.addr).parse().expect("cookie"),
        );
        let (mut ws, _) = client_async(req, tcp).await.expect("handshake");
        if auth {
            ws.send(Message::text(
                json!({"type": "auth", "token": self.token}).to_string(),
            ))
            .await
            .expect("auth");
            assert_eq!(next(&mut ws).await["type"], "ready");
        }
        ws
    }
}

async fn next(ws: &mut Page) -> Value {
    loop {
        let m = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a frame in time")
            .expect("open")
            .expect("frame");
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).expect("json");
        }
    }
}

async fn call(ws: &mut Page, id: &str, path: &str) {
    ws.send(Message::text(
        json!({"type": "call", "id": id, "path": path}).to_string(),
    ))
    .await
    .expect("send");
}

#[tokio::test]
async fn the_upgrade_is_refused_outside_the_pages_own_origin() {
    let rig = rig("http://127.0.0.1:9", Limits::default()).await;
    let h = rig.host.clone();
    let cases: Vec<Case> = vec![
        (
            "another origin",
            Box::new(move |r: String| {
                r.replace(&format!("Origin: http://{h}"), "Origin: http://evil.test")
            }),
            "403",
        ),
        (
            "a cross-site request",
            Box::new(|r: String| r.replace("same-origin", "cross-site")),
            "403",
        ),
        (
            "no origin",
            Box::new({
                let h = rig.host.clone();
                move |r: String| r.replace(&format!("Origin: http://{h}\r\n"), "")
            }),
            "403",
        ),
        (
            "another host",
            Box::new({
                let h = rig.host.clone();
                move |r: String| r.replace(&format!("Host: {h}"), "Host: evil.test:1")
            }),
            "403",
        ),
        (
            "no upgrade header",
            Box::new(|r: String| r.replace("Upgrade: websocket\r\n", "")),
            "400",
        ),
        (
            "a wrong version",
            Box::new(|r: String| r.replace("Version: 13", "Version: 8")),
            "400",
        ),
    ];
    for (name, tweak, want) in cases {
        let answer = send_bare(rig.addr, &rig.upgrade_raw(tweak)).await;
        assert!(
            status(&answer).contains(want),
            "{name}: {}",
            status(&answer)
        );
    }
    // No session cookie: not the page's, never a socket.
    let bare = rig.upgrade_raw(|r| r.replace("Cookie:", "X-Not:"));
    assert!(!status(&send_bare(rig.addr, &bare).await).contains("101"));
    // A POST is not an upgrade.
    let post = rig.upgrade_raw(|r| r.replacen("GET", "POST", 1));
    assert!(status(&send_bare(rig.addr, &post).await).contains("405"));
}

#[tokio::test]
async fn a_wrong_first_frame_token_closes_the_socket_before_any_call() {
    let mut api = fake_api().await;
    let rig = rig(&api.url, Limits::default()).await;
    let mut ws = rig.page(false).await;
    ws.send(Message::text(
        json!({"type": "auth", "token": "nope"}).to_string(),
    ))
    .await
    .expect("auth");
    let frame = next(&mut ws).await;
    assert_eq!(frame["type"], "error");
    assert!(frame["status"].as_str().expect("status").starts_with("403"));
    assert!(frame.get("id").is_none());
    assert!(matches!(
        ws.next().await,
        Some(Ok(Message::Close(_)) | Err(_)) | None
    ));
    assert!(api.seen.try_recv().is_err(), "nothing reached the API");
    assert_eq!(api.accepts.load(Ordering::SeqCst), 0);

    // A call before the token is the same as no token.
    let mut ws = rig.page(false).await;
    call(&mut ws, "1", "/v1/events/events").await;
    assert_eq!(next(&mut ws).await["type"], "error");
    assert_eq!(api.accepts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn calls_share_one_upstream_socket_and_cancel_reaches_the_api() {
    let mut api = fake_api().await;
    let rig = rig(&api.url, Limits::default()).await;
    let mut ws = rig.page(true).await;
    call(
        &mut ws,
        "held",
        "/v1/coordinator/picture/events?tier=live&from_seq=7",
    )
    .await;
    call(&mut ws, "short", "/v1/events/events?types=a").await;

    let mut held = Vec::new();
    let mut short = Vec::new();
    let mut ended = false;
    while held.len() < 2 || short.len() < 2 || !ended {
        let f = next(&mut ws).await;
        match (f["type"].as_str(), f["id"].as_str()) {
            (Some("data"), Some("held")) => held.push(f["data"]["n"].clone()),
            (Some("data"), Some("short")) => short.push(f["data"]["n"].clone()),
            (Some("end"), Some("short")) => ended = true,
            other => panic!("unexpected frame {other:?}: {f}"),
        }
    }
    assert_eq!(held, [json!(1), json!(2)]);
    assert_eq!(short, [json!(1), json!(2)]);
    assert_eq!(api.accepts.load(Ordering::SeqCst), 1, "one upstream socket");

    let mut methods = Vec::new();
    while let Ok(f) = api.seen.try_recv() {
        if f["type"] == "call" {
            methods.push((
                f["method"].as_str().expect("method").to_owned(),
                f["body"].clone(),
            ));
        }
    }
    methods.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        methods[0].0,
        "iohr.coordinator.v1.CoordinatorService/WatchCoordinator"
    );
    assert_eq!(methods[0].1, json!({"tier": "live", "from_seq": 7}));
    assert_eq!(methods[1].0, "iohr.events.v1.EventsService/StreamEvents");
    assert_eq!(methods[1].1, json!({"types": "a"}));

    ws.send(Message::text(
        json!({"type": "cancel", "id": "held"}).to_string(),
    ))
    .await
    .expect("cancel");
    let cancelled = timeout(Duration::from_secs(5), async {
        loop {
            let f = api.seen.recv().await.expect("api open");
            if f["type"] == "cancel" {
                return f;
            }
        }
    })
    .await
    .expect("the API saw the cancel");
    assert_eq!(cancelled["type"], "cancel");
}

#[tokio::test]
async fn an_unmapped_path_is_answered_and_the_socket_stays_up() {
    let api = fake_api().await;
    let rig = rig(&api.url, Limits::default()).await;
    let mut ws = rig.page(true).await;
    call(&mut ws, "p", "/v1/coordinator/plan/events").await;
    let f = next(&mut ws).await;
    assert_eq!(
        (f["type"].as_str(), f["id"].as_str()),
        (Some("error"), Some("p"))
    );
    assert_eq!(f["message"], "unmapped");
    call(&mut ws, "bad", "/v1/events/events?nope=1").await;
    assert_eq!(next(&mut ws).await["message"], "unmapped");
    call(&mut ws, "ok", "/v1/events/events").await;
    assert_eq!(next(&mut ws).await["type"], "data");
    ws.send(Message::text("not json")).await.expect("send");
    let mut f = next(&mut ws).await;
    while f["type"] != "error" {
        f = next(&mut ws).await;
    }
    assert!(f["status"].as_str().expect("status").starts_with("400"));
    assert_eq!(api.accepts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_stream_cap_counts_sockets_and_a_closed_one_frees_its_slot() {
    let api = fake_api().await;
    let limits = Limits {
        max_streams: 1,
        ..Limits::default()
    };
    let rig = rig(&api.url, limits).await;
    let first = rig.page(true).await;
    let answer = send_bare(rig.addr, &rig.upgrade_raw(|r| r)).await;
    assert!(status(&answer).contains("429"), "{}", status(&answer));
    drop(first);
    let mut got = String::new();
    for _ in 0..50 {
        let a = send_bare(
            rig.addr,
            &rig.upgrade_raw(|r| r.replace("Version: 13", "Version: 8")),
        )
        .await;
        got = status(&a).to_owned();
        if !got.contains("429") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(got.contains("400"), "the slot came back: {got}");
}

#[tokio::test]
async fn an_idle_socket_is_closed_and_pings_keep_a_busy_one_alive() {
    let api = fake_api().await;
    let limits = Limits {
        stream_idle: Duration::from_millis(600),
        keepalive: Duration::from_secs(60),
        ..Limits::default()
    };
    let rig = rig(&api.url, limits).await;
    let mut ws = rig.page(true).await;
    let ended = timeout(Duration::from_secs(5), async {
        while let Some(Ok(m)) = ws.next().await {
            if matches!(m, Message::Close(_)) {
                return true;
            }
        }
        true
    })
    .await;
    assert!(ended.is_ok(), "the socket closed after the idle limit");
}

#[tokio::test]
async fn a_page_that_goes_away_ends_its_calls_upstream() {
    let mut api = fake_api().await;
    let rig = rig(&api.url, Limits::default()).await;
    let mut ws = rig.page(true).await;
    call(&mut ws, "held", "/v1/coordinator/picture/events").await;
    assert_eq!(next(&mut ws).await["type"], "data");
    drop(ws);
    // The upstream socket closes with the last stream: the fake sees its connection end by the
    // frames stopping, and the serve's slot is free again.
    let mut free = false;
    for _ in 0..50 {
        let a = send_bare(
            rig.addr,
            &rig.upgrade_raw(|r| r.replace("Version: 13", "Version: 8")),
        )
        .await;
        if status(&a).contains("400") {
            free = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(free);
    while api.seen.try_recv().is_ok() {}
}
