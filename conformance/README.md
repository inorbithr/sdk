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
| `cases/<area>/<name>.yaml` | The cases, grouped by area (`auth`, `errors`, `retries`, `operations`, `sse`, `socket`, and from M6 `credentials`, `middleware`, `transport`) |
| `vector.schema.json`, `vectors/<kind>/<name>.yaml` | Pure-function vectors, run by each language as unit tests without a server: configuration resolution, config file paths, `no_proxy`, rate-limit headers, durations (`docs/config.md` section 9.2) |
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
replay server. Header names are lower case. A request header value is a literal, `*` (present, any
value), `$name` (captured the first time, equal afterwards) or `~regex` (the regex matches
the whole value); `headers_absent` lists headers that must not be sent. `json` bodies match as subsets on requests and are sent
verbatim on responses; `absent` lists the request body's top-level fields that must not be sent. `fault: reset` closes the connection instead of answering.
`delay_ms` holds the answer back; `chunked: { bytes: 3, delay_ms: 50 }` sends the body
with chunked transfer encoding, 3 bytes at a time, 50 ms apart.

## The replay server

`conformance/server` plays the API and the token endpoint, one case at a time, on four
listeners that share one session: plain HTTP, TLS, mTLS and a CONNECT proxy. `mise run
conformance:server` runs it; `mise run conformance:server:build` builds
`conformance/server/bin/replay` for the drivers, which start one server per test run:

```sh
replay --addr 127.0.0.1:0 --cases conformance/cases
# replay: listening on http://127.0.0.1:41733     (the first line; drivers read it)
# replay: tls on https://127.0.0.1:41734, mtls on https://127.0.0.1:41735, proxy on http://127.0.0.1:41736
# replay: ca /tmp/replay-pki-123/ca.pem, client certificate .../client.pem, key .../client-key.pem
```

Only the first line is a contract; the listeners' locations reach a driver in every
`/_case` answer. `--https-addr`, `--mtls-addr` and `--proxy-addr` pin the other
listeners (by default the plain listener's host, a free port each), and `--dir` names
the directory for the certificate files (by default a new temporary one, removed when the
server stops on SIGINT or SIGTERM; one left by a killed server is removed by a later
start once it is a day old).

At start the server makes a throwaway CA (ECDSA P-256, valid 7 days) and signs a server
leaf for `127.0.0.1`, `::1` and `localhost` and a client certificate with subject
`CN=conformance-client`. It writes `ca.pem`, `client.pem` and `client-key.pem` (PKCS#8,
mode 0600); the CA's and the server's keys never leave memory. Nothing of it is
committed. Every listener speaks HTTP/1.1 only, so an answer behaves the same over TLS as
over plain HTTP.

- **TLS** trusts nobody's client certificate and asks for none.
- **mTLS** refuses a handshake without a client certificate signed by the CA.
- **The proxy** answers `CONNECT` to the replay's own listeners only (any loopback name
  or the listener's host, with a listener's port; anything else is `403`), and a plain
  `http://` request in absolute form to the plain listener. The listener at the other
  end knows which requests came through it, so `via` can be matched. The
  `Proxy-Authorization` the client sent on `CONNECT` appears on each request the tunnel
  carried as the `proxy-authorization` header, so a case can match it.

The binary is also a fake `iohr` for the `cli` credential source (docs/config.md section
5.4): a driver points `cli_path` at it, and

```sh
replay auth token --profile dev --format json
# {"access_token":"cli-dev","expires_at":"2026-10-04T10:15:00Z","profile":"dev"}
```

prints the token for any profile, `expires_at` 15 minutes ahead in UTC. It carries no
`account`: the fake has no login to take one from, and an SDK reads only the token and
its expiry. For the profile `missing` it prints one line on standard error and exits 1;
a call without `--profile` or with a format other than `json` exits 2.

For a driver without a YAML reader, `replay vectors conformance/vectors` prints every
vector as one JSON document, `{"vectors": [...]}` in path order (the Go driver reads
them this way).

A driver then:

