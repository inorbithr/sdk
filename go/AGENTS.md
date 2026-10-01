# Go package

Module `github.com/inorbithr/sdk/go`, package `inorbit`, Go 1.26 or newer. Read the root
`AGENTS.md` and `docs/design.md` first; this file adds only what is specific to Go.

## Commands

- `mise run go:check`: `gofmt -l`, `go vet`, `golangci-lint run`, `go test -race ./...`
- `mise run go:fmt`, `mise run go:gen` (oapi-codegen into `generated/`)
- `mise run conformance:go`
- API compatibility: `gorelease -base=<last go/v tag>` runs in CI on every PR.

## Layout (planned)

```
go/
  go.mod                 module github.com/inorbithr/sdk/go
  client.go              Client, NewClient, functional options
  auth.go                TokenProvider, client-credentials provider (single flight)
  errors.go              APIError, Code, Detail types; errors.As targets
  retry.go               backoff, Retry-After
  accounts.go, me.go     one file per API area, thin methods over the transport
  generated/             oapi-codegen models only; do not edit
  internal/              transport, user agent, conformance driver
  otel/                  optional OpenTelemetry hooks, its own go.mod (submodule)
```

## Rules

- Standard library only at run time (`net/http`, `encoding/json`). A third-party import
  in the main module needs an ADR.
- `ctx context.Context` is the first argument of every call that does I/O; honour its
  deadline over the retry budget.
- Options are functional (`inorbit.WithBaseURL(...)`); `Client` is safe for concurrent
  use and holds no per-call state.
- Errors: return `*APIError` (wrapping is fine) so callers use `errors.As`; sentinel
  values only for `ErrNotImplemented`. Never `panic` outside `init`-time programmer
  errors.
- `int64` fields from the wire are strings; generated models carry `string`, the
  public types expose `int64` and convert at the boundary.
- `String()` and `GoString()` on credentials redact the secret; a test asserts it.
- Doc comments start with the identifier and are full sentences; every exported
  identifier has one.
- A nested module (`otel/`) is versioned and tagged separately (`go/otel/vX.Y.Z`).
- Never move or delete a released tag; retract instead (`docs/releasing.md`).
