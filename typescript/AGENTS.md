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

- `mise run ts:check`: `biome ci`, `tsc --noEmit`, `vitest run`, `publint`
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
    client.ts         Client, the builder options, the one request path
    auth.ts           TokenProvider, client credentials (single flight), static token
    errors.ts         InOrbitError tree, Code, Detail
    retry.ts, hooks.ts, int64.ts
    codegen.ts        what generated surfaces import (VERSION, pathSegment); semver-tracked
    generated/        written by iohr; never edit
  test/
    conformance/      the driver for conformance/cases
```

## Rules

- Zero runtime dependencies: global `fetch`, `AbortSignal`, `TextDecoder`. A dependency
  needs an ADR.
- No top-level await; no Node built-ins in `src/` (`node:` imports), so the same build
  runs in browsers and Workers.
- Built with `tsc` only, no bundler: `dist/` mirrors `src/`, with `.d.ts` files.
- `int64` values are `bigint` in the public types (`Int64` reads a decimal string).
- Errors are classes with a stable `name` and `code`; `instanceof` and `err.code` work.
- Every call takes an options bag with `signal` and `timeout`.
- `toJSON` and `util.inspect` output of credentials redacts the secret; a test asserts it.
- Publishing stays off until the first release (M4).
