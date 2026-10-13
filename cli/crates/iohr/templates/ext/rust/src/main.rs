//! @@ID@@: writes `$STATE/heartbeat.json` every minute and asks iohr's token channel for
//! a token with the scope its manifest declares. Run it with `iohr ext dev`.
//!
//! What iohr gives it: `STATE` and `CACHE` (its own folders), `IOHR_EXT_TOKEN_SOCKET`
//! and `IOHR_EXT_API` (the token channel and the API), `IOHR_EXT_DEV=1` under
//! `iohr ext dev`. It never sees your sign-in: no `IOHR_TOKEN`, and no `iohr` on its
//! `PATH`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use inorbithr::{ExtSocketToken, TokenProvider as _};

const ID: &str = "@@ID@@";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SCOPES: [&str; 1] = ["identity:read"];

fn dir(var: &str) -> Result<PathBuf, String> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{var} is not set: run this with iohr ext dev"))
}

/// Asks the channel for a token; the token stays in memory and is never printed.
async fn token_state() -> &'static str {
    let Some(provider) = ExtSocketToken::from_env() else {
        return "unavailable";
    };
    match provider.scopes(SCOPES).token().await {
        Ok(_token) => "granted",
        Err(e) => {
            eprintln!("token refused: {e}");
            "refused"
        }
    }
}

async fn heartbeat(state: &Path) -> std::io::Result<()> {
    let token = token_state().await;
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let beat = serde_json::json!({
        "id": ID,
        "version": VERSION,
        "state": "ok",
        "at_unix": at,
        "token": token,
        "dev": std::env::var("IOHR_EXT_DEV").as_deref() == Ok("1"),
    });
    let tmp = state.join(".heartbeat.json.tmp");
    std::fs::write(&tmp, format!("{beat}\n"))?;
    std::fs::rename(&tmp, state.join("heartbeat.json"))?;
    println!("heartbeat at {at} (token {token})");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let state = dir("STATE")?;
    dir("CACHE")?;
    let every = std::env::var("HEARTBEAT_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    println!("{ID} {VERSION} starting; state in {}", state.display());
    loop {
        heartbeat(&state).await.map_err(|e| e.to_string())?;
        if std::env::var_os("IOHR_EXT_ONCE").is_some() {
            return Ok(());
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(every)) => {}
            _ = tokio::signal::ctrl_c() => {
                println!("stopping");
                return Ok(());
            }
        }
    }
}
