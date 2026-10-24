//! The account's events (scope `events:read`) over the one multiplexed /v1/ws socket. Only the `streams` option differs from stream-sse.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin stream-socket
//! ```

use inorbithr::public::{EventsStreamEventsParams, Surface as _};
use inorbithr::{Client, Error, Streams};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::builder().streams(Streams::Socket).load()?; // reads INORBIT_*
    let mut events = client
        .events()
        .stream_events(&EventsStreamEventsParams::default())
        .await?;
    let mut n = 0;
    while let Some(event) = events.next().await {
        let event = event?;
        n += 1;
        println!("{} {} {}", event.occurred_at, event.type_, event.id);
    }
    println!("{n} events");
    Ok(())
}
