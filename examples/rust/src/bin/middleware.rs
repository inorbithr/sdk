//! The pipeline (docs/config.md section 7): a middleware of your own at the per-call
//! slot, a built-in switched off by name, and an idempotency key passed with one call.
//!
//! ```sh
//! INORBIT_TOKEN=... cargo run --bin middleware
//! ```

use std::time::Instant;

use inorbithr::middleware::{BoxFuture, CallOptions, Middleware, Next, Request, Response};
use inorbithr::public::{CreateEndpointRequest, Surface as _};
use inorbithr::{Client, Error};

/// Tags every call with the team that makes it, and times it, retries included.
struct Team(&'static str);

impl Middleware for Team {
    fn name(&self) -> &'static str {
        "team"
    }

    fn handle<'a>(
        &'a self,
        mut req: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response, Error>> {
        req.headers_mut().insert("x-team", self.0);
        Box::pin(async move {
            let operation = req.info().operation();
            let started = Instant::now();
            let result = next.run(req).await;
            eprintln!("{operation} took {:?}", started.elapsed());
            result
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let client: Client = Client::builder()
        .pipeline(|p| p.add_per_call(Team("payments")).remove("rate_limit"))
        .load()?;
    println!("pipeline {}", client.config().describe()["pipeline"]);

    // The same key on every attempt, so the API answers a repeat with the first result.
    let body = CreateEndpointRequest {
        url: Some("https://hooks.example.com/inorbit".into()),
        ..Default::default()
    };
    let created = client
        .with_options(CallOptions::new().idempotency_key("endpoint-hooks-example"))
        .events()
        .create_endpoint(&body)
        .await?;
    println!(
        "created, key {:?}, replayed {}",
        created.raw.idempotency_key, created.raw.idempotency_replayed
    );
    Ok(())
}
