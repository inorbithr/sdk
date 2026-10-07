# Swift package

SwiftPM package `InOrbit` (product `InOrbit`, `import InOrbit`), Swift 5.9 tools, iOS 15,
macOS 12, tvOS 15, watchOS 8, visionOS 1 and Linux ([ADR 0016](../docs/adr/0016-swift.md)).
Read the root `AGENTS.md`, `docs/design.md` (section 12 above all) and ADR 0011 first;
this file adds only what is specific to Swift.

## Runtime and surface

`Package.swift` is at the repository root, because SwiftPM reads it from there; the code
is under `swift/`. The package is the hand-written **runtime** plus the **public surface**
`iohr sdk generate --lang swift` writes into `Sources/InOrbit/Generated/`. The runtime
holds no operation; the surface calls `Client.request` and `Client.stream` with a
`Codegen.Operation`.

A generated surface follows the Rust and C# shape: one marker protocol per operation
(`AllowsListDigests`) and one per area (`AllowsRadarArea`), adopted by the profile types
whose cut holds them, a handle per tag (`RadarHandle<P>`), and the operations in
extensions constrained to the marker, so a call a profile may not make does not compile.
The runtime's own surface adds its markers to the runtime's `Public`. Generated files
name every runtime type by module (`InOrbit.Client`), so a model of the same name cannot
shadow it; runtime types therefore must not take a name the API uses for a model
(`Settings`, `Key`, `Value`, `Field`, ... the runtime's internal settings type is
`EnvSettings` for that reason). `Codegen.Version1` is checked at compile time.

Models spell out `Codable`: properties `lowerCamel`, keys the wire names, a decimal-string
64-bit integer through `WireInt64` (an `Int64` everywhere else), an optional left out of
the body when `nil`, unknown fields ignored. String enums keep a value this version does
not know; an inline enum is a `String`.

## Commands

- `mise run swift:check`: `swift format lint --strict` (config `swift/.swift-format`),
  `swift build` with complete strict concurrency and warnings as errors, the unit tests
- `mise run swift:fmt`, `mise run swift:gen` (`iohr sdk generate` into `Generated/`)
- `mise run conformance:swift`: the driver, `swift test --filter ConformanceTests`
- `xcodebuild -scheme InOrbit -destination 'generic/platform=iOS' build` for an Apple
  device build; Linux: `docker run --rm -v "$PWD":/pkg -w /pkg swift:6.1 swift test`
- The generator's target is `cli/crates/iohr-codegen/src/swift/`; golden files under
  `tests/golden/*/expected/swift/`; `IOHR_TEST_COMPILE=swift mise run cli:compile-test`
  builds a generated surface against this package and proves the wrong profile does not.

## Layout

```
Package.swift                  (repository root) targets swift/Sources/InOrbit and swift/Tests/InOrbitTests
swift/
  .swift-format                4 spaces, 120 columns
  Sources/InOrbit/
    Client.swift               Client<P>, fromEnv, EnvSettings, Core: the request path, retries, the 401 refresh
    Stream.swift               streams (design.md section 7): the lazy stream, EventStreamParser, the idle guard
    Auth.swift                 TokenProvider, StaticTokenProvider, ClientCredentialsProvider (cache, single flight)
    Transport.swift            HTTPTransport, URLSessionTransport (ephemeral, TLS 1.2, no redirects)
    Errors.swift               InOrbitError, APIError, Code, Detail, the other error kinds
    Retry.swift                RetryPolicy, withTimeout
    Codegen.swift              what surfaces call: Operation, pathSegment, queryValues, json, pages
    Options.swift, Profile.swift, Response.swift, Secret.swift, JSONValue.swift, Wire.swift, Version.swift
    Generated/                 written by iohr; never edit
  Tests/InOrbitTests/
    ConformanceTests.swift     the driver for conformance/cases
    RuntimeTests.swift         redaction, wire, errors, the SSE parser, pages
```

## Rules

- No dependency beyond Foundation (and FoundationNetworking on Linux). A new one needs a
  reason in the PR and in ADR 0016.
- Everything public is `Sendable`, and the package builds clean with
  `-strict-concurrency=complete -warnings-as-errors`.
- Credentials are `Secret` from the moment they are read; `reveal()` only where a header is
  built. A test prints every new way a value can be formatted.
- `URLSessionTransport` keeps TLS verification on, never follows a redirect, and stores
  nothing on disk. A custom `HTTPTransport` must do the same [SR-01, SR-02].
- A conformance case this runtime does not pass yet is `pending: [swift]` with the parity
  issue in `notes`; never weaken the case.
- Commit scope `swift`.
