//! `GET /api/live`: one WebSocket for all of the page's live streams (RFC 0217, slice 2).
//!
//! A browser keeps six HTTP/1.1 connections per origin, and every server-sent event stream
//! holds one for as long as it is open. The page opens this one socket instead and asks for
//! many streams over it; `iohr ui serve` carries them to the API on one upstream `/v1/ws`,
//! through the SDK's own socket multiplexer (`Streams::Socket`), not a second one.
//!
//! The socket is used only for the routes the API has a socket call for ([`route`]); any
//! other path is answered `unmapped` and the page reads it as server-sent events through
//! `/api/proxy`, which stays the default.
//!
//! Frames are JSON text. From the page: `{"type":"auth","token":"..."}` first, then
//! `{"type":"call","id":"1","path":"/v1/coordinator/picture/events?tier=live"}` and
//! `{"type":"cancel","id":"1"}`. To the page: `{"type":"ready"}`, then per call
//! `{"type":"data","id","data"}`, `{"type":"end","id"}` or
//! `{"type":"error","id","status","message"}`; an error without an id ends the socket.
//!
//! The guards are the proxy's: the `Host` and session cookie are checked on the upgrade
//! like any request, with `Sec-Fetch-Site: same-origin` and an exact `Origin`. A browser
//! cannot set a header on a WebSocket handshake, so the per-serve token (the proxy's
//! `x-iohr-token`) travels in the first frame instead, compared in constant time, and no call
//! is read before it. That is the one place this differs from the proxy; the cookie is
//! `SameSite=Strict` and the `Origin` is exact, so another site cannot open the socket.
//! The socket takes one of the serve's stream slots, closes after the idle limit with
//! nothing in either direction, is pinged to keep it alive, and ends when the serve stops.
//! Frames are small (64 KiB) and a socket carries at most [`MAX_CALLS`] calls. Nothing a
//! call carries (path values, tokens, events) is logged.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt as _, StreamExt as _};
use inorbithr::{Method, Operation};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use super::session::{Access, forwarded, safe, status_line};
use super::{Panels, Reply, Request, WRITE_WINDOW};
use crate::context::Api;
use crate::error::Error;

/// The most calls one page socket carries at once; the API's own cap is 32.
const MAX_CALLS: usize = 16;
/// The largest frame either way.
const MAX_FRAME: usize = 64 * 1024;
/// Frames waiting for the page; a page that stops reading ends its socket.
const QUEUE: usize = 256;

/// How a query value is sent in the call's request message.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Int,
    Bool,
}

/// One streaming call the API has on its socket: the RPC, the path's parameter if any,
/// and the query fields it takes.
struct Route {
    rpc: &'static str,
    /// The path template; `{}` is the one path parameter.
    template: &'static [&'static str],
    param: &'static str,
    fields: &'static [(&'static str, Kind)],
}

const ROUTES: &[Route] = &[
    Route {
        rpc: "iohr.coordinator.v1.CoordinatorService/WatchCoordinator",
        template: &["v1", "coordinator", "picture", "events"],
        param: "",
        fields: &[
            ("account_id", Kind::Text),
            ("from_seq", Kind::Int),
            ("tier", Kind::Text),
        ],
    },
    Route {
        rpc: "iohr.coordinator.v1.CoordinatorService/WatchQueuedAsks",
        template: &["v1", "coordinator", "asks", "events"],
        param: "",
        fields: &[("source", Kind::Text), ("account_id", Kind::Text)],
    },
    Route {
        rpc: "iohr.coordinator.v1.CoordinatorService/WatchThread",
        template: &["v1", "coordinator", "threads", "{}", "events"],
        param: "thread_id",
        fields: &[("account_id", Kind::Text)],
    },
    Route {
        rpc: "iohr.coordinator.v1.CoordinatorService/WatchOpenItems",
        template: &["v1", "coordinator", "open-items", "events"],
        param: "",
        fields: &[("mine", Kind::Bool), ("account_id", Kind::Text)],
    },
    Route {
        rpc: "iohr.approvals.v1.ApprovalsService/Watch",
        template: &["v1", "approvals", "{}", "events"],
        param: "approval_id",
        fields: &[("account_id", Kind::Text)],
    },
    Route {
        rpc: "iohr.sensors.v1.SensorsService/StreamSignals",
        template: &["v1", "atlas", "signals", "events"],
        param: "",
        fields: &[
            ("account_id", Kind::Text),
            ("sensors", Kind::Text),
            ("after", Kind::Text),
        ],
    },
    Route {
        rpc: "iohr.events.v1.EventsService/StreamEvents",
        template: &["v1", "events", "events"],
        param: "",
        fields: &[("types", Kind::Text), ("account_id", Kind::Text)],
    },
];

