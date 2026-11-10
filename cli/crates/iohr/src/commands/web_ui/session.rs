//! Who the browser acts as, and how it gets there (RFC 0217, round 4).
//!
//! The page never holds a credential. `iohr ui serve` signs in with the one flow `iohr
//! login` uses (the system browser, PKCE, the same profile and credential store) and calls
//! the API for the page with that credential; the page sees only what the API answered.
//!
//! Reaching the page takes the launch secret, which `iohr ui open` (or `serve --open`)
//! puts in the first address as `?launch=`. It is one-shot: the first exchange spends it, a
//! second use is refused, and the server makes a fresh one and records it for the next `iohr
//! ui open`. The server compares it in constant time,
//! answers `303` to `/` and sets a separate session cookie (`HttpOnly`, `SameSite=Strict`,
//! one name per port, so two serves on one machine do not overwrite each other), so the
//! secret leaves the address bar and the history. Every answer but that exchange needs the
//! cookie. Cookies are not scoped to a port: the guards that stay are the `Host`,
//! `Sec-Fetch-Site` and `Origin` checks and the `x-iohr-token` header, which only the
//! UI's own script can read from its page. A process of the same operating-system user is
//! outside the threat (it can read the credential store the same way).
//!
//! Routes (all under `/api`, all from the page's own origin with the token header):
//! `GET /api/session` is the state, `POST /api/session/login` starts the sign-in in the
//! background (the person's browser opens, as with `iohr login --web`; the page asks again
//! until it settles), `POST /api/session/logout` ends it as `iohr logout` does, and
//! `/api/proxy/v1/...` forwards one call (a read, a write with a JSON body, or a
//! server-sent event stream) to the API with the signed-in credential (`forward`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use iohr_auth::Kind;

use super::forward::Limits;
use super::{Reply, Request, decoded_path, percent_decode, same_bytes};
use crate::Env;
use crate::cli::{Global, How, Login};
use crate::context::{Session, session};
use crate::error::Error;
use crate::output::Out;

/// How long a loaded credential serves the page's calls before it is read again, so a
/// sign-out made elsewhere is seen within a minute and a Keychain is not asked per call.
const SESSION_TTL: Duration = Duration::from_secs(60);
/// The most query pairs and the longest key or value a forwarded read may carry.
const MAX_PAIRS: usize = 32;
const MAX_PART: usize = 512;

/// Where the browser-side sign-in stands.
enum Phase {
    Idle,
    Waiting,
    Failed(String),
}

/// What the gate decided for one request.
pub(super) enum Entry {
    /// The cookie is the session's: go on.
    Open,
    /// The launch secret: set the cookie and redirect.
    Exchange,
    /// Neither.
    Refused,
}

/// Where `iohr ui open` reads the launch secret: the record the server rewrites when a
/// secret is spent.
struct Record {
    folder: PathBuf,
    url: String,
    root: PathBuf,
}

/// The launch secret, the session cookie it is exchanged for, and the sign-in state.
pub(super) struct Access {
    launch: Mutex<String>,
    record: Mutex<Option<Record>>,
    session: String,
    cookie: String,
    global: Global,
    env: Env,
    phase: Mutex<Phase>,
    loaded: Mutex<Option<(Instant, Arc<Session>)>>,
    limits: Limits,
    streams: Arc<Semaphore>,
    closed: watch::Sender<bool>,
}

impl Access {
    pub(super) fn new(
        global: Global,
        env: Env,
        port: u16,
        limits: Limits,
    ) -> std::io::Result<Self> {
        Ok(Self {
            launch: Mutex::new(super::random_hex()?),
            record: Mutex::new(None),
            session: super::random_hex()?,
            cookie: format!("iohr_ui_{port}"),
            global,
            env,
            phase: Mutex::new(Phase::Idle),
            loaded: Mutex::new(None),
            streams: Arc::new(Semaphore::new(limits.max_streams)),
            closed: watch::channel(false).0,
            limits,
        })
    }

    pub(super) fn limits(&self) -> Limits {
        self.limits
    }

