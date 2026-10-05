"""Where a call's bearer token comes from.

A token you hold (`StaticToken`), a token file (`TokenFile`), an API key exchanged for
short-lived tokens (`ClientCredentials`), the `iohr` login (`CliToken`), your own
provider wrapped in `CachedToken`, or a chain of them (`ChainedCredential`,
`DefaultCredential`). Each has an `asyncio` twin. The caching rules are
`docs/config.md` section 5.3.
"""

from __future__ import annotations

import asyncio
import base64
import contextlib
import json
import os
import subprocess
import threading
import time
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, field
from datetime import datetime
from typing import TYPE_CHECKING, Any, Protocol, cast, runtime_checkable
from urllib.parse import quote

import httpx

from inorbithr._errors import AuthError, ConfigError, InOrbitError
from inorbithr._retry import backoff, retry_after, retryable_status

if TYPE_CHECKING:
    from inorbithr._config import Credential, LoadOptions

#: The audience every token for the API is asked for.
AUDIENCE = "iohr-api"
#: Where an API key is exchanged for a token.
DEFAULT_TOKEN_URL = "https://auth.inorbit.hr/oauth2/token"  # noqa: S105 - a URL, not a secret

_TOKEN_RETRIES = 2
_TOKEN_TIMEOUT = 30.0
_REFRESH_AT = 0.8
_SOFT_RETRY = 5.0
_FILE_CHECK = 60.0
_CLI_TIMEOUT = 10.0
_REDACTED = "<redacted>"

#: Called with a log event's name and fields: how providers report a failed refresh.
Observer = Callable[[str, Mapping[str, object]], None]


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


# --- caching -------------------------------------------------------------------------


class _Cache:
    """The rules of section 5.3 for one provider: refresh ahead, soft expiry."""

    def __init__(self) -> None:
        self.token: Token | None = None
        self.issued = 0.0
        self.not_before = 0.0
        self.observer: Observer | None = None

    def usable(self, now: float) -> Token | None:
        """The cached token while it needs no refresh, or while a refresh may not be tried."""
        t = self.token
        if t is None:
            return None
        if t.expires_at is None:
            return t
        lifetime = t.expires_at - self.issued
        if now - self.issued < lifetime * _REFRESH_AT:
            return t
        if now < self.not_before and now < t.expires_at:
            return t
        return None

    def store(self, t: Token, now: float) -> Token:
        self.token, self.issued, self.not_before = t, now, 0.0
        if self.observer is not None:
            self.observer("token_exchange", {"error": None})
        return t

    def failed(self, error: InOrbitError, now: float) -> Token:
        """The token to use after a failed refresh; raises when there is none."""
        if self.observer is not None:
            self.observer("token_exchange", {"error": type(error).__name__})
        t = self.token
        if t is None or (t.expires_at is not None and now >= t.expires_at):
            raise error
        self.not_before = now + _SOFT_RETRY
        if self.observer is not None:
            self.observer("token_refresh_failed", {"reason": error.kind})
        return t


class _SyncCached:
    """Single flight with a lock: one refresh however many threads wait."""

    def __init__(self, fetch: Callable[[], Token]) -> None:
        self._fetch = fetch
        self._cache = _Cache()
        self._lock = threading.Lock()

    def get(self) -> Token:
        with self._lock:
            now = time.time()
            t = self._cache.usable(now)
            if t is not None:
                return t
            try:
                return self._cache.store(self._fetch(), now)
            except InOrbitError as e:
                return self._cache.failed(e, now)

    def invalidate(self) -> None:
        with self._lock:
            self._cache.token = None

    def observe(self, observer: Observer | None) -> None:
        self._cache.observer = observer


class _AsyncCached:
    """Single flight with a shared task: a caller that gives up does not cancel it."""

    def __init__(self, fetch: Callable[[], Awaitable[Token]]) -> None:
        self._fetch = fetch
        self._cache = _Cache()
        self._task: asyncio.Task[Token] | None = None

    async def get(self) -> Token:
        t = self._cache.usable(time.time())
        if t is not None:
            return t
        task = self._task
        if task is None or task.done():
            task = asyncio.ensure_future(self._refresh())
            task.add_done_callback(_retrieve)
            self._task = task
        return await asyncio.shield(task)

    async def _refresh(self) -> Token:
        now = time.time()
        try:
            return self._cache.store(await self._fetch(), now)
        except InOrbitError as e:
            return self._cache.failed(e, now)
        finally:
            self._task = None

    def invalidate(self) -> None:
        self._cache.token = None

    def observe(self, observer: Observer | None) -> None:
        self._cache.observer = observer