/// The socket call a stream path maps to, as an operation the SDK opens over its socket;
/// `None` when the API has no socket call for it (the page reads it as server-sent events).
/// A query key the call does not take, or a value that is not of its kind, is refused.
fn route(path_and_query: &str) -> Option<Operation<'static>> {
    let (path, pairs) = forwarded(&format!("/api/proxy{path_and_query}"))?;
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let found = ROUTES.iter().find(|r| {
        r.template.len() == segments.len()
            && r.template
                .iter()
                .zip(&segments)
                .all(|(t, s)| *t == "{}" || t == s)
    })?;
    let mut op = Operation::new(Method::Get, path.clone()).rpc(found.rpc);
    if let Some(at) = found.template.iter().position(|t| *t == "{}") {
        op = op.field(found.param, segments[at]);
    }
    for (key, value) in pairs {
        let (_, kind) = found.fields.iter().find(|(k, _)| *k == key)?;
        op = match kind {
            Kind::Text => op.field(&key, value),
            Kind::Int => op.field(&key, value.parse::<i64>().ok()?),
            Kind::Bool => op.field(&key, value.parse::<bool>().ok()?),
        };
    }
    Some(op)
}

/// Whether the upgrade request is the page's own: the strict form of the proxy's write
/// check, without the token (a handshake cannot carry the header; the first frame does).
pub(super) fn upgrade_allowed(request: &Request, own: &str) -> bool {
    let site_ok = request
        .fetch_site
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("same-origin"));
    let origin_ok = request
        .origin
        .as_deref()
        .and_then(|o| o.strip_prefix("http://"))
        .is_some_and(|h| h == own);
    site_ok && origin_ok
}

/// Whether the request asks for a version 13 WebSocket upgrade, and its key.
pub(super) fn upgrade_key(request: &Request) -> Option<&str> {
    let upgrade = request
        .upgrade
        .as_deref()
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"));
    let version = request.ws_version.as_deref() == Some("13");
    let key = request.ws_key.as_deref().filter(|k| !k.is_empty())?;
    (upgrade && version).then_some(key)
}

fn text(value: &Value) -> Message {
    Message::text(value.to_string())
}

fn error(id: Option<&str>, status: &str, message: &str) -> Value {
    let mut v = json!({ "type": "error", "status": status, "message": message });
    if let Some(id) = id {
        v["id"] = id.into();
    }
    v
}

/// Answers an upgrade the gates let through: the handshake, then the socket's life.
pub(super) async fn serve(
    mut stream: TcpStream,
    request: &Request,
    rest: &[u8],
    panels: &Panels,
    access: &Access,
) {
    let Some(key) = upgrade_key(request) else {
        let reply = Reply::problem("400 Bad Request", "this is a WebSocket address");
        let _ = timeout(WRITE_WINDOW, reply.write(&mut stream)).await;
        return;
    };
    // The client waits for the 101 before it sends anything.
    if !rest.is_empty() {
        let reply = Reply::error("400 Bad Request");
        let _ = timeout(WRITE_WINDOW, reply.write(&mut stream)).await;
        return;
    }
    let Some(permit) = access.stream_permit() else {
        let reply = Reply::problem("429 Too Many Requests", "too many live streams");
        let _ = timeout(WRITE_WINDOW, reply.write(&mut stream)).await;
        return;
    };
    let head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        derive_accept_key(key.as_bytes())
    );
    let wrote = timeout(WRITE_WINDOW, async {
        stream.write_all(head.as_bytes()).await?;
        stream.flush().await
    })
    .await;
    if !matches!(wrote, Ok(Ok(()))) {
        return;
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let socket = WebSocketStream::from_raw_socket(stream, Role::Server, Some(config)).await;
    run(socket, panels, access).await;
    drop(permit);
}

