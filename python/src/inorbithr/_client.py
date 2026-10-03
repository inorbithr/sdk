"""The clients: configuration and the one request path every operation goes through.

The retry and token rules are design.md's sections 3 to 6. `Client` blocks, `AsyncClient`
awaits; both behave the same.
"""

from __future__ import annotations

import asyncio
import ipaddress
import json
import os
import platform
import time
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Any, Generic, Literal, TypeAlias, TypeVar, cast
from urllib.parse import urlencode, urlsplit

import httpx
from pydantic import BaseModel, TypeAdapter, ValidationError

from inorbithr._auth import (
    DEFAULT_TOKEN_URL,
    AsyncClientCredentials,
    AsyncStaticToken,
    AsyncTokenProvider,
    ClientCredentials,
    StaticToken,
    TokenProvider,
)
from inorbithr._errors import (
    ApiConnectionError,
    ApiError,
    ApiTimeoutError,
    ConfigError,
    DecodeError,
    InOrbitError,
    RawResponse,
    TooLargeError,
)
from inorbithr._hooks import Attempt, Hook
from inorbithr._retry import backoff, request_id, retry_after, retryable_status
from inorbithr._version import SDK_VERSION

#: Where the API is.
DEFAULT_BASE_URL = "https://api.inorbit.hr"
#: The largest answer read, 16 MiB.
MAX_BODY = 16 * 1024 * 1024

