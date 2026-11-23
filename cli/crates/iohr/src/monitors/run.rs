//! Runs a monitor once from here, the way the agent would judge it, and says pass, fail or skip.
//!
//! It speaks plain HTTP/1.1 and TCP over the standard library, so the crate stays small and
//! portable: http, sse and tcp run; https, tls and the other surfaces are skipped (the agent runs
//! them). Like the agent it judges the status, the headers, the time and, for sse, the count of
//! data events. With `expect.shape` and a [`Shapes`] it also reads the body (at most 1 MiB, or 20
//! events), checks it against its schema in memory and drops it (see [`super::shape`]); the agent
//! cannot do that yet (RFC 0008). With a [`Cli`], cli checks run the project's own binary in a
//! throwaway folder (see [`super::cli`]); without one they are skipped.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use super::cli::Cli;
use super::shape::{MAX_BODY, MAX_EVENTS, Shapes, check_body};
use super::{Monitor, Shape, Surface, Target};

/// How long one run may take when it names no `max_ms` (the platform's limit is 30s).
pub const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEAD: usize = 64 * 1024;

/// What one run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail(String),
    /// Not run here, and why.
    Skip(String),
}

/// One run of one monitor.
#[derive(Debug, Clone)]
pub struct Ran {
    pub name: String,
    pub verdict: Verdict,
    pub ms: u64,
    pub status: Option<u16>,
}

impl Ran {
    /// The line to print: `PASS name 12ms`, `FAIL name: why`, `FAIL shape name 200 /at: expected
    /// X, found Y`, or `SKIP name: why`.
    pub fn line(&self) -> String {
        match &self.verdict {
            Verdict::Pass => format!("PASS {} {}ms", self.name, self.ms),
            Verdict::Fail(why) => match why.strip_prefix("shape ") {
                Some(rest) => format!("FAIL shape {} {rest}", self.name),
                None => format!("FAIL {}: {why}", self.name),
            },
            Verdict::Skip(why) => format!("SKIP {}: {why}", self.name),
        }
    }
}

/// Runs `m` once, judging the status, headers and time only; `expect.shape` is not checked.
pub fn run(m: &Monitor) -> Ran {
    run_with(m, None, None)
}

/// Runs `m` once; with `shapes`, an `expect.shape` (and a cli `expect.request`) is checked too;
/// with `cli`, a cli check runs.
pub fn run_with(m: &Monitor, shapes: Option<&dyn Shapes>, cli: Option<&Cli>) -> Ran {
    let start = Instant::now();
    let (verdict, status) = match (&m.by, m.surface, &m.target) {
        (Some(by), ..) => (Verdict::Skip(format!("refused by the {by}, never sent")), None),
        (None, Surface::Http | Surface::Sse, Target::Url(u)) if u.scheme() == "http" => http(m, u, start, shapes),
        (None, Surface::Http | Surface::Sse, _) => (Verdict::Skip("https runs in the agent, not here".into()), None),
        (None, Surface::Tcp, Target::HostPort { host, port }) => (tcp(host, *port, timeout(m)), None),
        (None, Surface::Cli, _) => match cli {
            Some(c) => (c.run(m, shapes), None),
            None => (Verdict::Skip("cli checks run with the binary just built (iohr monitors run)".into()), None),
        },
        (None, s, _) => (Verdict::Skip(format!("{} runs in the agent, not here", s.as_str())), None),
    };
    let ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let verdict = match verdict {
        Verdict::Pass => match m.expect.max_ms {
            Some(max) if ms > max => Verdict::Fail(format!("took {ms}ms, more than {max}ms")),
            _ => Verdict::Pass,
        },
        v => v,
    };
    Ran { name: m.name.clone(), verdict, ms, status }
}

/// Runs every monitor once, or only the ones named in `only`, in file order. A name in `only`
/// that no monitor has is wrong usage.
pub fn run_all(monitors: &[Monitor], only: &[String], shapes: Option<&dyn Shapes>, cli: Option<&Cli>) -> Result<Vec<Ran>, super::Error> {
    if let Some(unknown) = only.iter().find(|n| !monitors.iter().any(|m| &m.name == *n)) {
        return Err(super::Error::Usage(format!("no monitor named {unknown}")));
    }
    Ok(monitors.iter().filter(|m| only.is_empty() || only.contains(&m.name)).map(|m| run_with(m, shapes, cli)).collect())
}

