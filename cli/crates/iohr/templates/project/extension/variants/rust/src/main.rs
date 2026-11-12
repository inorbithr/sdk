//! @@ID@@: the demo project's private macOS service.
//!
//! Every minute it writes `$STATE/heartbeat.json` (the health InOrbit reads) and asks iohr's
//! token channel for a token with the one scope its manifest declares. It also serves a tiny
//! panel on 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads `/api/heartbeat`.
//!
//! Run it with `cargo run` under `iohr ext dev`'s STATE and CACHE, or once with
//! `IOHR_EXT_ONCE=1` as the self-test does. Standard library only.

mod channel;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ID: &str = "@@ID@@";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SCOPES: [&str; 1] = ["identity:read"];
const PANEL: &str = include_str!("../panel/index.html");

/// `YYYY-MM-DDTHH:MM:SSZ` from seconds since the epoch (civil-from-days, Howard Hinnant).
fn utc(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn write_json(state: &Path, name: &str, json: &str) -> std::io::Result<()> {
    let tmp = state.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, state.join(name))
}

fn heartbeat(state: &Path, beats: u64) -> std::io::Result<String> {
    let tok = match channel::token(&SCOPES) {
        Ok(_token) => "granted", // kept in memory only, never printed
        Err(e) => {
            eprintln!("token refused: {e}");
            "refused"
        }
    };
    let at = utc(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs());
    let dev = std::env::var("IOHR_EXT_DEV").as_deref() == Ok("1");
    let beat = format!(
        "{{\"id\":\"{ID}\",\"version\":\"{VERSION}\",\"state\":\"ok\",\"at\":\"{at}\",\"beats\":{beats},\"token\":\"{tok}\",\"dev\":{dev}}}"
    );
    write_json(state, "heartbeat.json", &beat)?;
    println!("heartbeat {beats} at {at} (token {tok})");
    Ok(beat)
}

fn answer(mut conn: TcpStream, latest: &Mutex<String>) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = [0_u8; 2048];
    let n = conn.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let target = request.split_whitespace().nth(1).unwrap_or("");
    let (status, kind, body) = match target {
        "/api/heartbeat" => (
            "200 OK",
            "application/json",
            latest.lock().map(|b| b.clone()).unwrap_or_default(),
        ),
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", PANEL.to_owned()),
        _ => ("404 Not Found", "text/plain", "not found".to_owned()),
    };
    write!(
        conn,
        "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Listens on 127.0.0.1 only; the port is `PANEL_PORT` or one the system picks, and it is
/// written to `$STATE/panel.json` so the app knows where the panel is.
fn serve(state: &Path, latest: Arc<Mutex<String>>) -> std::io::Result<()> {
    let port: u16 = std::env::var("PANEL_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let url = format!("http://{}/", listener.local_addr()?);
    write_json(state, "panel.json", &format!("{{\"id\":\"{ID}\",\"url\":\"{url}\"}}"))?;
    println!("panel at {url}");
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let _ = answer(conn, &latest);
        }
    });
    Ok(())
}

fn run() -> Result<(), String> {
    let (Some(state), Some(_)) = (std::env::var_os("STATE"), std::env::var_os("CACHE")) else {
        return Err("STATE and CACHE are not set: run this with iohr ext dev".into());
    };
    let state = Path::new(&state);
    let every = std::env::var("HEARTBEAT_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let once = std::env::var_os("IOHR_EXT_ONCE").is_some();
    println!("{ID} {VERSION} starting; state in {}", state.display());
    let latest = Arc::new(Mutex::new(String::from("{}")));
    if !once {
        serve(state, Arc::clone(&latest)).map_err(|e| e.to_string())?;
    }
    for beats in 1.. {
        let beat = heartbeat(state, beats).map_err(|e| e.to_string())?;
        if let Ok(mut l) = latest.lock() {
            *l = beat;
        }
        if once {
            break;
        }
        std::thread::sleep(Duration::from_secs(every));
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
