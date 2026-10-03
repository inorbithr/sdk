"""Where a call's bearer token comes from.

A token you hold (`StaticToken`), an API key exchanged for short-lived tokens
(`ClientCredentials`, `AsyncClientCredentials`), or your own provider.
"""

from __future__ import annotations

import asyncio
import threading
import time
from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable
from urllib.parse import quote

import httpx

from inorbithr._errors import AuthError
from inorbithr._retry import backoff, retry_after, retryable_status

#: The audience every token for the API is asked for.
AUDIENCE = "iohr-api"
#: Where an API key is exchanged for a token.
DEFAULT_TOKEN_URL = "https://auth.inorbit.hr/oauth2/token"  # noqa: S105 - a URL, not a secret

_TOKEN_RETRIES = 2
_TOKEN_TIMEOUT = 30.0
_REFRESH_AT = 0.8
_REDACTED = "<redacted>"


@dataclass(frozen=True)
class Token:
    """A bearer token and, when known, when it stops working."""

    access: str = field(repr=False)
    """The token. Never log it."""
    expires_at: float | None = None
    """When it expires, in `time.time()` seconds, if the provider knows."""


@runtime_checkable
class TokenProvider(Protocol):
    """Hands out a valid token to `Client`; told when the API refused the last one."""

    def token(self) -> Token:
        """A token for the next attempt."""
        ...

    def invalidate(self) -> None:
        """The API answered 401 with the last token: drop any cached one."""
        ...


@runtime_checkable
class AsyncTokenProvider(Protocol):
    """Hands out a valid token to `AsyncClient`; told when the API refused the last one."""

    async def token(self) -> Token:
        """A token for the next attempt."""
        ...

    async def invalidate(self) -> None:
        """The API answered 401 with the last token: drop any cached one."""
        ...


class StaticToken:
    """A token you already hold, such as an API token from the console."""

    def __init__(self, token: str) -> None:
        """A provider that always hands out `token`."""
        self._token = token

    def token(self) -> Token:
        """The token."""
        return Token(self._token)

    def invalidate(self) -> None:
        """Nothing to drop: the token is what it is."""

    def __repr__(self) -> str:
        """Never the token."""
        return f"StaticToken({_REDACTED})"


class AsyncStaticToken:
    """`StaticToken` for `AsyncClient`."""

    def __init__(self, token: str) -> None:
        """A provider that always hands out `token`."""
        self._token = token

    async def token(self) -> Token:
        """The token."""
        return Token(self._token)

    async def invalidate(self) -> None:
        """Nothing to drop: the token is what it is."""

    def __repr__(self) -> str:
        """Never the token."""
        return f"AsyncStaticToken({_REDACTED})"


@dataclass(frozen=True)
class _Cached:
    access: str = field(repr=False)
    issued: float
    lifetime: float

    def fresh(self, now: float) -> bool:
        return now - self.issued < self.lifetime * _REFRESH_AT

    def token(self) -> Token:
        return Token(self.access, self.issued + self.lifetime)


class _Exchange:
    """What both providers share: the form, the refusal, the answer."""

    def __init__(self, key_id: str, key_secret: str, scopes: list[str], token_url: str) -> None:
        self.key_id = key_id
        self._secret = key_secret
        self.scopes = list(scopes)
        self.token_url = token_url

    def auth(self) -> tuple[str, str]:
        # RFC 6749 section 2.3.1: each part is form-encoded before Basic authentication.
        return (quote(self.key_id, safe=""), quote(self._secret, safe=""))

    def form(self) -> dict[str, str]:
        return {
            "grant_type": "client_credentials",
            "audience": AUDIENCE,
            "scope": " ".join(self.scopes),
        }

    def read(self, resp: httpx.Response) -> _Cached:
        if not 200 <= resp.status_code < 300:
            refusal = _object(resp)
            error = refusal.get("error")
            error = error if isinstance(error, str) and error else f"HTTP {resp.status_code}"
            described = refusal.get("error_description")
            extra = f" ({described})" if isinstance(described, str) and described else ""
            raise AuthError(
                f"the token exchange for key {self.key_id} failed: {error}{extra}", error
            )
        answer = _object(resp)
        access = answer.get("access_token")
        if not isinstance(access, str):
            raise AuthError("the token endpoint: the token answer could not be read")
        expires_in = answer.get("expires_in")
        lifetime = float(expires_in) if isinstance(expires_in, int | float) else 900.0
        return _Cached(access, time.time(), lifetime)

    def describe(self, cls: str) -> str:
        return f"{cls}({self.key_id}, secret: {_REDACTED})"


def _object(resp: httpx.Response) -> dict[str, Any]:
    try:
        value = resp.json()
    except ValueError:
        return {}
    if isinstance(value, dict):
        return {str(k): v for k, v in value.items()}  # pyright: ignore[reportUnknownVariableType, reportUnknownArgumentType]
    return {}