def _retrieve(task: asyncio.Task[Token]) -> None:
    """Marks a refresh's exception retrieved: every waiter gets it from `shield`."""
    if not task.cancelled():
        task.exception()


def observe(provider: object, observer: Observer | None) -> None:
    """Lets a built-in provider report failed refreshes to its client's log."""
    attach = getattr(provider, "_observe", None)
    if callable(attach):
        attach(observer)


# --- static tokens -------------------------------------------------------------------


class StaticToken:
    """A token you already hold, such as an API token from the console.

    It is never refreshed: a 401 fails the call (section 5.3).
    """

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


# --- token files ---------------------------------------------------------------------


def jwt_expiry(token: str) -> float | None:
    """A JWT's `exp` claim, read and not verified; `None` for anything else."""
    parts = token.split(".")
    if len(parts) != 3:
        return None
    payload = parts[1] + "=" * (-len(parts[1]) % 4)
    try:
        claims = json.loads(base64.urlsafe_b64decode(payload))
    except ValueError:
        return None
    exp = cast("dict[str, object]", claims).get("exp") if isinstance(claims, dict) else None
    return float(exp) if isinstance(exp, int | float) and not isinstance(exp, bool) else None


class TokenFile:
    """A bearer token read from a file, such as a mounted Kubernetes Secret.

    The file is read at first use, then again when its modification time or size changes
    (checked at most once a minute), and right after the API refuses the token. A JWT's
    `exp` claim is the token's expiry. A file that disappears keeps the cached token until
    the API refuses it.
    """

    def __init__(self, path: str | os.PathLike[str]) -> None:
        """A provider reading `path`."""
        self.path = os.fspath(path)
        self._lock = threading.Lock()
        self._token: Token | None = None
        self._stamp: tuple[float, int] | None = None
        self._checked = 0.0
        self._forced = True

    def token(self) -> Token:
        """The file's token, read again when it changed.

        Raises:
            AuthError: The file cannot be read and no token is cached, or the API refused
                the cached one.
        """
        with self._lock:
            now = time.monotonic()
            t = self._token
            expired = t is not None and t.expires_at is not None and time.time() >= t.expires_at
            if (
                t is not None
                and not self._forced
                and not expired
                and now - self._checked < _FILE_CHECK
            ):
                return t
            self._checked = now
            try:
                st = os.stat(self.path)
                stamp = (st.st_mtime, st.st_size)
                if t is not None and not self._forced and stamp == self._stamp and not expired:
                    return t
                with open(self.path, encoding="utf-8") as f:
                    access = f.read().strip()
            except OSError:
                if t is not None and not self._forced:
                    return t
                raise AuthError(f"cannot read the token file {self.path}") from None
            if not access:
                raise AuthError(f"the token file {self.path} is empty")
            self._token, self._stamp, self._forced = Token(access, jwt_expiry(access)), stamp, False
            return self._token

    def invalidate(self) -> None:
        """The API refused the token: read the file again at the next call."""
        with self._lock:
            self._forced = True

    def __repr__(self) -> str:
        """The path, never the token."""
        return f"TokenFile({self.path!r})"


class AsyncTokenFile:
    """`TokenFile` for `AsyncClient`. Reading a small file does not need a thread."""

    def __init__(self, path: str | os.PathLike[str]) -> None:
        """A provider reading `path`."""
        self._inner = TokenFile(path)

    @property
    def path(self) -> str:
        """The file read."""
        return self._inner.path

    async def token(self) -> Token:
        """The file's token, read again when it changed.

        Raises:
            AuthError: The file cannot be read and no token is cached.
        """
        return self._inner.token()

    async def invalidate(self) -> None:
        """The API refused the token: read the file again at the next call."""
        self._inner.invalidate()

    def __repr__(self) -> str:
        """The path, never the token."""
        return f"AsyncTokenFile({self.path!r})"


# --- client credentials --------------------------------------------------------------


