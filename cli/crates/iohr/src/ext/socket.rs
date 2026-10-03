//! The token channel between `iohr` and a running extension (SR-27).
//!
//! `iohr` listens on a Unix socket, mode 0600, in a fresh directory of mode 0700 under
//! the temporary directory (a named pipe with an unguessable name on Windows, refusing
//! remote clients), and passes its path in `IOHR_EXT_TOKEN_SOCKET`. The extension sends
//! one HTTP/1.1 request per connection:
//!
//! ```text
//! POST /token
//! {"scopes": ["agents:write"]}
//! ```
//!
//! and gets `{"access_token": "...", "expires_at": "<RFC 3339>"}`, or an error
//! `{"error": "...", "message": "..."}` with status 400, 403, 404, 405, 413 or 503.
//! Scopes must be among the extension's manifest scopes; none means all of them. The
//! refresh token never crosses the channel. The socket and its directory are removed
//! when the extension exits.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use iohr_auth::Redacted;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// A token handed to an extension.
#[derive(Debug)]
pub struct Minted {
    /// The access token.
    pub access_token: Redacted<String>,
    /// When it expires, RFC 3339.
    pub expires_at: String,
}

/// Why no token was handed out.
#[derive(Debug)]
pub enum MintError {
    /// The credential does not hold a scope that was asked for.
    Forbidden(String),
    /// No credential is available (not signed in, sign-in service unreachable).
    Unavailable(String),
}

/// Produces tokens from the profile's credential.
pub trait Minter: Send + Sync + 'static {
    /// A token holding at least `scopes`.
    fn mint(&self, scopes: Vec<String>) -> impl Future<Output = Result<Minted, MintError>> + Send;
}

/// A request read from the channel.
#[derive(Debug, PartialEq, Eq)]
pub struct Head {
    /// The method.
    pub method: String,
    /// The path.
    pub path: String,
    /// The body's length.
    pub content_length: usize,
}

/// A refusal: status and code.
pub type Refusal = (u16, &'static str);

/// Reads a request head (request line and headers, ending in a blank line).
///
/// # Errors
///
/// A [`Refusal`] when it is not a request this channel answers.
pub fn parse_head(head: &[u8]) -> Result<Head, Refusal> {
    let text = std::str::from_utf8(head).map_err(|_| (400, "bad_request"))?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (first.next(), first.next(), first.next(), first.next())
    else {
        return Err((400, "bad_request"));
    };
    if !version.starts_with("HTTP/1.") {
        return Err((400, "bad_request"));
    }
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or((400, "bad_request"))?;
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse().map_err(|_| (400, "bad_request"))?;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err((400, "bad_request"));
        }
    }
    if content_length > MAX_BODY {
        return Err((413, "too_large"));
    }
    Ok(Head {
        method: method.to_owned(),
        path: path.to_owned(),
        content_length,
    })
}

/// The scopes a token request asks for, checked against the manifest's: none asked
/// means all of them.
///
/// # Errors
///
/// A [`Refusal`] for a body that is not the JSON shape, or a scope outside `allowed`.
pub fn requested_scopes(body: &[u8], allowed: &[String]) -> Result<Vec<String>, Refusal> {
    #[derive(Deserialize)]
    struct Wire {
        #[serde(default)]
        scopes: Option<Vec<String>>,
    }
    let wire: Wire = if body.iter().all(u8::is_ascii_whitespace) {
        Wire { scopes: None }
    } else {
        serde_json::from_slice(body).map_err(|_| (400, "bad_request"))?
    };
    let scopes = match wire.scopes {
        Some(s) if !s.is_empty() => s,
        _ => allowed.to_vec(),
    };
    if scopes.iter().any(|s| !allowed.contains(s)) {
        return Err((403, "scope_not_declared"));
    }
    Ok(scopes)
}

/// What a connection handler needs.
pub(crate) struct Ctx<M> {
    pub(crate) minter: M,
    pub(crate) allowed: Vec<String>,
    pub(crate) verbose: bool,
}

/// Answers one request on `conn`.
pub(crate) async fn handle<S, M>(mut conn: S, ctx: Arc<Ctx<M>>)
where
    S: AsyncRead + AsyncWrite + Unpin,
    M: Minter,
{
    let answer = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut conn)).await {
        Ok(Ok((head, body))) => answer(&head, &body, &ctx).await,
        Ok(Err(r)) => refusal(r, "the request could not be read"),
        Err(_) => refusal((400, "timeout"), "the request did not arrive in time"),
    };
    let _ = conn.write_all(&answer).await;
    let _ = conn.shutdown().await;
}

async fn answer<M: Minter>(head: &Head, body: &[u8], ctx: &Ctx<M>) -> Vec<u8> {
    if head.path != "/token" {
        return refusal((404, "not_found"), "the only path is POST /token");
    }
    if head.method != "POST" {
        return refusal((405, "method_not_allowed"), "the only path is POST /token");
    }
    let scopes = match requested_scopes(body, &ctx.allowed) {
        Ok(s) => s,
        Err(r) => {
            return refusal(
                r,
                if r.0 == 403 {
                    "a scope that is not in the extension's manifest was asked for"
                } else {
                    "the body is {\"scopes\": [...]}"
                },
            );
        }
    };
    let outcome = ctx.minter.mint(scopes.clone()).await;
    if ctx.verbose {
        crate::api::note(&format!(
            "the extension asked for a token with {}: {}",
            scopes.join(" "),
            if outcome.is_ok() {
                "granted"
            } else {
                "refused"
            }
        ));
    }
    match outcome {
        Ok(m) => {
            let body = serde_json::json!({
                "access_token": m.access_token.expose(),
                "expires_at": m.expires_at,
            })
            .to_string();
            response(200, &body)
        }
        Err(MintError::Forbidden(m)) => refusal((403, "scope_not_held"), &m),
        Err(MintError::Unavailable(m)) => refusal((503, "unavailable"), &m),
    }
}