/// Waits for the first frame to be the right token.
async fn authenticate(
    socket: &mut WebSocketStream<TcpStream>,
    panels: &Panels,
    window: std::time::Duration,
) -> bool {
    let first = timeout(window, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(t))) => return Some(t),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    })
    .await;
    let Ok(Some(frame)) = first else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&frame) else {
        return false;
    };
    value.get("type").and_then(Value::as_str) == Some("auth")
        && panels.token_matches(value.get("token").and_then(Value::as_str))
}

async fn close_with(socket: &mut WebSocketStream<TcpStream>, frame: &Value) {
    let _ = timeout(WRITE_WINDOW, async {
        let _ = socket.send(text(frame)).await;
        let _ = socket.close(None).await;
    })
    .await;
}

/// One call's life: open the stream over the shared upstream socket and pass what comes.
async fn call(
    api: Arc<Api>,
    id: String,
    op: Operation<'static>,
    out: mpsc::Sender<Value>,
    open: std::time::Duration,
) {
    let opened = timeout(open, api.client().stream::<Value>(op)).await;
    let mut upstream = match opened {
        Err(_) => {
            let _ = out
                .send(error(
                    Some(&id),
                    "504 Gateway Timeout",
                    "the API did not answer in time",
                ))
                .await;
            return;
        }
        Ok(Err(e)) => {
            let status = e.status().map_or("502 Bad Gateway", status_line);
            let _ = out
                .send(error(Some(&id), status, &safe(&Error::from(e))))
                .await;
            return;
        }
        Ok(Ok(events)) => events,
    };
    while let Some(next) = upstream.next().await {
        let item = match next {
            Ok(event) => json!({ "type": "data", "id": id, "data": event }),
            Err(e) => {
                let status = e.status().map_or("502 Bad Gateway", status_line);
                let _ = out
                    .send(error(Some(&id), status, &safe(&Error::from(e))))
                    .await;
                return;
            }
        };
        if out.send(item).await.is_err() {
            return;
        }
    }
    let _ = out.send(json!({ "type": "end", "id": id })).await;
}

