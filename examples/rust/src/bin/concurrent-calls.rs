//! Several calls at once on one client. The client is cheap to share and safe to use from
//! many tasks, and callers that need a token at the same moment share one exchange. Here
//! `tokio::join!` runs three calls together; use `tokio::spawn` with a cloned client to
//! spread them over threads.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin concurrent-calls
//! ```

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::from_env()?;
    let (a, b, c) = tokio::join!(client.me(), client.me(), client.me());
    let subjects = [a?.value.subject, b?.value.subject, c?.value.subject];
    println!("3 calls at once: {subjects:?}");
    Ok(())
}
