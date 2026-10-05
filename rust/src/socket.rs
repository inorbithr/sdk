//! The `/v1/ws` socket: every stream of a client over one connection
//! (`docs/design.md` section 7, `spec/frames.json`).
//!
//! One task owns the connection and the table of calls in flight. It opens the socket
//! when the first stream starts and closes it when the last one ends; it reconnects
//! when the server ends the socket (any id-less error but `unauthenticated`), the
//! connection drops, or nothing arrives for the stream idle timeout, and then issues
//! every call that had not ended again. Streams are reads, so a repeated call is safe.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use crate::error::{ApiError, Error, RawResponse};
use crate::middleware::builtins::Shared;
use crate::middleware::{Body, Engine};
use crate::retry::{BACKOFF_BASE, BACKOFF_CAP, backoff};
use crate::stream::{EventStream, Guard, Item, QUEUE, envelope_error};

/// The largest frame the client sends, as the platform's `x-iohr-limits`.
const MAX_SEND: usize = 256 * 1024;

/// What the socket task needs from its client: the upgrade goes through the client's
/// pipeline (`docs/config.md` section 7.12).
pub(crate) struct Ctx {
    pub(crate) base: Url,
    pub(crate) host: String,
    pub(crate) engine: Arc<Engine>,
    pub(crate) shared: Arc<Shared>,
    pub(crate) profile: &'static str,
    pub(crate) max_retries: u32,
    pub(crate) idle: Duration,
}

enum Command {
    Open {
        method: String,
        body: Value,
        out: mpsc::Sender<Result<Item, Error>>,
        ack: oneshot::Sender<Result<u64, Error>>,
    },
    Cancel(u64),
}

struct Call {
    method: String,
    body: Value,
    out: mpsc::Sender<Result<Item, Error>>,
}

/// The client's socket: the sender to its task while one runs.
#[derive(Default)]
pub(crate) struct Hub {
    tx: Mutex<Option<mpsc::UnboundedSender<Command>>>,
}

