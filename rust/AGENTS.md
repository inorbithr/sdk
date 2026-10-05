# Rust crate

Crate `inorbithr`, edition 2024, MSRV 1.94 (`rust-version` in `Cargo.toml`). Read the
root `AGENTS.md`, `docs/design.md` and ADR 0011 first; this file adds only what is
specific to Rust.

## Commands

- `mise run rust:check`: `cargo fmt --check`, `cargo clippy --all-targets --all-features
  -- -D warnings`, `cargo nextest run`, `cargo test --doc`, `cargo deny check`
- `mise run rust:fmt`; `mise run rust:gen` regenerates `src/generated/` (the public
  surface) with `iohr sdk generate` from `cli/`; CI fails on a diff.
- `mise run conformance:rust`: builds the replay server and runs every case through
  `tests/conformance.rs` (`IOHR_TEST_REQUIRE_REPLAY=1` makes a missing server a failure;
  without it the test prints a note and skips).
- `mise run rust:features`: every feature on its own (`cargo hack`).
- API compatibility: `cargo semver-checks` against the last `rust/v` tag, in CI.
- MSRV: CI builds with 1.94 as well as stable.

## Layout

```
rust/
  Cargo.toml        publish = false until the first release is declared
  build.rs          records the compiler's version for the user agent
  src/
    lib.rs          crate docs with a compiling example, re-exports, prelude, __codegen
    client.rs       Client<P: Profile>, ClientBuilder, Operation, Method, Response
    auth.rs         Token, TokenProvider, ClientCredentials (single flight), StaticToken
    error.rs        Error, ApiError, Code (17 + Unknown), Detail, AuthError, ConfigError
    retry.rs        the retry policy: Retry-After, full jitter
    hooks.rs        Attempt, Hook (on_retry)
    config/         load: the resolver (precedence, file, chain), LoadOptions, describe(), no_proxy
    middleware/     Pipeline, Middleware, Request/Response/Next, the twelve built-ins, logging
    transport.rs    the reqwest client and the /v1/ws upgrade: proxy, CONNECT, trust, mTLS, pins
    ratelimit.rs    RateLimit snapshots from X-RateLimit-* and the IETF fields
    otel.rs         spans and metrics (feature otel)
    profile.rs      Profile, Public, the INORBIT_<NAME>_ environment prefix
    stream.rs       EventStream<T>, Streams, the server-sent events parser and reader
    socket.rs       the /v1/ws socket task: one connection, calls by id, reconnect and re-issue
    secret.rs       Secret<T>: redacted, not Serialize, zeroed on drop
    int64.rs        Int64: a decimal string on the wire
    generated/      the public surface, written by `iohr sdk generate`; do not edit
  tests/conformance.rs   the driver for conformance/cases and conformance/vectors
examples/rust/      programs CI compiles and the README quotes
```

## Rules

- The runtime holds no operation. Operations are generated: the public surface into
  `src/generated/`, a developer's into their repository. A new route is a spec sync and
  a regeneration, never a hand-written method.
- Dependencies: the table in `README.md`. Anything else needs a reason there and in
  the PR. No runtime type leaks into the public API beyond `tokio` futures being `Send`;
  headers are `Headers`, methods are `Method`.
- Features: `default = ["rustls", "tracing"]`; `otel` and `encrypted-key` are opt-in. Every feature combination in `rust:features` must
  build; without `rustls` the client reaches loopback only.
- Clippy pedantic on, warnings are errors; `unwrap`/`expect` only in tests and examples;
  `unsafe` forbidden (`#![forbid(unsafe_code)]`).
- Public enums and structs that can grow are `#[non_exhaustive]`; large error variants
  are boxed.
- `Debug` on credentials and tokens is written by hand and redacts; a test asserts it.
- Every public fallible function documents `# Errors`; crate docs have one example that
  runs as a doctest against nothing (`no_run`).
- `__codegen::VERSION` is the contract with generated surfaces: bump it when a surface
  written for the previous runtime would not compile or would behave differently.
- Streams (design.md section 7): a reader task feeds a bounded channel (64 items) the
  `EventStream` drains, so a slow caller holds back its own stream; dropping the stream
  aborts the SSE reader or sends the socket a `cancel`. The socket task owns the
  connection and the table of calls; it closes the socket when the last call ends.
- The conformance driver calls the public API only (`Client::send` and the generated
  surface), never internals.
