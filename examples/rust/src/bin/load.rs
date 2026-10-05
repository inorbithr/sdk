//! A client from wherever its configuration is: code, `INORBIT_*` variables, the config
//! file `iohr login` writes, then the `iohr` login itself (docs/config.md). Prints where
//! each setting came from, then calls `GET /v1/me`.
//!
//! ```sh
//! iohr login && cargo run --bin load
//! # or INORBIT_TOKEN=... / INORBIT_KEY_ID=ak_... INORBIT_KEY_SECRET_FILE=... INORBIT_SCOPES=identity:read
//! ```

use std::time::Duration;

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    // An option set in code always wins over the environment and the file.
    let client: Client = Client::builder()
        .user_agent_suffix("load-example/1.0")
        .total_timeout(Duration::from_secs(60))
        .load()?;
    let described = client.config().describe();
    println!("credential from {}", described["credential"]["source"]);
    println!("timeout {}", described["settings"]["timeout"]);
    let me = client.me().await?;
    println!(
        "{} ({} attempt), request id {}",
        me.value.subject, me.raw.attempts, me.raw.request_id
    );
    Ok(())
}
