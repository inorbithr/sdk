# Rust crate

Crate `inorbithr`, edition 2024, MSRV 1.94 (`rust-version` in `Cargo.toml`). Read the
root `AGENTS.md` and `docs/design.md` first; this file adds only what is specific to Rust.

## Commands

- `mise run rust:check`: `cargo fmt --check`, `cargo clippy --all-targets --all-features
  -- -D warnings`, `cargo nextest run`, `cargo test --doc`, `cargo deny check`
- `mise run rust:fmt`, `mise run rust:gen` (typify into `src/generated/`)
- `mise run conformance:rust`
- API compatibility: `cargo semver-checks` against the last `rust/v` tag, in CI.
- MSRV: CI builds with 1.94 as well as stable.

## Layout (planned)

```
rust/
  Cargo.toml
  src/
    lib.rs          re-exports, crate docs with a compiling example
    client.rs       Client, ClientBuilder
    auth.rs         TokenProvider trait, client-credentials provider (single flight)
    error.rs        Error (thiserror, #[non_exhaustive]), ApiError, Code, Detail
    retry.rs
    accounts.rs, me.rs
    generated/      typify models; do not edit
  tests/
    conformance.rs  the driver for conformance/cases
```

## Rules

- Dependencies: `reqwest` 0.13 (rustls by default), `tokio`, `serde`, `serde_json`,
  `thiserror`, `url`, `time` (RFC 3339). Anything else needs a reason in the PR.
- Features: `default = ["rustls"]`, `native-tls`, `otel`; later `sse`, `ws`. Every
  feature combination in CI's `cargo hack --each-feature` must build.
- Clippy pedantic on, warnings are errors; `unwrap`/`expect` only in tests and examples;
  `unsafe` forbidden (`#![forbid(unsafe_code)]`).
- No runtime types leak into the public API beyond `tokio` futures being `Send`.
- Public enums and structs that can grow are `#[non_exhaustive]`.
- `Debug` on credentials and tokens is written by hand and redacts; a test asserts it.
- Every public fallible function documents `# Errors`; crate docs have one example that
  runs as a doctest against nothing (`no_run`).