fn timeout(m: &Monitor) -> Duration {
    m.expect.max_ms.map_or(TIMEOUT, |ms| Duration::from_millis(ms.saturating_mul(2)).clamp(Duration::from_secs(1), TIMEOUT))
}

fn connect(host: &str, port: u16, limit: Duration) -> Result<TcpStream, String> {
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.collect();
    let mut last = format!("{host}: no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, limit) {
            Ok(s) => return Ok(s),
            Err(e) => last = format!("{a}: {e}"),
        }
    }
    Err(last)
}

fn tcp(host: &str, port: u16, limit: Duration) -> Verdict {
    match connect(host, port, limit) {
        Ok(_) => Verdict::Pass,
        Err(e) => Verdict::Fail(e),
    }
}

/// A response's status line and headers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    /// Lower-case names, in order.
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Parses `HTTP/1.1 200 OK\r\nname: value\r\n...` up to the blank line.
pub fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Some(Head { status, headers })
}

/// Judges a head as the agent does: the status (any under 400 when none is named), each header
/// by its start, and a redirect's location.
pub fn judge(m: &Monitor, head: &Head) -> Verdict {
    match m.expect.status {
        Some(want) if head.status != want => return Verdict::Fail(format!("answered {}, wanted {want}", head.status)),
        None if head.status >= 400 => return Verdict::Fail(format!("answered {}", head.status)),
        _ => {}
    }
    for (k, want) in &m.expect.headers {
        match head.get(k) {
            Some(v) if v.starts_with(want.as_str()) => {}
            Some(v) => return Verdict::Fail(format!("{k} is {v:?}, wanted {want:?}")),
            None => return Verdict::Fail(format!("no {k} header")),
        }
    }
    let loc = head.get("location").unwrap_or_default();
    if let Some(want) = &m.expect.location
        && loc != want
    {
        return Verdict::Fail(format!("location is {loc:?}, wanted {want:?}"));
    }
    if let Some(want) = &m.expect.location_prefix
        && !loc.starts_with(want.as_str())
    {
        return Verdict::Fail(format!("location is {loc:?}, wanted it to start with {want:?}"));
    }
    if m.surface == Surface::Sse {
        if !(200..300).contains(&head.status) {
            return Verdict::Fail(format!("answered {}, wanted 2xx", head.status));
        }
        let ct = head.get("content-type").unwrap_or_default();
        if !ct.starts_with("text/event-stream") {
            return Verdict::Fail(format!("content-type is {ct:?}, not an event stream"));
        }
    }
    Verdict::Pass
}

