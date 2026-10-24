//! One unary call: `GET /v1/me` with the key or token the environment holds. The SDK gets
//! a token, sends it, and returns the typed answer.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin unary-get
//! ```

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    // INORBIT_TOKEN, or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES.
    let client: Client = Client::from_env()?;
    let me = client.me().await?;
    println!(
        "{}, a {}, with scopes {:?}",
        me.value.subject, me.value.kind, me.value.scopes
    );
    Ok(())
}
