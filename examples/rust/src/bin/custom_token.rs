//! Plug in a token provider of your own: here one that reads a token the `iohr`
//! command line wrote to a file, and matches on the error codes a call can fail with.
//!
//! ```sh
//! iohr token create --name demo --scope identity:read --days 7 > /tmp/demo.token
//! TOKEN_FILE=/tmp/demo.token cargo run --bin custom_token
//! ```

use std::path::PathBuf;

use inorbithr::{AuthError, Client, Code, Error, Method, Operation, Secret, Token, TokenProvider};

/// Reads the token from a file on every request, so a rotated file is picked up.
struct FromFile(PathBuf);

impl TokenProvider for FromFile {
    async fn token(&self) -> Result<Token, AuthError> {
        let text = tokio::fs::read_to_string(&self.0)
            .await
            .map_err(|e| AuthError::Provider(format!("cannot read {}: {e}", self.0.display())))?;
        Ok(Token::new(Secret::from(text.trim()), None))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let path =
        std::env::var_os("TOKEN_FILE").map_or_else(|| PathBuf::from("demo.token"), PathBuf::from);
    let client: Client = Client::builder().token_provider(FromFile(path)).build()?;
    let result: Result<inorbithr::Response<serde_json::Value>, Error> = client
        .request(Operation::new(Method::Get, "/v1/accounts/me"))
        .await;
    // Nothing read from the token file reaches the output: what is printed is what the
    // outcome means, not what came back.
    match result {
        Ok(_) => println!("the token works: account:read is granted"),
        Err(Error::Api(e)) if e.code == Code::Forbidden => {
            println!("the token holds no account:read scope");
        }
        Err(Error::Api(e)) if e.code == Code::RateLimited => {
            println!("rate limited; try again shortly");
        }
        Err(e) => return Err(e),
    }
    Ok(())
}
