# InOrbit SDK for Rust

The `inorbithr` crate is the **runtime** of the Rust SDK: the client, its credentials,
retries, errors and hooks. The operations you call sit on top of it as a **surface**:
the public one this crate ships with, or one `iohr sdk generate` writes into your own
repository with exactly the operations your credentials may call (platform RFC 0020,
[ADR 0011](../docs/adr/0011-runtime-and-surface.md)).

Not on crates.io yet (`publish = false` until the generated public surface lands and the
first release is declared). Until then, use it by path:

```toml
[dependencies]
inorbithr = { git = "https://github.com/inorbithr/sdk", subdir = "rust" }
```

Edition 2024, Rust 1.94 or later.

## First call

Credentials come from the environment: an API token in `INORBIT_TOKEN` (made in the
console or with `iohr token create`), or an API key in `INORBIT_KEY_ID` and
`INORBIT_KEY_SECRET` with the scopes to ask for in `INORBIT_SCOPES`. The client exchanges
a key for a 15-minute token, caches it and refreshes it; a token is sent as it is.

```rust
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
```

That is [`examples/rust/src/bin/whoami.rs`](../examples/rust/src/bin/whoami.rs);
[`custom_token.rs`](../examples/rust/src/bin/custom_token.rs) beside it plugs in a
`TokenProvider` of its own and matches on error codes.

## What the client does

- **Retries** connection failures, timeouts and `429`, `503`, `504`, on idempotent
  calls only, 2 times by default, honouring `Retry-After` up to 60 s and otherwise
  backing off with jitter from 0.5 s to 8 s. A `401` gets one fresh token and one more
  attempt.
- **Errors** are one family, `Error`: `Api` carries the platform's error `code` (an enum
  with an `Unknown` fallback that keeps the slug), its `details` and the raw answer;
  `Connection`, `Timeout`, `Auth` and `Config` say what failed and what to do.
- **Secrets never print.** `Secret` redacts in `Debug` and `Display`, has no
  `Serialize`, and is zeroed on drop.
- **Raw access.** Every answer carries `raw`: status, headers, body and the request ids
  both ways; `Client::send` returns it alone, for a route the surface does not model.
- **Hooks** see every attempt (method, path, attempt number, request id, status), never
  a header, a query value or a body.
- Talks to `api.inorbit.hr` and `auth.inorbit.hr` only, over HTTPS (plain HTTP to
  loopback for a local stack), with no telemetry.

`docs/design.md` is the design every language follows; the shared conformance cases
run against this crate with `mise run conformance:rust`.

## The surface

Every public operation is on the client through `inorbithr::public`, generated into the
crate from the synced contract:

```rust
use inorbithr::public::Surface as _;
let digests = client.radar().list_digests(&Default::default()).await?;
let me = client.accounts().get_me().await?;
```

A surface for your own credentials, with exactly the operations they may call and a
compile-time refusal of the rest, comes from `iohr sdk generate` ([docs/design.md
section 12](../docs/design.md#12-runtime-and-surface)).

## Not here yet

- Server-sent events and the socket (`docs/design.md` section 7) come with the streaming
  milestone.
- `native-tls`, `ca_bundle`, `client_cert`, `proxy`, `pinned_keys` and `region` are
  designed but not built; the client uses rustls with the platform's trust roots and the
  system proxy settings.

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
