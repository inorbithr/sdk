# InOrbit SDK for Rust

The `inorbithr` crate is the **runtime** of the Rust SDK: the client, its credentials,
retries, errors and hooks. The operations you call sit on top of it as a **surface**:
the public one this crate ships with, or one `iohr sdk generate` writes into your own
repository with exactly the operations your credentials may call (platform RFC 0020,
[ADR 0011](../docs/adr/0011-runtime-and-surface.md)).

On [crates.io](https://crates.io/crates/inorbithr) since 0.1.0:

```sh
cargo add inorbithr
```

Edition 2024, Rust 1.94 or later.

## First call

`Client::load()` finds its configuration the way the other five SDKs do: code first,
then `INORBIT_*` variables, then the config file `iohr login` writes, then defaults, each
setting on its own ([docs/config.md](../docs/config.md)). Credentials come from the first
source that has any: code, the environment (`INORBIT_TOKEN`, `INORBIT_TOKEN_FILE`, or
`INORBIT_KEY_ID` with `INORBIT_KEY_SECRET` or `INORBIT_KEY_SECRET_FILE` and
`INORBIT_SCOPES`), the config file's profile, and last your `iohr login`. A developer who
has signed in with `iohr` needs nothing else.

```rust
use std::time::Duration;

use inorbithr::public::Surface as _;
use inorbithr::{Client, Error};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    // An option set in code always wins over the environment and the file.
    let client: Client = Client::builder()
        .user_agent_suffix("load-example/1.0")
        .total_timeout(Duration::from_secs(60))
        .load()?;
    let described = client.config().describe();
    println!("credential from {}", described["credential"]["source"]);
    let me = client.me().await?;
    println!("{} ({} attempt), request id {}", me.value.subject, me.raw.attempts, me.raw.request_id);
    Ok(())
}
```

That is [`examples/rust/src/bin/load.rs`](../examples/rust/src/bin/load.rs).
`client.config().describe()` (or `iohr sdk config` on the command line) says what the
client uses and where each value came from, with secrets redacted; a configuration that
cannot work fails `load` with one `ConfigError::Invalid` listing every problem.

Two other ways to build a client stay as they were:
`Client::builder()...build()` reads only what you set in code (for libraries and tests),
and `Client::from_env()` reads the six variables it always read; `load` supersedes it.
[`whoami.rs`](../examples/rust/src/bin/whoami.rs) uses `from_env`, and
[`custom_token.rs`](../examples/rust/src/bin/custom_token.rs) plugs in a `TokenProvider`
of its own and matches on error codes.

Recipes for CI, Kubernetes, a corporate proxy, a private CA, mTLS, serverless and
OpenTelemetry are in [docs/recipes.md](../docs/recipes.md).

## What the client does

- **Credentials**: an API key is exchanged for a 15-minute token, cached, refreshed when
  a fifth of its life is left (one refresh at a time, the old token kept if a refresh
  fails); a secret file is read before every exchange and a token file again when it
  changes, so rotated Kubernetes Secrets need no restart. `TokenFile`, `CliToken`,
  `CachedToken` and `ChainedCredential` are public for a chain of your own.
- **Retries** connection failures, timeouts and `429`, `503`, `504`, on idempotent calls
  and on writes that take an `Idempotency-Key` (a key per call, the same on every
  attempt), 2 times by default, honouring `Retry-After` (seconds or a date) up to 60 s and
  otherwise backing off with jitter from 0.5 s to 8 s. A retry budget per client turns a
  storm of retries into fast failures. A `401` gets one fresh token and one more attempt.
- **Timeouts**: 10 s to connect, 30 s per attempt (the whole body included), 120 s per
  call with every retry and wait, 45 s of silence on a stream.
- **Transport**: `HTTPS_PROXY` and `NO_PROXY` (lower case first) or `INORBIT_PROXY`, a CA
  bundle added to the system's trust, mTLS with a client certificate that may rotate,
  pinned keys, or your own `reqwest::Client` with `http_client`. The `/v1/ws` socket uses
  the same proxy and TLS settings.
- **Errors** are one family, `Error`: `Api` carries the platform's error `code` (an enum
  with an `Unknown` fallback that keeps the slug), its `details` and the raw answer;
  `Connection`, `Timeout`, `Auth` and `Config` say what failed and what to do.
- **Secrets never print.** `Secret` redacts in `Debug` and `Display`, has no
  `Serialize`, and is zeroed on drop. The config file may not hold one.
- **Raw access.** Every answer carries `raw`: status, headers, body, the request ids both
  ways, the idempotency key and the rate-limit snapshot; `Client::send` returns it alone,
  for a route the surface does not model.
- **Logging and tracing**: off until `INORBIT_LOG` (or `.log(..)`) is set; records go to
  `tracing` events with target `inorbithr`, or to your `logger`, and never carry a body, a
  query value, a credential or a header outside the allowlist. With the `otel` feature
  each call is an OpenTelemetry span with one HTTP client span per attempt.
- Talks to `api.inorbit.hr` and `auth.inorbit.hr` only, over HTTPS (plain HTTP to
  loopback for a local stack), with no telemetry.

`docs/design.md` is the design every language follows; the shared conformance cases
and vectors run against this crate with `mise run conformance:rust`.

## The pipeline

