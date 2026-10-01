# Conformance

One behaviour, one case, four languages. Each case describes the exchange between an SDK
and the API as data: what the SDK must send, what the server answers, and what the SDK
must return. A replay server plays the server side; each SDK runs a small driver that
performs the case's action and reports the result. A case that passes in three
languages and fails in the fourth is a parity bug.

## Layout

| Path | Contents |
|---|---|
| `case.schema.json` | JSON Schema for a case file; editors validate against it |
| `cases/<area>/<name>.yaml` | The cases, grouped by area (`auth`, `errors`, `retries`, `operations`, later `sse`, `socket`) |
| `server/` | The replay server (to be written; see below) |
| `drivers/` | Nothing yet; each language keeps its driver next to its tests (`go/internal/conformance`, `rust/tests/conformance.rs`, `typescript/test/conformance`, `python/tests/conformance`) |

## Anatomy of a case

```yaml
name: rate-limited-get-honours-retry-after
area: retries
summary: A 429 with Retry-After is retried after the stated delay, then succeeds.
action: { op: me }                 # what the driver calls
client: { max_retries: 2 }         # options beyond the defaults
exchanges:                         # in order; the server fails the case on any other request
  - request:  { method: POST, path: /oauth2/token }
    response: { status: 200, json: { access_token: t1, token_type: bearer, expires_in: 899 } }
  - request:  { method: GET, path: /v1/me, headers: { authorization: Bearer t1 } }
    response: { status: 429, headers: { retry-after: "1" }, json: { code: rate_limited, error: Slow down., details: [] } }
  - request:  { method: GET, path: /v1/me, min_delay_ms: 1000 }
    response: { status: 200, json: { subject: ak_1, kind: client, scopes: [identity:read] } }
expect:
  ok: { subject: ak_1 }
  attempts: 2
```

Unless a case says otherwise, the driver builds the client with `key_id: ak_test`,
`key_secret: s3cr3t`, `scopes: [identity:read]`, `max_retries: 2`, and `base_url` and `token_url` pointing at the
replay server. Header names are lower case. `json` bodies match as subsets on requests and are sent
verbatim on responses. `fault: reset` closes the connection instead of answering.

## The replay server

Requirements for whoever writes it (tracked in the issue "conformance: replay server"):

- One binary, no dependencies at run time, started by `mise run conformance`.
- Serves the API and the token endpoint on one port; the SDK under test points both
  `base_url` and `token_url` at it.
- Loads one case per session (`POST /_case` with the case name), replays the exchanges,
  and reports `GET /_result` (pass, or the first mismatch with the expected and the
  actual request).
- Supports delays, connection resets, chunked and slow bodies, server-sent events and
  WebSocket frames, because the stock mock servers (Prism and similar) cover none of the
  failure modes.

## Writing a case

1. Copy the closest case. Name it for the behaviour, not the bug.
2. Run `mise run conformance` and watch it fail in every language.
3. Implement in all four, or mark the case `pending: [<lang>]` with an issue link.
