//! Observability on the standard library alone: JSON logs, spans that follow the W3C
//! `traceparent` header, and request counters in the Prometheus text format.
//!
//! Nothing here records a header, a query string or an environment value: a log line and a span
//! name the route that matched, the status, the duration and the trace id.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A value in a log record.
pub enum Val<'a> {
    /// Text, escaped on the way out.
    Str(&'a str),
    /// A whole number.
    Int(u64),
    /// A decimal number.
    Float(f64),
}

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One JSON object per line to a sink.
pub struct Logger {
    sink: Mutex<Box<dyn Write + Send>>,
}

impl Logger {
    /// A logger writing to `sink`.
    pub fn new(sink: impl Write + Send + 'static) -> Self {
        Self {
            sink: Mutex::new(Box::new(sink)),
        }
    }

    /// One record at `level` with `fields`.
    pub fn log(&self, level: &str, msg: &str, fields: &[(&str, Val)]) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let mut line = format!("{{\"ts_unix_ms\":{ts},\"level\":\"{level}\",\"msg\":");
        escape(msg, &mut line);
        for (k, v) in fields {
            line.push(',');
            escape(k, &mut line);
            line.push(':');
            match v {
                Val::Str(s) => escape(s, &mut line),
                Val::Int(n) => line.push_str(&n.to_string()),
                Val::Float(f) => line.push_str(&format!("{f:.3}")),
            }
        }
        line.push_str("}\n");
        if let Ok(mut sink) = self.sink.lock() {
            let _ = sink.write_all(line.as_bytes());
            let _ = sink.flush();
        }
    }

    /// An INFO record.
    pub fn info(&self, msg: &str, fields: &[(&str, Val)]) {
        self.log("INFO", msg, fields);
    }

    /// An ERROR record.
    pub fn error(&self, msg: &str, fields: &[(&str, Val)]) {
        self.log("ERROR", msg, fields);
    }
}

/// `n` random bytes as hex. The standard library seeds `RandomState` from the operating
/// system: unpredictable enough for trace ids, which are identifiers and not secrets.
fn random_hex(n: usize) -> String {
    let mut out = String::with_capacity(n * 2);
    while out.len() < n * 2 {
        let word = RandomState::new().build_hasher().finish();
        out.push_str(&format!("{word:016x}"));
    }
    out.truncate(n * 2);
    out
}

fn hex_of(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && s.bytes().any(|b| b != b'0')
}

/// One request's trace: the id it joined (or started) and its own id.
pub struct Span {
    /// The trace this request belongs to.
    pub trace_id: String,
    /// This request's own span.
    pub span_id: String,
    /// The caller's span, when a valid `traceparent` came in.
    pub parent_id: Option<String>,
    started: Instant,
}

impl Span {
    /// Joins the trace of a valid traceparent header, or starts a new one.
    pub fn start(traceparent: Option<&str>) -> Self {
        let span_id = random_hex(8);
        let started = Instant::now();
        if let Some(h) = traceparent {
            let p: Vec<&str> = h.split('-').collect();
            if p.len() == 4
                && p[0] == "00"
                && hex_of(p[1], 32)
                && hex_of(p[2], 16)
                && p[3].len() == 2
            {
                return Self {
                    trace_id: p[1].to_owned(),
                    span_id,
                    parent_id: Some(p[2].to_owned()),
                    started,
                };
            }
        }
        Self {
            trace_id: random_hex(16),
            span_id,
            parent_id: None,
            started,
        }
    }

    /// The traceparent to send back (and on to callers): this span is the parent.
    pub fn header(&self) -> String {
        format!("00-{}-{}-01", self.trace_id, self.span_id)
    }

    /// Milliseconds since the span started.
    pub fn millis(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }
}

/// Request counts by route and status. Routes are matched patterns, never raw paths, so the
/// label set is bounded by the code and holds no user input.
#[derive(Default)]
pub struct Metrics {
    counts: Mutex<BTreeMap<(String, u16), u64>>,
}

impl Metrics {
    /// Counts one request.
    pub fn inc(&self, route: &str, status: u16) {
        if let Ok(mut c) = self.counts.lock() {
            *c.entry((route.to_owned(), status)).or_insert(0) += 1;
        }
    }

    /// The counters in the Prometheus text format.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# HELP http_requests_total Requests served, by route and status.\n# TYPE http_requests_total counter\n",
        );
        if let Ok(c) = self.counts.lock() {
            for ((route, status), n) in c.iter() {
                out.push_str(&format!(
                    "http_requests_total{{route=\"{route}\",status=\"{status}\"}} {n}\n"
                ));
            }
        }
        out
    }
}
