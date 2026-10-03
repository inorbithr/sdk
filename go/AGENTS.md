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
  codegen/               what generated surfaces import (Version, PathSegment)
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
- Doc comments start with the identifier and are full sentences.
- Never move or delete a released tag; retract instead (`docs/releasing.md`).
- Publishing stays off until the first release (M4).