impl Hub {
    /// The client went away: once its streams end, the task finds no sender left.
    pub(crate) fn close(&self) {
        *self.lock() = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<mpsc::UnboundedSender<Command>>> {
        self.tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts one call on the socket; answers once its `call` frame is on a connected
    /// socket.
    pub(crate) async fn open<T: serde::de::DeserializeOwned>(
        self: &Arc<Self>,
        ctx: &Arc<Ctx>,
        method: &str,
        body: Value,
    ) -> Result<EventStream<T>, Error> {
        let (out, rx) = mpsc::channel(QUEUE);
        let (ack, acked) = oneshot::channel();
        let tx = {
            let mut slot = self.lock();
            let tx = match slot.as_ref() {
                Some(tx) if !tx.is_closed() => tx.clone(),
                _ => {
                    let (tx, commands) = mpsc::unbounded_channel();
                    tokio::spawn(run(Arc::clone(ctx), Arc::clone(self), commands));
                    *slot = Some(tx.clone());
                    tx
                }
            };
            tx.send(Command::Open {
                method: method.to_owned(),
                body,
                out,
                ack,
            })
            .map_err(|_| closed(&ctx.host))?;
            tx
        };
        let id = acked.await.map_err(|_| closed(&ctx.host))??;
        let cancel = move || {
            let _ = tx.send(Command::Cancel(id));
        };
        Ok(EventStream::new(rx, Guard::Call(Box::new(cancel))))
    }
}

fn closed(host: &str) -> Error {
    Error::Connection {
        host: host.to_owned(),
        reason: "the socket closed".into(),
    }
}

/// How one connection ended.
enum Ended {
    /// No call left and no command waiting: the task is done.
    Idle,
    /// Reconnect, after this long.
    Reconnect(Option<Duration>),
    /// Every call failed for good: the key was revoked, or the client went away.
    Over,
}

/// The socket task.
async fn run(ctx: Arc<Ctx>, hub: Arc<Hub>, mut commands: mpsc::UnboundedReceiver<Command>) {
    let mut calls: BTreeMap<u64, Call> = BTreeMap::new();
    let mut next_id: u64 = 0;
    let mut failures: u32 = 0;
    let mut wait: Option<Duration> = None;
    loop {
        if let Some(w) = wait.take() {
            tokio::time::sleep(w).await;
        }
        // Nothing left to serve: every stream was dropped while the socket was down.
        if calls.values().all(|c| c.out.is_closed()) && commands.is_empty() {
            calls.clear();
            let mut slot = hub.lock();
            if commands.is_empty() {
                *slot = None;
                return;
            }
        }
        let ws = match connect(&ctx).await {
            Ok(ws) => ws,
            Err(error) => {
                let copy = clone_error(&error, &ctx.host);
                fail(&hub, &mut calls, &mut commands, error, &|| {
                    clone_error(&copy, &ctx.host)
                });
                return;
            }
        };
        let connected = std::time::Instant::now();
        let ended = serve(
            &ctx,
            &hub,
            ws,
            &mut calls,
            &mut commands,
            &mut next_id,
            &mut failures,
        )
        .await;
        // A connection that lived past the idle timeout was not a failure.
        if connected.elapsed() > ctx.idle {
            failures = 0;
        }
        match ended {
            Ended::Idle | Ended::Over => return,
            Ended::Reconnect(after) => {
                // A reconnect draws from the client's retry budget (section 7.4).
                let cost = if after.is_some() { 5 } else { 10 };
                let paid = ctx.shared.budget.as_ref().is_none_or(|b| b.take(cost));
                if failures >= ctx.max_retries.max(1) || !paid {
                    fail(&hub, &mut calls, &mut commands, closed(&ctx.host), &|| {
                        closed(&ctx.host)
                    });
                    return;
                }
                wait = Some(after.unwrap_or_else(|| backoff(failures, BACKOFF_BASE, BACKOFF_CAP)));
                failures += 1;
            }
        }
    }
}

/// A copy of an error for each further call that fails with it (errors are not
/// `Clone`).
fn clone_error(e: &Error, host: &str) -> Error {
    match e {
        Error::Api(api) => ApiError::parse(api.raw.clone()).into(),
        Error::Timeout { secs, .. } => Error::Timeout {
            host: host.to_owned(),
            secs: *secs,
        },
        Error::Config(c) => Error::Config(c.clone()),
        other => Error::Connection {
            host: host.to_owned(),
            reason: other.to_string(),
        },
    }
}

/// Ends every call and every waiting open, the first with `first` and the others with
/// `more()`, and lets the task go.
fn fail(
    hub: &Hub,
    calls: &mut BTreeMap<u64, Call>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    first: Error,
    more: &dyn Fn() -> Error,
) {
    let mut first = Some(first);
    let mut next = || first.take().unwrap_or_else(more);
    let mut slot = hub.lock();
    *slot = None;
    commands.close();
    while let Ok(cmd) = commands.try_recv() {
        if let Command::Open { ack, .. } = cmd {
            let _ = ack.send(Err(next()));
        }
    }
    drop(slot);
    for (_, call) in std::mem::take(calls) {
        let _ = call.out.try_send(Err(next()));
    }
}

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub(crate) type Ws = WebSocketStream<Box<dyn Io>>;

/// One connection, through the client's pipeline: its request id, user agent, token
/// (one fresh one after a `401`), retries and timeouts apply to the upgrade.
async fn connect(ctx: &Ctx) -> Result<Ws, Error> {
    let req = crate::client::upgrade_request(&ctx.base, ctx.profile);
    let state = std::sync::Arc::clone(&req.info.state);
    let resp = ctx.engine.run(req).await?;
    match resp.body {
        Body::Socket(ws) => Ok(ws),
        Body::Bytes(body) => {
            let mut raw = RawResponse::part(
                resp.status,
                body,
                state.request_id.get().cloned().unwrap_or_default(),
            );
            raw.server_request_id = resp.headers.get("x-request-id").map(str::to_owned);
            raw.headers = resp.headers;
            raw.attempts = state.attempts();
            Err(ApiError::parse(raw).into())
        }
        Body::Stream(_) => Err(Error::Connection {
            host: ctx.host.clone(),
            reason: "the upgrade was answered with a stream".into(),
        }),
    }
}

/// A frame from the server.
#[derive(Deserialize)]
struct Frame {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    code: String,
}

/// Serves one connection until it ends.
#[allow(clippy::too_many_lines)] // one select loop, read top to bottom
async fn serve(
    ctx: &Ctx,
    hub: &Hub,
    ws: Ws,
    calls: &mut BTreeMap<u64, Call>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    next_id: &mut u64,
    failures: &mut u32,
) -> Ended {
    let (mut sink, mut frames) = ws.split();
    // Calls that had not ended when the last connection did go again.
    for (id, call) in calls.iter() {
        if send_call(&mut sink, *id, &call.method, &call.body)
            .await
            .is_err()
        {
            return Ended::Reconnect(None);
        }
    }
    loop {
        tokio::select! {
            cmd = commands.recv() => {
                let Some(cmd) = cmd else {
                    let _ = sink.close().await;
                    return Ended::Over;
                };
                if !command(cmd, &mut sink, calls, next_id).await {
                    return Ended::Reconnect(None);
                }
            }
            frame = tokio::time::timeout(ctx.idle, frames.next()) => {
                let text = match frame {
                    Err(_) => return Ended::Reconnect(Some(Duration::ZERO)),
                    Ok(Some(Ok(Message::Text(text)))) => text,
                    Ok(Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_)))) => continue,
                    Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) => return Ended::Reconnect(None),
                };
                let Ok(frame) = serde_json::from_str::<Frame>(&text) else { continue };
                let id = frame.id.as_deref().and_then(|i| i.parse::<u64>().ok());
                match (frame.kind.as_str(), id) {
                    ("data", Some(id)) => {
                        *failures = 0;
                        if let Some(call) = calls.get(&id) {
                            let body = frame.body.unwrap_or(Value::Null);
                            if call.out.send(Ok(Item::Value(body))).await.is_err() {
                                calls.remove(&id);
                                let _ = send(&mut sink, &json!({"type": "cancel", "id": id.to_string()})).await;
                            }
                        }
                    }
                    ("end", Some(id)) => {
                        calls.remove(&id);
                    }
                    ("error", Some(id)) => {
                        if let Some(call) = calls.remove(&id) {
                            let _ = call.out.send(Err(envelope_error(text.as_bytes().to_vec(), ""))).await;
                        }
                    }
                    ("error", None) if frame.id.is_none() => {
                        if frame.code == "unauthenticated" {
                            let _ = sink.close().await;
                            let bytes = text.as_bytes().to_vec();
                            let first = envelope_error(bytes.clone(), "");
                            fail(hub, calls, commands, first, &|| envelope_error(bytes.clone(), ""));
                            return Ended::Over;
                        }
                        let _ = sink.close().await;
                        let after = serde_json::from_str::<Value>(&text)
                            .ok()
                            .and_then(|v| {
                                v["details"].as_array()?.iter().find_map(|d| {
                                    (d["type"] == "retry").then(|| d["after_seconds"].as_u64()).flatten()
                                })
                            })
                            .map(Duration::from_secs);
                        return Ended::Reconnect(after);
                    }
                    _ => {}
                }
            }
        }
        if calls.is_empty() {
            // The last stream ended: close, unless a command is already waiting.
            let waiting = {
                let mut slot = hub.lock();
                let cmd = commands.try_recv().ok();
                if cmd.is_none() {
                    *slot = None;
                }
                cmd
            };
            if let Some(cmd) = waiting {
                if !command(cmd, &mut sink, calls, next_id).await {
                    return Ended::Reconnect(None);
                }
            } else {
                let _ = sink.close().await;
                return Ended::Idle;
            }
        }
    }
}

type Sink = futures_util::stream::SplitSink<Ws, Message>;

/// Acts on one command; `false` when the connection failed doing so.
async fn command(
    cmd: Command,
    sink: &mut Sink,
    calls: &mut BTreeMap<u64, Call>,
    next_id: &mut u64,
) -> bool {
    match cmd {
        Command::Open {
            method,
            body,
            out,
            ack,
        } => {
            *next_id += 1;
            let id = *next_id;
            let sent = send_call(sink, id, &method, &body).await;
            calls.insert(id, Call { method, body, out });
            let _ = ack.send(Ok(id));
            sent.is_ok()
        }
        Command::Cancel(id) => {
            if calls.remove(&id).is_some() {
                return send(sink, &json!({"type": "cancel", "id": id.to_string()}))
                    .await
                    .is_ok();
            }
            true
        }
    }
}

async fn send_call(sink: &mut Sink, id: u64, method: &str, body: &Value) -> Result<(), ()> {
    send(
        sink,
        &json!({"type": "call", "id": id.to_string(), "method": method, "body": body}),
    )
    .await
}

async fn send(sink: &mut Sink, frame: &Value) -> Result<(), ()> {
    let text = frame.to_string();
    if text.len() > MAX_SEND {
        return Err(());
    }
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}
