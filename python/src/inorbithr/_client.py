"""The clients: configuration and the one request path every operation goes through.

Three ways to build one (`docs/config.md` section 1): explicit options (code only),
`from_env` (credentials and URLs from the environment, as before) and `load` (code, the
environment, the config file, the `iohr` login, defaults). Every call goes through the
named middleware pipeline of section 7. `Client` blocks, `AsyncClient` awaits; both
behave the same.
"""

from __future__ import annotations

import contextlib
import contextvars
import ipaddress
import json
import os
import threading
import time
from collections.abc import AsyncIterator, Callable, Generator, Iterator, Sequence
from dataclasses import dataclass, field
from datetime import timedelta
from typing import TYPE_CHECKING, Any, Generic, Literal, TypeAlias, TypedDict, TypeVar, Unpack, cast
from urllib.parse import urlencode, urlsplit

import httpx
from pydantic import BaseModel, TypeAdapter, ValidationError

from inorbithr._auth import (
    DEFAULT_TOKEN_URL,
    AsyncTokenProvider,
    TokenProvider,
    async_provider,
    observe,
    sync_provider,
)
from inorbithr._config import (
    LoadOptions,
    Resolution,
    ResolvedConfig,
    env_name,
    resolve,
)
from inorbithr._errors import (
    ApiConnectionError,
    ApiError,
    ApiTimeoutError,
    ConfigError,
    DecodeError,
    InOrbitError,
    RawResponse,
)
from inorbithr._hooks import Attempt, Hook
from inorbithr._pipeline import (
    IDEMPOTENT_METHODS,
    MAX_BODY,
    AsyncCallNext,
    AsyncHooksMiddleware,
    AsyncLoggingMiddleware,
    AsyncMiddleware,
    CallInfo,
    CallNext,
    Engine,
    HooksMiddleware,
    LoggingMiddleware,
    Middleware,
    Pipeline,
    SdkRequest,
    SdkResponse,
    _Call,  # pyright: ignore[reportPrivateUsage]
    acompose,
    async_builtins,
    async_transport,
    compose,
    error_type,
    state_of,
    sync_builtins,
    sync_transport,
)
from inorbithr._ratelimit import RateLimit
from inorbithr._retry import RetryBudget, request_id
from inorbithr._stream import (
    AsyncSocket,
    AsyncStream,
    SocketSettings,
    SseParser,
    Stream,
    StreamTransport,
    SyncSocket,
    call_body,
    decode_item,
    problem_error,
)
from inorbithr._telemetry import Log, Redact, Telemetry
from inorbithr._transport import NetSettings, ProxyRule, async_http, sync_http, user_agent

if TYPE_CHECKING:
    import logging
    import ssl

__all__ = [
    "DEFAULT_BASE_URL",
    "MAX_BODY",
    "AsyncClient",
    "Client",
    "Method",
    "Operation",
    "Response",
]

#: Where the API is.
DEFAULT_BASE_URL = "https://api.inorbit.hr"

