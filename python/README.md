# InOrbit SDK for Python

The runtime for the InOrbit API, and its public surface, in a blocking and an `asyncio`
form. Not released yet: nothing is on PyPI before the first release.

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
- Python 3.11 or newer; the only dependencies are `httpx` and `pydantic` 2.

How the SDKs behave in every language: [docs/design.md](../docs/design.md).
