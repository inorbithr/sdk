# InOrbit SDK for TypeScript and JavaScript

On [npm](https://www.npmjs.com/package/@inorbithr/sdk) and [JSR](https://jsr.io/@inorbithr/sdk)
since 0.1.0. Node 22.12 or later, Bun, Deno and browsers. One runtime dependency,
[`smol-toml`](https://www.npmjs.com/package/smol-toml) (no dependencies of its own), to
read the `iohr` config file.

```sh
npm install @inorbithr/sdk     # or: pnpm add @inorbithr/sdk, deno add jsr:@inorbithr/sdk
```

```ts
import { Public } from "@inorbithr/sdk";

// The environment, the iohr config file and the iohr login, in that order.
const api = Public.load();
const { value: me } = await api.me();
const { value: page } = await api.radar.listDigests({ limit: 3 });
```

`load` finds credentials the way the [configuration contract](../docs/config.md) says,
the first source that has any winning:

1. code: `Public.load({ token })`, `{ keyId, keySecret, scopes }`, `{ tokenFile }` or a
   `tokenProvider`;
2. the environment: `INORBIT_TOKEN`, `INORBIT_TOKEN_FILE`, or `INORBIT_KEY_ID` with
   `INORBIT_KEY_SECRET` (or `INORBIT_KEY_SECRET_FILE`, read before every token exchange)
   and `INORBIT_SCOPES`;
3. workload identity: reserved until the platform offers it;
4. the profile's table in the `iohr` config file (`token_file`, or `key_id` with
   `key_secret_file`);
5. the developer's `iohr login`, through `iohr auth token` (Node, Deno and Bun).

After `iohr login`, a script needs nothing else. In production, pin the source:
`INORBIT_CREDENTIAL_SOURCES=env`. Every other setting (timeouts, retries, proxy, CA
bundle, client certificate, logging, rate limits) resolves on its own from code, then
`INORBIT_*`, then the config file, then its default. To see what a client will use and
where each value came from, secrets redacted:

```ts
console.log(JSON.stringify(api.client.config().describe(), null, 2));
```

`iohr sdk config` prints the same document from the command line. A bad value fails
`load` with one `ConfigError` whose `problems` list every setting at fault and its source.
[docs/recipes.md](../docs/recipes.md) has ready setups for CI, Kubernetes, corporate
proxies, mTLS gateways and serverless.

`new Client({ ... })` reads code only, for libraries and tests; `fromEnv()` keeps its
old behaviour (six variables, credentials and URLs) and is superseded by `load`.

## Middleware

Every call goes through a named pipeline: `request_id`, `user_agent`,
`idempotency_key`, `call_tracing` and `deadline` once per call, then `retry`, then
`auth`, `rate_limit`, `attempt_tracing`, `logging`, `hooks` and `timeout` on every
attempt. Add your own at either stage, or before, after or instead of any built-in:

```ts
import { Client, type Middleware, Public } from "@inorbithr/sdk";

const team: Middleware = {
  name: "team",
  async handle(request, next) {
    request.headers.set("x-team", "payments"); // sees the finished request, token included
    return next(request);
  },
};

const api = new Public(
  Client.load({ pipeline: (p) => p.addPerRetry(team).remove("rate_limit") }),
);
```

`retry`, `auth` and `timeout` can be replaced but not removed. A middleware must not log
secrets or bodies, and must not read a stream's body (`request.info.stream`). A circuit
breaker such as cockatiel belongs at the per-call slot (`addPerCall`). See
[examples/typescript/src/load.ts](../examples/typescript/src/load.ts).

## Behaviour

- Writes whose operation takes an `Idempotency-Key` (the 16 create and trigger
  operations that declare it, such as `events.createEndpoint`) get one per call, sent unchanged on every attempt, so they are retried like
  reads. Pass your own with `{ idempotencyKey }`; it comes back on the result and on the
  error (`idempotencyKey`, `idempotencyReplayed`).
- Retries: 2 by default, full-jitter backoff, `Retry-After` in seconds or as an HTTP
  date up to `retryAfterMax` (60 s), and a per-client retry budget (a token bucket) that
  turns an outage into fast failures. Every call has a total deadline, `totalTimeout`
  (120 s); a per-call `timeout` shortens it.
- Rate limits: every result carries `raw.rateLimit` (`limit`, `remaining`, `reset` in
  milliseconds), and `client.rateLimit()` the latest; `rateLimit: "wait"` holds a call
  until an empty window resets.
- Logging is off until `log` is set (`INORBIT_LOG=info`); records go to your `logger`
  (`debug`, `info`, `warn`, `error`) or the console. They hold metadata only: never a
  body, a query value, a token or a cookie. Headers appear only with `logHeaders`, values
  only from the allowlist. `redact` sees every record last.
- OpenTelemetry: with [`@opentelemetry/api`](https://www.npmjs.com/package/@opentelemetry/api)
  installed (an optional peer dependency), each call is an `INTERNAL` span with one
  `CLIENT` span per attempt, following the HTTP client conventions, and the attempt
  sends `traceparent`. `tracing: false` turns it off; a `traceparent` you pass per call is
  then sent as is. In a bundle or on Deno, pass the module: `{ opentelemetry: api }`.
- Every call takes `{ signal, timeout, idempotencyKey, traceparent }` last.

## Transport

The global `fetch` sends every request, unless a setting needs more than it can be told
per client. Then, on Node, Deno and Bun, the SDK sends through `node:https` itself, with
no extra package:

- `proxy` (and `HTTPS_PROXY`/`NO_PROXY` through `load`), with the SDK's own `no_proxy`
  rules and CONNECT for every request, the token exchange included;
- `caBundle`, added to the trust store (`systemTrust: false` trusts it alone);
- `clientCert` and `clientKey` (mTLS), `pinnedKeys`, and `connectTimeout`.

To use your own transport instead, pass `fetch`, or an `undici` `dispatcher` (also used
for the `/v1/ws` socket). Proxy and TLS settings then belong to it: set in code next to
it they are a `ConfigError`, from the environment or the file they are ignored and listed
in `describe()`. The socket (`streams: "socket"`) takes the global `WebSocket`, which
cannot be pointed at a proxy or a CA bundle without a dispatcher.

In browsers and Workers there is no environment, no config file and no `iohr`: `load`
reads code only and `describe()` says why the file and `cli` sources were skipped.
Proxies, CA bundles, client certificates and pinning are the browser's business there,
and setting them is a `ConfigError`.

## Generated surfaces

`Public` holds every operation an API credential may call. For a client cut to what
your own credentials may call, generate one with the command line and commit it:

```sh
iohr sdk generate --lang typescript --for ci --out src/iohr
```

Each profile becomes a class (`Ci`) whose methods are only the operations its cut
holds; calling one it lacks is a type error. Plain JavaScript gets the same classes
without the check. `Ci.load()` reads `INORBIT_CI_*` (settings fall back to `INORBIT_*`,
credentials never do) and `[profiles.ci]`.

- 64-bit integers are `bigint`; errors are classes under `InOrbitError` with a stable
  `kind` and, for API errors, `code` and `status`.
- Every request field is optional and left out when unset; an answer's field is
  optional (`?`) unless the API always sends it. Timestamps stay the strings the API
  sent; `parseTimestamp` reads one, `""` (unset) as `undefined`.
- A paged list has an `all<Operation>` beside its page method, an async generator that
  follows the next-page token: `for await (const d of api.radar.allListDigests()) {}`;
  breaking out fetches nothing more, and `signal` stops it between pages.
- A stream (the account's events, scope `events:read`) is an async generator:
  `for await (const ev of api.events.streamEvents({ types: "key.created" })) {}`.
  The first step opens it (an opening error is thrown there), breaking out or `signal`
  closes it, an `error` event ends it with that `ApiError`, and 45 s without an event
  or a keep-alive ends it with `TimeoutError` (`streamIdleTimeout`). By default each
  stream is a server-sent events request through the pipeline; `streams: "socket"`
  carries every stream of the client over one `/v1/ws` connection, reconnecting and
  opening the streams again when the server ends it. The socket sends the token as a
  header, which Node 22+, Deno and Bun allow and browsers do not: browsers use the
  default. See [examples/typescript/src/events.ts](../examples/typescript/src/events.ts).

## More

- Configuration, credentials and the pipeline, in every language: [docs/config.md](../docs/config.md)
- Recipes: [docs/recipes.md](../docs/recipes.md)
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
