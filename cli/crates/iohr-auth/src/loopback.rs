//! The loopback redirect of a browser sign-in (RFC 8252, section 7.3).
//!
//! A listener on `127.0.0.1` (or `[::1]` when IPv4 loopback is unavailable) on a port
//! the operating system picks. It answers anything but `GET /callback` with an error
//! and keeps waiting; the first `GET /callback` ends it, and its page is sent once the
//! sign-in has succeeded or failed. The pages (`crate::page`) load nothing from
//! anywhere, run only their own hashed script, and tell the browser not to send a
//! referrer.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, timeout, timeout_at};

use crate::page::{Page, Rendered};

/// How long a browser sign-in may take before the listener gives up.
pub const SIGN_IN_WINDOW: Duration = Duration::from_mins(5);
/// How long one connection may take to send its request head.
const READ_WINDOW: Duration = Duration::from_secs(10);
/// The largest request head read; a redirect with a code and state fits in far less.
pub const MAX_HEAD: usize = 8 * 1024;

/// The query of the one `GET /callback` request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Callback {
    /// The authorization code.
    pub code: Option<String>,
    /// The `state` sent with the authorization request.
    pub state: Option<String>,
    /// An OAuth error code, such as `access_denied`.
    pub error: Option<String>,
    /// The error's description.
    pub error_description: Option<String>,
    /// The issuer, when the server sends it (RFC 9207).
    pub iss: Option<String>,
}

/// What a request head turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// `GET /callback?...`.
    Callback(Callback),
    /// Any other path, such as `/favicon.ico`.
    OtherPath,
    /// Another method on `/callback`.
    OtherMethod,
}

/// Why a request head was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    /// Not an HTTP/1 request line.
    #[error("not an HTTP request")]
    Malformed,
    /// A query parameter appeared twice.
    #[error("a parameter appeared twice")]
    Repeated,
}

/// Reads an HTTP/1 request head. Only the request line is used; headers are ignored.
///
/// # Errors
///
/// [`ParseError`] when the line is not `METHOD /target HTTP/1.x`, the target is not
/// origin-form, or a callback parameter is repeated.
pub fn parse_request(head: &[u8]) -> Result<Request, ParseError> {
    let line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or(ParseError::Malformed)?;
    let line = std::str::from_utf8(&head[..line_end]).map_err(|_| ParseError::Malformed)?;
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ParseError::Malformed);
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || !target.starts_with('/') || method.is_empty()
    {
        return Err(ParseError::Malformed);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        return Ok(Request::OtherPath);
    }
    if method != "GET" {
        return Ok(Request::OtherMethod);
    }
    let mut cb = Callback::default();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let slot = match &*key {
            "code" => &mut cb.code,
            "state" => &mut cb.state,
            "error" => &mut cb.error,
            "error_description" => &mut cb.error_description,
            "iss" => &mut cb.iss,
            _ => continue,
        };
        if slot.replace(value.into_owned()).is_some() {
            return Err(ParseError::Repeated);
        }
    }
    Ok(Request::Callback(cb))
}

/// A listener waiting for the browser to come back.
#[derive(Debug)]
pub struct Listener {
    inner: TcpListener,
    redirect_uri: String,
}

impl Listener {
    /// Binds a random port on `127.0.0.1`, or on `[::1]` when that fails.
    ///
    /// # Errors
    ///
    /// The I/O error when neither loopback address can be bound.
    pub async fn bind() -> io::Result<Self> {
        let inner = match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, 0))).await?,
        };
        let addr = inner.local_addr()?;
        let redirect_uri = match addr {
            SocketAddr::V4(a) => format!("http://127.0.0.1:{}/callback", a.port()),
            SocketAddr::V6(a) => format!("http://[::1]:{}/callback", a.port()),
        };
        Ok(Self {
            inner,
            redirect_uri,
        })
    }

    /// The redirect URI to send in the authorization request.
    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Waits for the one `GET /callback`, at most `window`. Other requests are answered
    /// at once and the wait goes on; the callback's answer is left to the caller, who
    /// sends it with [`Reply::send`] once the sign-in has succeeded or failed.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::TimedOut`] when the window passes, or the listener's I/O error.
    pub async fn accept_callback(self, window: Duration) -> io::Result<(Callback, Reply)> {
        let deadline = Instant::now() + window;
        loop {
            let (mut stream, _) =
                timeout_at(deadline, self.inner.accept())
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the browser did not come back in time",
                        )
                    })??;
            let Ok(Ok(head)) = timeout(READ_WINDOW, read_head(&mut stream)).await else {
                continue;
            };
            match parse_request(&head) {
                Ok(Request::Callback(cb)) => return Ok((cb, Reply { stream })),
                Ok(Request::OtherMethod) => {
                    let _ = respond(&mut stream, "405 Method Not Allowed", Page::NotHere).await;
                }
                Ok(Request::OtherPath) | Err(_) => {
                    let _ = respond(&mut stream, "404 Not Found", Page::NotHere).await;
                }
            }
        }
    }
}

/// The browser's open connection, waiting for the page that says how the sign-in went.
#[derive(Debug)]
pub struct Reply {
    stream: TcpStream,
}

impl Reply {
    /// Sends `page` and closes the connection. A browser that went away is not an
    /// error worth reporting: the terminal has the outcome either way.
    pub(crate) async fn send(mut self, page: Page<'_>) {
        let _ = timeout(READ_WINDOW, respond(&mut self.stream, "200 OK", page)).await;
    }
}

async fn read_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(head);
        }
        head.extend_from_slice(&buf[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() >= MAX_HEAD {
            head.truncate(MAX_HEAD);
            return Ok(head);
        }
    }
}

async fn respond(stream: &mut TcpStream, status: &str, page: Page<'_>) -> io::Result<()> {
    let Rendered { body, csp } = page.render();
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Content-Security-Policy: {csp}\r\n\
         Referrer-Policy: no-referrer\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::{Callback, ParseError, Request, parse_request};

    #[test]
    fn reads_the_callback() {
        let head =
            b"GET /callback?code=c%2B1&state=s&scope=openid+x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let Request::Callback(cb) = parse_request(head).unwrap() else {
            panic!()
        };
        assert_eq!(cb.code.as_deref(), Some("c+1"));
        assert_eq!(cb.state.as_deref(), Some("s"));
        assert_eq!(cb.error, None);
    }

    #[test]
    fn other_paths_methods_and_garbage() {
        assert_eq!(
            parse_request(b"GET /favicon.ico HTTP/1.1\r\n\r\n"),
            Ok(Request::OtherPath)
        );
        assert_eq!(
            parse_request(b"POST /callback HTTP/1.1\r\n\r\n"),
            Ok(Request::OtherMethod)
        );
        assert_eq!(
            parse_request(b"GET /callback HTTP/1.1\r\n\r\n"),
            Ok(Request::Callback(Callback::default()))
        );
        for bad in [
            &b""[..],
            b"GET /callback",
            b"GET http://x/callback HTTP/1.1\r\n",
            b"GET  /callback HTTP/1.1\r\n",
            b"GET /callback HTTP/2\r\n",
            b"\xff\xfe /callback HTTP/1.1\r\n",
        ] {
            assert_eq!(parse_request(bad), Err(ParseError::Malformed), "{bad:?}");
        }
        assert_eq!(
            parse_request(b"GET /callback?state=a&state=b HTTP/1.1\r\n\r\n"),
            Err(ParseError::Repeated)
        );
    }
}
