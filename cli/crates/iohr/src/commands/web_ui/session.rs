//! Who the browser acts as, and how it gets there (RFC 0217, round 4).
//!
//! The page never holds a credential. `iohr ui serve` signs in with the one flow `iohr
//! login` uses (the system browser, PKCE, the same profile and credential store) and calls
//! the API for the page with that credential; the page sees only what the API answered.
//!
//! Reaching the page takes the per-launch secret, which `iohr ui open` (or `serve --open`)
//! puts in the first address as `?launch=`. The server compares it in constant time,
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
//! `GET /api/proxy/v1/...` forwards one read to the API with the signed-in credential.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use inorbithr::Method;
use iohr_auth::Kind;

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

/// The launch secret, the session cookie it is exchanged for, and the sign-in state.
pub(super) struct Access {
    launch: String,
    session: String,
    cookie: String,
    global: Global,
    env: Env,
    phase: Mutex<Phase>,
    loaded: Mutex<Option<(Instant, Arc<Session>)>>,
}

impl Access {
    pub(super) fn new(global: Global, env: Env, port: u16) -> std::io::Result<Self> {
        Ok(Self {
            launch: super::random_hex()?,
            session: super::random_hex()?,
            cookie: format!("iohr_ui_{port}"),
            global,
            env,
            phase: Mutex::new(Phase::Idle),
            loaded: Mutex::new(None),
        })
    }

    /// The secret `iohr ui open` puts in the first address; never printed by `serve`.
    pub(super) fn launch(&self) -> &str {
        &self.launch
    }

    /// `<cookie name>=<session>|<launch secret>`, for the tests' browser.
    #[cfg(test)]
    pub(super) fn cookie_line(&self) -> String {
        format!("{}={}|{}", self.cookie, self.session, self.launch)
    }

    /// Decides one request: a valid session cookie, else the launch secret in the query.
    pub(super) fn entry(&self, request: &Request) -> Entry {
        if self.cookie_ok(request) {
            return Entry::Open;
        }
        let asked = request
            .target
            .split_once('?')
            .map(|(_, q)| q.split('#').next().unwrap_or_default())
            .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("launch=")));
        if asked.is_some_and(|a| same_bytes(a.as_bytes(), self.launch.as_bytes())) {
            Entry::Exchange
        } else {
            Entry::Refused
        }
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
    async fn current(&self) -> Result<Arc<Session>, Error> {
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
            Err(e) => body["reason"] = e.to_string().into(),
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
                Err(e) => Phase::Failed(e.to_string()),
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
            Err(e) => Reply::problem("400 Bad Request", &e.to_string()),
        }
    }

    /// `GET /api/proxy/v1/...`: one read of the API with the signed-in credential. The
    /// path must be under `/v1/` with plain segments; the answer's status and JSON body
    /// pass on, and nothing of the credential does.
    pub(super) async fn proxy(&self, request: &Request) -> Reply {
        let Some((path, query)) = forwarded(&request.target) else {
            return Reply::error("400 Bad Request");
        };
        let s = match self.current().await {
            Ok(s) => s,
            Err(Error::NotSignedIn(_)) => {
                return Reply::problem("401 Unauthorized", "not signed in");
            }
            Err(e) => return Reply::problem("502 Bad Gateway", &e.to_string()),
        };
        let pairs: Vec<(&str, &str)> = query
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        match s.api.send(Method::Get, &path, &pairs, None).await {
            Ok(raw) => {
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
            Err(e) => {
                let status = e.status().map_or("502 Bad Gateway", status_line);
                Reply::problem(status, &Error::from(e).to_string())
            }
        }
    }
}

/// The API path and query a `/api/proxy/...` target names, or `None` when it is not a plain
/// read under `/v1/`.
fn forwarded(target: &str) -> Option<(String, Vec<(String, String)>)> {
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
fn status_line(status: u16) -> &'static str {
    match status {
        200 => "200 OK",
        204 => "204 No Content",
        400 => "400 Bad Request",
        401 => "401 Unauthorized",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        409 => "409 Conflict",
        429 => "429 Too Many Requests",
        503 => "503 Service Unavailable",
        504 => "504 Gateway Timeout",
        _ => "502 Bad Gateway",
    }
}