/// The socket after its handshake.
async fn run(mut socket: WebSocketStream<TcpStream>, panels: &Panels, access: &Access) {
    let limits = access.limits();
    if !authenticate(&mut socket, panels, limits.body_window).await {
        close_with(&mut socket, &error(None, "403 Forbidden", "not allowed")).await;
        return;
    }
    let api = match access.current().await {
        Ok(s) => access.socket_api(&s).map(Arc::new),
        Err(e) => Err(e),
    };
    let api = match api {
        Ok(api) => api,
        Err(Error::NotSignedIn(_)) => {
            close_with(
                &mut socket,
                &error(None, "401 Unauthorized", "not signed in"),
            )
            .await;
            return;
        }
        Err(e) => {
            close_with(&mut socket, &error(None, "502 Bad Gateway", &safe(&e))).await;
            return;
        }
    };
    if socket
        .send(text(&json!({ "type": "ready" })))
        .await
        .is_err()
    {
        return;
    }
    let (out, mut queue) = mpsc::channel::<Value>(QUEUE);
    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut live: HashMap<String, AbortHandle> = HashMap::new();
    let mut closing = access.closing();
    let mut beat = interval_at(Instant::now() + limits.keepalive, limits.keepalive);
    beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let idle = sleep(limits.stream_idle);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            incoming = socket.next() => {
                let Some(Ok(message)) = incoming else { break };
                idle.as_mut().reset(Instant::now() + limits.stream_idle);
                match message {
                    Message::Text(frame) => {
                        let reply = handle(&frame, &api, &out, &mut tasks, &mut live, limits.call_window);
                        if let Some(reply) = reply
                            && socket.send(text(&reply)).await.is_err()
                        {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    Message::Binary(_) => {
                        close_with(&mut socket, &error(None, "400 Bad Request", "frames are text")).await;
                        break;
                    }
                    _ => {}
                }
            }
            Some(frame) = queue.recv() => {
                if frame.get("type").and_then(Value::as_str) != Some("data")
                    && let Some(id) = frame.get("id").and_then(Value::as_str)
                {
                    live.remove(id);
                }
                idle.as_mut().reset(Instant::now() + limits.stream_idle);
                if timeout(WRITE_WINDOW, socket.send(text(&frame))).await.map_or(true, |r| r.is_err()) {
                    break;
                }
            }
            _ = beat.tick() => {
                if timeout(WRITE_WINDOW, socket.send(Message::Ping(Vec::new().into()))).await.map_or(true, |r| r.is_err()) {
                    break;
                }
            }
            () = &mut idle => break,
            _ = closing.changed() => break,
        }
    }
    // Ends every call: the tasks are aborted with the set, the upstream socket closes with
    // the client.
    tasks.abort_all();
    let _ = timeout(WRITE_WINDOW, socket.close(None)).await;
}

/// One frame from the page after the handshake: its immediate answer, if any.
fn handle(
    frame: &str,
    api: &Arc<Api>,
    out: &mpsc::Sender<Value>,
    tasks: &mut JoinSet<()>,
    live: &mut HashMap<String, AbortHandle>,
    open: std::time::Duration,
) -> Option<Value> {
    let Ok(value) = serde_json::from_str::<Value>(frame) else {
        return Some(error(None, "400 Bad Request", "the frame is not JSON"));
    };
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| !i.is_empty() && i.len() <= 64);
    match (value.get("type").and_then(Value::as_str), id) {
        (Some("call"), Some(id)) => {
            if live.contains_key(id) {
                return Some(error(Some(id), "409 Conflict", "that id is in use"));
            }
            if live.len() >= MAX_CALLS {
                return Some(error(
                    Some(id),
                    "429 Too Many Requests",
                    "too many calls on this socket",
                ));
            }
            let path = value
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(op) = route(path) else {
                return Some(error(Some(id), "404 Not Found", "unmapped"));
            };
            let handle = tasks.spawn(call(Arc::clone(api), id.to_owned(), op, out.clone(), open));
            live.insert(id.to_owned(), handle);
            None
        }
        (Some("cancel"), Some(id)) => {
            if let Some(handle) = live.remove(id) {
                handle.abort();
            }
            None
        }
        _ => Some(error(id, "400 Bad Request", "unknown frame")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(op: &Operation) -> String {
        format!("{op:?}")
    }

    #[test]
    fn mapped_paths_become_socket_calls() {
        let picture = route("/v1/coordinator/picture/events?tier=live&from_seq=7").expect("mapped");
        assert!(fields(&picture).contains("WatchCoordinator"));
        let thread = route("/v1/coordinator/threads/t-1/events").expect("mapped");
        assert!(fields(&thread).contains("t-1"));
    }

    #[test]
    fn unmapped_or_malformed_paths_are_not_calls() {
        assert!(route("/v1/coordinator/plan/events").is_none());
        assert!(route("/v1/coordinator/picture/events?nope=1").is_none());
        assert!(route("/v1/coordinator/picture/events?from_seq=x").is_none());
        assert!(route("/v1/coordinator/open-items/events?mine=maybe").is_none());
        assert!(route("/v1/../coordinator/picture/events").is_none());
        assert!(route("/other").is_none());
    }
}
