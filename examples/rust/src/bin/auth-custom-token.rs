//! A token provider of your own: anything that implements `TokenProvider`, here one that
//! would read a secret store. The SDK asks it before every attempt. Never print the token.
//!
//! ```sh
//! cargo run --bin auth-custom-token
//! ```

use inorbithr::public::Surface as _;
use inorbithr::{AuthError, Client, Error, Secret, Token, TokenProvider};

/// Stands in for wherever your tokens live.
struct Vault;

impl TokenProvider for Vault {
    async fn token(&self) -> Result<Token, AuthError> {
        Ok(Token::new(Secret::from("custom-token-1"), None)) // pass the expiry when you know it
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let mut builder = Client::builder().token_provider(Vault);
    if let Ok(base) = std::env::var("INORBIT_BASE_URL") {
        builder = builder.base_url(base); // unset means the public API
    }
    let client: Client = builder.build()?;
    let me = client.me().await?;
    println!("{} signed in with a custom provider", me.value.subject);
    Ok(())
}
