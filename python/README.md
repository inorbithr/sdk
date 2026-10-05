# InOrbit SDK for Python

The runtime for the InOrbit API, and its public surface, in a blocking and an `asyncio`
form. On [PyPI](https://pypi.org/project/inorbithr/) since 0.1.0, for Python 3.11 or later:

```sh
pip install inorbithr     # or: uv add inorbithr
```

```python
from inorbithr import Public

api = Public.load()  # code, then INORBIT_*, then the iohr config file, then `iohr login`
me = api.me().value
digests = api.radar.list_digests(limit=3).value
```

`load` takes each setting from the first place that sets it: an option in code
(`Public.load(timeout=10)`), the environment (`INORBIT_TIMEOUT=10s`), your profile in
the command line's config file, then the default. Credentials come from the first source
of the chain that has any: code, `INORBIT_TOKEN` / `INORBIT_TOKEN_FILE` /
`INORBIT_KEY_ID` with `INORBIT_KEY_SECRET` (or `_SECRET_FILE`) and `INORBIT_SCOPES`, the
config file's profile, and last the developer's `iohr login`. Nothing is contacted until
the first call. `api.client.config().describe()` (or `iohr sdk config`) shows what was
used and where it came from, secrets redacted. Settings, sources and the chain are in
[docs/config.md](../docs/config.md); ready-made setups for CI, Kubernetes, proxies,
private CAs and serverless in [docs/recipes.md](../docs/recipes.md).

`AsyncPublic` is the same on `asyncio` (`await api.me()`). `Public.from_env()` and
`Client(...)` keep working: `from_env` reads only the credential and the URLs from the
environment, and explicit options read nothing but code. [examples/python](../examples/python)
holds programs CI type-checks.

Every call goes through a named pipeline of middlewares, which you can add to, reorder,
replace or trim. A per-retry middleware sees every attempt, `Authorization` included; a
per-call one sees the call once (a circuit breaker goes there):

```python
import time

from inorbithr import CallNext, Middleware, Pipeline, Public, SdkRequest, SdkResponse


class Timing:
    name = "timing"

    def __call__(self, request: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        started = time.monotonic()
        response = call_next(request)
        print(request.info.operation, request.info.attempt, time.monotonic() - started)
        return response


def pipeline(p: Pipeline[Middleware]) -> None:
    p.add_per_retry(Timing()).remove("rate_limit")


api = Public.load(pipeline=pipeline)
```

`AsyncClient` takes `AsyncMiddleware`, whose `__call__` is `async` and awaits
`call_next`. A middleware must not log secrets or bodies; the built-ins do not.

A surface cut to what your own credentials may call comes from the command line:

```sh
iohr sdk generate --lang python --for default --out src/iohr
```

It writes one class per profile, blocking and `asyncio`, each holding only the operations
its cut holds; pyright and mypy refuse a call a profile may not make.

- Errors are one tree under `InOrbitError`; `ApiError` carries the API's `code`, `status`
  and typed `details`.
- Idempotent calls (`GET`, `PUT`, `DELETE`) and writes that take `Idempotency-Key` are
  retried on 429, 503, 504 and connection failures, at most `max_retries` (2) times,
  honouring `Retry-After` up to `retry_after_max` (60 s). A write that takes the header
  gets one key per call, sent on every attempt (pass your own with `idempotency_key=`);
  other writes are not retried. Retries draw from a per-client budget, so a real outage
  fails fast instead of multiplying traffic.
- Every attempt is held to `timeout` (30 s), body included, and every call to
  `total_timeout` (120 s since 0.2.2), retries and waits included. A
  `timeout=` on a call, or `inorbithr.call_options(timeout=...)`, replaces `timeout` and
  caps `total_timeout`.
- Results and errors carry `request_id`, `server_request_id`, `idempotency_key` and the
  `rate_limit` snapshot; `client.rate_limit()` is the latest. `rate_limit="wait"` holds
  a call until an exhausted window resets.
- Logging is off until `log` is set (`INORBIT_LOG=info`); records go to
  `logging.getLogger("inorbithr")` with the field names of
  [docs/config.md](../docs/config.md) section 7.9, never a body, a query value, a token
  or a secret. `redact=` sees every record last.
- With `pip install "inorbithr[otel]"`, each call is an `INTERNAL` span and each attempt
  a `CLIENT` span on your OpenTelemetry providers, and `traceparent` is sent; without it,
  `inorbithr.call_options(traceparent=...)` passes yours through unchanged.
- TLS is verified against the operating system's trust store (`truststore`; on Linux,
  OpenSSL's, which also reads `SSL_CERT_FILE`); `ca_bundle` adds a private CA, and
  `client_cert` with `client_key` present a certificate. `proxy`, `no_proxy` and the
  standard `https_proxy` variables apply to calls, the token exchange and the socket.
  After a certificate rotation, build a new client.
- 64-bit integers are `int`, sent as decimal strings; a message field the API left out is
  `None`; fields this version does not know are kept.
- Every request field is optional (`None` by default) and left out when unset; an
  answer's field is optional unless the API always sends it. Timestamps stay the strings
  the API sent; `parse_timestamp` reads one, `""` (unset) as `None`.
- A paged list has an `all_<operation>` beside its page method that follows the next-page
  token: `for d in api.radar.all_list_digests(): ...`, or `async for` on the `asyncio`
  class; stopping the loop fetches nothing more.
- A streaming operation hands back a `Stream` that opens on the first step of the loop
  and yields one model per event; `AsyncStream` on the `asyncio` class. Leaving the
  `with` (or `close()`) stops it:

  ```python
  with api.events.stream_events(types="key.created") as events:
      for event in events:
          print(event.type, event.id)
  ```

  Streams open over server-sent events, or with `Client(streams="socket")` every stream of
  the client shares one `/v1/ws` connection, opened again when the server ends it. A
  stream silent for `stream_idle_timeout` (45 s) ends with `ApiTimeoutError`; an `error`
  event or frame is an `ApiError`; a revoked key ends the stream with `unauthenticated`.
- Python 3.11 or newer; the dependencies are `httpx`, `pydantic` 2, `websockets` (for
  the socket) and `truststore` (the system's trust store); the config file is read with
  the standard library's `tomllib`, logs go through `logging`.

How the SDKs behave in every language: [docs/design.md](../docs/design.md).
