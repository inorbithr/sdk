//! A client from `load`, the way an application builds one: code, then the `INORBIT_*`
//! variables, then the config file, then the `iohr` login, each setting from the first that
//! sets it (docs/config.md). `describe` says where each setting came from, secrets
//! redacted, which is what `iohr sdk config` prints.
//!
//! ```sh
//! iohr login && cargo run --bin config-load
//! ```

use std::time::Duration;

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::builder()
        .total_timeout(Duration::from_secs(60)) // code wins over the rest
        .load()?;
    let described = client.config().describe();
    println!("credential from {}", described["credential"]["source"]);
    println!("timeout {}", described["settings"]["timeout"]);
    let me = client.me().await?;
    println!("{}, a {}", me.value.subject, me.value.kind);
    Ok(())
}
