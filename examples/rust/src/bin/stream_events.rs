//! Read the account's events as they happen: `events.stream_events` (scope
//! `events:read`), over server-sent events, or over the `/v1/ws` socket with
//! `STREAMS=socket`. Prints each event's type and id, never its data.
//!
//! ```sh
//! INORBIT_KEY_ID=ak_... INORBIT_KEY_SECRET=... INORBIT_SCOPES=events:read \
//!   cargo run --bin stream_events
//! ```

use inorbithr::public::{EventsStreamEventsParams, Surface as _};
use inorbithr::{Client, Code, Error, Streams};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let mut builder = Client::builder();
    if std::env::var("STREAMS").as_deref() == Ok("socket") {
        builder = builder.streams(Streams::Socket);
    }
    // Credentials from the environment, as `Client::from_env` reads them.
    let client: Client = builder
        .key(
            std::env::var("INORBIT_KEY_ID").unwrap_or_default(),
            std::env::var("INORBIT_KEY_SECRET").unwrap_or_default(),
        )
        .scopes(["events:read"])
        .build()?;
    let mut events = client
        .events()
        .stream_events(&EventsStreamEventsParams::default())
        .await?;
    while let Some(event) = events.next().await {
        match event {
            Ok(event) => println!("{} {}", event.type_, event.id),
            // A revoked key's stream ends with this; a new key is a new decision.
            Err(Error::Api(e)) if e.code == Code::Unauthenticated => {
                println!("the key was revoked");
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
