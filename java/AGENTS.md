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
    Client.java           the client, its builder, the one request path
    auth/, errors/, ...   TokenProvider, client credentials (single flight); ApiError, Code, Detail
    Int64.java, Hook.java
    codegen/              what generated surfaces import (VERSION, pathSegment)
    generated/            written by iohr; never edit
  src/test/java/hr/inorbit/sdk/
    conformance/          the driver for conformance/cases
```

## Rules

- Runtime dependencies: `java.net.http` (the JDK) and Jackson databind only. A
  dependency needs an ADR.
- Synchronous calls return the answer; `CompletableFuture` variants sit beside them.
- Errors: `InOrbitException` base, `ApiException` with `code()`, `status()`,
  `details()`; connection, timeout, auth and config exceptions as subclasses. Unchecked.
- `int64` values are `long` through `Int64`; a message field the gateway left out is
  `null` (`Optional` in accessors where it reads better).
- `toString()` of credentials redacts the secret; a test asserts it.
- Javadoc on every public type and method.
- Publishing stays off until the first release (M4); Maven Central needs the
  `hr.inorbit` namespace verified by a DNS record first.
