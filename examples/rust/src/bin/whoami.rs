//! Who the API thinks you are: `GET /v1/me` with the credentials in the environment.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin whoami
//! # or INORBIT_KEY_ID=ak_... INORBIT_KEY_SECRET=... INORBIT_SCOPES=identity:read
//! ```

use inorbithr::{Client, Error, Method, Operation, Response};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::from_env()?;
    let me: Response<serde_json::Value> = client
        .request(Operation::new(Method::Get, "/v1/me"))
        .await?;
    println!(
        "{} ({}), scopes {}, request id {}",
        me.value["subject"], me.value["kind"], me.value["scopes"], me.raw.request_id
    );
    Ok(())
}