Every call goes through named middlewares, outermost first: `request_id`, `user_agent`,
`idempotency_key`, `call_tracing`, `deadline`, then `retry`, then on every attempt
`auth`, `rate_limit`, `attempt_tracing`, `logging`, `hooks` and `timeout`
([docs/config.md section 7](../docs/config.md#7-the-middleware-pipeline)). A middleware of
your own implements `Middleware` and goes in by name: `add_per_call` (once per call,
before `retry`), `add_per_retry` (on every attempt, before `timeout`), `insert_before`,
`insert_after`, `replace` or `remove`. `retry`, `auth` and `timeout` can be replaced but
not removed.

```rust
use std::time::Instant;

use inorbithr::middleware::{BoxFuture, CallOptions, Middleware, Next, Request, Response};
use inorbithr::{Client, Error};

struct Team(&'static str);

impl Middleware for Team {
    fn name(&self) -> &'static str {
        "team"
    }

    fn handle<'a>(&'a self, mut req: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response, Error>> {
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

let client: Client = Client::builder()
    .pipeline(|p| p.add_per_call(Team("payments")).remove("rate_limit"))
    .load()?;
```

Per-call options (a timeout, an idempotency key, a `traceparent`) go on a copy of the
client: `client.with_options(CallOptions::new().idempotency_key("order-42")).events().create_endpoint(&body)`.
[`middleware.rs`](../examples/rust/src/bin/middleware.rs) is the complete program. A
middleware of your own must not log secrets or bodies; the built-ins never do.

## The surface

Every public operation is on the client through `inorbithr::public`, generated into the
crate from the synced contract:

```rust
use inorbithr::public::Surface as _;
let digests = client.radar().list_digests(&Default::default()).await?;
let me = client.accounts().get_me().await?;
```

A paged list has an `all_<operation>` beside its page method that follows the next-page
token; the pager is the runtime's own `Pages`, so no `futures` dependency:

```rust
let mut digests = client.radar().all_list_digests(&Default::default());
while let Some(digest) = digests.next().await {
    let digest = digest?;
}
```

Stopping early fetches nothing more; `.collect().await` gathers every item.

Every request field is optional and left out of the body when unset, so a request is
built from its default (`CreateEndpointRequest { url: Some(url.into()), ..Default::default() }`);
an answer's field is an `Option` unless the API always sends it. Timestamps stay the
strings the API sent; `inorbithr::parse_timestamp` reads one, `""` (unset) as `None`.

## Streams

A streaming operation (today the account's events, `events.stream_events`, scope
`events:read`) answers an `EventStream<T>`: a `futures_core::Stream` of `Result<T, Error>`
with an inherent `next().await` ([docs/design.md section 7](../docs/design.md#7-streaming)):

```rust
use inorbithr::public::{EventsStreamEventsParams, Surface as _};
let mut events = client.events().stream_events(&EventsStreamEventsParams::default()).await?;
while let Some(event) = events.next().await {
    let event = event?;
    println!("{} {}", event.type_, event.id);
}
```

The `.await` opens the stream with the rules of any `GET` (a fresh token after a `401`,
retries after `429`, `503`, `504`); errors after that arrive as the stream's last item.
Dropping the stream closes it. By default each stream is server-sent events;
`Client::builder().streams(Streams::Socket)` sends every stream of the client over one
`/v1/ws` connection, which the client reconnects, and on which it issues again every
call that had not ended, when the server ends the socket or it goes silent. A stream
silent for `stream_idle_timeout` (45 s; the server sends a keep-alive every 15 s) fails
with `Error::Timeout` over server-sent events. A revoked key ends the stream with
`Error::Api` and code `unauthenticated`. `examples/rust/src/bin/stream_events.rs` is a
complete program.

A surface for your own credentials, with exactly the operations they may call and a
compile-time refusal of the rest, comes from `iohr sdk generate` ([docs/design.md
section 12](../docs/design.md#12-runtime-and-surface)).

## Features

| Feature | Default | What it adds |
|---|---|---|
| `rustls` | on | HTTPS through rustls with the platform's trust roots; without it, plain-HTTP loopback only |
| `tracing` | on | log records as `tracing` events, target `inorbithr` |
| `otel` | off | OpenTelemetry spans and metrics through the `opentelemetry` API crate, from the global providers or `tracer_provider` / `meter_provider` |
| `encrypted-key` | off | a client key encrypted with `client_key_password` (PKCS#8) |

## Not here yet

- `native-tls` and `region` are designed but not built; workload identity is reserved
  until the platform can exchange an outside token ([docs/config.md section 5.5](../docs/config.md#55-what-the-platform-accepts-today)).
- A `tower` adapter for the pipeline can follow behind a feature; today a `tower` service
  is wrapped in a `Middleware` by hand.

## Dependencies

Each runtime dependency, and why (SR-20):

| Crate | Reason |
|---|---|
| reqwest (rustls) | HTTPS without OpenSSL |
| tokio | timers for backoff and the single-flight token exchange |
| serde, serde_json | the wire format |
| thiserror | typed errors |
| url | URL checks and joins |
| time | RFC 3339 timestamps in models |
| getrandom | request ids and retry jitter |
| zeroize | secrets wiped on drop |
| futures-core, futures-util | the `Stream` trait an `EventStream` implements, and the socket's stream and sink combinators (both already come with reqwest) |
| tokio-tungstenite | the `/v1/ws` socket's WebSocket protocol, over the crate's own TCP and TLS (no TLS feature of its own) |
| rustls, tokio-rustls, rustls-platform-verifier (feature `rustls`) | the socket's TLS: the same provider and platform verifier reqwest uses |
| rustls-webpki (feature `rustls`) | the server key a pin is checked against, with the parser rustls already uses |
| toml | the config file the `iohr` command line shares (docs/config.md section 4) |
| base64 | pinned keys, a JWT's expiry, proxy credentials (already a dependency of reqwest) |
| tracing (feature `tracing`, default) | log records as `tracing` events |
| opentelemetry (feature `otel`) | spans and metrics, the API crate only |
| pkcs8 (feature `encrypted-key`) | decrypting a password-protected client key |
