//! Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
//!
//! iohr passes `IOHR_EXT_TOKEN_SOCKET` (a 0600 Unix socket, or a local named pipe on
//! Windows). The program sends `POST /token {"scopes": [...]}` and gets
//! `{"access_token", "expires_at"}` for scopes its manifest declares, and a refusal for any
//! other. It is the only credential this project has: never read `IOHR_TOKEN` or run
//! `iohr auth token`.

use std::io::{Read, Write};

const MAX_ANSWER: u64 = 64 * 1024;

#[cfg(unix)]
fn connect(path: &str) -> std::io::Result<std::os::unix::net::UnixStream> {
    let conn = std::os::unix::net::UnixStream::connect(path)?;
    let timeout = Some(std::time::Duration::from_secs(10));
    conn.set_read_timeout(timeout)?;
    conn.set_write_timeout(timeout)?;
    Ok(conn)
}

/// On Windows iohr serves a named pipe (`\\.\pipe\iohr-ext-…`), which opens like a file.
#[cfg(windows)]
fn connect(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}

/// iohr refused the request, or the channel could not be reached.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The text between the quotes after `"key":`, for the flat answers iohr gives.
fn field(body: &str, key: &str) -> Option<String> {
    let rest = &body[body.find(&format!("\"{key}\""))? + key.len() + 2..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

/// Asks for an access token for `scopes`; the token stays with the caller.
pub fn token(scopes: &[&str]) -> Result<String, Refused> {
    let path = std::env::var("IOHR_EXT_TOKEN_SOCKET").unwrap_or_default();
    if path.trim().is_empty() {
        return Err(Refused(
            "IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev".into(),
        ));
    }
    let unreachable = |e: std::io::Error| Refused(format!("cannot reach iohr's token channel: {e}"));
    let mut conn = connect(path.trim()).map_err(unreachable)?;
    let list = scopes.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(",");
    let body = format!("{{\"scopes\":[{list}]}}");
    let request = format!(
        "POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    conn.write_all(request.as_bytes()).map_err(unreachable)?;
    let mut raw = String::new();
    conn.take(MAX_ANSWER).read_to_string(&mut raw).map_err(unreachable)?;
    let (head, payload) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head.split(' ').nth(1).unwrap_or("");
    if status != "200" {
        return Err(Refused(format!(
            "{} {}: {}",
            if status.is_empty() { "no answer" } else { status },
            field(payload, "error").unwrap_or_default(),
            field(payload, "message").unwrap_or_default()
        )));
    }
    field(payload, "access_token").ok_or_else(|| Refused("iohr's answer holds no token".into()))
}
