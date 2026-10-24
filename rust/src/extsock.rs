//! Tokens from `iohr`'s extension channel (ADR 0012 of the command line, platform RFC
//! 0028): when `iohr` runs a program as an extension, or `iohr ext dev` runs one being
//! built, it passes `IOHR_EXT_TOKEN_SOCKET` (a 0600 Unix socket in a 0700 directory, a
//! named pipe on Windows) and `IOHR_EXT_API`. The program sends `POST /token
//! {"scopes": [...]}` and gets `{"access_token", "expires_at"}` for scopes its manifest
//! declares, and nothing for any other. The refresh token never leaves `iohr`.
//!
//! When `IOHR_EXT_TOKEN_SOCKET` is set, [`Client::load`](crate::Client::load) uses this
//! channel and nothing else: `INORBIT_TOKEN`, keys, the config file and the `iohr`
//! login are not read, so an extension cannot reach past its manifest's scopes.
//! Ported from the agent's client (dataplane `crates/iohr-agent/src/extsock.rs`).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::Token;
use crate::auth::{Cache, Cached, TokenProvider};
use crate::error::AuthError;
use crate::secret::Secret;

/// The variable that names the channel.
pub const SOCKET_ENV: &str = "IOHR_EXT_TOKEN_SOCKET";
/// The variable that names the API's origin for an extension.
pub const API_ENV: &str = "IOHR_EXT_API";
/// The largest answer read from the channel.
const MAX_ANSWER: usize = 64 * 1024;
/// How long one exchange may take. `iohr` bounds its own work (reading the request,
/// then minting the token, 10 s and 30 s) and answers a refusal with its reason when
/// that runs out, so this waits longer than both together: the program gets `iohr`'s
/// reason, never its own guess.
const TIMEOUT: Duration = Duration::from_secs(45);

/// Short-lived tokens from `iohr`'s extension channel, for the scopes asked for (none
/// means every scope the extension's manifest declares). Cached and refreshed with the
/// rules of the built-in sources.
///
/// ```no_run
/// # async fn f() -> Result<(), Box<dyn std::error::Error>> {
/// use inorbithr::{ExtSocketToken, TokenProvider as _};
///
/// if let Some(provider) = ExtSocketToken::from_env() {
///     let provider = provider.scopes(["identity:read"]);
///     let _token = provider.token().await?;
/// }
/// # Ok(()) }
/// ```
pub struct ExtSocketToken {
    socket: PathBuf,
    scopes: Arc<Vec<String>>,
    cache: Arc<Cache>,
}

impl fmt::Debug for ExtSocketToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtSocketToken")
            .field("socket", &self.socket)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

impl ExtSocketToken {
    /// Tokens from the channel at `socket`.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            scopes: Arc::new(Vec::new()),
            cache: Arc::new(Cache::default()),
        }
    }

    /// The channel `IOHR_EXT_TOKEN_SOCKET` names, or `None` when this program does not
    /// run as an extension.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        socket_from(std::env::var_os(SOCKET_ENV).map(|v| v.to_string_lossy().into_owned()))
            .map(Self::new)
    }

    /// Asks for these scopes only (each must be in the manifest).
    #[must_use]
    pub fn scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = Arc::new(scopes.into_iter().map(Into::into).collect());
        self.cache = Arc::new(Cache::default());
        self
    }

    /// The channel's path.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

