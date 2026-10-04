# Conformance

One behaviour, one case, six languages. Each case describes the exchange between an SDK
and the API as data: what the SDK must send, what the server answers, and what the SDK
must return. A replay server plays the server side; each SDK runs a small driver that
performs the case's action and reports the result. A case that passes in five
languages and fails in the sixth is a parity bug.

## Layout

| Path | Contents |
|---|---|
| `case.schema.json` | JSON Schema for a case file; editors validate against it |
| `cases/<area>/<name>.yaml` | The cases, grouped by area (`auth`, `errors`, `retries`, `operations`, `sse`, `socket`) |
| `server/` | The replay server, a Go program with one dependency (a YAML parser) |
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
verbatim on responses; `absent` lists the request body's top-level fields that must not be sent. `fault: reset` closes the connection instead of answering.
`delay_ms` holds the answer back; `chunked: { bytes: 3, delay_ms: 50 }` sends the body
with chunked transfer encoding, 3 bytes at a time, 50 ms apart.

## The replay server

`conformance/server` plays the API and the token endpoint on one port, one case at a
time. `mise run conformance:server` runs it; `mise run conformance:server:build` builds
`conformance/server/bin/replay` for the drivers, which start one server per test run:

```sh
replay --addr 127.0.0.1:0 --cases conformance/cases
# replay: listening on http://127.0.0.1:41733     (the first line; drivers read it)
```

A driver then:

1. `GET /_cases` lists every case as `area/name`, so a driver needs no YAML reader.
2. `POST /_case` with `{"name": "auth/token-is-cached"}` (or the bare name, or
   `{"case": {...}}` for a case written inline). This starts a new session, and the
   answer carries the loaded case as JSON (`client`, `action`, `expect`, `pending`).
3. Points `base_url` and `token_url` at the server and runs the case's action.
4. `GET /_result`:

```json
{ "status": "pass", "case": "token-is-cached", "used": 4, "total": 4,
  "token_exchanges": 1, "attempts": 3 }
```

`status` is `pass`, `fail` (with `mismatch`: the exchange index, the reason, the expected
and the actual request), `incomplete` (with `next`: the request still expected) or
`no_case`. `token_exchanges` counts requests to `/oauth2/token`, `attempts` every other
request; drivers compare them with the case's `expect`.

Rules the server applies:

- Requests match the case's exchanges in order. A case whose action has `concurrent: N`
  lets a request match any of the next N unused exchanges, since parallel calls arrive
  in any order.
- `min_delay_ms` and `max_delay_ms` are measured from the previous answer.
- The first mismatch fails the case for good; that request, and every one after it, gets
  `400` with code `conformance_mismatch`, which no SDK retries.
- `sse: { events: [...], hold_ms }` answers `text/event-stream`: each event is a
  `comment`, a `data` object (named by `event`, default `message`), `raw` text written as
  is, with an optional `delay_ms` before it; after the last one the body ends, or stays
  open and silent for `hold_ms` first (what an idle timeout is tested against).
- `socket: { steps: [...] }` answers a WebSocket upgrade (a minimal RFC 6455 server, no
  dependency) and runs the steps in order: `expect` is a subset of the client's next
  frame, and `as` names the call id it carries; `send` sends a frame, where a string
  `"$name"` is the id named before; `close` sends a close frame; `delay_ms` pauses. A
  frame that does not match fails the case like a request that does not match. After the
  steps the server waits up to 5 s for the client to close.
- A streaming case's action names `take: N` when the caller stops after N items, its
  client may set `streams: socket` and `stream_idle_timeout_ms`, and its `expect.items`
  lists what the stream yielded, in order, each as a subset, the count exact.

`mise run conformance:server:check` runs gofmt, vet, golangci-lint and a self-test that
plays every case's exchanges against the server and expects a pass.

## Writing a case

1. Copy the closest case. Name it for the behaviour, not the bug.
2. Run `mise run conformance` and watch it fail in every language.
3. Implement in every language, or mark the case `pending: [<lang>]` with an issue link.
