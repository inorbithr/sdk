//! The simplest credential: an API token in `INORBIT_TOKEN`, sent as the bearer as is.
//! There is no key exchange and nothing to refresh. In code the same is
//! `Client::builder().token(...)`.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin auth-token
//! ```

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error, Secret};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let token = std::env::var("INORBIT_TOKEN").unwrap_or_default();
    let mut builder = Client::builder().token(Secret::new(token));
    if let Ok(base) = std::env::var("INORBIT_BASE_URL") {
        builder = builder.base_url(base); // unset means the public API
    }
    let client: Client = builder.build()?;
    let me = client.me().await?;
    println!("{} signed in with a token", me.value.subject);
    Ok(())
}
