# Python package

Distribution and import name `inorbithr`, Python 3.11 or newer. Read the root
`AGENTS.md`, `docs/design.md` (section 12 above all) and ADR 0011 first; this file adds
only what is specific to Python.

## Runtime and surface

The package is the hand-written **runtime** plus the **public surface** `iohr sdk generate
--lang python` writes into `src/inorbithr/_generated/`. The runtime holds no operation;
the surface calls the runtime's one request path. The Rust crate (`rust/`) is the
reference: mirror its client, builder, token providers, errors, retries, hooks and
`Int64`.

A generated surface has one class per profile, in a sync and an async form, each with a
handle per tag (`client.radar.list_digests(...)`); a profile's class has only the
operations its cut holds, so a call it may not make is a type error under pyright and
mypy. Both forms are rendered by the generator; there is no `unasync` step.

## Commands

- `mise run py:check`: `ruff format --check`, `ruff check`, `pyright` (strict),
  `mypy --strict`, `pytest`
- `mise run py:fmt`, `mise run py:gen` (`iohr sdk generate` into `_generated/`)
- `mise run conformance:py`: the driver in `tests/conformance/`, against both clients
- The generator's Python target lives in `cli/crates/iohr-codegen/src/python/`; golden
  files under `tests/golden/*/expected/python/`; `IOHR_TEST_COMPILE=python mise run
  cli:compile-test` type-checks a generated surface and its negative case.
- uv manages the environment and the lockfile; run tools as `uv run <tool>`.

## Layout

```
python/
  pyproject.toml          build backend uv_build, requires-python >=3.11
  src/inorbithr/
    __init__.py           public exports, __version__
    py.typed
    runtime.py            the runtime alone: what a generated surface imports
    codegen.py            the contract with iohr (VERSION, path_segment, check)
    _client.py            Client and AsyncClient (explicit, from_env, load), Operation, Response
    _config.py            load's resolution: the catalogue, the config file, the credential chain
    _pipeline.py          the middleware pipeline, its built-ins (sync and async), the transport
    _transport.py         proxies and no_proxy, trust (truststore), mTLS, the user agent
    _telemetry.py         logging through `logging`, OpenTelemetry when installed
    _stream.py            Stream/AsyncStream, the event-stream parser, the /v1/ws socket
    _auth.py              token providers: static, file, key, iohr login, cached, chained
    _errors.py, _retry.py, _ratelimit.py, _hooks.py, _int64.py, _version.py
    _generated/           written by iohr; never edit
  tests/
    conformance/          the driver, run against both clients
```

## Rules

- Runtime dependencies: `httpx`, `pydantic` 2, `websockets` 15+ (the socket's client,
  sync and asyncio, with proxy support; httpx has no WebSocket) and `truststore` (the
  operating system's trust store, as docs/config.md section 6.3 asks; pip's own
  verifier, no dependencies) only. The config file is read with `tomllib`, logs go to
  `logging`. OpenTelemetry is the optional extra `otel` (`opentelemetry-api`), imported
  only when installed.
- Configuration (docs/config.md): explicit construction reads code only, `from_env`
  stays as it was, `load` resolves code, the environment, the file and the `iohr` login.
  The CLI's `cli/crates/iohr/src/sdk_config.rs` is the reference resolver; edge cases are
  settled there and in `conformance/vectors/`, which `tests/conformance/test_vectors.py`
  runs.
- The pipeline's built-ins come in a blocking and an `asyncio` form with the same names
  and order (`_pipeline.py`); a behaviour change lands in both.
- Streams (design.md section 7): `Client.stream(op, into)` hands back `Stream[T]`,
  `AsyncClient.stream` an `AsyncStream[T]`; both open lazily. A socket has one reader
  (a thread, or a task) feeding a bounded queue per stream; the connection closes after
  the last stream. `codegen.VERSION` is 2 (stream methods); a surface for 1 still imports.
- Public functions take keyword-only arguments after the path parameters, so fields can
  be added without breaking callers.
- Errors: `InOrbitError` base with `.kind`; `ApiError` with `.code`, `.status`,
  `.details`, `.raw`; `ApiConnectionError`, `ApiTimeoutError` (named so they do not shadow
  the built-ins), `AuthError`, `ConfigError`, `TooLargeError`, `DecodeError`.
- Sync and async token providers are separate classes (`ClientCredentials`,
  `AsyncClientCredentials`): a protocol cannot hold both forms of one method.
- `int64` values are `int`; a message field the gateway left out is `None`.
- `__repr__` of credentials redacts the secret; a test asserts it.
- Google-style docstrings on every public name.
- Publishing stays off until the first release (M4): `pyproject.toml` carries the
  `Private :: Do Not Upload` classifier, which PyPI refuses; the release that publishes
  removes it.
