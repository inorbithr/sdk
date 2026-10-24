//! The account's events (scope `events:read`) over server-sent events: each event is typed and the stream ends when the server ends it.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin stream-sse
//! ```

use inorbithr::public::{EventsStreamEventsParams, Surface as _};
use inorbithr::{Client, Error, Streams};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::builder().streams(Streams::Sse).load()?; // reads INORBIT_*
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
