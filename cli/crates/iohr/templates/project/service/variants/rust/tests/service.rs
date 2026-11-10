//! The passes-on-init test: boots the service on a free port and checks what a service owes.

#![allow(clippy::unwrap_used, reason = "a test fails by panicking")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use @@MODULE@@::obs::Logger;
use @@MODULE@@::{Service, serve};

/// A log sink the server writes to while the test reads it.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

struct Booted {
    addr: String,
    logs: Sink,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Booted {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let logs = Sink::default();
        let svc = Arc::new(Service::new(Logger::new(logs.clone())));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (Arc::clone(&svc), Arc::clone(&stop));
        let server = thread::spawn(move || serve(&listener, &s, &st).unwrap());
        for _ in 0..200 {
            if svc.ready.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        Self {
            addr,
            logs,
            stop,
            server: Some(server),
        }
    }

    /// One request: (status, response head, body).
    fn get(&self, target: &str, headers: &[(&str, &str)]) -> (u16, String, String) {
        let mut s = std::net::TcpStream::connect(&self.addr).unwrap();
        let mut req = format!("GET {target} HTTP/1.1\r\nHost: test\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        s.write_all(format!("{req}\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap();
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, head.to_owned(), body.to_owned())
    }
}

impl Drop for Booted {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.server.take() {
            h.join().unwrap();
        }
    }
}

fn field(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let i = line.find(&pat)? + pat.len();
    Some(line[i..i + line[i..].find('"')?].to_owned())
}

#[test]
fn probes_answer() {
    let s = Booted::start();
    assert_eq!(s.get("/healthz", &[]).0, 200);
    assert_eq!(s.get("/readyz", &[]).0, 200);
    let (code, _, body) = s.get("/v1/hello?name=Ada", &[]);
    assert_eq!((code, body.trim()), (200, "{\"message\":\"Hello, Ada\"}"));
    assert_eq!(s.get("/nope", &[]).0, 404);
}

#[test]
fn one_span_with_the_route_and_the_trace_id_in_the_log() {
    let s = Booted::start();
    let trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    let tp = format!("00-{trace}-00f067aa0ba902b7-01");
    let (_, head, _) = s.get(
        "/v1/hello?name=secret-query-value",
        &[
            ("traceparent", &tp),
            ("Authorization", "Bearer secret-header-value"),
        ],
    );
    assert!(
        head.contains(&format!("traceparent: 00-{trace}-")),
        "{head}"
    );
    let log = s.logs.text();
    let spans: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("\"msg\":\"span\""))
        .collect();
    let requests: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("\"msg\":\"request\""))
        .collect();
    assert_eq!((spans.len(), requests.len()), (1, 1), "{log}");
    assert_eq!(field(spans[0], "route").as_deref(), Some("/v1/hello"));
    assert_eq!(field(spans[0], "trace_id").as_deref(), Some(trace));
    assert_eq!(
        field(spans[0], "parent_span_id").as_deref(),
        Some("00f067aa0ba902b7")
    );
    assert_eq!(field(requests[0], "trace_id").as_deref(), Some(trace));
    for secret in ["secret-query-value", "secret-header-value"] {
        assert!(!log.contains(secret), "the log holds {secret}");
    }
    // No value of the environment is in the log.
    for (_, v) in std::env::vars() {
        if v.len() >= 12 {
            assert!(!log.contains(&v), "the log holds an environment value");
        }
    }
}

#[test]
fn an_unknown_path_is_never_logged_and_metrics_count_by_route() {
    let s = Booted::start();
    s.get("/private/user-1234", &[]);
    s.get("/healthz", &[]);
    assert!(!s.logs.text().contains("user-1234"));
    let (_, _, metrics) = s.get("/metrics", &[]);
    assert!(
        metrics.contains("route=\"unmatched\",status=\"404\""),
        "{metrics}"
    );
    assert!(
        metrics.contains("route=\"/healthz\",status=\"200\""),
        "{metrics}"
    );
}
