# Python package

Distribution and import name `inorbithr`, Python 3.11 or newer. Read the root
`AGENTS.md` and `docs/design.md` first; this file adds only what is specific to Python.

## Commands

- `mise run py:check`: `ruff format --check`, `ruff check`, `pyright` (strict),
  `mypy --strict`, `pytest`
- `mise run py:fmt`, `mise run py:gen` (datamodel-code-generator into
  `src/inorbithr/_generated/`, then `unasync` to write the sync client)
- `mise run conformance:py`
- uv manages the environment and the lockfile; run tools as `uv run <tool>`.

## Layout (planned)

```
python/
  pyproject.toml          build backend uv_build, requires-python >=3.11
  src/inorbithr/
    __init__.py           public exports, __version__
    py.typed
    _async/               the hand-written async client (source of truth)
    _sync/                generated from _async by unasync; do not edit
    _generated/           pydantic models; do not edit
    _auth.py, _errors.py, _retry.py
  tests/
    conformance/          the driver, run against both clients
```

## Rules

- Runtime dependencies: `httpx` and `pydantic` 2 only.
- Write the async client; the sync client is generated from it with `unasync` by
  `mise run py:gen`. A fix in `_sync/` is a fix in `_async/`.
- Public functions take keyword-only arguments after the path parameters, so fields can
  be added without breaking callers.
- Errors: `InOrbitError` base, `APIError` with `.code`, `.status`, `.details`,
  `.headers`; connection, timeout, auth and config errors as subclasses.
- `int64` values are `int`; timestamps are `datetime` with `tzinfo=UTC`; `""` from the
  wire is `None`.
- `__repr__` of credentials redacts the secret; a test asserts it.
- Google-style docstrings on every public name; `griffe check` runs in CI against the
  last `python/v` tag.