#: An HTTP method.
Method: TypeAlias = Literal["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
_IDEMPOTENT = frozenset({"GET", "PUT", "DELETE", "HEAD"})

T = TypeVar("T")


@dataclass(frozen=True)
class Operation:
    """One call, as a generated surface builds it."""

    method: Method
    """The method."""
    path: str
    """The path, parameters bound and encoded."""
    name: str | None = None
    """What hooks see (`radar.list_digests`)."""
    query: Sequence[tuple[str, object]] = ()
    """Query parameters; `None` ones are left out, lists repeat the name."""
    body: object = None
    """The JSON body: a model, or anything `json.dumps` writes."""
    scopes: Sequence[str] = ()
    """The scopes the operation needs, for the record."""
    idempotent: bool = False
    """Retry it like an idempotent method although its method is not."""


@dataclass(frozen=True)
class Response(Generic[T]):
    """A typed answer and the raw one beside it."""

    value: T
    """The answer, typed."""
    raw: RawResponse = field(repr=False)
    """The answer as it came."""


_ADAPTERS: dict[object, TypeAdapter[Any]] = {}


def _adapter(into: object) -> TypeAdapter[Any]:
    found = _ADAPTERS.get(into)
    if found is None:
        found = TypeAdapter(cast("type[Any]", into))
        _ADAPTERS[into] = found
    return found


def _check_url(what: str, raw: str, *, origin_only: bool) -> str:
    try:
        url = urlsplit(raw)
        host = url.hostname or ""
    except ValueError as e:
        raise ConfigError(f"{what} is not usable: {e}") from None
    loopback = host == "localhost"
    if not loopback:
        try:
            loopback = ipaddress.ip_address(host).is_loopback
        except ValueError:
            loopback = False
    if not (url.scheme == "https" or (url.scheme == "http" and loopback)) or not host:
        raise ConfigError(
            f"{what} is not usable: it must use https (plain http only to this machine)"
        )
    if url.username is not None or url.password is not None or url.fragment:
        raise ConfigError(f"{what} is not usable: it must not carry credentials or a fragment")
    if origin_only and (url.path not in ("", "/") or url.query):
        raise ConfigError(
            f"{what} is not usable: it is an origin only, such as https://api.inorbit.hr"
        )
    return f"{url.scheme}://{url.netloc}" if origin_only else raw


def _user_agent(suffix: str | None) -> str:
    ua = (
        f"inorbithr-sdk-python/{SDK_VERSION} python/{platform.python_version()} "
        f"{platform.system().lower() or 'unknown'}/{platform.machine().lower() or 'unknown'}"
    )
    return f"{ua} {suffix}" if suffix else ua


def _query_value(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _body(value: object) -> bytes:
    if isinstance(value, BaseModel):
        return value.model_dump_json(by_alias=True, exclude_unset=True).encode()
    return json.dumps(value, default=_default).encode()


def _default(value: object) -> object:
    if isinstance(value, BaseModel):
        return value.model_dump(mode="json", by_alias=True, exclude_unset=True)
    raise TypeError(f"not JSON: {type(value).__name__}")


def _env(profile: str | None) -> tuple[str, dict[str, Any]]:
    env = os.environ
    prefix = f"INORBIT_{profile}_" if profile else "INORBIT_"

    def own(name: str) -> str | None:
        return env.get(f"{prefix}{name}") or None

    def shared(name: str) -> str | None:
        return own(name) or env.get(f"INORBIT_{name}") or None

    found: dict[str, Any] = {}
    for key, var in (("base_url", "BASE_URL"), ("token_url", "TOKEN_URL")):
        value = shared(var)
        if value is not None:
            found[key] = value
    token = own("TOKEN")
    if token is not None:
        found["token"] = token
        return prefix, found
    key_id, key_secret = own("KEY_ID"), own("KEY_SECRET")
    if key_id is not None and key_secret is not None:
        scopes = (own("SCOPES") or "").split()
        if not scopes:
            raise ConfigError(
                f'no scopes: set {prefix}SCOPES (space-separated, such as "identity:read '
                'account:read")'
            )
        found.update(key_id=key_id, key_secret=key_secret, scopes=scopes)
        return prefix, found
    raise ConfigError(
        f"no credentials: set {prefix}TOKEN, or {prefix}KEY_ID and {prefix}KEY_SECRET"
    )


@dataclass(frozen=True)
class _Outcome:
    kind: Literal["done", "unauthorized", "retry"]
    raw: RawResponse | None = None
    wait: float | None = None
    error: InOrbitError | None = None


class _Config:
    """What both clients share: the base URL, limits, user agent and hooks."""

    def __init__(
        self,
        base_url: str,
        timeout: float,
        max_retries: int,
        user_agent_suffix: str | None,
        hooks: Sequence[Hook],
    ) -> None:
        self.base = _check_url("the base URL", base_url, origin_only=True)
        self.host = urlsplit(self.base).netloc
        self.timeout = timeout
        self.max_retries = max_retries
        self.user_agent = _user_agent(user_agent_suffix)
        self.hooks = tuple(hooks)

    def url(self, op: Operation) -> str:
        p = op.path
        if not p.startswith("/") or p.startswith("//") or "?" in p or "#" in p:
            raise ConfigError(
                f"the path {p!r} is not usable: it starts with one / and has no query"
            )
        pairs: list[tuple[str, str]] = []
        for name, value in op.query:
            values: Sequence[object] = (
                cast("Sequence[object]", value) if isinstance(value, list | tuple) else [value]
            )
            pairs.extend((name, _query_value(v)) for v in values if v is not None)
        return self.base + p + (f"?{urlencode(pairs)}" if pairs else "")

    def request(self, op: Operation, url: str, access: str, attempt: Attempt) -> httpx.Request:
        headers = {
            "authorization": f"Bearer {access}",
            "accept": "application/json",
            "user-agent": self.user_agent,
            "x-request-id": attempt.request_id,
        }
        content: bytes | None = None
        if op.body is not None:
            headers["content-type"] = "application/json"
            content = _body(op.body)
        for h in self.hooks:
            h.on_request(attempt)
        return httpx.Request(op.method, url, headers=headers, content=content)

    def attempt(self, op: Operation, number: int, rid: str) -> Attempt:
        return Attempt(op.name or op.path, op.method, op.path, number, rid)

    def outcome(
        self, attempt: Attempt, status: int, headers: httpx.Headers, body: bytes
    ) -> _Outcome:
        raw = RawResponse(status, headers, body, attempt.request_id, attempt.number)
        for h in self.hooks:
            h.on_response(attempt, raw)
        if status == 401:
            return _Outcome("unauthorized", raw)
        if retryable_status(status):
            return _Outcome("retry", raw, retry_after(headers))
        if 200 <= status < 300:
            return _Outcome("done", raw)
        raise ApiError(raw)

    def transport(self, e: httpx.TransportError, timeout: float) -> _Outcome:
        if isinstance(e, httpx.TimeoutException):
            return _Outcome("retry", error=ApiTimeoutError(self.host, timeout))
        return _Outcome("retry", error=ApiConnectionError(self.host, type(e).__name__))

    def failed(self, attempt: Attempt, error: InOrbitError) -> None:
        for h in self.hooks:
            h.on_error(attempt, error)

    @staticmethod
    def too_large(headers: httpx.Headers) -> bool:
        length = headers.get("content-length", "")
        return length.isdigit() and int(length) > MAX_BODY

    @staticmethod
    def decode(raw: RawResponse, into: object) -> Any:  # noqa: ANN401 - the caller's type
        try:
            value: object = json.loads(raw.body) if raw.body else {}
            return _adapter(into).validate_python(value)
        except (ValueError, ValidationError) as e:
            reason = e.errors()[0]["msg"] if isinstance(e, ValidationError) else str(e)
            raise DecodeError(reason, raw) from None


class Client:
    """A blocking client for one credential. Safe to share between threads.

    Example:
        ```python
        from inorbithr import Client

        client = Client.from_env()  # INORBIT_TOKEN, or INORBIT_KEY_ID + _KEY_SECRET + _SCOPES
        ```
    """

    def __init__(
        self,
        *,
        token: str | None = None,
        key_id: str | None = None,
        key_secret: str | None = None,
        scopes: Sequence[str] | None = None,
        token_provider: TokenProvider | None = None,
        base_url: str = DEFAULT_BASE_URL,
        token_url: str = DEFAULT_TOKEN_URL,
        timeout: float = 30.0,
        max_retries: int = 2,
        user_agent_suffix: str | None = None,
        hooks: Sequence[Hook] = (),
        http_client: httpx.Client | None = None,
    ) -> None:
        """A client with one credential.

        Give `token`, or `key_id` with `key_secret` and `scopes`, or a `token_provider`.

        Args:
            token: An API token (from the console or `iohr token create`).
            key_id: An API key's id.
            key_secret: An API key's secret.
            scopes: The scopes to ask for with a key; no default.
            token_provider: Your own token source.
            base_url: The API's origin (plain http only to this machine).
            token_url: The token endpoint for a key.
            timeout: Seconds each attempt may take.
            max_retries: Retries after the first attempt (0 disables).
            user_agent_suffix: Appended to the user agent.
            hooks: Observers of every attempt.
            http_client: The `httpx.Client` to use (default: one of its own).

        Raises:
            ConfigError: No credential is given, a key has no scopes, or a URL is not https.
        """
        self._config = _Config(base_url, timeout, max_retries, user_agent_suffix, hooks)
        tokens = _check_url("the token URL", token_url, origin_only=False)
        self._owned = http_client is None
        self._http = http_client or httpx.Client(follow_redirects=False)
        if token_provider is not None:
            self._provider: TokenProvider = token_provider
        elif token:
            self._provider = StaticToken(token)
        elif key_id is not None and key_secret is not None:
            if not scopes:
                raise ConfigError(
                    'no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read '
                    'account:read")'
                )
            self._provider = ClientCredentials(
                key_id=key_id,
                key_secret=key_secret,
                scopes=list(scopes),
                token_url=tokens,
                http_client=self._http,
            )
        else:
            raise ConfigError(
                "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET"
            )

    @classmethod
    def from_env(cls, profile: str | None = None, **options: Any) -> Client:  # noqa: ANN401
        """A client from the environment.

        A named profile reads `INORBIT_<PROFILE>_TOKEN`, or `INORBIT_<PROFILE>_KEY_ID`,
        `_KEY_SECRET` and `_SCOPES`, and nothing else; without one, the bare `INORBIT_*`
        names. `INORBIT_BASE_URL` and `INORBIT_TOKEN_URL` apply to every profile.

        Args:
            profile: The profile's environment name (`ACME_CI`), or `None`.
            **options: Any other `Client` option.

        Raises:
            ConfigError: Naming the variables to set when no credential is there.
        """
        _, found = _env(profile)
        return cls(**found, **options)

    @property
    def base_url(self) -> str:
        """The API's origin this client calls."""
        return self._config.base

    def close(self) -> None:
        """Closes the HTTP client this client made itself."""
        if self._owned:
            self._http.close()

    def __enter__(self) -> Client:
        """The client, closed on exit."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the client."""
        self.close()

    def request(self, op: Operation, into: type[T], *, timeout: float | None = None) -> Response[T]:
        """Calls `op` and reads its JSON answer as `into`.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token, size and decoding failures.
        """
        raw = self.send(op, timeout=timeout)
        return Response(cast("T", self._config.decode(raw, into)), raw)

    def send(self, op: Operation, *, timeout: float | None = None) -> RawResponse:
        """Calls `op` and hands back the answer as it came, a 2xx one only.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token and size failures.
        """
        cfg = self._config
        url = cfg.url(op)
        rid = request_id()
        retry_safe = op.idempotent or op.method in _IDEMPOTENT
        retries = 0
        refreshed = False
        number = 0
        while True:
            number += 1
            attempt = cfg.attempt(op, number, rid)
            try:
                outcome = self._attempt(op, url, attempt, timeout or cfg.timeout)
            except InOrbitError as e:
                cfg.failed(attempt, e)
                raise
            if outcome.kind == "done" and outcome.raw is not None:
                return outcome.raw
            if outcome.kind == "unauthorized" and not refreshed:
                self._provider.invalidate()
                refreshed = True
                continue
            if outcome.kind == "retry" and retry_safe and retries < cfg.max_retries:
                time.sleep(outcome.wait if outcome.wait is not None else backoff(retries))
                retries += 1
                continue
            error = outcome.error or ApiError(cast("RawResponse", outcome.raw))
            cfg.failed(attempt, error)
            raise error

    def _attempt(self, op: Operation, url: str, attempt: Attempt, timeout: float) -> _Outcome:
        cfg = self._config
        request = cfg.request(op, url, self._provider.token().access, attempt)
        request.extensions["timeout"] = httpx.Timeout(timeout).as_dict()
        try:
            resp = self._http.send(request, stream=True)
        except httpx.TransportError as e:
            return cfg.transport(e, timeout)
        try:
            if cfg.too_large(resp.headers):
                raise TooLargeError
            body = bytearray()
            for chunk in resp.iter_bytes():
                body.extend(chunk)
                if len(body) > MAX_BODY:
                    raise TooLargeError
        except httpx.TransportError as e:
            return cfg.transport(e, timeout)
        finally:
            resp.close()
        return cfg.outcome(attempt, resp.status_code, resp.headers, bytes(body))


class AsyncClient:
    """`Client` for asyncio. Safe to share between tasks on one event loop.

    Example:
        ```python
        from inorbithr import AsyncClient

        client = AsyncClient.from_env()
        ```
    """

    def __init__(
        self,
        *,
        token: str | None = None,
        key_id: str | None = None,
        key_secret: str | None = None,
        scopes: Sequence[str] | None = None,
        token_provider: AsyncTokenProvider | None = None,
        base_url: str = DEFAULT_BASE_URL,
        token_url: str = DEFAULT_TOKEN_URL,
        timeout: float = 30.0,
        max_retries: int = 2,
        user_agent_suffix: str | None = None,
        hooks: Sequence[Hook] = (),
        http_client: httpx.AsyncClient | None = None,
    ) -> None:
        """A client with one credential; the options are `Client`'s.

        Raises:
            ConfigError: No credential is given, a key has no scopes, or a URL is not https.
        """
        self._config = _Config(base_url, timeout, max_retries, user_agent_suffix, hooks)
        tokens = _check_url("the token URL", token_url, origin_only=False)
        self._owned = http_client is None
        self._http = http_client or httpx.AsyncClient(follow_redirects=False)
        if token_provider is not None:
            self._provider: AsyncTokenProvider = token_provider
        elif token:
            self._provider = AsyncStaticToken(token)
        elif key_id is not None and key_secret is not None:
            if not scopes:
                raise ConfigError(
                    'no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read '
                    'account:read")'
                )
            self._provider = AsyncClientCredentials(
                key_id=key_id,
                key_secret=key_secret,
                scopes=list(scopes),
                token_url=tokens,
                http_client=self._http,
            )
        else:
            raise ConfigError(
                "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET"
            )

    @classmethod
    def from_env(cls, profile: str | None = None, **options: Any) -> AsyncClient:  # noqa: ANN401
        """A client from the environment, read as `Client.from_env` does.

        Raises:
            ConfigError: Naming the variables to set when no credential is there.
        """
        _, found = _env(profile)
        return cls(**found, **options)

    @property
    def base_url(self) -> str:
        """The API's origin this client calls."""
        return self._config.base

    async def aclose(self) -> None:
        """Closes the HTTP client this client made itself."""
        if self._owned:
            await self._http.aclose()

    async def __aenter__(self) -> AsyncClient:
        """The client, closed on exit."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the client."""
        await self.aclose()

    async def request(
        self, op: Operation, into: type[T], *, timeout: float | None = None
    ) -> Response[T]:
        """Calls `op` and reads its JSON answer as `into`.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token, size and decoding failures.
        """
        raw = await self.send(op, timeout=timeout)
        return Response(cast("T", self._config.decode(raw, into)), raw)

    async def send(self, op: Operation, *, timeout: float | None = None) -> RawResponse:
        """Calls `op` and hands back the answer as it came, a 2xx one only.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token and size failures.
        """
        cfg = self._config
        url = cfg.url(op)
        rid = request_id()
        retry_safe = op.idempotent or op.method in _IDEMPOTENT
        retries = 0
        refreshed = False
        number = 0
        while True:
            number += 1
            attempt = cfg.attempt(op, number, rid)
            try:
                outcome = await self._attempt(op, url, attempt, timeout or cfg.timeout)
            except InOrbitError as e:
                cfg.failed(attempt, e)
                raise
            if outcome.kind == "done" and outcome.raw is not None:
                return outcome.raw
            if outcome.kind == "unauthorized" and not refreshed:
                await self._provider.invalidate()
                refreshed = True
                continue
            if outcome.kind == "retry" and retry_safe and retries < cfg.max_retries:
                await asyncio.sleep(outcome.wait if outcome.wait is not None else backoff(retries))
                retries += 1
                continue
            error = outcome.error or ApiError(cast("RawResponse", outcome.raw))
            cfg.failed(attempt, error)
            raise error

    async def _attempt(self, op: Operation, url: str, attempt: Attempt, timeout: float) -> _Outcome:
        cfg = self._config
        request = cfg.request(op, url, (await self._provider.token()).access, attempt)
        request.extensions["timeout"] = httpx.Timeout(timeout).as_dict()
        try:
            resp = await self._http.send(request, stream=True)
        except httpx.TransportError as e:
            return cfg.transport(e, timeout)
        try:
            if cfg.too_large(resp.headers):
                raise TooLargeError
            body = bytearray()
            async for chunk in resp.aiter_bytes():
                body.extend(chunk)
                if len(body) > MAX_BODY:
                    raise TooLargeError
        except httpx.TransportError as e:
            return cfg.transport(e, timeout)
        finally:
            await resp.aclose()
        return cfg.outcome(attempt, resp.status_code, resp.headers, bytes(body))