fn http(m: &Monitor, u: &url::Url, start: Instant, shapes: Option<&dyn Shapes>) -> (Verdict, Option<u16>) {
    let limit = timeout(m);
    let (Some(host), Some(port)) = (u.host_str(), u.port_or_known_default()) else {
        return (Verdict::Fail("the URL has no host".into()), None);
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let mut s = match connect(host, port, limit) {
        Ok(s) => s,
        Err(e) => return (Verdict::Fail(e), None),
    };
    let _ = s.set_read_timeout(Some(limit));
    let _ = s.set_write_timeout(Some(limit));
    let path = &u[url::Position::BeforePath..url::Position::AfterQuery];
    let method = m.method.as_deref().unwrap_or("GET");
    let authority = &u[url::Position::BeforeHost..url::Position::AfterPort];
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: iohr-monitors\r\nConnection: close\r\n");
    let accept = if m.surface == Surface::Sse { "text/event-stream" } else { "*/*" };
    if !m.headers.contains_key("accept") {
        req.push_str(&format!("Accept: {accept}\r\n"));
    }
    for (k, v) in &m.headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = &m.body {
        if !m.headers.contains_key("content-type") {
            req.push_str("Content-Type: application/json\r\n");
        }
        req.push_str(&format!("Content-Length: {}\r\n\r\n{b}", b.len()));
    } else {
        req.push_str("\r\n");
    }
    if let Err(e) = s.write_all(req.as_bytes()) {
        return (Verdict::Fail(format!("send: {e}")), None);
    }
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return (Verdict::Fail("the head is larger than 64 KiB".into()), None);
        }
        match s.read(&mut chunk) {
            Ok(0) => return (Verdict::Fail("closed before it answered".into()), None),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return (Verdict::Fail(format!("no answer: {e}")), None),
        }
    };
    let Some(head) = parse_head(&String::from_utf8_lossy(&buf[..end])) else {
        return (Verdict::Fail("not an HTTP answer".into()), None);
    };
    let status = Some(head.status);
    let v = judge(m, &head);
    let shape = m.expect.shape.as_ref().zip(shapes);
    let wanted = if m.surface == Surface::Sse { m.events.min(MAX_EVENTS) } else { 0 };
    if v != Verdict::Pass || (shape.is_none() && wanted == 0) || (m.surface == Surface::Sse && wanted == 0) {
        return (v, status);
    }
    let code = head.status;
    let mut body = Body::new(&head);
    let mut read = |body: &mut Body| -> Result<bool, String> {
        if start.elapsed() > limit {
            return Ok(false);
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => Ok(false),
            Ok(n) => body.push(&chunk[..n]).map(|()| true),
        }
    };
    if let Err(e) = body.push(&buf[end..]) {
        return (Verdict::Fail(format!("shape {code} /: {e}")), status);
    }

    if m.surface == Surface::Http {
        // http with a shape: the whole body, at most 1 MiB, then checked and dropped.
        let Some((Shape::One(path), shapes)) = shape else { return (v, status) };
        while !body.done() {
            match read(&mut body) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => return (Verdict::Fail(format!("shape {code} /: {e}")), status),
            }
        }
        if !body.done() && body.length.is_some() {
            return (Verdict::Fail(format!("shape {code} /: the body ended early")), status);
        }
        let verdict = match check_body(shapes, path, &body.out) {
            Ok(()) => Verdict::Pass,
            Err(why) => Verdict::Fail(format!("shape {code} {why}")),
        };
        return (verdict, status);
    }

    // sse: read events (a block of lines with a data: line, ended by a blank line; comments do
    // not count) until `events` have come, checking each one's data against its schema.
    let mut seen = 0u32;
    loop {
        let (events, used) = take_events(&body.out);
        body.out.drain(..used);
        for e in events {
            seen += 1;
            if let Some((shape, shapes)) = shape {
                let path = match shape {
                    Shape::One(p) => Some(p),
                    Shape::Events(t) => t.get(&e.name),
                };
                let Some(path) = path else {
                    return (Verdict::Fail(format!("shape {code} event {}: expected a named event, found another event", seen)), status);
                };
                if let Err(why) = check_body(shapes, path, e.data.as_bytes()) {
                    return (Verdict::Fail(format!("shape {code} event {seen} ({}) {why}", e.name)), status);
                }
            }
            if seen >= wanted {
                return (Verdict::Pass, status);
            }
        }
        match read(&mut body) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => return (Verdict::Fail(format!("shape {code} /: {e}")), status),
        }
    }
    (Verdict::Fail(format!("{seen} data events, wanted {wanted}")), status)
}

/// A response body as it comes in: chunked framing undone, capped at [`MAX_BODY`].
struct Body {
    chunked: bool,
    length: Option<usize>,
    raw: Vec<u8>,
    out: Vec<u8>,
    total: usize,
    ended: bool,
}

impl Body {
    fn new(head: &Head) -> Body {
        let chunked = head.get("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked"));
        let length = if chunked { None } else { head.get("content-length").and_then(|l| l.parse().ok()) };
        Body { chunked, length, raw: Vec::new(), out: Vec::new(), total: 0, ended: false }
    }

    fn done(&self) -> bool {
        self.ended || self.length.is_some_and(|l| self.total >= l)
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        if !self.chunked {
            self.total += bytes.len();
            self.out.extend_from_slice(bytes);
        } else {
            self.raw.extend_from_slice(bytes);
            while let Some(i) = self.raw.windows(2).position(|w| w == b"\r\n") {
                let line = std::str::from_utf8(&self.raw[..i]).unwrap_or_default();
                let size = usize::from_str_radix(line.split(';').next().unwrap_or_default().trim(), 16).map_err(|_| "expected a chunk size, found something else".to_owned())?;
                if size == 0 {
                    self.ended = true;
                    self.raw.clear();
                    break;
                }
                if size > MAX_BODY {
                    return Err("expected at most 1 MiB, found more".into());
                }
                if self.raw.len() < i + 2 + size + 2 {
                    break;
                }
                self.out.extend_from_slice(&self.raw[i + 2..i + 2 + size]);
                self.total += size;
                self.raw.drain(..i + 2 + size + 2);
            }
        }
        if self.total > MAX_BODY {
            return Err("expected at most 1 MiB, found more".into());
        }
        Ok(())
    }
}

