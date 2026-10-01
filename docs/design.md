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

The names marked "at the first spec sync" are chosen with `/add-endpoint` once `spec/`
holds the operations. Server-sent events, the multiplexed socket (`/v1/ws`) and MCP are
designed below but not shipped until the platform opens them to API keys
([ADR 0004](adr/0004-transport-scope.md), as amended by ADR 0008).

## 2. The client

One entry type per language, constructed once and shared across threads or tasks.

| | Go | Rust | TypeScript | Python |
|---|---|---|---|---|
| Type | `inorbit.Client` | `inorbithr::Client` | `InOrbit` | `InOrbit`, `AsyncInOrbit` |
| Build | `inorbit.NewClient(opts...)` | `Client::builder()...build()?` | `new InOrbit({...})` | `InOrbit(...)` |
| Per call | `ctx context.Context` first | `.await` on a builder | `{ signal, timeout }` options | keyword `timeout=` |

Configuration, same names everywhere (case adjusted):

| Option | Default | Notes |
|---|---|---|
| `key_id`, `key_secret` | env `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` | Either both or a custom `token_provider`. |
| `scopes` | env `INORBIT_SCOPES` (space-separated) | The scopes to ask for, a subset of the key's. No default: a token with no scope can call nothing. |
| `token_provider` | client credentials (below) | Pluggable, for apps that already hold a token. |
| `base_url` | `https://api.inorbit.hr` | env `INORBIT_BASE_URL`. |
| `token_url` | `https://auth.inorbit.hr/oauth2/token` | env `INORBIT_TOKEN_URL`. |
| `timeout` | 30 s per attempt | Unary calls; streams use a connect timeout and a 45 s idle timeout (section 11). |
| `max_retries` | 2 | 0 disables. |
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

- **Wire format.** JSON, snake_case field names, every field present. 64-bit integers are
  decimal strings; timestamps are RFC 3339 strings, empty string when unset; enums are
  names. Each SDK exposes native types (`int64`/`i64`/`bigint`/`int`, a time type) and
  converts at its boundary.
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
  idempotency key (the platform does not support the header yet; ADR 0004).
- Delay: the server's `Retry-After` header or `retry` detail when present, capped at
  60 s; otherwise exponential backoff with full jitter, base 0.5 s, cap 8 s.
- `max_retries` counts retries, not attempts. The caller's deadline always wins over the
  retry budget.
- Every attempt is visible to hooks (section 8), with its number.

## 7. Streaming (designed, not shipped)

**Server-sent events.** Routes ending in `/events`. Parse per the WHATWG event-stream
rules (multi-line `data`, comments as keep-alives every 15 s). Each `data` is one JSON
object shaped like the REST answer. `event: error` carries the envelope and ends the
stream. The platform sets no event ids and does not honour `Last-Event-ID`, so the SDK
does not resume; it exposes the stream as the language's async iterator and lets the
caller reopen. Browsers use `fetch` streaming, not `EventSource`, because the request
needs an `Authorization` header.

**The socket (`/v1/ws`).** One connection per client, many calls over it:

- client frames `{"type":"call","id","method","body"}` and `{"type":"cancel","id"}`;
- server frames `data`, `end` and `error` (the envelope plus `id`);
- every call ends with exactly one `end` or `error`; 64 calls in flight at most; a frame
  is at most 256 KiB;
- the token is checked only at the upgrade, so the SDK reconnects before the token
  expires and re-issues calls that have not ended, only if they are idempotent.

**MCP.** Wrap the official MCP SDK of each language and supply the token; do not
reimplement the protocol.

## 8. Hooks and observability

- A middleware or hook point around every attempt: request out, response in, error.
  Enough to add logging, metrics or tracing without the SDK depending on any of them.
- `traceparent` is propagated from the caller's context (Go `context`, Python
  `contextvars`, TS options, Rust a request extension). OpenTelemetry integration ships as
  an optional extra (Go submodule, Rust feature, separate npm and PyPI extras), never as a
  hard dependency.
- The SDK logs nothing by default. With debug logging on, it logs method, path, status,
  attempt and duration, never bodies or credentials.

## 9. Pagination

No public route is paginated today. When one is, the platform's `cursor` convention
(empty when last) is exposed as the language's iterator over items (Go
`iter.Seq2[T, error]`, Rust `Stream`, TS `AsyncIterable`, Python `__iter__` /
`__aiter__`), with page-level access kept.

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