async fn read_request<S: AsyncRead + Unpin>(conn: &mut S) -> Result<(Head, Vec<u8>), Refusal> {
    let mut buf = Vec::with_capacity(1024);
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err((413, "too_large"));
        }
        let mut chunk = [0u8; 1024];
        let n = conn
            .read(&mut chunk)
            .await
            .map_err(|_| (400, "bad_request"))?;
        if n == 0 {
            return Err((400, "bad_request"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = parse_head(&buf[..end])?;
    let mut body = buf[end..].to_vec();
    if body.len() > head.content_length {
        return Err((400, "bad_request"));
    }
    while body.len() < head.content_length {
        let mut chunk = vec![0u8; head.content_length - body.len()];
        let n = conn
            .read(&mut chunk)
            .await
            .map_err(|_| (400, "bad_request"))?;
        if n == 0 {
            return Err((400, "bad_request"));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Ok((head, body))
}

fn refusal((status, code): Refusal, message: &str) -> Vec<u8> {
    response(
        status,
        &serde_json::json!({ "error": code, "message": message }).to_string(),
    )
}

fn response(status: u16, body: &str) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        _ => "Service Unavailable",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::sync::Arc;

    use iohr_auth::Redacted;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::{Ctx, MintError, Minted, Minter, handle, parse_head, requested_scopes};

    struct Fixed;

    impl Minter for Fixed {
        #[allow(clippy::unused_async_trait_impl, reason = "a fixed answer")]
        async fn mint(&self, scopes: Vec<String>) -> Result<Minted, MintError> {
            if scopes.iter().any(|s| s == "domains:read") {
                return Err(MintError::Forbidden(
                    "the profile does not hold domains:read".into(),
                ));
            }
            Ok(Minted {
                access_token: Redacted::new("tok".into()),
                expires_at: "2026-10-03T12:00:00Z".into(),
            })
        }
    }

    async fn ask(request: &str) -> String {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let ctx = Arc::new(Ctx {
            minter: Fixed,
            allowed: vec!["agents:write".into(), "domains:read".into()],
            verbose: false,
        });
        let task = tokio::spawn(handle(server, ctx));
        client.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        client.read_to_string(&mut out).await.unwrap();
        task.await.unwrap();
        out
    }

    fn post(body: &str) -> String {
        format!(
            "POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn grants_a_declared_scope_and_refuses_others() {
        let ok = ask(&post(r#"{"scopes":["agents:write"]}"#)).await;
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(
            ok.contains(r#""access_token":"tok""#) && ok.contains("expires_at"),
            "{ok}"
        );

        let undeclared = ask(&post(r#"{"scopes":["tokens:write"]}"#)).await;
        assert!(undeclared.starts_with("HTTP/1.1 403"), "{undeclared}");
        assert!(undeclared.contains("scope_not_declared"), "{undeclared}");

        let not_held = ask(&post(r#"{"scopes":["domains:read"]}"#)).await;
        assert!(not_held.contains("scope_not_held"), "{not_held}");

        // No scopes: all of the manifest's, which includes one the profile lacks.
        assert!(ask(&post("")).await.starts_with("HTTP/1.1 403"));
    }

    #[tokio::test]
    async fn answers_only_post_token() {
        assert!(
            ask("GET /token HTTP/1.1\r\n\r\n")
                .await
                .starts_with("HTTP/1.1 405")
        );
        assert!(
            ask(&post("{}").replace("/token", "/other"))
                .await
                .starts_with("HTTP/1.1 404")
        );
        assert!(ask("garbage\r\n\r\n").await.starts_with("HTTP/1.1 400"));
        assert!(ask(&post("not json")).await.starts_with("HTTP/1.1 400"));
        let huge = "POST /token HTTP/1.1\r\ncontent-length: 999999\r\n\r\n";
        assert!(ask(huge).await.starts_with("HTTP/1.1 413"));
    }

    #[test]
    fn heads_and_bodies() {
        let h = parse_head(b"POST /token HTTP/1.1\r\nContent-Length: 2\r\n\r\n").unwrap();
        assert_eq!((h.method.as_str(), h.content_length), ("POST", 2));
        assert!(parse_head(b"POST /token HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").is_err());
        assert!(parse_head(b"POST /token SPDY\r\n\r\n").is_err());
        let allowed = vec!["a:b".to_owned()];
        assert_eq!(requested_scopes(b"{}", &allowed).unwrap(), allowed);
        assert_eq!(
            requested_scopes(br#"{"scopes":[]}"#, &allowed).unwrap(),
            allowed
        );
        assert!(requested_scopes(br#"{"scopes":["c:d"]}"#, &allowed).is_err());
    }
}
