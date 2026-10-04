# SDK design

The rules every language follows. Each language applies them in its own idiom; the
concepts, names and behaviour stay the same. Sources: the
[Azure SDK general guidelines](https://azure.github.io/azure-sdk/general_introduction.html),
[Google AIP client library guidance](https://google.aip.dev/client-libraries), and the
platform's own contract (`spec/`). Where this page and an ADR disagree, the newer ADR wins
and this page is corrected in the same PR.

## 1. Scope

The SDK covers what an API key may call. An API key holds scopes chosen when it was made,
and each operation needs one ([ADR 0008](adr/0008-key-scopes-and-iohr-identifiers.md)).
As published on docs.inorbit.hr on 2026-10-01:

| Operation | Method and path | Scope | SDK name |
|---|---|---|---|
| Who am I | `GET /v1/me` | `identity:read` | `me` |
| The key's account | `GET /v1/accounts/me` | `account:read` | `accounts.get_me` |
| Units and usage | `GET /v1/accounts/orgs/{org_id}/units`, `/usage` | `usage:read` | `accounts.get_usage` and one more, named at the first spec sync |
| Unit categories | `GET /v1/accounts/units/categories` | `usage:read` | named at the first spec sync |
| Radar digests and items | `GET /v1/radar/digests`, `/v1/radar/digests/{id}`, `/v1/radar/items` | `radar:read` | named at the first spec sync |
| The plan's OpenAPI document | `GET /v1/openapi.json` | any key | not wrapped |

This table is the **public surface**: what the published package offers, generated
from `spec/openapi.json`. It is one surface of many. A developer generates their own
with `iohr sdk generate`, cut to what their credentials may call (section 12). Names are
the operation id in the language's case (`accounts.get_me`, `radar.list_digests`), the
bare ids flat (`me`). Streams are served over server-sent events or the multiplexed
socket (`/v1/ws`), section 7; MCP is designed there but not shipped
([ADR 0004](adr/0004-transport-scope.md), as amended by ADR 0008).

## 2. The client

One entry type per language, constructed once and shared across threads or tasks.

| | TypeScript | Python | Go | Java | C# | Rust |
|---|---|---|---|---|---|---|
| Type | `Client` | `Client`, `AsyncClient` | `inorbit.Client` | `hr.inorbit.sdk.Client` | `InOrbit.Sdk.Client<P>` | `inorbithr::Client<P>` |
| Build | `new Client({...})` | `Client(...)` | `inorbit.NewClient(opts...)` | `Client.builder()...build()` | `new Client<P>(new ClientOptions {...})` | `Client::builder()...build()?` |
| Load (M6) | `Client.load()` | `Client.load()` | `inorbit.Load()` | `Client.load()` | `Client.Load()` | `Client::load()?` |
| Per call | `{ signal, timeout }` options | keyword `timeout=` | `ctx context.Context` first | a sync call, a `CompletableFuture` beside it | `async`, `CancellationToken` last | `.await` |

Configuration, same names everywhere (case adjusted). [config.md](config.md) is the full
contract (ADR 0015): every setting's environment variable and config-file key, the
precedence (code, environment, config file, defaults), the credential chain and the
middleware pipeline. The table below is the summary as built before M6.

| Option | Default | Notes |
|---|---|---|
| `key_id`, `key_secret` | env `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` | Either both or a custom `token_provider`. |
| `scopes` | env `INORBIT_SCOPES` (space-separated) | The scopes to ask for, a subset of the key's. No default: a token with no scope can call nothing. |
| `token_provider` | client credentials (below) | Pluggable, for apps that already hold a token. |
| `base_url` | `https://api.inorbit.hr` | env `INORBIT_BASE_URL`. |
| `token_url` | `https://auth.inorbit.hr/oauth2/token` | env `INORBIT_TOKEN_URL`. |
| `timeout` | 30 s per attempt | Unary calls; streams use a connect timeout and a 45 s idle timeout (section 11). |
| `max_retries` | 2 | 0 disables. |
| `streams` | `sse` | `socket` opens every stream over one `/v1/ws` connection (section 7). |
| `stream_idle_timeout` | 45 s | A stream silent this long (no event, comment or ping) fails or, on the socket, reconnects. |
| `http_client` | the language default | Bring your own transport, proxies, TLS. |
| `region` | none (`base_url` decides) | `eu` or `us` when the API offers them; never a silent fallback (section 11). |
| `ca_bundle` | system trust store | Extra CA certificates in PEM, for TLS-inspecting proxies. |
| `client_cert` | none | Client certificate and key for mTLS; a signer hook where the language allows. |
| `proxy` | `HTTPS_PROXY` / `NO_PROXY` | Explicit proxy, with credentials treated as secrets. |
| `pinned_keys` | none (off) | SPKI SHA-256 pins, at least one backup; opt-in only. |
| `logger`, `redact` | off | Logging is off by default; `redact` sees every record first. |
| `user_agent_suffix` | none | Appended to the SDK's user agent. |

A client built with no credentials, or with credentials but no scopes, fails at
construction with a configuration error that names the missing environment variables.

## 3. Authentication

An API key is an OAuth2 client (`ak_...` id plus a secret shown once). The SDK exchanges
it for a 15-minute access token:

```
POST {token_url}
Authorization: Basic base64(key_id:key_secret)
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials&audience=iohr-api&scope=identity%3Aread%20account%3Aread
```

- `audience` is always `iohr-api`; `scope` is the configured `scopes`, space-separated.
  Asking for a scope the key does not hold fails with `invalid_scope`, surfaced as an
  `AuthError` naming the scope.

- Cache the token. Refresh when less than 20 % of its `expires_in` remains, or on a 401
  from the API (once per request, then surface the error).
- One refresh at a time per client: concurrent callers wait for the same exchange
  (single flight).
- The token endpoint is rate limited (20 requests per 10 s per address); never fetch a
  token per call.
- `token_provider` is the extension point: an interface with one async "give me a valid
  token" method, so callers can plug in a PKCE token or a token from elsewhere.
- The secret and the token are redacted everywhere they could print (`ak_1234...`,
  `secret: <redacted>`).

## 4. Requests and responses

- **Wire format.** JSON, snake_case field names. An answer carries every field the
  document marks `required`; a request leaves out what is unset (section 12, "Wire
  optionality"). 64-bit integers are decimal strings; timestamps are RFC 3339 strings,
  empty string when unset (each runtime's timestamp helper reads `""` as no value); enums
  are names. Each SDK exposes native types (`int64`/`i64`/`bigint`/`int`, a time type)
  and converts at its boundary.
- **Unknown fields are kept, not rejected.** The API adds fields within `/v1`; an older
  SDK must keep working. Typed models ignore unknown fields, and the raw response stays
  reachable.
- **Raw access.** Every call can return the raw response (status, headers, body) next to
  the typed value, for debugging and for fields the SDK does not model yet.
- **Headers sent**: `Authorization`, `User-Agent` (`inorbithr-sdk-<lang>/<version>
  <runtime>/<version> <os>/<arch>`), `Content-Type: application/json` on bodies, and a
  W3C `traceparent` when the caller supplies a trace context.

## 5. Errors

One error family per language, rooted in a single type the caller can match.

| Kind | When | Retryable |
|---|---|---|
| `ApiError` | The API answered with the problem envelope | per code |
| `ConnectionError` | DNS, TCP, TLS, reset before a response | yes |
| `TimeoutError` | The per-attempt timeout or the caller's deadline | attempt: yes; deadline: no |
| `AuthError` | The token exchange failed | no (except 429/5xx from the token endpoint) |
| `ConfigError` | Bad options at construction | no |

`ApiError` carries the envelope exactly as the platform sends it:

```json
{ "code": "rate_limited", "error": "Too many requests.", "details": [
  { "type": "retry", "after_seconds": 3 } ] }
```

- `code` is one of the frozen slugs in `spec/problem.json` (16 today), exposed as an enum
  with an `unknown` fallback that keeps the raw string.
- `details` are typed: `field` (field, description), `info` (reason, domain, metadata),
  `retry` (after_seconds). Unknown detail types are kept raw.
- The HTTP status and the response headers are on the error.
- **Not every error is the envelope.** The gateway answers some 401, 403 and 429 with a
  plain-text body. The SDK maps those by status (`401 -> unauthenticated`,
  `403 -> forbidden`, `429 -> rate_limited`, ...) and keeps the body text.
- Messages are written for the reader: what failed, the code, and what to do
  (see `docs/style.md`). The secret never appears in a message.

## 6. Retries

- Retried: connection errors, attempt timeouts, `429`, `503`, `504`, and the token
  exchange on the same conditions. Never retried: any other 4xx, `500`, `501`.
- Only idempotent methods are retried automatically (`GET`, `HEAD`, `PUT`, `DELETE`).
  `POST` is retried only when the operation is marked idempotent or the caller passes an
  idempotency key. Since RFC 0033 the platform takes `Idempotency-Key` on its create and
  trigger operations; M6 sends one on every such call and retries it (config.md
  section 7.5).
- Delay: the server's `Retry-After` header or `retry` detail when present, capped at
  60 s; otherwise exponential backoff with full jitter, base 0.5 s, cap 8 s.
- `max_retries` counts retries, not attempts. The caller's deadline always wins over the
  retry budget.
- Every attempt is visible to hooks (section 8), with its number.

## 7. Streaming

A streaming operation (its answer is `text/event-stream`; today the account's events,
`events.stream_events`, scope `events:read`) is a method of the same name that yields
the answer's model once per event. The platform's rules for streams are RFC 0048:
what a key may open is what its scopes admit, a key holds at most 32 open streams and
an account 128, a stream lives at most 24 hours, and a revoked key's open streams end
within seconds.

| Language | Method | Use |
|---|---|---|
| Rust | `stream_events(&params).await?` → `inorbithr::EventStream<T>`, a `futures_core::Stream<Item = Result<T, Error>>` with an inherent `next().await` | `while let Some(ev) = s.next().await { let ev = ev?; }` |
| TypeScript | `streamEvents(params, { signal })` → `AsyncGenerator<T>` | `for await (const ev of api.events.streamEvents())` |
| Python | `stream_events(...)` → `Stream[T]`, an `Iterator[T]` and a context manager; `AsyncStream[T]` (`AsyncIterator[T]`) on the asyncio class | `with api.events.stream_events() as s: for ev in s:` / `async for` |
| Go | `StreamEvents(ctx, params) iter.Seq2[*T, error]` | `for ev, err := range api.Events().StreamEvents(ctx, p)` |
| Java | `streamEvents(params)` → `EventStream<T>`, an `Iterator<T>`, `Iterable<T>` and `AutoCloseable` | `try (var s = api.events().streamEvents(p)) { for (var ev : s) ... }` |
| C# | `StreamEventsAsync(query, cancellationToken)` → `IAsyncEnumerable<T>` | `await foreach (var ev in client.Events().StreamEventsAsync())` |

The generator emits a stream method only for the profiles whose cut holds the
operation, behind the same marker as every other method, so a profile without
`events:read` does not compile a call to it (section 12).

**Opening.** A stream opens with a `GET` and the same rules as any other `GET`
(sections 3 and 6): one fresh token after a `401`, retries after a connection failure,
`429`, `503` or `504` with `Retry-After` honoured (too many open streams is a `429`), and
any other error answer is an `ApiError` before the first item. Where a language opens
lazily (TypeScript, Python, Go, Java, C#), the opening error is raised by the first
step of the iteration.

**Server-sent events** (the default, `streams: "sse"`). `Accept: text/event-stream`,
the query parameters as on REST. The body is parsed by the WHATWG event-stream rules:
lines end with LF, CRLF or CR; a line starting `:` is a comment (the server's
keep-alive, every 15 s); `data:` lines join with a newline, one leading space dropped;
`event:` names the event; a blank line dispatches; `id`, `retry` and unknown fields are
ignored. A default event's data is one JSON object, decoded as the model. An `error`
event's data is the error envelope: the stream ends with that `ApiError`, its status
from the code (`problem.json`'s `x-http-status`; a code the SDK does not know keeps its
slug and has no status, 0 or none as the language spells it). The end of the body ends the stream
cleanly. Bounds: an event's data is at most 1 MiB (`TooLarge` otherwise), and nothing
at all, not even a comment, for `stream_idle_timeout` (45 s) ends the stream with a
timeout error. The platform sets no event ids and honours no `Last-Event-ID`, so an
ended stream is not resumed: the caller opens it again.

**The socket** (`streams: "socket"`). Every stream of the client goes over one
`/v1/ws` connection (`wss://` for `https://`), opened when the first stream starts, with
`Authorization: Bearer`, the upgrade retried like a `GET`, and closed when the last
stream ends. A stream is one call:

- `{"type":"call","id":"<n>","method":"<x-iohr-rpc>","body":{…}}`, the body being the
  path and query parameters as one JSON object in wire names (unset ones left out); ids
  are the client's, never reused on a connection;
- each `data` frame's `body` is one item; `end` ends the stream cleanly; an `error` frame
  for the id ends it with that `ApiError`;
- a caller that stops reading (break, drop, cancel, close) sends `{"type":"cancel","id"}`
  and ignores what still arrives for that id;
- an `error` frame without an id concerns the socket. `unauthenticated` (the key was
  revoked) ends every stream on it with that error and nothing reconnects. Any other code
  (`unavailable`: the socket reached its longest life), a close, or silence past
  `stream_idle_timeout` (the server pings every 15 s) reconnects with a token from the
  provider and issues again every call that had not ended: streams are reads, so a
  call is safe to repeat. Reconnects follow the retry budget and backoff of section 6;
  the caller sees one stream;
- frames wait in a bounded queue per stream (64 items). The socket has one reader, so a
  stream whose queue is full pauses that reader: a caller that reads slowly holds back
  every stream on its client's socket, never the server's other connections or memory
  (open a second client for a stream read slowly). A frame from the client is at most
  256 KiB.

The frames are `spec/frames.json`, synced from the platform's `/frames.json`
(ADR 0004), which also carries the socket's limits. The token is checked once, at the
upgrade, and the platform does not cut a socket when that token expires, so the SDK does
not reconnect for token expiry.

**Where a language's WebSocket stops short.** Where the library answers pings itself
(Python's `websockets`, .NET's `ClientWebSocket`; Go's own client counts pings as activity), the socket's idle clock is the
library's own ping with `stream_idle_timeout` as its timeout (on .NET 8 a dead socket is
found by TCP). Hooks see the server-sent events requests, not the socket's upgrade. The web platform's `WebSocket`
(TypeScript) hides pings, the status of a refused upgrade, and backpressure. There the
socket has no idle clock (a dead socket is found by its close), a refused upgrade gets
one fresh token and then the retry budget, and a stream whose caller falls 64 items
behind is stopped with a `ConnectionError` and a cancel frame instead of holding the
server back. Browsers cannot set a header on a WebSocket, so `streams: "socket"` is for
Node, Deno and Bun; browsers use server-sent events.

Errors are those of section 5 everywhere; the conformance cases under
`conformance/cases/sse` and `conformance/cases/socket` hold every language to these
rules.

**MCP.** Wrap the official MCP SDK of each language and supply the token; do not
reimplement the protocol.

## 8. Hooks and observability

M6 replaces this section's mechanisms with the named pipeline of config.md section 7;
hooks stay and run inside it.

- A middleware or hook point around every attempt: request out, response in, error.
  Enough to add logging, metrics or tracing without the SDK depending on any of them.
- `traceparent` is propagated from the caller's context (Go `context`, Python
  `contextvars`, TS options, Rust a request extension). OpenTelemetry integration ships as
  an optional extra (Go submodule, Rust feature, separate npm and PyPI extras), never as a
  hard dependency.
- The SDK logs nothing by default. With debug logging on, it logs method, path, status,
  attempt and duration, never bodies or credentials.

## 9. Pagination

A paged operation takes `page_token` and answers one list with `next_page_token`, empty
after the last page (18 public routes on 2026-10-03, among them radar digests, webhook
deliveries, connections, monitors and the team audit log). The page method stays, and
the generated surface adds an iterator over the items beside it, in the language's own
idiom:

| Language | Iterator | Use |
|---|---|---|
| Go | `All<Op>(ctx, params) iter.Seq2[T, error]` | `for d, err := range api.Radar().AllListDigests(ctx, p)` |
| TypeScript | `all<Op>(params, options)` → `AsyncGenerator<T>` | `for await (const d of api.radar.allListDigests())` |
| Python | `all_<op>(...)` → `Iterator[T]`; `AsyncIterator[T]` on the asyncio class | `for d in api.radar.all_list_digests():` / `async for` |
| Rust | `all_<op>(&params)` → `inorbithr::Pages<'_, T>` | `while let Some(d) = pages.next().await` (or `.collect().await`) |
| Java | `all<Op>(params)` → `Pages<T>`, an `Iterable<T>` with `stream()` | `for (Digest d : api.radar().allListDigests())` |
| C# | `All<Op>Async(query, cancellationToken)` → `IAsyncEnumerable<T>` | `await foreach (var d in client.Radar().AllListDigestsAsync())` |

An iterator follows the token until it is empty, fetches nothing more once the caller
stops, stops at the first error (raising, throwing or yielding it as the language does),
stops when a page names itself as the next, and checks cancellation between pages where
the language has it (a `context.Context`, an `AbortSignal`, a `CancellationToken`, a
cancelled task). A token the caller passes starts the walk. Rust's iterator is a pager the
runtime owns rather than a `futures::Stream`, so the runtime takes no extra dependency.

## 10. Compatibility

- Each package follows SemVer independently ([ADR 0003](adr/0003-versioning-and-releases.md)).
  Adding an operation or a field is a minor change; removing or renaming anything public
  is major.
- Every public type is open to growth: Go structs, Rust `#[non_exhaustive]`, TS optional
  fields, Python keyword-only arguments.
- Minimum runtimes are in each language's README and change only in a minor release with
  a changelog line.

## 11. Transport security and data protection

The public API shapes that follow from the security requirements
([security/requirements.md](security/requirements.md)); the requirement ids are in
brackets.

- **TLS.** TLS 1.2 minimum [SR-01]; certificate verification cannot be turned off
  through the public API [SR-02]. `ca_bundle` adds trusted CAs [SR-03], `client_cert`
  enables mTLS [SR-04], `proxy` and the proxy environment variables route traffic
  [SR-05], and `pinned_keys` pins the API's public key when a caller opts in [SR-06].
  `base_url` and `token_url` must be `https://` except on loopback [SR-07].
- **FIPS.** No cryptography in the SDK; each language documents how to run on its
  runtime's FIPS-validated module, and calls itself FIPS-capable only [SR-08].
- **Region.** `region` or `base_url` fixes where requests go; retries stay in that
  region [SR-09].
- **Secrets.** `key_secret`, tokens, proxy passwords and private keys are `Secret`
  values that print as `<redacted>` [SR-10], are zeroed on drop in Rust [SR-11] and are
  kept only in memory [SR-12].
- **Logging.** Off by default; when on, metadata only, never bodies, query values or
  the `Authorization` header [SR-13]; `redact` sees every record and message first
  [SR-14]. Personal data never goes into URLs or SDK-built messages [SR-15]. No
  telemetry [SR-16].
- **Request ids.** Each request sends a client request id; every result and every error
  exposes `request_id` (the client's) and `server_request_id` when the API returns one
  [SR-17].
- **Idempotency.** Calls that create or change data accept an `idempotency_key`; the SDK
  generates one per logical call when retries are enabled and reuses it on every attempt,
  and retries such calls only when a key is present and the API supports it [SR-18].
- **Time.** Every call has a finite deadline; streams end on 45 s without data or
  keep-alive [SR-19].

## 12. Runtime and surface

Each language is one **runtime** and many **surfaces** ([ADR 0011](adr/0011-runtime-and-surface.md),
platform RFC 0020).

- **The runtime** is the published package: client, configuration, token handling,
  retries, errors, hooks, later streaming. It is hand-written, holds no operation, and
  is what the conformance cases test. In Rust it is the `inorbithr` crate.
- **A surface** is generated: the models and the thin operation methods for a set of
  operations, calling the runtime's one request path (`Operation` in Rust). The
  published package carries the public surface, generated into it from
  `spec/openapi.json` by the same generator (`mise run rust:gen`); a developer's
  repository carries one `iohr sdk generate` wrote for their credentials.

**Profiles.** A surface is generated for one or more profiles (`iohr profile`), each
the credential of one account. In Rust `Client<P: Profile>` is generic over a
zero-sized profile type the surface defines (`Personal`, `AcmeCi`); each operation has
a marker trait implemented for the profiles whose cut holds it, and the operation's
method is bounded by it, so a call the profile may not make does not compile:

```rust
use iohr::prelude::*;                       // the generated surface
let ci: Client<AcmeCi> = Client::from_env()?;   // INORBIT_ACME_CI_TOKEN, or _KEY_ID + _KEY_SECRET + _SCOPES
let digests = ci.radar().list_digests(&Default::default()).await?;
// personal.accounts().get_usage(..) does not compile when Personal lacks usage:read
```

A named profile reads `INORBIT_<PROFILE>_*` and nothing else; the public profile reads
the bare `INORBIT_*`. Operations with a service in their id are grouped under a handle
per tag (`accounts()`, `radar()`); bare ids are flat (`me()`). The same holds in every
language as far as its types reach (RFC 0020's table, ADR 0013):

| Language | A profile's client holds only its operations through |
|---|---|
| TypeScript | one class per profile with a handle per tag; plain JavaScript gets the methods unchecked |
| Python | one class per profile, sync and async, checked by pyright and mypy |
| Go | one package per profile wrapping the runtime's client |
| Java | one class per profile with a handle per tag; records for the models |
| C# | a marker interface per operation and extension methods constrained to it |
| Rust | a marker trait per operation bounding the method |

Every target renders from the generator's shared model (`iohr-codegen` `ir` for the
models, `context` for the operations); only the Rust target reads models through
typify.

**The cut and the lock.** The document a credential fetches from `GET /v1/openapi.json`
is cut to its plan narrowed by its scopes and stamped `info.x-iohr-cut` with the plan,
the account, the scopes and a hash of the cut's shape (`paths` and `components` without
prose or `x-iohr-*` keys, keys sorted). `iohr sdk generate` writes `iohr.lock` beside
the surface: the generator, the language, and per profile the hash, plan, scopes and
the `METHOD /path` lines the cut held, never a secret. Both are committed. `iohr sdk
check` fetches each profile's document again and fails with the lines added and removed
when a hash moved; `--files` also diffs the regenerated surface. In CI, where a person
cannot sign in, `IOHR_TOKEN_<PROFILE>` stands in for a profile.

**Wire optionality.** The document says it (core #218, `spec/README.md`): a message
only an answer carries lists as `required` exactly the fields the gateway always sends
(those without presence); a field with presence (a message, `optional`, a `oneof`) is
left out when unset and never required. A message a request carries marks nothing
required. So a generated surface reads a required answer field as present and every
other field as optional (`Option<T>` in Rust, `T | None` in Python, `?` in TypeScript,
a pointer in Go, nullable in C#, `null` in Java), and every request field is optional:
what is unset is left out of the body, and the platform reads it as its default. Go
fills a pointer field with `inorbit.Ptr(v)`; a Rust request is built with
`..Default::default()`.

**Unset timestamps** are `""` on the wire (rule N5, settled behaviour, not an upstream
ask). Models keep the string as sent; each runtime has one helper that reads a
timestamp and maps `""` to no value: `inorbithr::parse_timestamp` (Rust, `None`),
`parseTimestamp` (TypeScript, `undefined`), `parse_timestamp` (Python, `None`),
`inorbit.ParseTimestamp` (Go, the zero `time.Time`), `Timestamps.Parse` (C#, `null`),
`Timestamps.parse` (Java, an empty `Optional`).

**Streaming operations** (`text/event-stream`) are generated as stream methods
(section 7), gated by profile like every other operation.