    /// One of the serve's stream slots, or `None` when they are all taken.
    pub(super) fn stream_permit(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.streams).try_acquire_owned().ok()
    }

    /// Fires when the serve stops, so open streams end.
    pub(super) fn closing(&self) -> watch::Receiver<bool> {
        self.closed.subscribe()
    }

    /// Ends every open stream.
    pub(super) fn close(&self) {
        self.closed.send_replace(true);
    }

    /// The secret `iohr ui open` puts in the first address; never printed by `serve`. It
    /// is the current one: a spent secret is replaced.
    pub(super) fn launch(&self) -> String {
        self.launch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Names the record `iohr ui open` reads, so a spent secret's replacement is written
    /// there; `remember` has already written the first one.
    pub(super) fn keep_record(&self, folder: PathBuf, url: String, root: PathBuf) {
        *self.record.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Record { folder, url, root });
    }

    /// `<cookie name>=<session>|<launch secret>`, for the tests' browser.
    #[cfg(test)]
    pub(super) fn cookie_line(&self) -> String {
        format!("{}={}|{}", self.cookie, self.session, self.launch())
    }

    /// Decides one request: a valid session cookie, else the launch secret in the query of a
    /// GET or HEAD. The secret is spent here, in one step with the comparison, so two
    /// requests with it cannot both win; its replacement goes in the record.
    pub(super) fn entry(&self, request: &Request) -> Entry {
        if self.cookie_ok(request) {
            return Entry::Open;
        }
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            return Entry::Refused;
        }
        let asked = request
            .target
            .split_once('?')
            .map(|(_, q)| q.split('#').next().unwrap_or_default())
            .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("launch=")));
        let Some(asked) = asked else {
            return Entry::Refused;
        };
        let next = {
            let mut held = self.launch.lock().unwrap_or_else(PoisonError::into_inner);
            if !same_bytes(asked.as_bytes(), held.as_bytes()) {
                return Entry::Refused;
            }
            let Ok(next) = super::random_hex() else {
                return Entry::Refused;
            };
            held.clone_from(&next);
            next
        };
        if let Some(r) = &*self.record.lock().unwrap_or_else(PoisonError::into_inner) {
            // A failed write leaves `iohr ui open` with the spent secret; it then says to
            // restart the serve, which is the safe side of the failure.
            let _ = super::remember(&r.folder, &r.url, &r.root, &next);
        }
        Entry::Exchange
    }

    fn cookie_ok(&self, request: &Request) -> bool {
        let prefix = format!("{}=", self.cookie);
        request.cookie.as_deref().is_some_and(|all| {
            all.split(';').any(|c| {
                c.trim()
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|v| same_bytes(v.as_bytes(), self.session.as_bytes()))
            })
        })
    }

    /// `303` to `/` with the session cookie: the secret leaves the address.
    pub(super) fn exchange(&self) -> Reply {
        Reply {
            status: "303 See Other",
            headers: vec![
                "Location: /".to_owned(),
                format!(
                    "Set-Cookie: {}={}; HttpOnly; SameSite=Strict; Path=/",
                    self.cookie, self.session
                ),
            ],
            ..Reply::error("303 See Other")
        }
    }

    fn phase(&self) -> std::sync::MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn forget(&self) {
        *self.loaded.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// The signed-in session, loaded the way every command loads it, kept for
    /// [`SESSION_TTL`].
    pub(super) async fn current(&self) -> Result<Arc<Session>, Error> {
        {
            let held = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, s)) = held.as_ref()
                && at.elapsed() < SESSION_TTL
            {
                return Ok(Arc::clone(s));
            }
        }
        let fresh = Arc::new(session(&self.global, &self.env).await?);
        *self.loaded.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), Arc::clone(&fresh)));
        Ok(fresh)
    }

    /// `GET /api/session`: whether this machine is signed in, as whom, and how the
    /// sign-in the page started is going. No credential is ever in it.
    pub(super) async fn state(&self) -> Reply {
        let (login, reason) = match &*self.phase() {
            Phase::Idle => ("idle", None),
            Phase::Waiting => ("waiting", None),
            Phase::Failed(why) => ("failed", Some(why.clone())),
        };
        let mut body = serde_json::json!({ "signed_in": false, "login": login });
        if let Some(why) = reason {
            body["reason"] = why.into();
        }
        match self.current().await {
            Ok(s) => {
                body["signed_in"] = true.into();
                body["profile"] = s.label.clone().into();
                body["kind"] = match s.kind {
                    Kind::Person => "person",
                    Kind::Token => "token",
                    _ => "other",
                }
                .into();
                body["account"] = s.account.clone().into();
                body["subject"] = s.claims.sub.clone().into();
                body["scopes"] = s.claims.scp.clone().into();
                if let Some(exp) = s.claims.exp {
                    body["expires_at"] = exp.into();
                }
            }
            Err(Error::NotSignedIn(_)) => {}
            Err(e) => body["reason"] = safe(&e).into(),
        }
        Reply::json(&body)
    }

    /// `POST /api/session/login`: starts `iohr login --web` in the background and answers
    /// at once; one at a time (a second ask while one waits answers the same).
    pub(super) fn login(self: &Arc<Self>) -> Reply {
        {
            let mut phase = self.phase();
            if matches!(*phase, Phase::Waiting) {
                return Reply::json(&serde_json::json!({ "login": "waiting" }));
            }
            *phase = Phase::Waiting;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let args = Login {
                how: How {
                    web: true,
                    device: false,
                    with_token: false,
                },
                insecure_storage: false,
                scopes: Vec::new(),
                account: None,
            };
            let done =
                super::super::login::login(&me.global, &me.env, &args, Out { json: false }).await;
            me.forget();
            *me.phase() = match done {
                Ok(()) => Phase::Idle,
                Err(e) => Phase::Failed(safe(&e)),
            };
        });
        Reply::json(&serde_json::json!({ "login": "waiting" }))
    }

    /// `POST /api/session/logout`: `iohr logout` for the profile this serve acts as.
    pub(super) async fn logout(&self) -> Reply {
        let done = super::super::login::logout(&self.global, Out { json: false }).await;
        self.forget();
        match done {
            Ok(()) => Reply::json(&serde_json::json!({ "signed_in": false })),
            Err(e) => Reply::problem("400 Bad Request", &safe(&e)),
        }
    }
}

