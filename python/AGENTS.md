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
    _client.py            Client and AsyncClient, Operation, Response, the one request path
    _stream.py            Stream/AsyncStream, the event-stream parser, the /v1/ws socket
    _auth.py, _errors.py, _retry.py, _hooks.py, _int64.py, _version.py
    _generated/           written by iohr; never edit
  tests/
    conformance/          the driver, run against both clients
```

## Rules

- Runtime dependencies: `httpx`, `pydantic` 2 and `websockets` (the socket's client,
  sync and asyncio; httpx has no WebSocket) only.
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
