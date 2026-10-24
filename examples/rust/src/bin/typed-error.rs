//! How a refused call comes back: `Error::Api` with the status, the code, the message and
//! the request id. Match on the code, never on the text. This example expects the 403 and
//! exits 0 when it gets it.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin typed-error
//! ```

use inorbithr::public::Surface as _;
use inorbithr::{Client, Code, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::from_env()?;
    match client.me().await {
        Ok(_) => {
            eprintln!("expected the API to refuse the call");
            std::process::exit(1);
        }
        Err(Error::Api(e)) if e.code == Code::Forbidden => {
            println!(
                "refused: status {}, code {:?}, {} (request {})",
                e.status, e.code, e.message, e.raw.request_id
            );
            Ok(())
        }
        // A network error, a timeout or another code.
        Err(e) => Err(e),
    }
}