/// An error's text as the page may see it: the runner's redaction (`crate::attach::scrub`:
/// values after a secret's name, bearer values, JWTs, GitHub tokens, long secret-looking
/// words, URLs and addresses) applied to what a failed call says. A failure can quote a
/// header, a URL with a credential, or a body; none of that reaches the browser.
pub(super) fn safe(error: &dyn std::fmt::Display) -> String {
    crate::attach::scrub(&error.to_string())
}

/// The API path and query a `/api/proxy/...` target names, or `None` when it is not a plain
/// read under `/v1/`.
pub(super) fn forwarded(target: &str) -> Option<(String, Vec<(String, String)>)> {
    let path = decoded_path(target)?;
    let path = path.strip_prefix("/api/proxy")?;
    let plain = path.starts_with("/v1/")
        && path.len() <= MAX_PART
        && path
            .split('/')
            .skip(1)
            .all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains(['\\', '?', '#']))
        && !path.chars().any(char::is_control);
    if !plain {
        return None;
    }
    let mut pairs = Vec::new();
    if let Some((_, q)) = target.split_once('?') {
        for pair in q.split('#').next().unwrap_or_default().split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let k = percent_decode(&k.replace('+', " "))?;
            let v = percent_decode(&v.replace('+', " "))?;
            if pairs.len() >= MAX_PART.min(MAX_PAIRS)
                || k.is_empty()
                || k.len() > MAX_PART
                || v.len() > MAX_PART
                || k.chars().chain(v.chars()).any(char::is_control)
            {
                return None;
            }
            pairs.push((k, v));
        }
    }
    Some((path.to_owned(), pairs))
}

/// The status line for an API status the page may see; any other is a bad gateway.
pub(super) fn status_line(status: u16) -> &'static str {
    match status {
        200 => "200 OK",
        201 => "201 Created",
        202 => "202 Accepted",
        204 => "204 No Content",
        400 => "400 Bad Request",
        401 => "401 Unauthorized",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        409 => "409 Conflict",
        410 => "410 Gone",
        412 => "412 Precondition Failed",
        413 => "413 Payload Too Large",
        415 => "415 Unsupported Media Type",
        422 => "422 Unprocessable Entity",
        429 => "429 Too Many Requests",
        503 => "503 Service Unavailable",
        504 => "504 Gateway Timeout",
        _ => "502 Bad Gateway",
    }
}