1. `GET /_cases` lists every case as `area/name`, so a driver needs no YAML reader.
2. `POST /_case` with `{"name": "auth/token-is-cached"}` (or the bare name, or
   `{"case": {...}}` for a case written inline). This starts a new session, and the
   answer carries the loaded case as JSON (`client`, `action`, `expect`, `pending`) and,
   next to it, the listeners and files:

   ```json
   { "loaded": "a-private-ca-is-trusted", "exchanges": 2, "case": { ... },
     "base_url": "https://127.0.0.1:41734", "http_url": "http://127.0.0.1:41733",
     "https_url": "https://127.0.0.1:41734", "mtls_url": "https://127.0.0.1:41735",
     "proxy_url": "http://127.0.0.1:41736", "ca_file": "/tmp/replay-pki-123/ca.pem",
     "client_cert_file": "/tmp/replay-pki-123/client.pem",
     "client_key_file": "/tmp/replay-pki-123/client-key.pem" }
   ```

   `base_url` is the listener the case's `client.transport` selects: `http_url` for
   none or `http`, `https_url` for `https` and `proxy` (with `proxy_url` as the proxy),
   `mtls_url` for `mtls`. It is what `{replay}` stands for in `env` and `config_file`.
3. Points `base_url` and `token_url` (`{base_url}/oauth2/token`) at it, sets `ca_bundle`,
   `client_cert`, `client_key` and `proxy` as `client.transport` says, and runs the
   case's action.
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

- Header matchers: `*` needs the header with any value; `$name` takes the value the
  first time a matching exchange carries it and wants the same value afterwards, for the
  rest of the session (captures are kept only when the whole exchange matched, and are
  separate from the socket steps' `$name` ids); `~regex` is Go RE2 syntax and must match
  the whole value. A literal that starts with `*`, `$` or `~` is written as a regex
  (`~\$5`). A pattern that does not compile, or a `$` with no name, fails the load.
- `headers_absent` fails a request that carries any of the named headers. `via: proxy`
  wants the request through the proxy, `via: direct` not; `client_cert` wants the
  subject the client presented (`CN=conformance-client`, or just the common name), so
  it fails over plain HTTP and TLS, where no certificate is presented.
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
plays every case's exchanges against the server and expects a pass. The self-test sends
values that satisfy the matchers (any value for `*`, one fixed value per `$name`, a
string generated from the regex for `~`), leaves out `headers_absent`, and sends each
request through the listener the case's `client.transport` and the request's `via` and
`client_cert` call for, so every case, the pending ones included, plays to a pass.

## Configuration and middleware cases (M6)

The cases under `credentials`, `middleware`, `transport` and the M6 cases in `retries`
use the configuration schema fields below. All six runtimes pass every one of them (each
runtime's 0.2.2, 2026-10-05), and no case is `pending`. The replay server supports what
they need from it
(`docs/config.md` section 9.1): the header matchers, `headers_absent`, `via` and
`client_cert`, the TLS, mTLS and proxy listeners, their locations in the `/_case`
answer, and the fake `iohr`; its self-test plays all of them to a pass.

The rest is the driver's. Where each new field is handled:

| Field | Handled by |
|---|---|
| request `headers` matchers, `headers_absent`, `via`, `client_cert` | server |
| `client.transport` | driver, with the server's `base_url`, `proxy_url`, `ca_file`, `client_cert_file`, `client_key_file` |
| `client.cli` | driver (`cli_path` = the replay binary), the server binary answers |
| `client.load`, `env`, `config_file`, `files`, `profile`, `credential_sources`; `{replay}` (= `base_url`) and `{dir}` substitution | driver |
| `client.pipeline`, `log`, `log_headers`, `log_allow_headers`, `tracing`, `rate_limit`, `total_timeout_ms`, `retry_budget_capacity`, `no_proxy` | driver |
| `action.options`, `action.rewrite` (file rotation between calls) | driver |
| `expect.probes` (with the same matchers as request headers), `logs`, `spans`, `rate_limit`, `idempotency_key`, `config` | driver |

## Writing a case

1. Copy the closest case. Name it for the behaviour, not the bug.
2. Run `mise run conformance` and watch it fail in every language.
3. Implement in every language, or mark the case `pending: [<lang>]` with an issue link.
