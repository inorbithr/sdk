//! Streams: an operation that answers `text/event-stream`, read as server-sent events
//! or over the `/v1/ws` socket (`docs/design.md` section 7).

use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

use crate::error::{ApiError, Error, Headers, RawResponse};

/// Items a stream holds before its reader waits for the caller (design.md section 7).
pub(crate) const QUEUE: usize = 64;
/// The largest event a stream reads, 1 MiB.
pub(crate) const MAX_EVENT: usize = 1024 * 1024;

/// How a client opens its streams.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Streams {
    /// Server-sent events: one `GET` per stream (the default).
    #[default]
    Sse,
    /// Every stream of the client over one multiplexed `/v1/ws` connection.
    Socket,
}

/// One item on its way from a reader to the caller, still JSON.
#[derive(Debug)]
pub(crate) enum Item {
    /// A server-sent event's data.
    Text(String),
    /// A socket `data` frame's body.
    Value(serde_json::Value),
}

/// What stops the reader when the caller drops the stream.
pub(crate) enum Guard {
    /// A server-sent events reader task, aborted (its connection closes with it).
    Task(tokio::task::AbortHandle),
    /// A call on the socket, cancelled with a `cancel` frame.
    Call(Box<dyn FnMut() + Send>),
}

impl Drop for Guard {
    fn drop(&mut self) {
        match self {
            Self::Task(handle) => handle.abort(),
            Self::Call(cancel) => cancel(),
        }
    }
}

/// The events of a stream, in order, from a generated stream method.
///
/// It is a [`futures_core::Stream`] of `Result<T, Error>`, and has an inherent
/// [`next`](Self::next) for a plain loop. It ends cleanly when the server ends the
/// stream, or after one error: an `error` event or frame (an [`Error::Api`]), the
/// connection failing, or a silence past the client's stream idle timeout
/// ([`Error::Timeout`]). Dropping it stops the stream: the connection closes, or the
/// call on the socket is cancelled. A reader holds at most 64 events the caller has
/// not taken yet, then waits.
///
/// ```no_run
/// # async fn read(mut events: inorbithr::EventStream<serde_json::Value>) -> Result<(), inorbithr::Error> {
/// while let Some(event) = events.next().await {
///     let event = event?;
///     println!("{event}");
/// }
/// # Ok(())
/// # }
/// ```
pub struct EventStream<T> {
    rx: mpsc::Receiver<Result<Item, Error>>,
    _guard: Guard,
    done: bool,
    _item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for EventStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventStream")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl<T: DeserializeOwned> EventStream<T> {
    pub(crate) fn new(rx: mpsc::Receiver<Result<Item, Error>>, guard: Guard) -> Self {
        Self {
            rx,
            _guard: guard,
            done: false,
            _item: PhantomData,
        }
    }

    /// The next event; `None` once the stream has ended, cleanly or after an error.
    ///
    /// # Errors
    ///
    /// The error the stream ended with; nothing follows it.
    #[allow(clippy::should_implement_trait)] // an async `next`, not `Iterator::next`
    pub async fn next(&mut self) -> Option<Result<T, Error>> {
        if self.done {
            return None;
        }
        let item = self.rx.recv().await;
        self.take(item)
    }

    fn take(&mut self, item: Option<Result<Item, Error>>) -> Option<Result<T, Error>> {
        let out = match item? {
            Ok(Item::Text(text)) => serde_json::from_str(&text).map_err(|e| decode(&e, text)),
            Ok(Item::Value(value)) => {
                let text = value.to_string();
                serde_json::from_value(value).map_err(|e| decode(&e, text))
            }
            Err(e) => Err(e),
        };
        if out.is_err() {
            self.done = true;
            self.rx.close();
        }
        Some(out)
    }
}

impl<T: DeserializeOwned> futures_core::Stream for EventStream<T> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        match this.rx.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(item) => Poll::Ready(this.take(item)),
        }
    }
}

fn decode(e: &serde_json::Error, text: String) -> Error {
    Error::Decode {
        reason: e.to_string(),
        raw: Box::new(RawResponse::part(200, text.into_bytes(), String::new())),
    }
}

/// The error an `error` event or frame carries: the envelope, its status from the code.
pub(crate) fn envelope_error(body: Vec<u8>, request_id: &str) -> Error {
    #[derive(serde::Deserialize)]
    struct Code {
        #[serde(default)]
        code: String,
    }
    let code = serde_json::from_slice::<Code>(&body)
        .map(|c| c.code)
        .unwrap_or_default();
    let status = crate::Code::from(code.as_str())
        .http_status()
        .unwrap_or(500);
    ApiError::parse(RawResponse::part(status, body, request_id.to_owned())).into()
}

/// The WHATWG event-stream parser: bytes in, events out, bounded.
#[derive(Debug, Default)]
pub(crate) struct Parser {
    line: Vec<u8>,
    data: String,
    has_data: bool,
    event: String,
    after_cr: bool,
}

/// One dispatched event: its name (empty for the default) and its data.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Event {
    pub(crate) name: String,
    pub(crate) data: String,
}

