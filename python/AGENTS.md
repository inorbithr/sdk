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
    _client.py            Client and AsyncClient, the builder, the one request path
    _auth.py, _errors.py, _retry.py, _hooks.py, _int64.py
    _codegen.py           what generated surfaces import (VERSION, path_segment)
    _generated/           written by iohr; never edit
  tests/
    conformance/          the driver, run against both clients
```

## Rules

- Runtime dependencies: `httpx` and `pydantic` 2 only.
- Public functions take keyword-only arguments after the path parameters, so fields can
  be added without breaking callers.
- Errors: `InOrbitError` base, `ApiError` with `.code`, `.status`, `.details`,
  `.headers`; connection, timeout, auth and config errors as subclasses.
- `int64` values are `int`; a message field the gateway left out is `None`.
- `__repr__` of credentials redacts the secret; a test asserts it.
- Google-style docstrings on every public name.
- Publishing stays off until the first release (M4).
