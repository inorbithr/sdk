# InOrbit SDK for Python

The runtime for the InOrbit API, and its public surface, in a blocking and an `asyncio`
form. On [PyPI](https://pypi.org/project/inorbithr/) since 0.1.0, for Python 3.11 or later:

```sh
pip install inorbithr     # or: uv add inorbithr
```

```python
from inorbithr import Public

api = Public.from_env()  # INORBIT_TOKEN, or INORBIT_KEY_ID + INORBIT_KEY_SECRET + INORBIT_SCOPES
me = api.me().value
digests = api.radar.list_digests(limit=3).value
```

`AsyncPublic` is the same on `asyncio` (`await api.me()`). [examples/python](../examples/python)
holds both as programs CI type-checks.

A surface cut to what your own credentials may call comes from the command line:

```sh
iohr sdk generate --lang python --for default --out src/iohr
```

It writes one class per profile, blocking and `asyncio`, each holding only the operations
its cut holds; pyright and mypy refuse a call a profile may not make.

- Errors are one tree under `InOrbitError`; `ApiError` carries the API's `code`, `status`
  and typed `details`.
- Idempotent calls (`GET`, `PUT`, `DELETE`) are retried on 429, 503, 504 and connection failures,
  at most twice, honouring `Retry-After`; writes are not.
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
- Python 3.11 or newer; the dependencies are `httpx`, `pydantic` 2 and `websockets` (for
  the socket).

How the SDKs behave in every language: [docs/design.md](../docs/design.md).
