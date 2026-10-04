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
use tokio_tungstenite::tungstenite::{self, Message, client::IntoClientRequest as _};
use url::Url;

use crate::auth::DynProvider;
use crate::error::{ApiError, ConfigError, Error, Headers, RawResponse};
use crate::retry::{backoff, request_id, retry_after, retryable_status};
use crate::stream::{EventStream, Guard, Item, QUEUE, envelope_error};

/// The socket's path.
const PATH: &str = "/v1/ws";
/// The largest frame the client sends, as the platform's `x-iohr-limits`.
const MAX_SEND: usize = 256 * 1024;

/// What the socket task needs from its client.
pub(crate) struct Ctx {
    pub(crate) base: Url,
    pub(crate) host: String,
    pub(crate) provider: Arc<dyn DynProvider>,
    pub(crate) max_retries: u32,
    pub(crate) idle: Duration,
    pub(crate) timeout: Duration,
    pub(crate) user_agent: String,
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
        let ws = match connect(&ctx).await {
            Ok(ws) => ws,
            Err((error, retry)) => {
                if retry.is_some() && failures < ctx.max_retries {
                    wait = Some(retry.flatten().unwrap_or_else(|| backoff(failures)));
                    failures += 1;
                    continue;
                }
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
                if failures >= ctx.max_retries.max(1) {
                    fail(&hub, &mut calls, &mut commands, closed(&ctx.host), &|| {
                        closed(&ctx.host)
                    });
                    return;
                }
                wait = Some(after.unwrap_or_else(|| backoff(failures)));
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

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

type Ws = WebSocketStream<Box<dyn Io>>;

/// One connection: the error, and `Some(wait)` when another attempt may help.
async fn connect(ctx: &Ctx) -> Result<Ws, (Error, Option<Option<Duration>>)> {
    let mut refreshed = false;
    loop {
        match tokio::time::timeout(ctx.timeout, attempt(ctx)).await {
            Err(_) => {
                return Err((
                    Error::Timeout {
                        host: ctx.host.clone(),
                        secs: ctx.timeout.as_secs(),
                    },
                    Some(None),
                ));
            }
            Ok(Ok(ws)) => return Ok(ws),
            Ok(Err(Upgrade::Unauthorized(raw))) => {
                if refreshed {
                    return Err((ApiError::parse(raw).into(), None));
                }
                ctx.provider.invalidate().await;
                refreshed = true;
            }
            Ok(Err(Upgrade::Refused(raw))) => {
                let wait = retry_after(&raw.headers);
                let retry = retryable_status(raw.status).then_some(wait);
                return Err((ApiError::parse(raw).into(), retry));
            }
            Ok(Err(Upgrade::Failed(e, retry))) => return Err((e, retry.then_some(None))),
        }
    }
}

enum Upgrade {
    Unauthorized(RawResponse),
    Refused(RawResponse),
    Failed(Error, bool),
}

async fn attempt(ctx: &Ctx) -> Result<Ws, Upgrade> {
    let token = ctx
        .provider
        .token()
        .await
        .map_err(|e| Upgrade::Failed(Error::Auth(e), false))?;
    let mut url = ctx.base.clone();
    let secure = url.scheme() == "https";
    let _ = url.set_scheme(if secure { "wss" } else { "ws" });
    url.set_path(PATH);
    let host = url.host_str().unwrap_or_default().to_owned();
    let port = url
        .port_or_known_default()
        .unwrap_or(if secure { 443 } else { 80 });
    let id = request_id();
    let config = |e: String| {
        Upgrade::Failed(
            ConfigError::InvalidUrl {
                what: "base_url",
                reason: e,
            }
            .into(),
            false,
        )
    };
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| config(e.to_string()))?;
    let headers = request.headers_mut();
    for (name, value) in [
        ("authorization", format!("Bearer {}", token.expose())),
        ("user-agent", ctx.user_agent.clone()),
        ("x-request-id", id.clone()),
    ] {
        headers.insert(
            name,
            value
                .parse()
                .map_err(|_| config(format!("{name} is not a header value")))?,
        );
    }
    let failed = |reason: String| {
        Upgrade::Failed(
            Error::Connection {
                host: ctx.host.clone(),
                reason,
            },
            true,
        )
    };
    let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| failed(e.to_string()))?;
    let _ = tcp.set_nodelay(true);
    let io: Box<dyn Io> = if secure {
        tls(&host, tcp).await.map_err(|e| match e {
            Error::Config(c) => Upgrade::Failed(Error::Config(c), false),
            other => failed(other.to_string()),
        })?
    } else {
        Box::new(tcp)
    };
    let mut config = tungstenite::protocol::WebSocketConfig::default();
    config.max_message_size = Some(crate::error::MAX_BODY);
    config.max_frame_size = Some(crate::error::MAX_BODY);
    match tokio_tungstenite::client_async_with_config(request, io, Some(config)).await {
        Ok((ws, _)) => Ok(ws),
        Err(tungstenite::Error::Http(resp)) => {
            let status = resp.status().as_u16();
            let headers = Headers::new(resp.headers().iter().map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    v.to_str().unwrap_or_default().to_owned(),
                )
            }));
            let body = resp.into_body().unwrap_or_default();
            let mut raw = RawResponse::part(status, body, id);
            raw.server_request_id = headers.get("x-request-id").map(str::to_owned);
            raw.headers = headers;
            Err(if status == 401 {
                Upgrade::Unauthorized(raw)
            } else {
                Upgrade::Refused(raw)
            })
        }
        Err(e) => Err(failed(e.to_string())),
    }
}

#[cfg(feature = "rustls")]
async fn tls(host: &str, tcp: tokio::net::TcpStream) -> Result<Box<dyn Io>, Error> {
    use rustls_platform_verifier::BuilderVerifierExt as _;
    let http = |e: String| -> Error { ConfigError::Http(e).into() };
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| http(e.to_string()))?
        .with_platform_verifier()
        .map_err(|e| http(e.to_string()))?
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|e| http(e.to_string()))?;
    let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|e| Error::Connection {
            host: host.to_owned(),
            reason: e.to_string(),
        })?;
    Ok(Box::new(stream))
}

#[cfg(not(feature = "rustls"))]
#[allow(clippy::unused_async)] // the same signature as with rustls
async fn tls(_host: &str, _tcp: tokio::net::TcpStream) -> Result<Box<dyn Io>, Error> {
    Err(ConfigError::Http("TLS needs the `rustls` feature".into()).into())
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
