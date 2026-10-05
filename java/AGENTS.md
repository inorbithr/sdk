# Java package

Maven Central `hr.inorbit:inorbit-sdk`, module and package `hr.inorbit.sdk`, Java 17 or
newer (ADR 0013). Read the root `AGENTS.md`, `docs/design.md` (section 12 above all) and
ADR 0011 first; this file adds only what is specific to Java.

## Runtime and surface

The artifact is the hand-written **runtime** plus the **public surface** `iohr sdk
generate --lang java` writes into `src/main/java/hr/inorbit/sdk/generated/`. The runtime
holds no operation; the surface calls the runtime's one request path. The Rust crate
(`rust/`) is the reference: mirror its client, builder, token providers, errors,
retries, hooks and `Int64`.

A generated surface has one class per profile (`AcmeCi`) with a handle per tag
(`ci.radar().listDigests(...)`) and records for the models; a profile's class has only
the operations its cut holds, so a call it may not make does not compile. `--package`
names the surface's package.

## Commands

- `mise run java:check`: `mvn verify` (compile with `-Werror` at `--release 17`, tests)
- `mise run java:fmt`, `mise run java:gen` (`iohr sdk generate` into `generated/`)
- `mise run conformance:java`: the driver in `src/test/java/.../conformance/`
  (`mvn -Pconformance verify`)
- The generator's Java target lives in `cli/crates/iohr-codegen/src/java/`; golden files
  under `tests/golden/*/expected/java/`; `IOHR_TEST_COMPILE=java mise run
  cli:compile-test` compiles a generated surface against this artifact and proves the
  wrong profile does not.

## Layout

```
java/
  pom.xml                 hr.inorbit:inorbit-sdk, --release 17, -Werror
  src/main/java/hr/inorbit/sdk/
    Client.java           the client, its builder (build, load), the one request path
    Config.java           resolution (docs/config.md 2-5), a port of the Rust resolver; pure
    Engine.java           the twelve built-in middlewares and the transport
    Transport.java        HttpClient: NoProxy selector, SSLContext (CA, mTLS, pins)
    SdkLog.java, OtelTelemetry.java   logging (System.Logger) and optional OpenTelemetry
    middleware/           Middleware, Chain, Request, Response, CallInfo, Pipeline (public)
    auth/, errors/, ...   TokenProvider, ClientCredentials, TokenFile, CliToken, CachedToken,
                          ChainedCredential, DefaultCredential; ApiError, Code, Detail
    Int64.java, Hook.java
    codegen/              what generated surfaces import (VERSION, pathSegment)
    generated/            written by iohr; never edit
  src/test/java/hr/inorbit/sdk/
    conformance/          the driver for conformance/cases
    VectorsTest.java      conformance/vectors, read through `replay vectors` (no YAML reader)
```

## Rules

- Runtime dependencies: `java.net.http` (the JDK), Jackson databind, and
  `jackson-dataformat-toml` for the config file (ADR 0015). `opentelemetry-api` is an
  `optional` dependency: only `OtelTelemetry` (loaded when it is on the class path) and the
  builder's `tracerProvider`/`meterProvider` signatures name its types. A dependency needs an ADR.
- M6 (docs/config.md): `Client.load()` / `Builder.load(LoadOptions)` resolve through
  `Config`, `Builder.build()` resolves the same way with an empty environment and no file,
  keeping its old messages. Every call runs `Engine`'s pipeline; per-call options are a
  view (`withOptions`, `withTimeout`). A refused static token is an `AuthException` only
  under `load`. The socket upgrade cannot go through the pipeline (`java.net.http.WebSocket`
  takes no request); its reconnects draw from the retry budget.
- Synchronous calls return the answer; `CompletableFuture` variants sit beside them.
- Errors: `InOrbitException` base, `ApiException` with `code()`, `status()`,
  `details()`; connection, timeout, auth and config exceptions as subclasses. Unchecked.
- `int64` values are `long` through `Int64`; a message field the gateway left out is
  `null` (`Optional` in accessors where it reads better).
- `toString()` of credentials redacts the secret; a test asserts it.
- Streams (design.md section 7): `Client.stream` answers an `EventStream<T>`; `SseSource` parses
  server-sent events from a `Flow` subscriber that requests one chunk at a time (the bound),
  `SocketHub` runs every stream of a client on one `java.net.http.WebSocket` (reconnect and
  re-issue, cancel frames, a 64-item queue per call, the idle clock on `Streams.TIMER`). The
  generator gives a stream operation one method returning `EventStream<T>` (no async twin)
  and passes its parameters both as the query and as typed `field`s for the socket's call
  frame, with `rpc` from `x-iohr-rpc`.
- Javadoc on every public type and method.
- Publishing stays off until the first release (M4); Maven Central needs the
  `hr.inorbit` namespace verified by a DNS record first.