impl Parser {
    /// Feeds `bytes`, handing every complete event to `out`.
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] when a line or an event's data passes [`MAX_EVENT`].
    pub(crate) fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) -> Result<(), Error> {
        for &b in bytes {
            match b {
                b'\n' if self.after_cr => self.after_cr = false,
                b'\n' | b'\r' => {
                    self.after_cr = b == b'\r';
                    let line = std::mem::take(&mut self.line);
                    self.line(&line, out)?;
                }
                _ => {
                    self.after_cr = false;
                    if self.line.len() >= MAX_EVENT {
                        return Err(Error::TooLarge);
                    }
                    self.line.push(b);
                }
            }
        }
        Ok(())
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<Event>) -> Result<(), Error> {
        if line.is_empty() {
            if self.has_data {
                out.push(Event {
                    name: std::mem::take(&mut self.event),
                    data: std::mem::take(&mut self.data),
                });
            }
            self.has_data = false;
            self.data.clear();
            self.event.clear();
            return Ok(());
        }
        if line[0] == b':' {
            return Ok(());
        }
        let text = String::from_utf8_lossy(line);
        let (field, value) = match text.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (text.as_ref(), ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
                if self.data.len() > MAX_EVENT {
                    return Err(Error::TooLarge);
                }
            }
            "event" => value.clone_into(&mut self.event),
            _ => {}
        }
        Ok(())
    }
}

/// Reads a server-sent events body into `tx` until it ends, fails or goes silent for
/// `idle`.
pub(crate) async fn read_sse(
    mut resp: reqwest::Response,
    tx: mpsc::Sender<Result<Item, Error>>,
    idle: Duration,
    host: String,
    request_id: String,
) {
    let mut parser = Parser::default();
    let mut events = Vec::new();
    loop {
        let chunk = match tokio::time::timeout(idle, resp.chunk()).await {
            Err(_) => {
                let _ = tx
                    .send(Err(Error::Timeout {
                        host,
                        secs: idle.as_secs().max(1),
                    }))
                    .await;
                return;
            }
            Ok(Err(e)) => {
                let _ = tx
                    .send(Err(Error::Connection {
                        host,
                        reason: crate::auth::transport_reason(&e),
                    }))
                    .await;
                return;
            }
            Ok(Ok(None)) => return,
            Ok(Ok(Some(chunk))) => chunk,
        };
        if let Err(e) = parser.feed(&chunk, &mut events) {
            let _ = tx.send(Err(e)).await;
            return;
        }
        for event in events.drain(..) {
            let item = if event.name == "error" {
                Err(envelope_error(event.data.into_bytes(), &request_id))
            } else {
                Ok(Item::Text(event.data))
            };
            let last = item.is_err();
            if tx.send(item).await.is_err() || last {
                return;
            }
        }
    }
}

/// Headers of a stream that opened, for the hooks (the body is the stream).
pub(crate) fn opened(resp: &reqwest::Response, request_id: &str, attempts: u32) -> RawResponse {
    let headers = Headers::new(resp.headers().iter().map(|(k, v)| {
        (
            k.as_str().to_owned(),
            v.to_str().unwrap_or_default().to_owned(),
        )
    }));
    let mut raw = RawResponse::part(resp.status().as_u16(), Vec::new(), request_id.to_owned());
    raw.server_request_id = headers.get("x-request-id").map(str::to_owned);
    raw.headers = headers;
    raw.attempts = attempts;
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&[u8]]) -> Vec<Event> {
        let mut p = Parser::default();
        let mut out = Vec::new();
        for c in chunks {
            p.feed(c, &mut out).unwrap();
        }
        out
    }

    #[test]
    fn data_lines_join_and_comments_and_other_fields_are_skipped() {
        let got = parse(&[b": open\n\nid: 7\nretry: 9\nfoo: x\ndata: {\"a\":\ndata:  1}\n\n"]);
        assert_eq!(
            got,
            [Event {
                name: String::new(),
                data: "{\"a\":\n 1}".into()
            }]
        );
    }

    #[test]
    fn crlf_and_cr_end_lines_even_across_chunks() {
        let got = parse(&[b"event: error\r", b"\ndata: x\r\n\r", b"\ndata: y\r\r"]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "error");
        assert_eq!(got[0].data, "x");
        assert_eq!(got[1].name, "");
        assert_eq!(got[1].data, "y");
    }

    #[test]
    fn an_event_over_the_bound_is_too_large() {
        let mut p = Parser::default();
        let big = vec![b'a'; MAX_EVENT + 1];
        assert!(matches!(
            p.feed(&big, &mut Vec::new()),
            Err(Error::TooLarge)
        ));
    }

    #[test]
    fn an_error_envelope_takes_its_status_from_its_code() {
        let e = envelope_error(
            br#"{"code":"forbidden","error":"no","details":[]}"#.to_vec(),
            "r",
        );
        let Error::Api(api) = e else { panic!() };
        assert_eq!(api.status, 403);
        assert_eq!(api.code, crate::Code::Forbidden);
    }
}
