//! The page's calls of the API through `/api/proxy/v1/...` (RFC 0217, the live slice): a
//! read, a write with a JSON body, or a server-sent event stream.
//!
//! Every call acts as the profile `iohr login` signed in; the credential never reaches the
//! page and nothing of a body or a token is logged. The gates run before this module is
//! reached (`web_ui::reply_to`): the `Host` check, the session cookie, a same-origin
//! caller and the per-serve token header; a write also needs the exact `Origin`
//! (`post_caller_allowed`). What this module adds is bounds: the body is capped and read
//! within a window, a call has a deadline, a stream has an idle timeout, and only a few
//! streams run at once per serve. A stream is closed when the page goes away (the
//! connection reads end of file), when the serve stops, or when the API ends it.

use std::time::Duration;

use inorbithr::{Method, Operation};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep, timeout};

use super::session::{Access, forwarded, safe, status_line};
use super::{CSP, Reply, Request, SECURITY_HEADERS, WRITE_WINDOW};
use crate::error::Error;

/// The bounds of the proxy; the defaults are the shipped ones, tests make them small.
#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    /// The largest request body a write may carry.
    pub(super) max_body: usize,
    /// How long the body may take to arrive.
    pub(super) body_window: Duration,
    /// How long one call of the API may take, retries included.
    pub(super) call_window: Duration,
    /// Streams open at once per serve. A browser keeps six connections per origin; two
    /// stay free for reads and writes.
    pub(super) max_streams: usize,
    /// A stream with no event from the API for this long is closed (the page reconnects).
    pub(super) stream_idle: Duration,
    /// The comment line that tells the page the stream is alive, and the server that the
    /// page is.
    pub(super) keepalive: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body: 1024 * 1024,
            body_window: Duration::from_secs(10),
            call_window: Duration::from_secs(125),
            max_streams: 4,
            stream_idle: Duration::from_secs(300),
            keepalive: Duration::from_secs(15),
        }
    }
}

/// Answers one `/api/proxy/...` request on `stream`: `rest` is what the head read left of
/// the body.
pub(super) async fn forward(
    stream: &mut TcpStream,
    request: &Request,
    rest: Vec<u8>,
    access: &Access,
) {
    let Some((path, query)) = forwarded(&request.target) else {
        write(stream, &Reply::error("400 Bad Request")).await;
        return;
    };
    let pairs: Vec<(&str, &str)> = query
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let wants_events = request
        .accept
        .as_deref()
        .is_some_and(|a| a.contains("text/event-stream"));
    if request.method == "GET" && wants_events {
        events(stream, access, &path, &pairs).await;
        return;
    }
    let reply = call(stream, request, rest, access, &path, &pairs).await;
    write(stream, &reply).await;
}

async fn write(stream: &mut TcpStream, reply: &Reply) {
    let _ = timeout(WRITE_WINDOW, reply.write(stream)).await;
}

/// One read or write: the API's status and JSON body pass on, nothing else does.
/// The API areas the page may write to (RFC 0217): the ones a view of it manages. A write elsewhere is refused
/// before it is read, so a new area is a deliberate change here.
const WRITE_PREFIXES: [&str; 8] = [
    "/v1/accounts/",
    "/v1/approvals/",
    "/v1/atlas/",
    "/v1/coordinator/",
    "/v1/decisions/",
    "/v1/sensors/",
    "/v1/planner/",
    "/v1/canvas/",
];

async fn call(
    stream: &mut TcpStream,
    request: &Request,
    rest: Vec<u8>,
    access: &Access,
    path: &str,
    pairs: &[(&str, &str)],
) -> Reply {
    let limits = access.limits();
    let writes = !matches!(request.method.as_str(), "GET" | "HEAD");
    if writes && !WRITE_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return Reply::problem("403 Forbidden", "the page may not write there");
    }
    let method = match request.method.as_str() {
        "GET" | "HEAD" => Method::Get,
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "PATCH" => Method::Patch,
        "DELETE" => Method::Delete,
        _ => return Reply::error("405 Method Not Allowed"),
    };
    let reads = matches!(method, Method::Get | Method::Delete);
    let body = match read_body(stream, request, rest, limits, reads).await {
        Ok(b) => b,
        Err(reply) => return reply,
    };
    let s = match access.current().await {
        Ok(s) => s,
        Err(Error::NotSignedIn(_)) => return Reply::problem("401 Unauthorized", "not signed in"),
        Err(e) => return Reply::problem("502 Bad Gateway", &safe(&e)),
    };
    let sent = timeout(
        limits.call_window,
        s.api.send(method, path, pairs, body.as_ref()),
    )
    .await;
    let mut reply = match sent {
        Err(_) => Reply::problem("504 Gateway Timeout", "the API did not answer in time"),
        Ok(Ok(raw)) => {
            let json = raw
                .headers
                .get("content-type")
                .is_some_and(|c| c.contains("json"));
            Reply {
                status: status_line(raw.status),
                content_type: if json {
                    "application/json"
                } else {
                    "text/plain; charset=utf-8"
                },
                body: raw.body,
                ..Reply::error("200 OK")
            }
        }
        Ok(Err(e)) => {
            let status = e.status().map_or("502 Bad Gateway", status_line);
            Reply::problem(status, &safe(&Error::from(e)))
        }
    };
    reply.head_only = request.method == "HEAD";
    reply
}

