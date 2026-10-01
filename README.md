# InOrbit SDK

Official client libraries for the [InOrbit API](https://docs.inorbit.hr) in Go, Rust,
TypeScript and Python.

> **Status: design phase.** Nothing is published yet. This repository holds the design,
> the contract and the shared test cases the four SDKs are built against. Follow along,
> or open an issue with what you would use the SDK for.

| Language | Package | Minimum runtime | Status |
|---|---|---|---|
| Go | `github.com/inorbithr/sdk/go` | Go 1.26 | not released |
| Rust | [`inorbithr`](https://crates.io/crates/inorbithr) | Rust 1.94 | not released |
| TypeScript | [`@inorbithr/sdk`](https://www.npmjs.com/package/@inorbithr/sdk) | Node 22.12, Bun, Deno, browsers | not released |
| Python | [`inorbithr`](https://pypi.org/project/inorbithr/) | Python 3.11 | not released |

## What the SDKs will do

- Exchange your API key for a short-lived token, cache it and refresh it, so you never
  handle tokens yourself.
- Return typed results and one error type per language, carrying the API's error `code`
  and `details`.
- Retry what is safe to retry (`429`, `503`, `504`, connection failures), honouring
  `Retry-After`.
- Behave the same in every language: one set of [conformance cases](conformance/) runs
  against all four.

Today an API key can read its own identity, its account, and its account's usage.
Streaming (server-sent events and the WebSocket) follows when the API opens it to keys.
See [docs/design.md](docs/design.md) for the full design.

## Repository layout

| Path | Contents |
|---|---|
| [`spec/`](spec/) | The API contract, synced from the platform |
| [`conformance/`](conformance/) | Shared behaviour cases every SDK must pass |
| [`go/`](go/), [`rust/`](rust/), [`typescript/`](typescript/), [`python/`](python/) | One package per language |
| [`examples/`](examples/) | Small programs that CI compiles |
| [`docs/`](docs/) | Design, decisions (ADRs), releasing, style |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## License

[Apache License 2.0](LICENSE).