#: An HTTP method.
Method: TypeAlias = Literal["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]

#: A duration in code: seconds, or a `timedelta`.
Seconds: TypeAlias = float | timedelta

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
    rpc: str = ""
    """A stream's RPC (`x-iohr-rpc`), what its call on the socket names."""
    params: Sequence[tuple[str, object]] = ()
    """A stream's path parameters by wire name, unencoded, for its call on the socket."""
    idempotency_key: bool = False
    """The operation takes `Idempotency-Key`: the call sends one and may be retried."""
    template: str = ""
    """The path template (`/v1/radar/digests/{digest_id}`) for span names; empty when unknown."""


@dataclass(frozen=True)
class Response(Generic[T]):
    """A typed answer and the raw one beside it."""

    value: T
    """The answer, typed."""
    raw: RawResponse = field(repr=False)
    """The answer as it came."""

    @property
    def request_id(self) -> str:
        """The `x-request-id` the call sent."""
        return self.raw.request_id

    @property
    def server_request_id(self) -> str | None:
        """The request id the API answered with, if any."""
        return self.raw.server_request_id

    @property
    def idempotency_key(self) -> str | None:
        """The `Idempotency-Key` the call sent; `None` when the operation takes none."""
        return self.raw.idempotency_key

    @property
    def idempotency_replayed(self) -> bool:
        """Whether the API answered from an earlier call with the same key."""
        return self.raw.idempotency_replayed

    @property
    def rate_limit(self) -> RateLimit | None:
        """What the answer's rate-limit headers said."""
        return self.raw.rate_limit


# --- per-call options ----------------------------------------------------------------


@dataclass(frozen=True)
class _CallOptions:
    traceparent: str | None = None
    timeout: float | None = None


_CALL_OPTIONS: contextvars.ContextVar[_CallOptions] = contextvars.ContextVar(
    "inorbithr_call_options",
    default=_CallOptions(),  # noqa: B039 - immutable
)


@contextlib.contextmanager
def call_options(
    *, traceparent: str | None = None, timeout: Seconds | None = None
) -> Generator[None]:
    """Options for every call made inside the block, in this thread or task.

    Args:
        traceparent: A W3C `traceparent` to continue: the call's span is its child, or,
            with tracing off, it is sent unchanged.
        timeout: Replaces `timeout` for each attempt and shortens `total_timeout`.

    Example:
        ```python
        with inorbithr.call_options(traceparent=incoming_traceparent):
            api.me()
        ```
    """
    seconds = timeout.total_seconds() if isinstance(timeout, timedelta) else timeout
    token = _CALL_OPTIONS.set(_CallOptions(traceparent, seconds))
    try:
        yield
    finally:
        _CALL_OPTIONS.reset(token)


# --- helpers -------------------------------------------------------------------------

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
    """What `from_env` reads: credentials and the two URLs, nothing else."""
    env = os.environ
    prefix = f"INORBIT_{env_name(profile)}_" if profile else "INORBIT_"

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


def _seconds(v: Seconds | None) -> float | None:
    return v.total_seconds() if isinstance(v, timedelta) else v


class _CommonOptions(TypedDict, total=False):
    token: str
    key_id: str
    key_secret: str
    key_secret_file: str | os.PathLike[str]
    token_file: str | os.PathLike[str]
    scopes: Sequence[str]
    base_url: str
    token_url: str
    connect_timeout: Seconds
    timeout: Seconds
    total_timeout: Seconds
    stream_idle_timeout: Seconds
    max_retries: int
    retry_base_delay: Seconds
    retry_max_delay: Seconds
    retry_after_max: Seconds
    retry_budget: bool
    retry_budget_capacity: int
    streams: StreamTransport
    proxy: str
    no_proxy: Sequence[str] | str
    ca_bundle: str | os.PathLike[str]
    system_trust: bool
    client_cert: str | os.PathLike[str]
    client_key: str | os.PathLike[str]
    client_key_password: str
    pinned_keys: Sequence[str]
    log: Literal["off", "error", "warn", "info", "debug"]
    log_headers: bool
    log_allow_headers: Sequence[str]
    logger: logging.Logger
    redact: Redact
    tracing: bool
    metrics: bool
    tracer_provider: object
    meter_provider: object
    rate_limit: Literal["observe", "wait", "off"]
    user_agent_suffix: str
    hooks: Sequence[Hook]
    profile: str
    config_file: str | os.PathLike[str]
    credential_sources: Sequence[str]
    cli_path: str | os.PathLike[str]


class ClientOptions(_CommonOptions, total=False):
    """What `Client.load` takes.

    Every setting of `docs/config.md` section 3 under its catalogue name, and the
    code-only options.
    """

    token_provider: TokenProvider
    http_client: httpx.Client
    pipeline: Callable[[Pipeline[Middleware]], object]


class AsyncClientOptions(_CommonOptions, total=False):
    """What `AsyncClient.load` takes: `ClientOptions` with the `asyncio` forms."""

    token_provider: AsyncTokenProvider
    http_client: httpx.AsyncClient
    pipeline: Callable[[Pipeline[AsyncMiddleware]], object]


#: Options that are objects, kept out of resolution's values.
_OBJECTS = (
    "hooks",
    "logger",
    "redact",
    "tracer_provider",
    "meter_provider",
    "pipeline",
    "retry_budget_capacity",
)
#: Transport settings: set in code, the client builds its own HTTP client.
_NET = (
    "connect_timeout",
    "proxy",
    "no_proxy",
    "ca_bundle",
    "system_trust",
    "client_cert",
    "client_key",
    "client_key_password",
    "pinned_keys",
)


@dataclass
class _Built:
    """A resolution and the objects code passed, ready to build a client from."""

    resolution: Resolution
    objects: dict[str, Any]
    own_transport: bool


def _resolve_code(given: dict[str, Any], *, explicit: bool, **load: Any) -> _Built:  # noqa: ANN401
    code = {k: v for k, v in given.items() if v is not None}
    objects = {k: code.pop(k) for k in _OBJECTS if k in code}
    for k in (
        "ca_bundle",
        "client_cert",
        "client_key",
        "key_secret_file",
        "token_file",
        "cli_path",
        "config_file",
    ):
        if k in code:
            code[k] = os.fspath(code[k])
    if explicit:
        code["config_file"] = "off"
        res = resolve(code, LoadOptions(env={}, home=""), found=lambda _: False)
    else:
        res = resolve(code, **load)
    base = _check_url("the base URL", str(res.values["base_url"]), origin_only=True)
    res.values["base_url"] = base
    _check_url("the token URL", str(res.values["token_url"]), origin_only=False)
    own = not explicit or any(k in code for k in _NET)
    return _Built(res, objects, own)


def _net(values: dict[str, Any], doc: dict[str, Any]) -> NetSettings:
    return NetSettings(
        connect_timeout=float(values.get("connect_timeout", 10.0)),
        timeout=float(values["timeout"]),
        rule=ProxyRule.from_settings(values, doc["settings"]),
        ca_bundle=values.get("ca_bundle"),
        system_trust=bool(values.get("system_trust", True)),
        client_cert=values.get("client_cert"),
        client_key=values.get("client_key"),
        client_key_password=values.get("client_key_password"),
        pinned_keys=tuple(values.get("pinned_keys") or ()),
    )


def _engine(built: _Built, provider: object, ua: str) -> Engine:
    v = built.resolution.values
    o = built.objects
    profile = v.get("profile")
    cred = built.resolution.credential
    log = Log(
        str(v.get("log", "off")),
        logger=o.get("logger"),
        headers=bool(v.get("log_headers", False)),
        allow=v.get("log_allow_headers") or (),
        redact=o.get("redact"),
        profile=profile,
    )
    telemetry = Telemetry(
        tracing=v.get("tracing"),
        metrics=v.get("metrics"),
        tracer_provider=o.get("tracer_provider"),
        meter_provider=o.get("meter_provider"),
    )
    base = str(v["base_url"])
    capacity = int(o.get("retry_budget_capacity", 500))
    engine = Engine(
        base=base,
        host=urlsplit(base).netloc,
        user_agent=ua,
        timeout=float(v["timeout"]),
        connect_timeout=float(v.get("connect_timeout", 10.0)),
        total_timeout=float(v["total_timeout"]),
        idle=float(v["stream_idle_timeout"]),
        max_retries=int(v["max_retries"]),
        retry_base_delay=float(v["retry_base_delay"]),
        retry_max_delay=float(v["retry_max_delay"]),
        retry_after_max=float(v["retry_after_max"]),
        rate_limit_mode=str(v["rate_limit"]),
        budget=RetryBudget(capacity, enabled=bool(v["retry_budget"])),
        log=log,
        telemetry=telemetry,
        hooks=tuple(o.get("hooks") or ()),
        profile=profile,
        credential_source=cred.source if cred is not None else "code",
    )

    def report(event: str, fields: Any) -> None:  # noqa: ANN401
        if event == "token_exchange":
            telemetry.record(
                "exchanges",
                1,
                {
                    "inorbit.credential.source": engine.credential_source,
                    "error.type": fields.get("error"),
                },
            )
        else:
            log.emit("warn", event, fields)

    observe(provider, report)
    return engine


def _describe(built: _Built, names: Sequence[str]) -> ResolvedConfig:
    doc = built.resolution.doc
    doc["pipeline"] = list(names)
    return ResolvedConfig(doc)


class _Core:
    """What both clients share: building a call, finishing it."""

    engine: Engine
    _resolved: ResolvedConfig
    _streams: StreamTransport
    _logs_calls: bool
    _hooks_calls: bool

    def _url(self, op: Operation) -> str:
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
        return self.engine.base + p + (f"?{urlencode(pairs)}" if pairs else "")

    def _prepare(
        self, op: Operation, *, timeout: float | None, idempotency_key: str | None, stream: bool
    ) -> SdkRequest:
        if idempotency_key is not None and not op.idempotency_key:
            raise ConfigError(
                f"{op.name or op.path} does not take an idempotency key: the API would ignore "
                "it, so repeating the call would not be safe"
            )
        options = _CALL_OPTIONS.get()
        url = self._url(op)
        template = op.template or (op.path if op.name else None)
        state = _Call(
            op,
            template,
            timeout if timeout is not None else options.timeout,
            options.traceparent,
            idempotency_key,
        )
        headers = httpx.Headers({"accept": "text/event-stream" if stream else "application/json"})
        content: bytes | None = None
        if op.body is not None:
            headers["content-type"] = "application/json"
            content = _body(op.body)
        info = CallInfo(
            operation=op.name or op.path,
            idempotent=op.idempotent or op.method in IDEMPOTENT_METHODS,
            idempotency_key=None,
            request_id=request_id(),
            attempt=0,
            deadline=None,
            stream=stream,
            profile=self.engine.profile,
        )
        return SdkRequest(op.method, url, headers, content, info, state)

    def _finish(self, req: SdkRequest, resp: SdkResponse) -> RawResponse:
        state = cast("_Call", state_of(req))
        raw = RawResponse(
            resp.status,
            resp.headers,
            resp.body or b"",
            req.info.request_id,
            max(state.attempts, 1),
            state.idempotency_key,
            state.rate_limit,
        )
        if not 200 <= resp.status < 300:
            if resp.stream is not None:
                resp.stream.close()
            raise self._failed(req, ApiError(raw))
        self._log_call(req, resp.status, None)
        return raw

    def _failed(self, req: SdkRequest, error: InOrbitError) -> InOrbitError:
        state = cast("_Call", state_of(req))
        if error.request_id is None:
            error.request_id = req.info.request_id
        if error.idempotency_key is None:
            error.idempotency_key = state.idempotency_key
        if self._hooks_calls and self.engine.hooks:
            attempt = state.last_attempt or Attempt(
                req.info.operation,
                req.method,
                state.op.path,
                max(state.attempts, 1),
                req.info.request_id,
                state.idempotency_key,
            )
            for h in self.engine.hooks:
                h.on_error(attempt, error)
        status = error.status if isinstance(error, ApiError) else None
        self._log_call(req, status, error)
        if self._logs_calls:
            self.engine.log.emit(
                "error",
                "call_failed",
                {
                    "operation": req.info.operation,
                    "error_kind": error.kind,
                    "error_code": error.code if isinstance(error, ApiError) else None,
                    "status": status,
                    "request_id": req.info.request_id,
                },
            )
        return error

    def _log_call(self, req: SdkRequest, status: int | None, error: InOrbitError | None) -> None:
        state = cast("_Call", state_of(req))
        duration = time.monotonic() - state.started
        self.engine.telemetry.record(
            "call",
            duration,
            {
                "inorbit.operation": req.info.operation,
                "error.type": None if error is None else error_type(error),
            },
        )
        if not self._logs_calls:
            return
        self.engine.log.emit(
            "info",
            "call",
            {
                "operation": req.info.operation,
                "status": status,
                "error_kind": None if error is None else error.kind,
                "error_code": error.code if isinstance(error, ApiError) else None,
                "attempts": max(state.attempts, 1),
                "duration_ms": round(duration * 1000),
                "request_id": req.info.request_id,
                "server_request_id": error.server_request_id
                if error is not None
                else state.server_request_id,
            },
        )

    def config(self) -> ResolvedConfig:
        """The effective configuration and where each value came from (`describe()`)."""
        return self._resolved

    def rate_limit(self) -> RateLimit | None:
        """The latest rate-limit snapshot any call of this client saw."""
        with self.engine.lock:
            return self.engine.latest

    @property
    def base_url(self) -> str:
        """The API's origin this client calls."""
        return self.engine.base

    @staticmethod
    def decode(raw: RawResponse, into: object) -> Any:  # noqa: ANN401 - the caller's type
        try:
            value: object = json.loads(raw.body) if raw.body else {}
            return _adapter(into).validate_python(value)
        except (ValueError, ValidationError) as e:
            reason = e.errors()[0]["msg"] if isinstance(e, ValidationError) else str(e)
            raise DecodeError(reason, raw) from None

    def _socket_settings(
        self, ctx: ssl.SSLContext | None, rule: ProxyRule | None
    ) -> SocketSettings:
        e = self.engine
        return SocketSettings(
            e.base, e.host, e.user_agent, e.timeout, e.idle, e.max_retries, ctx, rule, e.budget
        )

    def _socket_body(self, op: Operation) -> dict[str, object]:
        if not op.rpc:
            raise ConfigError(
                f"{op.name or op.path} names no RPC, so it cannot stream over the socket; "
                'use streams="sse"'
            )
        return call_body([*op.params, *op.query])


def _credential_kwargs(
    *,
    token: str | None,
    key_id: str | None,
    key_secret: str | None,
    key_secret_file: object,
    token_file: object,
    scopes: Sequence[str] | None,
    token_provider: object,
) -> dict[str, Any]:
    """The explicit constructor's credential, checked with today's messages."""
    if token_provider is not None:
        return {"token_provider": token_provider}
    if token:
        return {"token": token}
    if key_id is not None and (key_secret is not None or key_secret_file is not None):
        if not scopes:
            raise ConfigError(
                'no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read '
                'account:read")'
            )
        found: dict[str, Any] = {"key_id": key_id, "scopes": list(scopes)}
        if key_secret is not None:
            found["key_secret"] = key_secret
        else:
            found["key_secret_file"] = key_secret_file
        return found
    if token_file is not None:
        return {"token_file": token_file}
    raise ConfigError("no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET")


class Client(_Core):
    """A blocking client for one credential. Safe to share between threads.

    Example:
        ```python
        from inorbithr import Client

        client = Client.load()  # code, INORBIT_*, the config file, then `iohr login`
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
        timeout: Seconds = 30.0,
        max_retries: int = 2,
        user_agent_suffix: str | None = None,
        hooks: Sequence[Hook] = (),
        http_client: httpx.Client | None = None,
        streams: StreamTransport = "sse",
        stream_idle_timeout: Seconds = 45.0,
        key_secret_file: str | os.PathLike[str] | None = None,
        token_file: str | os.PathLike[str] | None = None,
        connect_timeout: Seconds | None = None,
        total_timeout: Seconds | None = None,
        retry_base_delay: Seconds | None = None,
        retry_max_delay: Seconds | None = None,
        retry_after_max: Seconds | None = None,
        retry_budget: bool | None = None,
        retry_budget_capacity: int | None = None,
        proxy: str | None = None,
        no_proxy: Sequence[str] | str | None = None,
        ca_bundle: str | os.PathLike[str] | None = None,
        system_trust: bool | None = None,
        client_cert: str | os.PathLike[str] | None = None,
        client_key: str | os.PathLike[str] | None = None,
        client_key_password: str | None = None,
        pinned_keys: Sequence[str] | None = None,
        log: Literal["off", "error", "warn", "info", "debug"] | None = None,
        log_headers: bool | None = None,
        log_allow_headers: Sequence[str] | None = None,
        logger: logging.Logger | None = None,
        redact: Redact | None = None,
        tracing: bool | None = None,
        metrics: bool | None = None,
        tracer_provider: object = None,
        meter_provider: object = None,
        rate_limit: Literal["observe", "wait", "off"] | None = None,
        pipeline: Callable[[Pipeline[Middleware]], object] | None = None,
    ) -> None:
        """A client with one credential, from these options alone (no environment, no file).

        Give `token`, `token_file`, `key_id` with `key_secret` (or `key_secret_file`) and
        `scopes`, or a `token_provider`. Every other option is a setting of
        `docs/config.md` section 3 under its catalogue name; durations are seconds or a
        `timedelta`. `Client.load` reads the environment and the config file too.

        Args:
            token: An API token (from the console or `iohr token create`).
            key_id: An API key's id.
            key_secret: An API key's secret.
            scopes: The scopes to ask for with a key; no default.
            token_provider: Your own token source.
            base_url: The API's origin (plain http only to this machine).
            token_url: The token endpoint for a key.
            timeout: Each attempt, the answer's body included (default 30 s).
            max_retries: Retries after the first attempt (0 disables).
            user_agent_suffix: Appended to the user agent.
            hooks: Observers of every attempt.
            http_client: The `httpx.Client` to use (default: one of its own). Proxy, trust
                and certificate options then belong to it.
            streams: How streams open: `"sse"` (server-sent events, the default), or
                `"socket"`, every stream over one `/v1/ws` connection.
            stream_idle_timeout: Seconds a stream may be silent (no event, no keep-alive)
                before it fails, or on the socket reconnects.
            key_secret_file: A file holding the key's secret, read before each exchange.
            token_file: A file holding a bearer token, read again when it changes.
            connect_timeout: DNS, TCP and TLS per connection (default 10 s).
            total_timeout: One call, every attempt and wait included (default 120 s).
            retry_base_delay: Backoff base (default 0.5 s).
            retry_max_delay: Backoff cap (default 8 s).
            retry_after_max: The longest `Retry-After` waited for (default 60 s).
            retry_budget: The per-client retry quota (default on).
            retry_budget_capacity: The quota's size (default 500), for tests.
            proxy: An `http://` or `https://` proxy URL, or `"off"`.
            no_proxy: Hosts reached directly (section 6.2).
            ca_bundle: PEM certificates added to the system's trust store.
            system_trust: `False` trusts `ca_bundle` only.
            client_cert: A PEM client certificate chain, for mTLS.
            client_key: The PEM private key for `client_cert`.
            client_key_password: The password of an encrypted `client_key`.
            pinned_keys: Base64 SHA-256 hashes of public keys to pin, at least two.
            log: The log level: `off` (default), `error`, `warn`, `info`, `debug`.
            log_headers: Log allowlisted header values at `debug`.
            log_allow_headers: Header names added to the allowlist.
            logger: Where records go (default: `logging.getLogger("inorbithr")`).
            redact: Sees every record last; returns it changed, or `None` to drop it.
            tracing: Spans through OpenTelemetry (default: on when installed).
            metrics: Metrics through OpenTelemetry (default: as `tracing`).
            tracer_provider: The tracer provider (default: the global one).
            meter_provider: The meter provider (default: the global one).
            rate_limit: `observe` (default), `wait` or `off` (section 7.8).
            pipeline: Edits the middleware pipeline: `lambda p: p.add_per_retry(m)`.

        Raises:
            ConfigError: No credential is given, a key has no scopes, a URL is not https,
                or a setting is not valid.
        """
        options = _explicit(locals())
        built = _resolve_code(options, explicit=True)
        self._setup(built, http_client, pipeline, token_provider)

    @classmethod
    def load(
        cls,
        *,
        load_options: LoadOptions | None = None,
        profile_type: str | None = None,
        **options: Unpack[ClientOptions],
    ) -> Client:
        """A client from code, the environment, the config file and the `iohr` login.

        Each setting takes the first source that sets it: these options, then
        `INORBIT_*`, then the config file's profile and `[sdk]` tables, then the default
        (`docs/config.md` section 2). Credentials come from the first source of the chain
        that has any (section 5.1). Nothing is contacted until the first call.

        Args:
            load_options: The environment, OS and home directory to read instead of the
                process's own.
            profile_type: For a generated surface: the typed profile to resolve for.
            **options: Any `Client` option, and `profile`, `config_file`,
                `credential_sources`, `cli_path`.

        Raises:
            ConfigError: Every problem found, each with its setting and source.
        """
        o = cast("dict[str, Any]", dict(options))
        if profile_type is not None and "profile" in o:
            raise ConfigError("a typed profile is its own profile; leave profile out")
        built = _resolve_code(o, explicit=False, options=load_options, profile_type=profile_type)
        self = cls.__new__(cls)
        self._setup(built, o.get("http_client"), o.get("pipeline"), o.get("token_provider"))
        return self

    @classmethod
    def from_env(cls, profile: str | None = None, **options: Any) -> Client:  # noqa: ANN401
        """A client from the environment, as before; `load` reads more.

        A named profile reads `INORBIT_<PROFILE>_TOKEN`, or `INORBIT_<PROFILE>_KEY_ID`,
        `_KEY_SECRET` and `_SCOPES`, and nothing else; without one, the bare `INORBIT_*`
        names. `INORBIT_<PROFILE>_BASE_URL` and `_TOKEN_URL` are read first, then
        `INORBIT_BASE_URL` and `INORBIT_TOKEN_URL`.

        Args:
            profile: The profile's name (`acme-ci` or `ACME_CI`), or `None`.
            **options: Any other `Client` option.

        Raises:
            ConfigError: Naming the variables to set when no credential is there.
        """
        _, found = _env(profile)
        return cls(**found, **options)

    def _setup(
        self,
        built: _Built,
        http_client: httpx.Client | None,
        edit: Callable[[Pipeline[Middleware]], object] | None,
        token_provider: TokenProvider | None,
    ) -> None:
        v = built.resolution.values
        ua = user_agent(v.get("user_agent_suffix"))
        self._ctx: ssl.SSLContext | None = None
        self._rule: ProxyRule | None = None
        self._owned = http_client is None
        if http_client is not None:
            self._http = http_client
        elif built.own_transport:
            net = _net(v, built.resolution.doc)
            self._http, self._ctx = sync_http(net)
            self._rule = net.rule
        else:
            self._http = httpx.Client(follow_redirects=False)
        cred = built.resolution.credential
        assert cred is not None
        if token_provider is not None:
            cred.provider = token_provider
        self._provider = sync_provider(
            cred, token_url=str(v["token_url"]), http=self._http, user_agent=ua
        )
        self.engine = _engine(built, self._provider, ua)
        p = sync_builtins(self.engine, self._provider)
        if edit is not None:
            edit(p)
        ms = p.middlewares()
        self._logs_calls = any(isinstance(m, LoggingMiddleware) for m in ms)
        self._hooks_calls = any(isinstance(m, HooksMiddleware) for m in ms)
        self._run: CallNext = compose(ms, sync_transport(self.engine, self._http))
        self._resolved = _describe(built, p.names)
        self._streams = cast("StreamTransport", v["streams"])
        self._socket: SyncSocket | None = None
        self._socket_lock = threading.Lock()

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

    def request(
        self,
        op: Operation,
        into: type[T],
        *,
        timeout: float | None = None,
        idempotency_key: str | None = None,
    ) -> Response[T]:
        """Calls `op` and reads its JSON answer as `into`.

        Args:
            op: The operation.
            into: The type the answer is read as.
            timeout: Replaces `timeout` for each attempt and shortens `total_timeout`.
            idempotency_key: The key to send, for an operation that takes one; one is
                generated otherwise.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token, size and decoding failures.
        """
        raw = self.send(op, timeout=timeout, idempotency_key=idempotency_key)
        return Response(cast("T", self.decode(raw, into)), raw)

    def send(
        self, op: Operation, *, timeout: float | None = None, idempotency_key: str | None = None
    ) -> RawResponse:
        """Calls `op` and hands back the answer as it came, a 2xx one only.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token and size failures.
        """
        req = self._prepare(op, timeout=timeout, idempotency_key=idempotency_key, stream=False)
        try:
            resp = self._run(req)
        except InOrbitError as e:
            raise self._failed(req, e) from None
        return self._finish(req, resp)

    def stream(self, op: Operation, into: type[T]) -> Stream[T]:
        """Opens the stream `op` answers and reads each event as `into`.

        The stream opens on the first step of the iteration, over server-sent events or
        the client's socket (`streams`); design.md section 7 has the rules.

        Raises:
            ConfigError: The socket is asked for and `op` names no RPC.
        """
        if self._streams == "socket":
            body = self._socket_body(op)
            return Stream(self._socket_items(op.rpc, body, into))
        return Stream(self._sse_items(op, into))

    def _socket_items(self, rpc: str, body: dict[str, object], into: type[T]) -> Iterator[T]:
        with self._socket_lock:
            if self._socket is None:
                self._socket = SyncSocket(
                    self._socket_settings(self._ctx, self._rule), self._provider
                )
            socket = self._socket
        yield from cast("Iterator[T]", socket.items(rpc, body, into))

    def _sse_items(self, op: Operation, into: type[T]) -> Iterator[T]:
        e = self.engine
        req = self._prepare(op, timeout=None, idempotency_key=None, stream=True)
        state = cast("_Call", state_of(req))
        try:
            resp = self._run(req)
        except InOrbitError as x:
            raise self._failed(req, x) from None
        raw = self._finish(req, resp)
        stream = cast("httpx.Response", resp.stream)
        parser = SseParser()
        try:
            for chunk in stream.iter_bytes():
                for name, data in parser.feed(chunk):
                    if name == "error":
                        raise problem_error(
                            data.encode(), raw.headers, raw.request_id, raw.attempts
                        )
                    yield cast("T", decode_item(data, into, raw.headers, raw.request_id))
        except httpx.TimeoutException:
            raise self._failed(req, ApiTimeoutError(e.host, e.idle)) from None
        except httpx.TransportError as x:
            raise self._failed(req, ApiConnectionError(e.host, type(x).__name__)) from None
        except InOrbitError as x:
            raise self._failed(req, x) from None
        finally:
            stream.close()
            for done in state.on_close:
                done()


class AsyncClient(_Core):
    """`Client` for asyncio. Safe to share between tasks on one event loop.

    Example:
        ```python
        from inorbithr import AsyncClient

        client = AsyncClient.load()
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
        timeout: Seconds = 30.0,
        max_retries: int = 2,
        user_agent_suffix: str | None = None,
        hooks: Sequence[Hook] = (),
        http_client: httpx.AsyncClient | None = None,
        streams: StreamTransport = "sse",
        stream_idle_timeout: Seconds = 45.0,
        key_secret_file: str | os.PathLike[str] | None = None,
        token_file: str | os.PathLike[str] | None = None,
        connect_timeout: Seconds | None = None,
        total_timeout: Seconds | None = None,
        retry_base_delay: Seconds | None = None,
        retry_max_delay: Seconds | None = None,
        retry_after_max: Seconds | None = None,
        retry_budget: bool | None = None,
        retry_budget_capacity: int | None = None,
        proxy: str | None = None,
        no_proxy: Sequence[str] | str | None = None,
        ca_bundle: str | os.PathLike[str] | None = None,
        system_trust: bool | None = None,
        client_cert: str | os.PathLike[str] | None = None,
        client_key: str | os.PathLike[str] | None = None,
        client_key_password: str | None = None,
        pinned_keys: Sequence[str] | None = None,
        log: Literal["off", "error", "warn", "info", "debug"] | None = None,
        log_headers: bool | None = None,
        log_allow_headers: Sequence[str] | None = None,
        logger: logging.Logger | None = None,
        redact: Redact | None = None,
        tracing: bool | None = None,
        metrics: bool | None = None,
        tracer_provider: object = None,
        meter_provider: object = None,
        rate_limit: Literal["observe", "wait", "off"] | None = None,
        pipeline: Callable[[Pipeline[AsyncMiddleware]], object] | None = None,
    ) -> None:
        """A client with one credential; the options are `Client`'s.

        Raises:
            ConfigError: No credential is given, a key has no scopes, a URL is not https,
                or a setting is not valid.
        """
        options = _explicit(locals())
        built = _resolve_code(options, explicit=True)
        self._setup(built, http_client, pipeline, token_provider)

    @classmethod
    def load(
        cls,
        *,
        load_options: LoadOptions | None = None,
        profile_type: str | None = None,
        **options: Unpack[AsyncClientOptions],
    ) -> AsyncClient:
        """A client from code, the environment, the config file and the `iohr` login.

        Resolved as `Client.load` resolves; the `iohr` login runs as a subprocess of the
        event loop.

        Raises:
            ConfigError: Every problem found, each with its setting and source.
        """
        o = cast("dict[str, Any]", dict(options))
        if profile_type is not None and "profile" in o:
            raise ConfigError("a typed profile is its own profile; leave profile out")
        built = _resolve_code(o, explicit=False, options=load_options, profile_type=profile_type)
        self = cls.__new__(cls)
        self._setup(built, o.get("http_client"), o.get("pipeline"), o.get("token_provider"))
        return self

    @classmethod
    def from_env(cls, profile: str | None = None, **options: Any) -> AsyncClient:  # noqa: ANN401
        """A client from the environment, read as `Client.from_env` does.

        Raises:
            ConfigError: Naming the variables to set when no credential is there.
        """
        _, found = _env(profile)
        return cls(**found, **options)

    def _setup(
        self,
        built: _Built,
        http_client: httpx.AsyncClient | None,
        edit: Callable[[Pipeline[AsyncMiddleware]], object] | None,
        token_provider: AsyncTokenProvider | None,
    ) -> None:
        v = built.resolution.values
        ua = user_agent(v.get("user_agent_suffix"))
        self._ctx: ssl.SSLContext | None = None
        self._rule: ProxyRule | None = None
        self._owned = http_client is None
        if http_client is not None:
            self._http = http_client
        elif built.own_transport:
            net = _net(v, built.resolution.doc)
            self._http, self._ctx = async_http(net)
            self._rule = net.rule
        else:
            self._http = httpx.AsyncClient(follow_redirects=False)
        cred = built.resolution.credential
        assert cred is not None
        if token_provider is not None:
            cred.provider = token_provider
        self._provider = async_provider(
            cred, token_url=str(v["token_url"]), http=self._http, user_agent=ua
        )
        self.engine = _engine(built, self._provider, ua)
        p = async_builtins(self.engine, self._provider)
        if edit is not None:
            edit(p)
        ms = p.middlewares()
        self._logs_calls = any(isinstance(m, AsyncLoggingMiddleware) for m in ms)
        self._hooks_calls = any(isinstance(m, AsyncHooksMiddleware) for m in ms)
        self._run: AsyncCallNext = acompose(ms, async_transport(self.engine, self._http))
        self._resolved = _describe(built, p.names)
        self._streams = cast("StreamTransport", v["streams"])
        self._socket: AsyncSocket | None = None

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
        self,
        op: Operation,
        into: type[T],
        *,
        timeout: float | None = None,
        idempotency_key: str | None = None,
    ) -> Response[T]:
        """Calls `op` and reads its JSON answer as `into`.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token, size and decoding failures.
        """
        raw = await self.send(op, timeout=timeout, idempotency_key=idempotency_key)
        return Response(cast("T", self.decode(raw, into)), raw)

    async def send(
        self, op: Operation, *, timeout: float | None = None, idempotency_key: str | None = None
    ) -> RawResponse:
        """Calls `op` and hands back the answer as it came, a 2xx one only.

        Raises:
            ApiError: For an error answer.
            InOrbitError: For connection, timeout, token and size failures.
        """
        req = self._prepare(op, timeout=timeout, idempotency_key=idempotency_key, stream=False)
        try:
            resp = await self._run(req)
        except InOrbitError as e:
            raise self._failed(req, e) from None
        return self._finish(req, resp)

    def stream(self, op: Operation, into: type[T]) -> AsyncStream[T]:
        """Opens the stream `op` answers and reads each event as `into`, for `async for`.

        The stream opens on the first step of the iteration, over server-sent events or
        the client's socket (`streams`); design.md section 7 has the rules.

        Raises:
            ConfigError: The socket is asked for and `op` names no RPC.
        """
        if self._streams == "socket":
            body = self._socket_body(op)
            return AsyncStream(self._socket_items(op.rpc, body, into))
        return AsyncStream(self._sse_items(op, into))

    async def _socket_items(
        self, rpc: str, body: dict[str, object], into: type[T]
    ) -> AsyncIterator[T]:
        if self._socket is None:
            self._socket = AsyncSocket(self._socket_settings(self._ctx, self._rule), self._provider)
        items = cast("AsyncIterator[T]", self._socket.items(rpc, body, into))
        try:
            async for item in items:
                yield item
        finally:
            aclose = getattr(items, "aclose", None)
            if aclose is not None:
                await aclose()

    async def _sse_items(self, op: Operation, into: type[T]) -> AsyncIterator[T]:
        e = self.engine
        req = self._prepare(op, timeout=None, idempotency_key=None, stream=True)
        state = cast("_Call", state_of(req))
        try:
            resp = await self._run(req)
        except InOrbitError as x:
            raise self._failed(req, x) from None
        if not 200 <= resp.status < 300 and resp.stream is not None:
            await resp.stream.aclose()
            resp.stream = None
        raw = self._finish(req, resp)
        stream = cast("httpx.Response", resp.stream)
        parser = SseParser()
        try:
            async for chunk in stream.aiter_bytes():
                for name, data in parser.feed(chunk):
                    if name == "error":
                        raise problem_error(
                            data.encode(), raw.headers, raw.request_id, raw.attempts
                        )
                    yield cast("T", decode_item(data, into, raw.headers, raw.request_id))
        except httpx.TimeoutException:
            raise self._failed(req, ApiTimeoutError(e.host, e.idle)) from None
        except httpx.TransportError as x:
            raise self._failed(req, ApiConnectionError(e.host, type(x).__name__)) from None
        except InOrbitError as x:
            raise self._failed(req, x) from None
        finally:
            await stream.aclose()
            for done in state.on_close:
                done()


#: Explicit-constructor arguments that equal their default are left to the default.
_DEFAULTS: dict[str, object] = {
    "base_url": DEFAULT_BASE_URL,
    "token_url": DEFAULT_TOKEN_URL,
    "timeout": 30.0,
    "max_retries": 2,
    "streams": "sse",
    "stream_idle_timeout": 45.0,
}


def _explicit(args: dict[str, Any]) -> dict[str, Any]:
    """An explicit constructor's arguments as resolution's code options."""
    args = {k: v for k, v in args.items() if k not in ("self", "__class__")}
    cred = _credential_kwargs(
        token=args.pop("token"),
        key_id=args.pop("key_id"),
        key_secret=args.pop("key_secret"),
        key_secret_file=args.pop("key_secret_file"),
        token_file=args.pop("token_file"),
        scopes=args.pop("scopes"),
        token_provider=args.pop("token_provider"),
    )
    http = args.pop("http_client")
    out: dict[str, Any] = {}
    for k, v in args.items():
        if v is None or (k in _DEFAULTS and v == _DEFAULTS[k]) or (k == "hooks" and not v):
            continue
        out[k] = v
    if http is not None:
        out["http_client"] = http
    out.update(cred)
    if "token_provider" in out:
        out.pop("scopes", None)
    for k in ("timeout", "stream_idle_timeout"):
        if k in out:
            out[k] = _seconds(cast("Seconds", out[k]))
    return out
