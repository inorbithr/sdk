# Go package

Module `github.com/inorbithr/sdk/go`, package `inorbit`, Go 1.26 or newer. Read the root
`AGENTS.md`, `docs/design.md` (section 12 above all) and ADR 0011 first; this file adds
only what is specific to Go.

## Runtime and surface

The module is the hand-written **runtime** plus the **public surface** `iohr sdk generate
--lang go` writes into `public/` (package `public`). The runtime holds no operation; the
surface calls the runtime's one request path. The Rust crate (`rust/`) is the
reference: mirror its client, options, token providers, errors, retries, hooks and
`Int64`.

A generated surface is one package per profile (`--package` names the parent) wrapping
the runtime's client, with a handle per tag (`ci.Radar().ListDigests(ctx, ...)`); a
profile's package has only the operations its cut holds, so a call it may not make does
not build. The models are shared in one package.

## Commands

- `mise run go:check`: `gofmt -l`, `go vet`, `golangci-lint run`, `go test -race ./...`
- `mise run go:fmt`, `mise run go:gen` (`iohr sdk generate` into `public/`)
- `mise run conformance:go`: the driver in `internal/conformance/`
- The generator's Go target lives in `cli/crates/iohr-codegen/src/go/`; golden files
  under `tests/golden/*/expected/go/`; `IOHR_TEST_COMPILE=go mise run cli:compile-test`
  builds a generated surface against this module and proves the wrong profile does not.
- API compatibility: `gorelease -base=<last go/v tag>` runs in CI on every PR.

## Layout

```
go/
  go.mod                 module github.com/inorbithr/sdk/go
  client.go              Client, NewClient, functional options, the one request path
  auth.go                TokenProvider, client credentials (single flight), static token
  errors.go              APIError, Code, Detail; errors.As targets
  retry.go, hooks.go, int64.go
  stream.go              Stream, server-sent events (parser, idle timeout), the GET that opens a stream
  socket.go, ws.go       the /v1/ws socket: one per client, reconnect and re-issue; a small RFC 6455 client
  codegen/               what generated surfaces import (Version, PathSegment, Pages)
  slog.go                SlogHook: log/slog records per attempt, nothing a call carries
  public/                written by iohr; never edit
  internal/              transport details, the conformance driver
```

## Rules

- Standard library only at run time (`net/http`, `encoding/json`). A third-party import
  in the main module needs an ADR.
- `ctx context.Context` is the first argument of every call that does I/O; honour its
  deadline over the retry budget.
- Options are functional (`inorbit.WithBaseURL(...)`); `Client` is safe for concurrent
  use and holds no per-call state.
- Errors: return `*APIError` (wrapping is fine) so callers use `errors.As`. Never
  `panic` outside programmer errors at construction.
- `String()` and `GoString()` on credentials redact the secret; a test asserts it.
- Doc comments start with the identifier and are full sentences. Usage lives in
  `Example` functions (`example_test.go`), compile-only when they need the network.
- Tests of anything timed (retries, `Retry-After`, backoff, timeouts, token lifetime,
  single flight) run in a `testing/synctest` bubble with an in-memory transport, so a
  minute's wait is exact and instant; a real socket would stop the bubble's clock.
- `codegen` is a contract with `iohr`: adding what new surfaces need adds a version
  constant (`V2` added `Pages`, `V3` streams: `Stream`, `Operation.RPC` and `Fields`)
  and keeps the old ones, so an old surface still builds.
- Streams follow docs/design.md section 7 and the `sse/` and `socket/` conformance
  cases. The WebSocket client is ours (`ws.go`, RFC 6455, client side only): the
  upgrade goes through the caller's `http.Client`, so its proxy and TLS settings hold,
  and the module stays on the standard library.
- A hook never sees or logs a header, body, query value or token; `SlogHook` logs the
  operation's name, not the bound path.
- Never move or delete a released tag; retract instead (`docs/releasing.md`).
- Publishing stays off until the first release (M4).
