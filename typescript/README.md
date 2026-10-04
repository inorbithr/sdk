# InOrbit SDK for TypeScript and JavaScript

On [npm](https://www.npmjs.com/package/@inorbithr/sdk) and [JSR](https://jsr.io/@inorbithr/sdk)
since 0.1.0. Node 22.12 or later, Bun, Deno and browsers; no runtime dependencies.

```sh
npm install @inorbithr/sdk     # or: pnpm add @inorbithr/sdk, deno add jsr:@inorbithr/sdk
```

```ts
import { Public } from "@inorbithr/sdk";

// INORBIT_TOKEN (an API token), or INORBIT_KEY_ID + INORBIT_KEY_SECRET + INORBIT_SCOPES
const api = Public.fromEnv();
const { value: me } = await api.me();
const { value: page } = await api.radar.listDigests({ limit: 3 });
```

`Public` holds every operation an API credential may call. For a client cut to what
your own credentials may call, generate one with the command line and commit it:

```sh
iohr sdk generate --lang typescript --for ci --out src/iohr
```

Each profile becomes a class (`Ci`) whose methods are only the operations its cut
holds; calling one it lacks is a type error. Plain JavaScript gets the same classes
without the check.

- Node 22.12 or newer, Bun, Deno, browsers and Workers; ESM only; no dependencies.
- 64-bit integers are `bigint`; errors are classes under `InOrbitError` with a stable
  `kind` and, for API errors, `code` and `status`.
- Every call takes `{ signal, timeout }` (milliseconds) last.
- Every request field is optional and left out when unset; an answer's field is
  optional (`?`) unless the API always sends it. Timestamps stay the strings the API
  sent; `parseTimestamp` reads one, `""` (unset) as `undefined`.
- A paged list has an `all<Operation>` beside its page method, an async generator that
  follows the next-page token: `for await (const d of api.radar.allListDigests()) {}`;
  breaking out fetches nothing more, and `signal` stops it between pages.
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