/// One server-sent event: its name (`message` when it names none) and its data lines joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub data: String,
}

/// The complete data events at the start of `bytes`, and how many bytes they took.
pub fn take_events(bytes: &[u8]) -> (Vec<Event>, usize) {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    let (mut name, mut data, mut has_data) = (String::new(), String::new(), false);
    let mut used = 0;
    let mut pos = 0;
    for raw in text.split_inclusive('\n') {
        pos += raw.len();
        if !raw.ends_with('\n') {
            break;
        }
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        if line.is_empty() {
            if has_data {
                out.push(Event { name: if name.is_empty() { "message".into() } else { std::mem::take(&mut name) }, data: std::mem::take(&mut data) });
            }
            name.clear();
            data.clear();
            has_data = false;
            used = pos;
        } else if let Some(v) = line.strip_prefix("data:").or((line == "data").then_some("")) {
            if has_data {
                data.push('\n');
            }
            data.push_str(v.strip_prefix(' ').unwrap_or(v));
            has_data = true;
        } else if let Some(v) = line.strip_prefix("event:") {
            name = v.trim().to_owned();
        }
    }
    (out, used)
}

/// Data events in an event stream's text so far.
pub fn count_events(text: &str) -> u32 {
    u32::try_from(take_events(text.as_bytes()).0.len()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use super::{File, check_files, parse};
    use std::net::TcpListener;
    use std::thread;

    /// A server that answers each connection with `answer`, `n` times.
    fn serve(answer: &'static str, n: usize) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for s in l.incoming().take(n) {
                let mut s = s.unwrap();
                let mut b = [0u8; 2048];
                let _ = s.read(&mut b);
                let _ = s.write_all(answer.as_bytes());
            }
        });
        port
    }

    fn monitor(toml: &str) -> Monitor {
        let r = check_files(&[File { path: "t.toml".into(), value: parse(toml).unwrap() }]);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        r.monitors.into_iter().next().unwrap()
    }

    #[test]
    fn http_judges_status_and_content_type_and_catches_the_html_fallback() {
        let json = serve("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}", 1);
        let html = serve("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\n\r\n<html>", 1);
        let gone = serve("HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\n\r\n{}", 1);
        let m = |port: u16| {
            monitor(&format!(
                "[[check]]\nname = \"p\"\ntarget = \"http://127.0.0.1:{port}/api/health/store\"\nevery = \"60s\"\nexpect = {{ status = 200, headers = {{ content-type = \"application/json\" }} }}\n"
            ))
        };
        assert_eq!(run(&m(json)).verdict, Verdict::Pass);
        let r = run(&m(html));
        assert_eq!(r.status, Some(200));
        assert!(matches!(&r.verdict, Verdict::Fail(w) if w.contains("text/html")), "{r:?}");
        assert!(matches!(run(&m(gone)).verdict, Verdict::Fail(w) if w.contains("503")));
    }

    #[test]
    fn a_refusal_with_an_answer_passes_on_that_answer() {
        let port = serve("HTTP/1.1 400 Bad Request\r\ncontent-type: text/plain\r\n\r\nupgrade", 1);
        let m = monitor(&format!("[[refuse]]\nname = \"ws-route\"\ntarget = \"http://127.0.0.1:{port}/api/trends/live\"\nevery = \"15m\"\nexpect = {{ status = 400 }}\n"));
        assert_eq!(run(&m).verdict, Verdict::Pass);
        let by = monitor("[[refuse]]\nname = \"meta\"\ntarget = \"http://169.254.169.254/\"\nevery = \"1h\"\nby = \"policy\"\n");
        assert!(matches!(run(&by).verdict, Verdict::Skip(_)));
    }

    #[test]
    fn sse_needs_an_event_stream_and_counts_data_events() {
        let port = serve("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n: hi\n\ndata: {\"a\":1}\n\ndata: 2\n\n", 2);
        let m = |events: u32| monitor(&format!("[[check]]\nname = \"s\"\nsurface = \"sse\"\ntarget = \"http://127.0.0.1:{port}/api/stream\"\nevery = \"5m\"\nevents = {events}\n"));
        assert_eq!(run(&m(2)).verdict, Verdict::Pass);
        assert!(matches!(run(&m(3)).verdict, Verdict::Fail(w) if w.contains("2 data events")));
        assert_eq!(count_events(": comment\n\ndata: x\r\n\r\n"), 1);
    }

    struct Inline;
    impl Shapes for Inline {
        fn validator(&self, shape: &str) -> Result<std::sync::Arc<jsonschema::Validator>, String> {
            let schema = match shape {
                "schemas/api/items/v0.json" => serde_json::json!({ "type": "object", "required": ["items"], "properties": { "items": { "type": "array", "items": { "type": "object", "required": ["score"], "properties": { "score": { "type": "integer" } } } } } }),
                "schemas/api/change/v0.json" => serde_json::json!({ "type": "object", "required": ["topic"], "properties": { "topic": { "type": "string" } } }),
                _ => return Err(format!("no schema {shape}")),
            };
            jsonschema::validator_for(&schema).map(std::sync::Arc::new).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn a_shape_check_reads_the_body_chunked_or_not_and_names_only_the_first_mismatch() {
        let good = serve("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 23\r\n\r\n{\"items\":[{\"score\":1}]}", 2);
        // Chunked, split mid-value, the fourth item's score a string holding a secret.
        let bad = serve(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n1b\r\n{\"items\":[{\"score\":1},{\"sco\r\n2f\r\nre\":2},{\"score\":3},{\"score\":\"hunter2-secret\"}]}\r\n0\r\n\r\n",
            1,
        );
        let m = |port: u16| {
            monitor(&format!(
                "[[check]]\nname = \"eye-items\"\ntarget = \"http://127.0.0.1:{port}/api/items\"\nevery = \"5m\"\nexpect = {{ status = 200, shape = \"schemas/api/items/v0.json\" }}\n"
            ))
        };
        assert_eq!(run_with(&m(good), Some(&Inline), None).verdict, Verdict::Pass);
        assert_eq!(run(&m(good)).verdict, Verdict::Pass, "no Shapes: head only");
        let r = run_with(&m(bad), Some(&Inline), None);
        assert_eq!(r.line(), "FAIL shape eye-items 200 /items/3/score: expected integer, found string");
        assert!(!format!("{r:?}").contains("hunter2"));
    }

    #[test]
    fn an_sse_shape_checks_each_event_against_its_named_schema() {
        let port = serve(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n: live\n\nevent: change\ndata: {\"topic\":\"learning\"}\n\nevent: change\ndata: {\"topic\":7}\n\n",
            3,
        );
        let m = |events: u32, shape: &str| {
            monitor(&format!(
                "[[check]]\nname = \"eye-stream\"\nsurface = \"sse\"\ntarget = \"http://127.0.0.1:{port}/api/stream\"\nevery = \"5m\"\nevents = {events}\nexpect = {{ status = 200, shape = {shape} }}\n"
            ))
        };
        let table = "{ change = \"schemas/api/change/v0.json\" }";
        assert_eq!(run_with(&m(1, table), Some(&Inline), None).verdict, Verdict::Pass);
        let r = run_with(&m(2, table), Some(&Inline), None);
        assert_eq!(r.line(), "FAIL shape eye-stream 200 event 2 (change) /topic: expected string, found integer");
        let r = run_with(&m(1, "{ item = \"schemas/api/items/v0.json\" }"), Some(&Inline), None);
        assert!(matches!(&r.verdict, Verdict::Fail(w) if w.contains("expected a named event")), "{r:?}");
    }

    #[test]
    fn events_keep_their_names_and_wait_for_the_blank_line() {
        let (ev, used) = take_events(b"event: item\ndata: {\"a\":\ndata: 1}\n\ndata: half");
        assert_eq!(ev, vec![Event { name: "item".into(), data: "{\"a\":\n1}".into() }]);
        assert_eq!(used, 34);
    }

    #[test]
    fn tcp_passes_on_connect_and_fails_on_refuse() {
        let open = serve("", 1);
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let m = |port: u16| monitor(&format!("[[check]]\nname = \"t\"\ntarget = \"127.0.0.1:{port}\"\nevery = \"5m\"\n"));
        assert_eq!(run(&m(open)).verdict, Verdict::Pass);
        assert!(matches!(run(&m(closed)).verdict, Verdict::Fail(_)));
        let tls = monitor("[[check]]\nname = \"c\"\nsurface = \"tls\"\ntarget = \"example.com\"\nevery = \"1d\"\n");
        assert!(matches!(run(&tls).verdict, Verdict::Skip(_)));
    }
}