class ClientCredentials:
    """Exchanges an API key for 15-minute tokens (OAuth client credentials).

    It caches the token, refreshes it when less than a fifth of its life is left or after
    a 401, and makes one exchange however many threads wait for it.
    """

    def __init__(
        self,
        *,
        key_id: str,
        key_secret: str,
        scopes: list[str],
        token_url: str = DEFAULT_TOKEN_URL,
        http_client: httpx.Client | None = None,
    ) -> None:
        """A provider for one key.

        Args:
            key_id: The key's id (`ak_...`).
            key_secret: The key's secret. Never log it.
            scopes: The scopes to ask for, a subset of the key's.
            token_url: The token endpoint.
            http_client: The HTTP client to use (default: one of its own).
        """
        self._exchange = _Exchange(key_id, key_secret, scopes, token_url)
        self._http = http_client or httpx.Client()
        self._lock = threading.Lock()
        self._cache: _Cached | None = None

    @property
    def key_id(self) -> str:
        """The key id this provider exchanges."""
        return self._exchange.key_id

    def token(self) -> Token:
        """A cached token while four fifths of its life are left, else a fresh one.

        Raises:
            AuthError: The exchange failed.
        """
        with self._lock:
            cached = self._cache
            if cached is None or not cached.fresh(time.time()):
                cached = self._fetch()
                self._cache = cached
            return cached.token()

    def invalidate(self) -> None:
        """Drops the cached token, so the next call exchanges again."""
        with self._lock:
            self._cache = None

    def _fetch(self) -> _Cached:
        for attempt in range(_TOKEN_RETRIES + 1):
            try:
                resp = self._http.post(
                    self._exchange.token_url,
                    data=self._exchange.form(),
                    auth=self._exchange.auth(),
                    timeout=_TOKEN_TIMEOUT,
                    follow_redirects=False,
                )
            except httpx.HTTPError as e:
                if attempt < _TOKEN_RETRIES:
                    time.sleep(backoff(attempt))
                    continue
                raise AuthError(f"the token endpoint: {type(e).__name__}") from e
            if retryable_status(resp.status_code) and attempt < _TOKEN_RETRIES:
                time.sleep(retry_after(resp.headers) or backoff(attempt))
                continue
            return self._exchange.read(resp)
        raise AuthError("the token endpoint: no answer")  # pragma: no cover

    def __repr__(self) -> str:
        """Never the secret."""
        return self._exchange.describe("ClientCredentials")


class AsyncClientCredentials:
    """`ClientCredentials` for `AsyncClient`: one exchange however many tasks wait."""

    def __init__(
        self,
        *,
        key_id: str,
        key_secret: str,
        scopes: list[str],
        token_url: str = DEFAULT_TOKEN_URL,
        http_client: httpx.AsyncClient | None = None,
    ) -> None:
        """A provider for one key.

        Args:
            key_id: The key's id (`ak_...`).
            key_secret: The key's secret. Never log it.
            scopes: The scopes to ask for, a subset of the key's.
            token_url: The token endpoint.
            http_client: The HTTP client to use (default: one of its own).
        """
        self._exchange = _Exchange(key_id, key_secret, scopes, token_url)
        self._http = http_client or httpx.AsyncClient()
        self._lock = asyncio.Lock()
        self._cache: _Cached | None = None

    @property
    def key_id(self) -> str:
        """The key id this provider exchanges."""
        return self._exchange.key_id

    async def token(self) -> Token:
        """A cached token while four fifths of its life are left, else a fresh one.

        Raises:
            AuthError: The exchange failed.
        """
        async with self._lock:
            cached = self._cache
            if cached is None or not cached.fresh(time.time()):
                cached = await self._fetch()
                self._cache = cached
            return cached.token()

    async def invalidate(self) -> None:
        """Drops the cached token, so the next call exchanges again."""
        async with self._lock:
            self._cache = None

    async def _fetch(self) -> _Cached:
        for attempt in range(_TOKEN_RETRIES + 1):
            try:
                resp = await self._http.post(
                    self._exchange.token_url,
                    data=self._exchange.form(),
                    auth=self._exchange.auth(),
                    timeout=_TOKEN_TIMEOUT,
                    follow_redirects=False,
                )
            except httpx.HTTPError as e:
                if attempt < _TOKEN_RETRIES:
                    await asyncio.sleep(backoff(attempt))
                    continue
                raise AuthError(f"the token endpoint: {type(e).__name__}") from e
            if retryable_status(resp.status_code) and attempt < _TOKEN_RETRIES:
                await asyncio.sleep(retry_after(resp.headers) or backoff(attempt))
                continue
            return self._exchange.read(resp)
        raise AuthError("the token endpoint: no answer")  # pragma: no cover

    def __repr__(self) -> str:
        """Never the secret."""
        return self._exchange.describe("AsyncClientCredentials")
