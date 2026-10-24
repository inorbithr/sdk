//! The SDK retrying for you. The API answers 429 with Retry-After; the SDK waits that long
//! and tries again, and the answer says how many attempts it took. Tune it with
//! `max_retries` and the other retry options on the builder.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin retries
//! ```

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::builder().max_retries(3).load()?; // reads INORBIT_*
    let me = client.me().await?;
    println!("{} after {} attempts", me.value.subject, me.raw.attempts);
    Ok(())
}