class _Exchange:
    """What both providers share: the secret, the form, the refusal, the answer."""

    def __init__(  # noqa: PLR0917 - internal, built once per provider
        self,
        key_id: str,
        key_secret: str | None,
        key_secret_file: str | None,
        scopes: list[str],
        token_url: str,
        user_agent: str | None,
    ) -> None:
        if (key_secret is None) == (key_secret_file is None):
            raise ConfigError("give key_secret or key_secret_file, one of them")
        self.key_id = key_id
        self._secret = key_secret
        self.secret_file = key_secret_file
        self.scopes = list(scopes)
        self.token_url = token_url
        self.headers = {"user-agent": user_agent} if user_agent else {}

    def secret(self) -> str:
        """The secret; a secret file is read before every exchange (rotation)."""
        if self._secret is not None:
            return self._secret
        assert self.secret_file is not None
        try:
            with open(self.secret_file, encoding="utf-8") as f:
                return f.read().strip()
        except OSError:
            raise AuthError(f"cannot read the key secret file {self.secret_file}") from None

    def auth(self) -> tuple[str, str]:
        # RFC 6749 section 2.3.1: each part is form-encoded before Basic authentication.
        return (quote(self.key_id, safe=""), quote(self.secret(), safe=""))

    def form(self) -> dict[str, str]:
        return {
            "grant_type": "client_credentials",
            "audience": AUDIENCE,
            "scope": " ".join(self.scopes),
        }

    def read(self, resp: httpx.Response) -> Token:
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
        return Token(access, time.time() + lifetime)

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
    a 401, and makes one exchange however many threads wait for it. When a refresh fails
    while the cached token is still valid, the cached token is used. A `key_secret_file`
    is read before every exchange, so a rotated secret needs no restart.
    """

    def __init__(
        self,
        *,
        key_id: str,
        key_secret: str | None = None,
        scopes: list[str],
        token_url: str = DEFAULT_TOKEN_URL,
        http_client: httpx.Client | None = None,
        key_secret_file: str | os.PathLike[str] | None = None,
        user_agent: str | None = None,
    ) -> None:
        """A provider for one key.

        Args:
            key_id: The key's id (`ak_...`).
            key_secret: The key's secret. Never log it.
            scopes: The scopes to ask for, a subset of the key's.
            token_url: The token endpoint.
            http_client: The HTTP client to use (default: one of its own).
            key_secret_file: A file holding the secret instead, read before every exchange.
            user_agent: The user agent the exchange sends.

        Raises:
            ConfigError: Neither or both of `key_secret` and `key_secret_file` are given.
        """
        secret_file = None if key_secret_file is None else os.fspath(key_secret_file)
        self._exchange = _Exchange(key_id, key_secret, secret_file, scopes, token_url, user_agent)
        self._http = http_client or httpx.Client()
        self._cached = _SyncCached(self._fetch)

    @property
    def key_id(self) -> str:
        """The key id this provider exchanges."""
        return self._exchange.key_id

    def token(self) -> Token:
        """A cached token while four fifths of its life are left, else a fresh one.

        Raises:
            AuthError: The exchange failed and no valid token is cached.
        """
        return self._cached.get()

    def invalidate(self) -> None:
        """Drops the cached token, so the next call exchanges again."""
        self._cached.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        self._cached.observe(observer)

    def _fetch(self) -> Token:
        x = self._exchange
        for attempt in range(_TOKEN_RETRIES + 1):
            try:
                resp = self._http.post(
                    x.token_url,
                    data=x.form(),
                    auth=x.auth(),
                    headers=x.headers,
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
            return x.read(resp)
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
        key_secret: str | None = None,
        scopes: list[str],
        token_url: str = DEFAULT_TOKEN_URL,
        http_client: httpx.AsyncClient | None = None,
        key_secret_file: str | os.PathLike[str] | None = None,
        user_agent: str | None = None,
    ) -> None:
        """A provider for one key; the arguments are `ClientCredentials`'s.

        Raises:
            ConfigError: Neither or both of `key_secret` and `key_secret_file` are given.
        """
        secret_file = None if key_secret_file is None else os.fspath(key_secret_file)
        self._exchange = _Exchange(key_id, key_secret, secret_file, scopes, token_url, user_agent)
        self._http = http_client or httpx.AsyncClient()
        self._cached = _AsyncCached(self._fetch)

    @property
    def key_id(self) -> str:
        """The key id this provider exchanges."""
        return self._exchange.key_id

    async def token(self) -> Token:
        """A cached token while four fifths of its life are left, else a fresh one.

        Raises:
            AuthError: The exchange failed and no valid token is cached.
        """
        return await self._cached.get()

    async def invalidate(self) -> None:
        """Drops the cached token, so the next call exchanges again."""
        self._cached.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        self._cached.observe(observer)

    async def _fetch(self) -> Token:
        x = self._exchange
        for attempt in range(_TOKEN_RETRIES + 1):
            try:
                resp = await self._http.post(
                    x.token_url,
                    data=x.form(),
                    auth=x.auth(),
                    headers=x.headers,
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
            return x.read(resp)
        raise AuthError("the token endpoint: no answer")  # pragma: no cover

    def __repr__(self) -> str:
        """Never the secret."""
        return self._exchange.describe("AsyncClientCredentials")


# --- the iohr login ------------------------------------------------------------------


def _cli_command(cli_path: str, profile: str) -> list[str]:
    return [cli_path, "auth", "token", "--profile", profile, "--format", "json"]


def _cli_answer(cli_path: str, code: int, out: bytes, err: bytes) -> Token:
    if code != 0:
        first = err.decode("utf-8", errors="replace").strip().splitlines()
        line = first[0][:200] if first else f"exit status {code}"
        raise AuthError(f"`iohr auth token` failed: {line}")
    # Standard output holds a token: it is never quoted in an error.
    refused = AuthError(f"{cli_path} answered something that is not a token")
    try:
        answer = json.loads(out)
    except ValueError:
        raise refused from None
    access = (
        cast("dict[str, object]", answer).get("access_token") if isinstance(answer, dict) else None
    )
    if not isinstance(access, str) or not access:
        raise refused
    expires = cast("dict[str, object]", answer).get("expires_at")
    try:
        expires_at = (
            None
            if expires is None
            else datetime.fromisoformat(str(expires).replace("Z", "+00:00")).timestamp()
        )
    except ValueError:
        raise refused from None
    return Token(access, expires_at)


class CliToken:
    """The `iohr` login: runs `iohr auth token --profile <name> --format json`.

    No shell, standard input closed, a 10 s limit, the inherited environment. The token
    is cached and the command runs again in the refresh window (section 5.4).
    """

    def __init__(self, profile: str, *, cli_path: str = "iohr") -> None:
        """A provider for one `iohr` profile.

        Args:
            profile: The command line's profile name.
            cli_path: The command line to run.
        """
        self.profile = profile
        self.cli_path = cli_path
        self._cached = _SyncCached(self._fetch)

    def token(self) -> Token:
        """A cached token while four fifths of its life are left, else a fresh one.

        Raises:
            AuthError: `iohr` failed (not signed in, not found, too slow).
        """
        return self._cached.get()

    def invalidate(self) -> None:
        """Drops the cached token, so the next call runs `iohr` again."""
        self._cached.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        self._cached.observe(observer)

    def _fetch(self) -> Token:
        try:
            done = subprocess.run(  # noqa: S603 - no shell; the program is the configured iohr
                _cli_command(self.cli_path, self.profile),
                stdin=subprocess.DEVNULL,
                capture_output=True,
                timeout=_CLI_TIMEOUT,
                check=False,
            )
        except subprocess.TimeoutExpired:
            raise AuthError(f"`iohr auth token` did not answer within {_CLI_TIMEOUT:g} s") from None
        except OSError:
            raise AuthError(f"cannot run {self.cli_path}: is iohr installed?") from None
        return _cli_answer(self.cli_path, done.returncode, done.stdout, done.stderr)

    def __repr__(self) -> str:
        """The profile and program, never the token."""
        return f"CliToken({self.profile!r}, cli_path={self.cli_path!r})"


class AsyncCliToken:
    """`CliToken` for `AsyncClient`: the command runs as an asyncio subprocess."""

    def __init__(self, profile: str, *, cli_path: str = "iohr") -> None:
        """A provider for one `iohr` profile; the arguments are `CliToken`'s."""
        self.profile = profile
        self.cli_path = cli_path
        self._cached = _AsyncCached(self._fetch)

    async def token(self) -> Token:
        """A cached token while four fifths of its life are left, else a fresh one.

        Raises:
            AuthError: `iohr` failed (not signed in, not found, too slow).
        """
        return await self._cached.get()

    async def invalidate(self) -> None:
        """Drops the cached token, so the next call runs `iohr` again."""
        self._cached.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        self._cached.observe(observer)

    async def _fetch(self) -> Token:
        try:
            proc = await asyncio.create_subprocess_exec(
                *_cli_command(self.cli_path, self.profile),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
        except OSError:
            raise AuthError(f"cannot run {self.cli_path}: is iohr installed?") from None
        try:
            out, err = await asyncio.wait_for(proc.communicate(), _CLI_TIMEOUT)
        except TimeoutError:
            with contextlib.suppress(ProcessLookupError):
                proc.kill()
            await proc.wait()
            raise AuthError(f"`iohr auth token` did not answer within {_CLI_TIMEOUT:g} s") from None
        return _cli_answer(self.cli_path, proc.returncode or 0, out, err)

    def __repr__(self) -> str:
        """The profile and program, never the token."""
        return f"AsyncCliToken({self.profile!r}, cli_path={self.cli_path!r})"


# --- wrappers ------------------------------------------------------------------------


class CachedToken:
    """Gives any provider the caching rules of section 5.3.

    Use it around a provider that fetches from a vault or a secrets manager: the token is
    cached in memory, refreshed when less than a fifth of its life is left, fetched once
    however many threads wait, and kept when a refresh fails while it is still valid.
    """

    def __init__(self, provider: TokenProvider) -> None:
        """Caches `provider`'s tokens; a token without `expires_at` is kept until a 401."""
        self._provider = provider
        self._cached = _SyncCached(provider.token)

    def token(self) -> Token:
        """The cached token, or a fresh one from the provider."""
        return self._cached.get()

    def invalidate(self) -> None:
        """Drops the cached token and tells the provider."""
        self._cached.invalidate()
        self._provider.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        self._cached.observe(observer)

    def __repr__(self) -> str:
        """The wrapped provider's own `repr`."""
        return f"CachedToken({self._provider!r})"


class AsyncCachedToken:
    """`CachedToken` for `AsyncClient`."""

    def __init__(self, provider: AsyncTokenProvider) -> None:
        """Caches `provider`'s tokens; a token without `expires_at` is kept until a 401."""
        self._provider = provider
        self._cached = _AsyncCached(provider.token)

    async def token(self) -> Token:
        """The cached token, or a fresh one from the provider."""
        return await self._cached.get()

    async def invalidate(self) -> None:
        """Drops the cached token and tells the provider."""
        self._cached.invalidate()
        await self._provider.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        self._cached.observe(observer)

    def __repr__(self) -> str:
        """The wrapped provider's own `repr`."""
        return f"AsyncCachedToken({self._provider!r})"


def _chain_error(tried: list[str]) -> AuthError:
    return AuthError("no credential in the chain gave a token; tried:\n  " + "\n  ".join(tried))


class ChainedCredential:
    """Tries providers in order and keeps the first that gives a token."""

    def __init__(self, *providers: TokenProvider) -> None:
        """A chain of `providers`, tried in this order."""
        if not providers:
            raise ConfigError("a chain needs at least one provider")
        self._providers = providers
        self._chosen: TokenProvider | None = None
        self._lock = threading.Lock()

    def token(self) -> Token:
        """A token from the chosen provider, choosing it on the first call.

        Raises:
            AuthError: Every provider failed; the message lists each and why.
        """
        if self._chosen is not None:
            return self._chosen.token()
        with self._lock:
            tried: list[str] = []
            for p in self._providers:
                try:
                    t = p.token()
                except InOrbitError as e:
                    tried.append(f"{p!r}: {e}")
                    continue
                self._chosen = p
                return t
            raise _chain_error(tried)

    def invalidate(self) -> None:
        """Tells the chosen provider its token was refused."""
        if self._chosen is not None:
            self._chosen.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        for p in self._providers:
            observe(p, observer)

    def __repr__(self) -> str:
        """The providers, never a secret."""
        return f"ChainedCredential({', '.join(repr(p) for p in self._providers)})"


class AsyncChainedCredential:
    """`ChainedCredential` for `AsyncClient`."""

    def __init__(self, *providers: AsyncTokenProvider) -> None:
        """A chain of `providers`, tried in this order."""
        if not providers:
            raise ConfigError("a chain needs at least one provider")
        self._providers = providers
        self._chosen: AsyncTokenProvider | None = None

    async def token(self) -> Token:
        """A token from the chosen provider, choosing it on the first call.

        Raises:
            AuthError: Every provider failed; the message lists each and why.
        """
        if self._chosen is not None:
            return await self._chosen.token()
        tried: list[str] = []
        for p in self._providers:
            try:
                t = await p.token()
            except InOrbitError as e:
                tried.append(f"{p!r}: {e}")
                continue
            self._chosen = p
            return t
        raise _chain_error(tried)

    async def invalidate(self) -> None:
        """Tells the chosen provider its token was refused."""
        if self._chosen is not None:
            await self._chosen.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        for p in self._providers:
            observe(p, observer)

    def __repr__(self) -> str:
        """The providers, never a secret."""
        return f"AsyncChainedCredential({', '.join(repr(p) for p in self._providers)})"


def sync_provider(
    cred: Credential, *, token_url: str, http: httpx.Client | None, user_agent: str | None
) -> TokenProvider:
    """The provider for the credential `load` chose."""
    if cred.provider is not None:
        return cast("TokenProvider", cred.provider)
    if cred.kind == "static_token":
        return StaticToken(cred.token or "")
    if cred.kind == "token_file":
        return TokenFile(cred.token_file or "")
    if cred.kind == "cli":
        return CliToken(cred.profile or "", cli_path=cred.cli_path)
    return ClientCredentials(
        key_id=cred.key_id or "",
        key_secret=cred.key_secret,
        key_secret_file=cred.key_secret_file,
        scopes=cred.scopes,
        token_url=token_url,
        http_client=http,
        user_agent=user_agent,
    )


def async_provider(
    cred: Credential, *, token_url: str, http: httpx.AsyncClient | None, user_agent: str | None
) -> AsyncTokenProvider:
    """The `asyncio` provider for the credential `load` chose."""
    if cred.provider is not None:
        return cast("AsyncTokenProvider", cred.provider)
    if cred.kind == "static_token":
        return AsyncStaticToken(cred.token or "")
    if cred.kind == "token_file":
        return AsyncTokenFile(cred.token_file or "")
    if cred.kind == "cli":
        return AsyncCliToken(cred.profile or "", cli_path=cred.cli_path)
    return AsyncClientCredentials(
        key_id=cred.key_id or "",
        key_secret=cred.key_secret,
        key_secret_file=cred.key_secret_file,
        scopes=cred.scopes,
        token_url=token_url,
        http_client=http,
        user_agent=user_agent,
    )


def _default_resolution(
    profile: str | None, load_options: LoadOptions | None
) -> tuple[Credential, str]:
    from inorbithr._config import resolve  # noqa: PLC0415 - avoids an import cycle

    code: dict[str, object] = {} if profile is None else {"profile": profile}
    r = resolve(code, load_options)
    assert r.credential is not None
    return r.credential, str(r.values["token_url"])


class DefaultCredential:
    """The credential chain of `docs/config.md` section 5.1 as a provider.

    It resolves as `Client.load` does (the environment, the config file, the `iohr`
    login) and delegates to the source it found.
    """

    def __init__(
        self,
        *,
        profile: str | None = None,
        load_options: LoadOptions | None = None,
        http_client: httpx.Client | None = None,
    ) -> None:
        """Resolves the chain now; no host is contacted.

        Raises:
            ConfigError: No source has credentials, or one is half set.
        """
        cred, token_url = _default_resolution(profile, load_options)
        self.source = cred.source
        """The source used: `env`, `file` or `cli`."""
        self._inner = sync_provider(cred, token_url=token_url, http=http_client, user_agent=None)

    def token(self) -> Token:
        """A token from the source found."""
        return self._inner.token()

    def invalidate(self) -> None:
        """Tells the source its token was refused."""
        self._inner.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        observe(self._inner, observer)

    def __repr__(self) -> str:
        """The source's provider, never a secret."""
        return f"DefaultCredential({self._inner!r})"


class AsyncDefaultCredential:
    """`DefaultCredential` for `AsyncClient`."""

    def __init__(
        self,
        *,
        profile: str | None = None,
        load_options: LoadOptions | None = None,
        http_client: httpx.AsyncClient | None = None,
    ) -> None:
        """Resolves the chain now; no host is contacted.

        Raises:
            ConfigError: No source has credentials, or one is half set.
        """
        cred, token_url = _default_resolution(profile, load_options)
        self.source = cred.source
        """The source used: `env`, `file` or `cli`."""
        self._inner = async_provider(cred, token_url=token_url, http=http_client, user_agent=None)

    async def token(self) -> Token:
        """A token from the source found."""
        return await self._inner.token()

    async def invalidate(self) -> None:
        """Tells the source its token was refused."""
        await self._inner.invalidate()

    def _observe(self, observer: Observer | None) -> None:
        observe(self._inner, observer)

    def __repr__(self) -> str:
        """The source's provider, never a secret."""
        return f"AsyncDefaultCredential({self._inner!r})"
