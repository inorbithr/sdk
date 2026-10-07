# InOrbit for Swift

The Swift package of the [InOrbit API](https://docs.inorbit.hr): the client for your iOS,
macOS, tvOS, watchOS, visionOS and server-side Swift apps. `async`/`await`, `Codable`
models, `URLSession` (FoundationNetworking on Linux), and no other dependency.

Status: the first runtime, not released yet. It passes the shared conformance cases for
authentication, errors, operations, retries and server-sent events (29 of 52). Configuration
files, the middleware pipeline, the TLS and proxy options and the `/v1/ws` socket come
next ([#172](https://github.com/inorbithr/sdk/issues/172)).

## Install

The package is at the root of this repository, so SwiftPM takes the repository URL
([ADR 0016](../docs/adr/0016-swift.md)). In `Package.swift`:

```swift
dependencies: [
    .package(url: "https://github.com/inorbithr/sdk", branch: "main"),
],
targets: [
    .target(name: "MyApp", dependencies: [.product(name: "InOrbit", package: "sdk")]),
]
```

In Xcode: File, Add Package Dependencies, `https://github.com/inorbithr/sdk`, product
`InOrbit`. Once release-please tags the first Swift version (`vX.Y.Z`), depend on
`from:` that version instead of `branch: "main"`.

Minimum: Swift 5.9 tools (Xcode 15), iOS 15, macOS 12, tvOS 15, watchOS 8, visionOS 1, or
Linux with Swift 5.9.

## First call

From [`examples/swift/Sources/Whoami/main.swift`](../examples/swift/Sources/Whoami/main.swift):

```swift
import InOrbit

let api = try Client<Public>.fromEnv()
do {
    let me = try await api.me().value
    print("\(me.subject) (\(me.kind)), scopes: \(me.scopes.joined(separator: " "))")
} catch let e as APIError {
    print("\(e.code): \(e.problem) (request id \(e.raw.requestId))")
}
```

`Client<Public>.fromEnv()` reads `INORBIT_TOKEN`, or `INORBIT_KEY_ID`,
`INORBIT_KEY_SECRET` and `INORBIT_SCOPES`, and `INORBIT_BASE_URL` when set. From code,
`Client<Public>(ClientOptions(...))` takes a `token`, a key (`keyId`, `keySecret`,
`scopes`), or a `tokenProvider` of your own: a type with one method,
`token(refresh:) async throws -> Secret`, for a token from a PKCE sign-in or from your
backend. Do not ship an API key secret inside an app.

## Pages and streams

From [`examples/swift/Sources/ListMonitors/main.swift`](../examples/swift/Sources/ListMonitors/main.swift):

```swift
var first: String?
for try await monitor in api.connections.allListMonitors(orgId: org) {
    print(monitor.monitorId, monitor.name)
    first = first ?? monitor.monitorId
}
if let first {
    let runs = try await api.connections.listMonitorRuns(orgId: org, monitorId: first).value
    print("latest runs of \(first): \(runs.runs.count)")
}
```

Operations are grouped by area (`api.connections`, `api.radar`, `api.events`, ...). Each
answers a `Response` with the typed `value` and the `raw` answer beside it. A paged list
also has `all<Operation>`, an `AsyncThrowingStream` of every item, page after page. A
stream, `api.events.streamEvents(...)`, is an `AsyncThrowingStream` of the account's events
over server-sent events; it opens on the first step of the loop. Breaking out of a loop,
or cancelling the task, stops the walk or closes the stream.

## Errors and retries

Every error conforms to `InOrbitError` and has a `kind` (`api`, `connection`, `timeout`,
`auth`, `config`, `too_large`, `decode`). The API's own errors are `APIError`, with the
platform's `code` (a code this version does not know is kept), `status`, `details` and the
request id.

Retried for you: connection failures, attempt timeouts, `429`, `503` and `504`, honouring
`Retry-After` (at most 60 s), up to `maxRetries` (2 by default). A write is retried only
when the operation takes an `Idempotency-Key`, which the package sends once per call and
repeats on every attempt. A `401` fetches a fresh token once. Messages never hold a
secret, and `Secret` prints as `<redacted>`.

## A client cut to your account

`iohr sdk generate --lang swift --for <profile> --out Sources/MyApp/IOHR` writes a surface
with only the operations your credential may call, as a profile type
(`Client<MyProfile>`): a call the profile may not make does not compile. Commit it with
`iohr.lock` and run `iohr sdk check` in CI.

## Developing

`mise run swift:check` (format lint, a build with complete concurrency checking and
warnings as errors, the unit tests), `mise run conformance:swift`, `mise run swift:gen`.
[AGENTS.md](AGENTS.md) has the layout and the rules.
