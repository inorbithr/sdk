# TypeScript package

`@inorbithr/sdk`, ESM only, Node 22.12 or newer, also Bun, Deno, browsers and Workers.
Read the root `AGENTS.md`, `docs/design.md` (section 12 above all) and ADR 0011 first;
this file adds only what is specific to TypeScript.

## Runtime and surface

The package is the hand-written **runtime** plus the **public surface** `iohr sdk generate
--lang typescript` writes into `src/generated/`. The runtime holds no operation; the
surface calls the runtime's one request path. The Rust crate (`rust/`) is the reference:
mirror its client, builder, token providers, errors, retries, hooks and `Int64`.

A generated surface has one class per profile (`AcmeCi`) with a handle per tag
(`client.radar.listDigests(...)`); a profile's class has only the operations its cut
holds, so a call it may not make is a type error. Plain JavaScript gets the same methods
without the check.

## Commands

- `mise run ts:check`: `biome ci`, `tsc --noEmit` (strict, `isolatedDeclarations`), the
  unit tests with `node --test`, and the build. The only dev dependencies are
  `typescript` and `@types/node`; tests use the built-in runner.
- `mise run ts:fmt`, `mise run ts:gen` (`iohr sdk generate` into `src/generated/`)
- `mise run conformance:ts`: the driver in `test/conformance/` against the replay server
- The generator's TypeScript target lives in `cli/crates/iohr-codegen/src/typescript/`;
  its golden files are `cli/crates/iohr-codegen/tests/golden/*/expected/typescript/` and
  `IOHR_TEST_COMPILE=typescript mise run cli:compile-test` type-checks a generated
  surface against this package and proves the wrong profile does not compile.
- Package manager: pnpm. Never commit a lockfile from another manager.

## Layout

```
typescript/
  package.json        "type": "module", "exports" with "types" first, "engines"
  tsconfig.json       strict, NodeNext, isolatedDeclarations
  src/
    index.ts          public exports only
    client.ts         Client, its options, load, fromEnv, ResolvedConfig, DefaultCredential
    settings.ts       resolution (docs/config.md sections 2 to 5): the catalogue, the config
                      file, the credential chain, describe()
    pipeline.ts       Pipeline, Middleware, SdkRequest, SdkResponse (config.md section 7)
    middleware.ts     the built-in middlewares, the retry budget, the fetch transport
    transport.ts      the node:https transport: proxy, CA bundle, mTLS, pinning, connect timeout
    proxy.ts, ratelimit.ts, telemetry.ts (logging, OpenTelemetry), platform.ts (Node built-ins)
    auth.ts           TokenProvider, CachedToken, client credentials, token file, iohr login, chains
    errors.ts         InOrbitError tree, Code, Detail, RawResponse
    stream.ts         the server-sent events parser and the /v1/ws socket (design.md section 7)
    retry.ts, hooks.ts, int64.ts, version.ts
    codegen.ts        what generated surfaces import (VERSION, pathSegment, the Int64 shapes); semver-tracked
    runtime.ts        the runtime alone, which a generated surface imports
    generated/        written by iohr; never edit (biome skips it; tsc checks it)
  test/
    conformance/      the driver for conformance/cases
    vectors.test.ts   conformance/vectors as unit tests
```

## Rules

- One runtime dependency, `smol-toml` (the config file, ADR 0015); otherwise global
  `fetch`, `AbortSignal`, `TextDecoder`. `@opentelemetry/api` is an optional peer,
  found with a dynamic import or passed as `opentelemetry`. Another dependency needs an
  ADR; dev dependencies are `typescript`, `@types/node`, `yaml` (the vectors) and
  `@opentelemetry/api` (the tracing case).
- No top-level await; no `node:` imports in `src/`, so the same build runs in browsers
  and Workers. Node's built-ins (files, `iohr`, `node:https` for proxy and TLS) are
  reached through `process.getBuiltinModule` in `platform.ts`, typed by local interfaces.
- `Client.load` is synchronous: it reads the environment and files at construction and
  contacts no host. Explicit construction and `fromEnv` keep their behaviour and messages.
- Built with `tsc` only, no bundler: `dist/` mirrors `src/`, with `.d.ts` files.
- `int64` values are `bigint` in the public types (`Int64` reads a decimal string).
- Errors are classes with a stable `name` and `code`; `instanceof` and `err.code` work.
- Every call takes an options bag with `signal` and `timeout`.
- Streams (design.md section 7): `Client.stream` is the one stream path, an async
  generator over server-sent events (fetch streaming, never `EventSource`, which cannot
  send the token) or, with `streams: "socket"`, over the client's one `/v1/ws`
  connection (the global `WebSocket` with an `Authorization` header: Node 22+, Deno,
  Bun). The WebSocket API hides pings and the refusal's status, so the socket has no
  idle clock of its own (the server closes a dead one) and a failed upgrade gets one
  fresh token, then the retry budget. It cannot pause the socket either: a reader 64
  items behind has its stream stopped with a `ConnectionError`.
- `toJSON` and `util.inspect` output of credentials redacts the secret; a test asserts it.
- `"private": true` in `package.json` keeps it off npm until the first release (M4).
