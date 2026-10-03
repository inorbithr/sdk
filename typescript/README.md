# InOrbit SDK for TypeScript and JavaScript

Not released yet: nothing is on npm. The package builds and passes the shared
conformance cases; it is published with the other SDKs at the first release.

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
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