/// A set, non-blank channel path.
pub(crate) fn socket_from(value: Option<String>) -> Option<PathBuf> {
    value
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

impl TokenProvider for ExtSocketToken {
    async fn token(&self) -> Result<Token, AuthError> {
        let socket = self.socket.clone();
        let scopes = Arc::clone(&self.scopes);
        self.cache
            .get(async move { within_deadline(fetch(&socket, &scopes)).await })
            .await
    }

    async fn invalidate(&self) {
        self.cache.invalidate();
    }
}

/// `exchange` with the client's deadline; the error names the seconds it waited.
async fn within_deadline(
    exchange: impl std::future::Future<Output = Result<Cached, AuthError>>,
) -> Result<Cached, AuthError> {
    tokio::time::timeout(TIMEOUT, exchange)
        .await
        .unwrap_or_else(|_| {
            Err(channel(format!(
                "iohr's token channel did not answer within {} s",
                TIMEOUT.as_secs()
            )))
        })
}

fn channel(message: impl Into<String>) -> AuthError {
    AuthError::Provider(message.into())
}

async fn fetch(socket: &Path, scopes: &[String]) -> Result<Cached, AuthError> {
    #[cfg(unix)]
    {
        let s = tokio::net::UnixStream::connect(socket).await.map_err(|e| {
            channel(format!(
                "cannot reach iohr's token channel {}: {e}",
                socket.display()
            ))
        })?;
        exchange(s, scopes).await
    }
    #[cfg(windows)]
    {
        let s = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(socket)
            .map_err(|e| {
                channel(format!(
                    "cannot reach iohr's token channel {}: {e}",
                    socket.display()
                ))
            })?;
        exchange(s, scopes).await
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (socket, scopes);
        Err(channel(
            "iohr's token channel needs Unix sockets or named pipes",
        ))
    }
}

/// One HTTP/1.1 request over the channel.
pub(crate) async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    scopes: &[String],
) -> Result<Cached, AuthError> {
    #[derive(Deserialize)]
    struct Answer {
        access_token: Secret<String>,
        #[serde(default)]
        expires_at: String,
    }
    #[derive(Deserialize)]
    struct Refusal {
        #[serde(default)]
        error: String,
        #[serde(default)]
        message: String,
    }
    let body = serde_json::json!({ "scopes": scopes }).to_string();
    let request = format!(
        "POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let io = |e: std::io::Error| channel(format!("iohr's token channel: {e}"));
    s.write_all(request.as_bytes()).await.map_err(io)?;
    let mut raw = zeroize::Zeroizing::new(Vec::new());
    (&mut s)
        .take(MAX_ANSWER as u64 + 1)
        .read_to_end(&mut raw)
        .await
        .map_err(io)?;
    if raw.len() > MAX_ANSWER {
        return Err(channel("iohr's token channel answered with too much"));
    }
    let (status, chunked, body) =
        split(&raw).ok_or_else(|| channel("iohr's token channel answered with no HTTP"))?;
    let body = zeroize::Zeroizing::new(if chunked {
        dechunk(body).ok_or_else(|| channel("iohr's token channel sent a bad chunked body"))?
    } else {
        body.to_vec()
    });
    if status != 200 {
        let r: Refusal = serde_json::from_slice(&body).unwrap_or(Refusal {
            error: String::new(),
            message: String::new(),
        });
        let what = if scopes.is_empty() {
            "the manifest's scopes".to_owned()
        } else {
            scopes.join(" ")
        };
        let printable =
            |s: &str| -> String { s.chars().filter(|c| !c.is_control()).take(200).collect() };
        return Err(channel(format!(
            "iohr refused a token for {what} ({status} {}): {}",
            printable(&r.error),
            printable(&r.message)
        )));
    }
    let answer: Answer = serde_json::from_slice(&body)
        .map_err(|_| channel("iohr's token channel answered with no token"))?;
    let lifetime = if answer.expires_at.is_empty() {
        None
    } else {
        let at = time::OffsetDateTime::parse(
            &answer.expires_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|_| channel("iohr's token channel sent an expires_at that is not RFC 3339"))?;
        Some(Duration::try_from(at - time::OffsetDateTime::now_utc()).unwrap_or_default())
    };
    Ok(Cached {
        access: answer.access_token,
        issued: Instant::now(),
        lifetime,
    })
}

/// The status, whether the body is chunked, and the body.
fn split(raw: &[u8]) -> Option<(u16, bool, &[u8])> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(raw.get(..end)?).ok()?;
    let status = head.split(' ').nth(1)?.parse().ok()?;
    let chunked = head.lines().skip(1).any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.trim().eq_ignore_ascii_case("chunked")
        })
    });
    Some((status, chunked, raw.get(end + 4..)?))
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(body.get(..line_end)?).ok()?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        body = body.get(line_end + 2..)?;
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::{ExtSocketToken, socket_from};
    use crate::TokenProvider as _;

    /// A channel that answers each connection with the next of `answers`, and checks
    /// the request asks for `agents:write`.
    fn fake_iohr(answers: Vec<&'static str>) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.sock");
        let l = tokio::net::UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            for answer in answers {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                assert!(req.starts_with("POST /token HTTP/1.1"), "{req}");
                assert!(req.contains(r#"{"scopes":["agents:write"]}"#), "{req}");
                s.write_all(answer.as_bytes()).await.unwrap();
            }
        });
        (dir, path)
    }

    #[tokio::test]
    async fn reads_a_token_by_length_or_chunked_and_caches_it() {
        let (_d, p) = fake_iohr(vec![
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"access_token\":\"at\",\"expires_at\":\"2100-01-01T00:00:00Z\"}",
        ]);
        let t = ExtSocketToken::new(&p).scopes(["agents:write"]);
        assert_eq!(t.token().await.unwrap().expose(), "at");
        // Cached: the fake answers only once.
        assert_eq!(t.token().await.unwrap().expose(), "at");

        let (_d, p) = fake_iohr(vec![
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n{\"access_token\":\r\n5\r\n\"at2\"\r\n1\r\n}\r\n0\r\n\r\n",
        ]);
        let t = ExtSocketToken::new(&p).scopes(["agents:write"]);
        assert_eq!(t.token().await.unwrap().expose(), "at2");
    }

    /// `iohr` may sign in or refresh before it answers (the first use of a profile took
    /// 14 s in a debug build): a slow answer inside its own deadline is still an answer,
    /// and one past ours is named with the real number of seconds.
    #[tokio::test(start_paused = true)]
    async fn waits_longer_than_the_channels_own_deadline() {
        use std::time::Duration;
        let cached = || super::Cached {
            access: crate::secret::Secret::new("slow".to_owned()),
            issued: std::time::Instant::now(),
            lifetime: None,
        };
        let slow = async {
            tokio::time::sleep(Duration::from_secs(14)).await;
            Ok(cached())
        };
        let got = super::within_deadline(slow).await.unwrap();
        assert_eq!(got.access.expose(), "slow");

        let stuck = async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok(cached())
        };
        let e = super::within_deadline(stuck)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("did not answer within 45 s"), "{e}");
    }

    #[tokio::test]
    async fn a_refusal_says_why_and_never_holds_a_token() {
        let (_d, p) = fake_iohr(vec![
            "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\n\r\n{\"error\":\"scope_not_declared\",\"message\":\"not in the manifest\"}",
        ]);
        let e = ExtSocketToken::new(&p)
            .scopes(["agents:write"])
            .token()
            .await
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("scope_not_declared") && e.contains("agents:write"),
            "{e}"
        );
    }

    #[tokio::test]
    async fn no_channel_is_an_error_not_a_hang() {
        let e = ExtSocketToken::new("/nonexistent/iohr.sock")
            .token()
            .await
            .unwrap_err();
        assert!(e.to_string().contains("cannot reach"), "{e}");
    }

    /// With the channel set, `load` uses it and nothing else: not `INORBIT_TOKEN`, and
    /// the API is `IOHR_EXT_API`.
    #[tokio::test]
    async fn load_takes_the_channel_and_nothing_else() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use crate::{Client, LoadOptions, Method, Operation};

        let api = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .and(header("authorization", "Bearer from-the-channel"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&api)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("token.sock");
        let l = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = s.read(&mut buf).await.unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\n\r\n{\"access_token\":\"from-the-channel\",\"expires_at\":\"2100-01-01T00:00:00Z\"}")
                .await
                .unwrap();
        });
        let env = |extra: &[(&str, &str)]| {
            let mut v: Vec<(String, String)> = vec![
                (
                    super::SOCKET_ENV.into(),
                    sock.to_string_lossy().into_owned(),
                ),
                (super::API_ENV.into(), api.uri()),
                ("INORBIT_TOKEN".into(), "from-the-environment".into()),
            ];
            v.extend(
                extra
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
            );
            LoadOptions::new()
                .env(v)
                .home(None::<std::path::PathBuf>)
                .cwd(dir.path())
        };
        let client: Client = Client::builder().load_options(env(&[])).load().unwrap();
        assert_eq!(client.base_url().trim_end_matches('/'), api.uri());
        let _: crate::Response<serde_json::Value> = client
            .request(Operation::new(Method::Get, "/v1/me"))
            .await
            .unwrap();

        let refused = Client::<crate::Public>::builder()
            .token("hard-coded")
            .load_options(env(&[]))
            .load()
            .unwrap_err()
            .to_string();
        assert!(refused.contains("IOHR_EXT_TOKEN_SOCKET"), "{refused}");
    }

    #[test]
    fn blank_is_unset() {
        assert_eq!(socket_from(None), None);
        assert_eq!(socket_from(Some("  ".into())), None);
        assert_eq!(
            socket_from(Some("/tmp/x.sock".into())).unwrap().to_str(),
            Some("/tmp/x.sock")
        );
    }
}
