//! The routes, the request parser, and the one place a request is traced, logged and counted.

pub mod obs;
pub mod signal;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use obs::{Logger, Metrics, Span, Val};

/// What the handlers share. `ready` is false until the server listens and again as soon as
/// shutdown starts, so a load balancer stops sending traffic before the process stops.
pub struct Service {
    /// Where records go.
    pub log: Logger,
    /// The request counters.
    pub metrics: Metrics,
    /// Whether `/readyz` answers 200.
    pub ready: AtomicBool,
}

impl Service {
    /// A service that logs to `log`.
    pub fn new(log: Logger) -> Self {
        Self {
            log,
            metrics: Metrics::default(),
            ready: AtomicBool::new(false),
        }
    }
}

/// A parsed request: the only parts the routes use.
struct Request {
    method: String,
    path: String,
    query: String,
    traceparent: Option<String>,
}

fn parse(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?, first.next()?);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let traceparent = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("traceparent"))
        .map(|(_, v)| v.trim().to_owned());
    Some(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        traceparent,
    })
}

/// The value of `key` in a query string, percent-decoded; `None` when it is absent or not text.
fn query_value(query: &str, key: &str) -> Option<Option<String>> {
    let raw = query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)?
        .1;
    let mut bytes = Vec::with_capacity(raw.len());
    let mut it = raw.bytes();
    while let Some(b) = it.next() {
        match b {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hex: Vec<u8> = it.by_ref().take(2).collect();
                let v = std::str::from_utf8(&hex)
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match v {
                    Some(v) => bytes.push(v),
                    None => return Some(None),
                }
            }
            b => bytes.push(b),
        }
    }
    Some(String::from_utf8(bytes).ok())
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What a route answers: the matched route, the status, the body and its content type.
fn route(svc: &Service, req: &Request) -> (&'static str, u16, String, &'static str) {
    const JSON: &str = "application/json";
    if req.method != "GET" {
        return (
            "unmatched",
            405,
            "{\"error\":\"method not allowed\"}\n".into(),
            JSON,
        );
    }
    match req.path.as_str() {
        "/healthz" => ("/healthz", 200, "{\"status\":\"ok\"}\n".into(), JSON),
        "/readyz" if svc.ready.load(Ordering::SeqCst) => {
            ("/readyz", 200, "{\"status\":\"ready\"}\n".into(), JSON)
        }
        "/readyz" => ("/readyz", 503, "{\"status\":\"not ready\"}\n".into(), JSON),
        "/metrics" => (
            "/metrics",
            200,
            svc.metrics.render(),
            "text/plain; version=0.0.4",
        ),
        // Your first route: replace it. The name comes from the query string, so it is
        // bounded and echoed back, never logged.
        "/v1/hello" => match query_value(&req.query, "name") {
            None => (
                "/v1/hello",
                200,
                "{\"message\":\"Hello, world\"}\n".into(),
                JSON,
            ),
            Some(Some(n)) if n.chars().count() <= 64 => {
                let n = if n.is_empty() { "world".to_owned() } else { n };
                (
                    "/v1/hello",
                    200,
                    format!("{{\"message\":{}}}\n", json_string(&format!("Hello, {n}"))),
                    JSON,
                )
            }
            Some(_) => (
                "/v1/hello",
                400,
                "{\"error\":\"name is at most 64 characters of text\"}\n".into(),
                JSON,
            ),
        },
        _ => ("unmatched", 404, "{\"error\":\"not found\"}\n".into(), JSON),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    }
}

/// Reads one request from `stream` and answers it, tracing, logging and counting it.
fn handle(svc: &Service, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(i);
        }
        if buf.len() > 16 * 1024 {
            break None;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let parsed = head_end.and_then(|i| parse(&String::from_utf8_lossy(&buf[..i])));
    let span = Span::start(parsed.as_ref().and_then(|r| r.traceparent.as_deref()));
    let (route_name, status, body, ctype) = match &parsed {
        Some(req) => route(svc, req),
        None => (
            "unmatched",
            if head_end.is_none() { 431 } else { 400 },
            "{\"error\":\"bad request\"}\n".into(),
            "application/json",
        ),
    };
    let reply = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\ntraceparent: {}\r\nConnection: close\r\n\r\n{body}",
        reason(status),
        body.len(),
        span.header()
    );
    let _ = stream.write_all(reply.as_bytes());
    let ms = span.millis();
    let method = parsed
        .as_ref()
        .map_or("GET", |r| if r.method == "GET" { "GET" } else { "OTHER" });
    let name = format!("{method} {route_name}");
    let mut fields = vec![
        ("trace_id", Val::Str(&span.trace_id)),
        ("route", Val::Str(route_name)),
        ("status", Val::Int(u64::from(status))),
        ("duration_ms", Val::Float(ms)),
    ];
    let mut span_fields = vec![
        ("span_id", Val::Str(&span.span_id)),
        ("name", Val::Str(&name)),
    ];
    if let Some(p) = &span.parent_id {
        span_fields.push(("parent_span_id", Val::Str(p)));
    }
    span_fields.extend(fields.iter().map(|(k, v)| (*k, clone_val(v))));
    svc.log.info("span", &span_fields);
    fields.insert(0, ("method", Val::Str(method)));
    svc.log.info("request", &fields);
    svc.metrics.inc(route_name, status);
}

fn clone_val<'a>(v: &Val<'a>) -> Val<'a> {
    match v {
        Val::Str(s) => Val::Str(s),
        Val::Int(n) => Val::Int(*n),
        Val::Float(f) => Val::Float(*f),
    }
}

/// Serves `listener` until `stop` is set, then waits for the requests in flight (10 s at most).
///
/// # Errors
///
/// When the listener cannot be made non-blocking.
pub fn serve(listener: &TcpListener, svc: &Arc<Service>, stop: &AtomicBool) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let in_flight = Arc::new(AtomicUsize::new(0));
    svc.ready.store(true, Ordering::SeqCst);
    let addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    svc.log.info("listening", &[("addr", Val::Str(&addr))]);
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let (svc, n) = (Arc::clone(svc), Arc::clone(&in_flight));
                n.fetch_add(1, Ordering::SeqCst);
                thread::spawn(move || {
                    handle(&svc, stream);
                    n.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
    svc.ready.store(false, Ordering::SeqCst);
    svc.log.info("shutting down", &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while in_flight.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
