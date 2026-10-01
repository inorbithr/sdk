# TypeScript package

`@inorbithr/sdk`, ESM only, Node 22.12 or newer, also Bun, Deno, browsers and Workers.
Read the root `AGENTS.md` and `docs/design.md` first; this file adds only what is
specific to TypeScript.

## Commands

- `mise run ts:check`: `biome ci`, `tsc --noEmit`, `vitest run`, `publint`,
  `attw --pack`, `api-extractor run` (fails on an unreported API change)
- `mise run ts:fmt`, `mise run ts:gen` (openapi-typescript into `src/generated/`)
- `mise run conformance:ts`
- Package manager: pnpm. Never commit a lockfile from another manager.

## Layout (planned)

```
typescript/
  package.json        "type": "module", "exports" with "types" first, "engines"
  tsconfig.json       strict, NodeNext, isolatedDeclarations
  src/
    index.ts          public exports only
    client.ts         InOrbit class and options
    auth.ts           TokenProvider, client-credentials provider (single flight)
    errors.ts         InOrbitError tree, Code, Detail
    retry.ts
    resources/        accounts.ts, me.ts
    generated/        openapi-typescript types; do not edit
  test/
    conformance/      the driver for conformance/cases
  etc/sdk.api.md      api-extractor report, committed
```

## Rules

- Zero runtime dependencies: global `fetch`, `AbortSignal`, `TextDecoder`,
  `ReadableStream`, `WebSocket`. A dependency needs an ADR.
- No top-level await (it breaks `require()` of the ESM build). No Node built-ins in
  `src/` (`node:` imports), so the same build runs in browsers and Workers.
- Built with `tsc` only, no bundler: `dist/` mirrors `src/`, with `.d.ts` files.
- `int64` values are `bigint` in the public types; generated types keep `string`.
- Errors are classes with a stable `name` and `code`; callers can use `instanceof` and
  `err.code`.
- Every call takes an options bag with `signal` and `timeout`; streams return
  `AsyncIterable`.
- `toJSON` and `util.inspect` output of credentials redacts the secret; a test asserts it.
- The api-extractor report (`etc/sdk.api.md`) is updated in the same PR as an API change.