/// The JSON body of a write, `None` when there is none. Anything but a plain
/// `Content-Length` body of JSON, within the cap and the window, is refused.
async fn read_body(
    stream: &mut TcpStream,
    request: &Request,
    mut have: Vec<u8>,
    limits: Limits,
    reads: bool,
) -> Result<Option<Value>, Reply> {
    if request.transfer_encoding.is_some() {
        return Err(Reply::error("411 Length Required"));
    }
    let length = match request.content_length.as_deref() {
        None => 0,
        Some(l) => l
            .parse::<usize>()
            .map_err(|_| Reply::error("400 Bad Request"))?,
    };
    if length == 0 {
        return Ok(None);
    }
    if reads {
        return Err(Reply::error("400 Bad Request"));
    }
    if length > limits.max_body {
        return Err(Reply::error("413 Payload Too Large"));
    }
    let json = request.content_type.as_deref().is_some_and(|c| {
        c.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json {
        return Err(Reply::error("415 Unsupported Media Type"));
    }
    have.truncate(length);
    let filled = timeout(limits.body_window, async {
        let mut chunk = [0_u8; 8192];
        while have.len() < length {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let take = n.min(length - have.len());
            have.extend_from_slice(&chunk[..take]);
        }
        Ok::<(), std::io::Error>(())
    })
    .await;
    if !matches!(filled, Ok(Ok(()))) {
        return Err(Reply::error("408 Request Timeout"));
    }
    serde_json::from_slice(&have)
        .map(Some)
        .map_err(|_| Reply::problem("400 Bad Request", "the body is not JSON"))
}

/// One event of a stream as the page reads it: `id:` when the event carries a resume
/// cursor (`sse_id`), then the JSON on a single `data:` line.
fn frame(event: &Value) -> String {
    let mut out = String::new();
    if let Some(id) = event.get("sse_id").and_then(Value::as_str)
        && !id.is_empty()
        && !id.contains(['\r', '\n'])
    {
        out.push_str("id: ");
        out.push_str(id);
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(&event.to_string());
    out.push_str("\n\n");
    out
}

/// `error` event: the same shape the page reads from a failed call.
fn error_frame(status: &str, message: &str) -> String {
    let body = serde_json::json!({ "error": { "status": status, "message": message } });
    format!("event: error\ndata: {body}\n\n")
}

/// `GET` with `Accept: text/event-stream`: opens the API's stream and passes its events on
/// as they come, with no buffering, until the API ends it, the idle timeout passes, the
/// page goes away, or the serve stops.
async fn events(stream: &mut TcpStream, access: &Access, path: &str, pairs: &[(&str, &str)]) {
    let limits = access.limits();
    let Some(_permit) = access.stream_permit() else {
        write(
            stream,
            &Reply::problem("429 Too Many Requests", "too many live streams"),
        )
        .await;
        return;
    };
    let s = match access.current().await {
        Ok(s) => s,
        Err(Error::NotSignedIn(_)) => {
            write(stream, &Reply::problem("401 Unauthorized", "not signed in")).await;
            return;
        }
        Err(e) => {
            write(stream, &Reply::problem("502 Bad Gateway", &safe(&e))).await;
            return;
        }
    };
    let mut op = Operation::new(Method::Get, path.to_owned());
    for (k, v) in pairs {
        op = op.query((*k).to_owned(), (*v).to_owned());
    }
    let opened = timeout(limits.call_window, s.api.client().stream::<Value>(op)).await;
    let mut upstream = match opened {
        Err(_) => {
            let late = Reply::problem("504 Gateway Timeout", "the API did not answer in time");
            write(stream, &late).await;
            return;
        }
        Ok(Err(e)) => {
            let status = e.status().map_or("502 Bad Gateway", status_line);
            write(stream, &Reply::problem(status, &safe(&Error::from(e)))).await;
            return;
        }
        Ok(Ok(events)) => events,
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
         Cache-Control: no-store, no-transform\r\nX-Accel-Buffering: no\r\n\
         Content-Security-Policy: {CSP}\r\n{SECURITY_HEADERS}Connection: close\r\n\r\n: open\n\n"
    );
    if !put(stream, head.as_bytes()).await {
        return;
    }
    let mut closing = access.closing();
    let mut beat = interval_at(Instant::now() + limits.keepalive, limits.keepalive);
    beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let idle = sleep(limits.stream_idle);
    tokio::pin!(idle);
    let mut probe = [0_u8; 256];
    loop {
        tokio::select! {
            next = upstream.next() => match next {
                Some(Ok(event)) => {
                    if !put(stream, frame(&event).as_bytes()).await {
                        return;
                    }
                    idle.as_mut().reset(Instant::now() + limits.stream_idle);
                }
                Some(Err(e)) => {
                    let status = e.status().map_or("502 Bad Gateway", status_line);
                    put(stream, error_frame(status, &safe(&Error::from(e))).as_bytes()).await;
                    break;
                }
                None => break,
            },
            _ = beat.tick() => {
                if !put(stream, b": keepalive\n\n").await {
                    return;
                }
            }
            // Nothing is expected from the page after its request; end of file or an
            // error means it went away.
            read = stream.read(&mut probe) => {
                if matches!(read, Ok(0) | Err(_)) {
                    return;
                }
            }
            () = &mut idle => break,
            _ = closing.changed() => break,
        }
    }
    let _ = timeout(WRITE_WINDOW, stream.shutdown()).await;
}

/// Writes `bytes` and flushes them to the page; false when it is gone or too slow.
async fn put(stream: &mut TcpStream, bytes: &[u8]) -> bool {
    matches!(
        timeout(WRITE_WINDOW, async {
            stream.write_all(bytes).await?;
            stream.flush().await
        })
        .await,
        Ok(Ok(()))
    )
}
