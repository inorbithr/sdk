"""The middleware pipeline: named steps every call goes through (`docs/config.md` section 7).

A call runs the per-call middlewares once, then `retry`, which runs the per-retry
middlewares once per attempt, then the transport. Both clients share the model; the
built-ins come in a blocking and an `asyncio` form.
"""

from __future__ import annotations

import asyncio
import json
import random
import threading
import time
import uuid
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass, field, replace
from typing import TYPE_CHECKING, Any, Generic, Protocol, TypeVar, cast

import httpx

from inorbithr._auth import AsyncStaticToken, AsyncTokenProvider, StaticToken, TokenProvider
from inorbithr._config import PIPELINE
from inorbithr._errors import (
    ApiConnectionError,
    ApiError,
    ApiTimeoutError,
    AuthError,
    ConfigError,
    InOrbitError,
    RawResponse,
    TooLargeError,
)
from inorbithr._hooks import Attempt, Hook
from inorbithr._ratelimit import RateLimit, parse_rate_limit
from inorbithr._retry import COST_OTHER, COST_THROTTLED, RetryBudget, backoff, retry_after_seconds
from inorbithr._telemetry import Log, Span, Telemetry, path_only, redacted_url

if TYPE_CHECKING:
    from inorbithr._client import Operation

#: The largest answer read, 16 MiB.
MAX_BODY = 16 * 1024 * 1024
#: Methods `retry` may repeat on their own.
IDEMPOTENT_METHODS = frozenset({"GET", "PUT", "DELETE", "HEAD"})
#: Built-ins that can be replaced but not removed (section 7.3).
PROTECTED = frozenset({"retry", "auth", "timeout"})


@dataclass(frozen=True)
class CallInfo:
    """What a middleware may read about the call it handles (section 7.13)."""

    operation: str
    """The operation (`radar.list_digests`), or the path for a raw call."""
    idempotent: bool
    """Whether `retry` may repeat it without an idempotency key."""
    idempotency_key: str | None
    """The `Idempotency-Key` the call sends, once `idempotency_key` has run."""
    request_id: str
    """The call's `x-request-id`, the same on every attempt."""
    attempt: int
    """1-based on each attempt; 0 in the per-call stage."""
    deadline: float | None
    """When the call must end, in `time.monotonic()` seconds, once `deadline` has run."""
    stream: bool
    """Whether the answer is a stream, whose body a middleware must not read."""
    profile: str | None
    """The configuration profile the client was loaded with."""


@dataclass
class SdkRequest:
    """A request as it goes through the pipeline. Headers may be changed in place."""

    method: str
    """The HTTP method."""
    url: str
    """The full URL."""
    headers: httpx.Headers
    """The headers sent; change them in place or replace the request."""
    body: bytes | None
    """The body, or `None`; never a stream."""
    info: CallInfo
    """The call's metadata, read only."""
    _state: _Call | None = field(default=None, repr=False, compare=False)
    """The SDK's own per-call state; not for middlewares."""


@dataclass
class SdkResponse:
    """An answer as it comes back through the pipeline."""

    status: int
    """The HTTP status."""
    headers: httpx.Headers
    """The answer's headers."""
    body: bytes | None = field(default=None, repr=False)
    """The body, or `None` for a stream."""
    stream: Any = field(default=None, repr=False)
    """The open answer when `info.stream`: a middleware must not read it."""


#: What a blocking middleware calls to run the rest of the pipeline.
CallNext = Callable[[SdkRequest], SdkResponse]
#: What an `asyncio` middleware awaits to run the rest of the pipeline.
AsyncCallNext = Callable[[SdkRequest], Awaitable[SdkResponse]]


class Middleware(Protocol):
    """One named step of a blocking client's pipeline.

    It may change the request, answer without calling `call_next`, call it more than
    once, or inspect what comes back. It must not log secrets or bodies.
    """

    @property
    def name(self) -> str:
        """The step's unique name in the pipeline."""
        ...

    def __call__(self, request: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Handles `request`, usually by calling `call_next`."""
        ...


class AsyncMiddleware(Protocol):
    """`Middleware` for `AsyncClient`: `__call__` is a coroutine."""

    @property
    def name(self) -> str:
        """The step's unique name in the pipeline."""
        ...

    async def __call__(self, request: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Handles `request`, usually by awaiting `call_next`."""
        ...


class _Named(Protocol):
    @property
    def name(self) -> str: ...


M = TypeVar("M", bound=_Named)


class Pipeline(Generic[M]):
    """A client's ordered list of middlewares, edited by name at construction.

    Every method returns the pipeline, so edits chain:
    `lambda p: p.add_per_retry(Probe()).remove("rate_limit")`.
    """

    def __init__(self, entries: Sequence[tuple[str, M]]) -> None:
        """The built-ins, outermost first."""
        self._entries: list[tuple[str, M]] = list(entries)

    @property
    def names(self) -> list[str]:
        """Every middleware's name, outermost first."""
        return [n for n, _ in self._entries]

    def _index(self, name: str) -> int:
        for i, (n, _) in enumerate(self._entries):
            if n == name:
                return i
        raise ConfigError(f"the pipeline has no middleware named {name!r}")

    def _fresh(self, m: M) -> str:
        name = m.name
        if not name or name in self.names:
            raise ConfigError(f"the pipeline already has a middleware named {name!r}")
        return name

    def add_per_call(self, m: M) -> Pipeline[M]:
        """Adds `m` just before `retry`, after earlier additions: once per call."""
        self._entries.insert(self._index("retry"), (self._fresh(m), m))
        return self

    def add_per_retry(self, m: M) -> Pipeline[M]:
        """Adds `m` just before `timeout`, after earlier additions: once per attempt."""
        self._entries.insert(self._index("timeout"), (self._fresh(m), m))
        return self

    def insert_before(self, name: str, m: M) -> Pipeline[M]:
        """Adds `m` just before the middleware named `name`."""
        self._entries.insert(self._index(name), (self._fresh(m), m))
        return self

    def insert_after(self, name: str, m: M) -> Pipeline[M]:
        """Adds `m` just after the middleware named `name`."""
        self._entries.insert(self._index(name) + 1, (self._fresh(m), m))
        return self

    def replace(self, name: str, m: M) -> Pipeline[M]:
        """Puts `m` in the place of `name`; it keeps that name."""
        self._entries[self._index(name)] = (name, m)
        return self

    def remove(self, name: str) -> Pipeline[M]:
        """Drops the middleware named `name`; `retry`, `auth` and `timeout` stay."""
        if name in PROTECTED:
            raise ConfigError(
                f"{name} cannot be removed; replace it, or switch it off with a setting "
                "(max_retries = 0 for retry)"
            )
        del self._entries[self._index(name)]
        return self

    def middlewares(self) -> list[M]:
        """The middlewares, outermost first."""
        return [m for _, m in self._entries]

    def __repr__(self) -> str:
        """The names, in order."""
        return f"Pipeline({self.names})"


# --- the shared state ----------------------------------------------------------------


@dataclass
class _Call:
    """One call's state, shared by every built-in across its attempts."""

    op: Operation
    template: str | None
    timeout: float | None
    traceparent: str | None
    idempotency_key: str | None
    started: float = field(default_factory=time.monotonic)
    attempts: int = 0
    refreshed: bool = False
    attempt_timeout: float | None = None
    rate_limit: RateLimit | None = None
    last_attempt: Attempt | None = None
    call_span: Span = None
    attempt_span: Span = None
    server_request_id: str | None = None
    on_close: list[Callable[[], None]] = field(default_factory=list[Callable[[], None]])


@dataclass
class Engine:
    """What every built-in of one client reads: settings, budget, log, telemetry."""

    base: str
    host: str
    user_agent: str
    timeout: float
    connect_timeout: float
    total_timeout: float
    idle: float
    max_retries: int
    retry_base_delay: float
    retry_max_delay: float
    retry_after_max: float
    rate_limit_mode: str
    budget: RetryBudget
    log: Log
    telemetry: Telemetry
    hooks: tuple[Hook, ...]
    profile: str | None
    credential_source: str
    latest: RateLimit | None = None
    lock: threading.Lock = field(default_factory=threading.Lock)

    def observe(self, rl: RateLimit) -> None:
        """Keeps the latest snapshot, which `wait` mode and `client.rate_limit()` read."""
        with self.lock:
            self.latest = rl

    def raw(self, req: SdkRequest, resp: SdkResponse) -> RawResponse:
        """The answer as callers and hooks see it."""
        state = state_of(req)
        return RawResponse(
            resp.status,
            resp.headers,
            resp.body or b"",
            req.info.request_id,
            max(req.info.attempt, 1),
            req.info.idempotency_key,
            state.rate_limit if state is not None else None,
        )

    def attempt_of(self, req: SdkRequest) -> Attempt:
        """The `Attempt` hooks see."""
        state = state_of(req)
        op = state.op if state is not None else None
        return Attempt(
            req.info.operation,
            req.method,
            op.path if op is not None else path_only(req.url),
            req.info.attempt,
            req.info.request_id,
            req.info.idempotency_key,
        )

    def log_ids(self, req: SdkRequest) -> dict[str, object]:
        """`trace_id` and `span_id` of the attempt's span, for log records."""
        state = state_of(req)
        trace_id, span_id = Telemetry.ids(state.attempt_span if state is not None else None)
        return {"trace_id": trace_id, "span_id": span_id}


def state_of(req: SdkRequest) -> _Call | None:
    """The SDK's own state of the call `req` belongs to."""
    return req._state  # pyright: ignore[reportPrivateUsage]


def error_type(e: InOrbitError) -> str:
    """The `error.type` attribute for an error: the status, or the error's class."""
    return str(e.status) if isinstance(e, ApiError) else type(e).__name__


def _retry_detail(resp: SdkResponse) -> float | None:
    if not resp.body:
        return None
    try:
        envelope = json.loads(resp.body)
    except ValueError:
        return None
    details = (
        cast("dict[str, object]", envelope).get("details") if isinstance(envelope, dict) else None
    )
    if not isinstance(details, list):
        return None
    for d in cast("list[object]", details):
        if isinstance(d, dict):
            dd = cast("dict[str, object]", d)
            after = dd.get("after_seconds")
            if dd.get("type") == "retry" and isinstance(after, int | float):
                return float(after)
    return None


@dataclass(frozen=True)
class _Verdict:
    """What `retry` decided after an attempt."""

    retry: bool
    delay: float = 0.0
    cost: int = 0
    reason: str = ""


def _decide(  # noqa: PLR0911 - one return per reason not to retry
    e: Engine, req: SdkRequest, resp: SdkResponse | None, err: InOrbitError | None, retries: int
) -> _Verdict:
    """Whether to retry, after how long, at what cost (sections 7.4 and 6.1)."""
    safe = req.info.idempotent or "idempotency-key" in req.headers
    if resp is not None:
        if resp.status not in (429, 503, 504):
            return _Verdict(retry=False)
        reason = str(resp.status)
    elif isinstance(err, ApiTimeoutError | ApiConnectionError):
        reason = err.kind
    else:
        return _Verdict(retry=False)
    if not safe or retries >= e.max_retries:
        return _Verdict(retry=False)
    asked = None
    if resp is not None:
        asked = retry_after_seconds(resp.headers)
        header = asked is not None
        if asked is None:
            asked = _retry_detail(resp)
        if asked is not None and asked > e.retry_after_max:
            return _Verdict(retry=False)
        cost = (
            COST_THROTTLED if resp.status == 429 or (resp.status == 503 and header) else COST_OTHER
        )
    else:
        cost = COST_OTHER
    delay = asked if asked is not None else backoff(retries, e.retry_base_delay, e.retry_max_delay)
    deadline = req.info.deadline
    if deadline is not None and time.monotonic() + delay >= deadline:
        return _Verdict(retry=False)
    if not e.budget.draw(cost):
        return _Verdict(retry=False)
    return _Verdict(retry=True, delay=delay, cost=cost, reason=reason)


def _before_retry(e: Engine, req: SdkRequest, v: _Verdict) -> None:
    attempt = e.attempt_of(req)
    for h in e.hooks:
        h.on_retry(attempt, v.reason, v.delay)
    e.log.emit(
        "warn",
        "retry",
        {
            "operation": req.info.operation,
            "attempt": req.info.attempt,
            "reason": v.reason,
            "delay_ms": round(v.delay * 1000),
            "request_id": req.info.request_id,
        },
    )
    e.telemetry.record(
        "retries", 1, {"inorbit.operation": req.info.operation, "inorbit.retry.reason": v.reason}
    )


def _attempt_request(req: SdkRequest, number: int) -> SdkRequest:
    state = state_of(req)
    if state is not None:
        state.attempts = number
    return replace(req, headers=httpx.Headers(req.headers), info=replace(req.info, attempt=number))


def _rate_wait(e: Engine, req: SdkRequest) -> float:
    """How long `wait` mode holds this attempt; raises when that passes the deadline."""
    if e.rate_limit_mode != "wait":
        return 0.0
    with e.lock:
        latest = e.latest
    wait = latest.wait() if latest is not None else 0.0
    if wait <= 0:
        return 0.0
    wait += random.uniform(0, 0.1)  # noqa: S311 - jitter, not a secret
    deadline = req.info.deadline
    if deadline is not None and time.monotonic() + wait > deadline:
        error = ApiTimeoutError(e.host, e.total_timeout)
        error.args = (
            f"the call's deadline passes while waiting for the rate-limit window ({e.host})",
        )
        raise error
    e.log.emit(
        "warn",
        "rate_limit_wait",
        {
            "operation": req.info.operation,
            "attempt": req.info.attempt,
            "delay_ms": round(wait * 1000),
            "request_id": req.info.request_id,
        },
    )
    return wait


def _observe_rate(e: Engine, req: SdkRequest, resp: SdkResponse) -> None:
    if e.rate_limit_mode == "off":
        return
    rl = parse_rate_limit(resp.headers)
    state = state_of(req)
    if rl is not None:
        e.observe(rl)
        if state is not None:
            state.rate_limit = rl


def _attempt_span(e: Engine, req: SdkRequest) -> Span:
    state = state_of(req)
    if not e.telemetry.tracing:
        if state is not None and state.traceparent and "traceparent" not in req.headers:
            req.headers["traceparent"] = state.traceparent
        return None
    url = httpx.URL(req.url)
    template = state.template if state is not None else None
    span = e.telemetry.start_attempt(
        state.call_span if state is not None else None,
        f"{req.method} {template}" if template else req.method,
        {
            "http.request.method": req.method,
            "server.address": url.host,
            "server.port": url.port or (443 if url.scheme == "https" else 80),
            "url.full": redacted_url(req.url),
            "url.template": template,
            "http.request.resend_count": req.info.attempt - 1 if req.info.attempt > 1 else None,
        },
    )
    if state is not None:
        state.attempt_span = span
    Telemetry.inject(span, req.headers)
    if state is not None and state.traceparent and "traceparent" not in req.headers:
        # No SDK behind the API (its spans do not record): the caller's goes as it came.
        req.headers["traceparent"] = state.traceparent
    return span


def _attempt_done(  # noqa: PLR0917 - internal, one call site per form
    e: Engine,
    req: SdkRequest,
    span: Span,
    started: float,
    resp: SdkResponse | None,
    err: InOrbitError | None,
) -> None:
    status = resp.status if resp is not None else None
    etype = (
        (str(status) if status is not None and status >= 400 else None)
        if err is None
        else error_type(err)
    )
    replayed = (
        resp.headers.get("idempotency-replayed", "").lower() == "true" if resp is not None else None
    )
    Telemetry.end(
        span,
        etype,
        {
            "http.response.status_code": status,
            "inorbit.server_request_id": resp.headers.get("x-request-id")
            if resp is not None
            else None,
            "inorbit.idempotency_replayed": replayed or None,
        },
    )
    url = httpx.URL(req.url)
    e.telemetry.record(
        "attempt",
        time.monotonic() - started,
        {
            "http.request.method": req.method,
            "server.address": url.host,
            "server.port": url.port or (443 if url.scheme == "https" else 80),
            "http.response.status_code": status,
            "error.type": etype,
        },
    )


def _log_request(e: Engine, req: SdkRequest) -> None:
    if not e.log.on("debug"):
        return
    e.log.emit(
        "debug",
        "request",
        {
            "operation": req.info.operation,
            "method": req.method,
            "path": path_only(req.url),
            "attempt": req.info.attempt,
            "request_id": req.info.request_id,
            "headers": e.log.headers(req.headers, response=False),
            **e.log_ids(req),
        },
    )


def _log_response(e: Engine, req: SdkRequest, resp: SdkResponse, started: float) -> None:
    if not e.log.on("debug"):
        return
    e.log.emit(
        "debug",
        "response",
        {
            "operation": req.info.operation,
            "method": req.method,
            "path": path_only(req.url),
            "attempt": req.info.attempt,
            "request_id": req.info.request_id,
            "status": resp.status,
            "duration_ms": round((time.monotonic() - started) * 1000),
            "server_request_id": resp.headers.get("x-request-id"),
            "headers": e.log.headers(resp.headers, response=True),
            **e.log_ids(req),
        },
    )


def _attempt_timeout(e: Engine, req: SdkRequest) -> float:
    state = state_of(req)
    t = state.timeout if state is not None and state.timeout is not None else e.timeout
    deadline = req.info.deadline
    if deadline is not None:
        left = deadline - time.monotonic()
        if left <= 0:
            raise ApiTimeoutError(
                e.host, e.total_timeout if state is None or state.timeout is None else state.timeout
            )
        t = min(t, left)
    if state is not None:
        state.attempt_timeout = t
    return t


def _refused(provider: object) -> AuthError:
    del provider
    return AuthError(
        "the API refused the token (HTTP 401): it is expired or revoked; create a new API token "
        "in the console or with `iohr token create`, and set it again"
    )


# --- the blocking built-ins ----------------------------------------------------------


class _Builtin:
    name = ""

    def __init__(self, e: Engine) -> None:
        self.e = e

    def __repr__(self) -> str:
        return f"<built-in {self.name}>"


class RequestIdMiddleware(_Builtin):
    """Sends the call's `x-request-id` on every attempt."""

    name = "request_id"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Sets the header."""
        req.headers["x-request-id"] = req.info.request_id
        return call_next(req)


class UserAgentMiddleware(_Builtin):
    """Sends the SDK's user agent."""

    name = "user_agent"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Sets the header."""
        req.headers["user-agent"] = self.e.user_agent
        return call_next(req)


def _with_key(req: SdkRequest) -> SdkRequest:
    state = state_of(req)
    if state is None or not state.op.idempotency_key:
        return req
    key = state.idempotency_key or str(uuid.uuid4())
    state.idempotency_key = key
    req.headers["idempotency-key"] = key
    return replace(req, info=replace(req.info, idempotency_key=key))


class IdempotencyKeyMiddleware(_Builtin):
    """Gives a write that takes `Idempotency-Key` one key for every attempt (section 7.5)."""

    name = "idempotency_key"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Sets the header once per call."""
        return call_next(_with_key(req))


def _call_span(e: Engine, req: SdkRequest) -> Span:
    state = state_of(req)
    if not e.telemetry.tracing or state is None:
        return None
    span = e.telemetry.start_call(
        req.info.operation,
        {"inorbit.operation": req.info.operation, "inorbit.request_id": req.info.request_id},
        state.traceparent,
    )
    state.call_span = span
    return span


def _call_span_done(
    req: SdkRequest,
    span: Span,
    resp: SdkResponse | None,
    err: InOrbitError | None,
) -> None:
    if span is None:
        return
    if err is not None:
        Telemetry.end(span, error_type(err))
    elif resp is not None and resp.status >= 300:
        Telemetry.end(span, str(resp.status))
    elif req.info.stream and (state := state_of(req)) is not None:
        state.on_close.append(lambda: Telemetry.end(span))
    else:
        Telemetry.end(span)


class CallTracingMiddleware(_Builtin):
    """One `INTERNAL` span per call; a stream's covers it until it ends."""

    name = "call_tracing"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Wraps the call in its span."""
        span = _call_span(self.e, req)
        try:
            resp = call_next(req)
        except InOrbitError as err:
            _call_span_done(req, span, None, err)
            raise
        _call_span_done(req, span, resp, None)
        return resp


def _with_deadline(e: Engine, req: SdkRequest) -> tuple[SdkRequest, float]:
    state = state_of(req)
    total = e.total_timeout
    if state is not None and state.timeout is not None:
        total = min(total, state.timeout)
    start = state.started if state is not None else time.monotonic()
    return replace(req, info=replace(req.info, deadline=start + total)), total


class DeadlineMiddleware(_Builtin):
    """Gives the call its deadline: `total_timeout`, or the caller's shorter one."""

    name = "deadline"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Sets `info.deadline`; `retry`, `rate_limit` and `timeout` hold to it."""
        return call_next(_with_deadline(self.e, req)[0])


def _discard(resp: SdkResponse) -> None:
    if resp.stream is not None:
        resp.stream.close()


async def _adiscard(resp: SdkResponse) -> None:
    if resp.stream is not None:
        await resp.stream.aclose()


class RetryMiddleware(_Builtin):
    """Runs the per-retry stage once per attempt, retrying by the rules of section 7.4."""

    name = "retry"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Attempts until success, a final answer, or the budget or deadline says stop."""
        e = self.e
        retries = 0
        last_cost = 0
        while True:
            attempt = _attempt_request(req, retries + 1)
            resp: SdkResponse | None = None
            err: InOrbitError | None = None
            try:
                resp = call_next(attempt)
            except (ApiTimeoutError, ApiConnectionError) as x:
                err = x
            v = _decide(e, attempt, resp, err, retries)
            if not v.retry:
                if resp is not None and 200 <= resp.status < 300:
                    e.budget.refund(last_cost if retries else 1)
                if err is not None:
                    raise err
                assert resp is not None
                return resp
            if resp is not None:
                _discard(resp)
            _before_retry(e, attempt, v)
            time.sleep(v.delay)
            retries += 1
            last_cost = v.cost


class AuthMiddleware(_Builtin):
    """`Authorization: Bearer <token>`, and one fresh token after a 401."""

    name = "auth"

    def __init__(self, e: Engine, provider: TokenProvider) -> None:
        """Sends `provider`'s token."""
        super().__init__(e)
        self.provider = provider

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Sets the header; resends once with a fresh token after a 401."""
        req.headers["authorization"] = f"Bearer {self.provider.token().access}"
        resp = call_next(req)
        state = state_of(req)
        if resp.status != 401 or state is None or state.refreshed:
            return resp
        state.refreshed = True
        if isinstance(self.provider, StaticToken):
            _discard(resp)
            raise _refused(self.provider)
        _discard(resp)
        self.provider.invalidate()
        again = replace(req, headers=httpx.Headers(req.headers))
        again.headers["authorization"] = f"Bearer {self.provider.token().access}"
        return call_next(again)


class RateLimitMiddleware(_Builtin):
    """Reads the rate-limit headers; in `wait` mode, holds an attempt until the reset."""

    name = "rate_limit"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Waits if asked, then keeps the answer's snapshot."""
        wait = _rate_wait(self.e, req)
        if wait > 0:
            time.sleep(wait)
        resp = call_next(req)
        _observe_rate(self.e, req, resp)
        return resp


class AttemptTracingMiddleware(_Builtin):
    """One `CLIENT` span per attempt, and `traceparent` from it (or from the caller)."""

    name = "attempt_tracing"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Wraps the attempt in its span."""
        started = time.monotonic()
        span = _attempt_span(self.e, req)
        try:
            resp = call_next(req)
        except InOrbitError as err:
            _attempt_done(self.e, req, span, started, None, err)
            raise
        _attempt_done(self.e, req, span, started, resp, None)
        return resp


class LoggingMiddleware(_Builtin):
    """`request` and `response` records at `debug` (section 7.9)."""

    name = "logging"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Logs the attempt and its answer, never a body or a secret."""
        started = time.monotonic()
        _log_request(self.e, req)
        resp = call_next(req)
        _log_response(self.e, req, resp, started)
        return resp


class HooksMiddleware(_Builtin):
    """Runs the client's `Hook`s on every attempt."""

    name = "hooks"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """`on_request`, then `on_response`."""
        attempt = self.e.attempt_of(req)
        state = state_of(req)
        if state is not None:
            state.last_attempt = attempt
        for h in self.e.hooks:
            h.on_request(attempt)
        resp = call_next(req)
        if self.e.hooks:
            raw = self.e.raw(req, resp)
            for h in self.e.hooks:
                h.on_response(attempt, raw)
        return resp


class TimeoutMiddleware(_Builtin):
    """Holds the attempt to `timeout`, or what is left of the call's deadline."""

    name = "timeout"

    def __call__(self, req: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Sets the attempt's limit, which the transport enforces through the body."""
        _attempt_timeout(self.e, req)
        return call_next(req)


# --- the asyncio built-ins -----------------------------------------------------------


class AsyncRequestIdMiddleware(_Builtin):
    """`RequestIdMiddleware` for `AsyncClient`."""

    name = "request_id"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Sets the header."""
        req.headers["x-request-id"] = req.info.request_id
        return await call_next(req)


class AsyncUserAgentMiddleware(_Builtin):
    """`UserAgentMiddleware` for `AsyncClient`."""

    name = "user_agent"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Sets the header."""
        req.headers["user-agent"] = self.e.user_agent
        return await call_next(req)


class AsyncIdempotencyKeyMiddleware(_Builtin):
    """`IdempotencyKeyMiddleware` for `AsyncClient`."""

    name = "idempotency_key"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Sets the header once per call."""
        return await call_next(_with_key(req))


class AsyncCallTracingMiddleware(_Builtin):
    """`CallTracingMiddleware` for `AsyncClient`."""

    name = "call_tracing"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Wraps the call in its span."""
        span = _call_span(self.e, req)
        try:
            resp = await call_next(req)
        except InOrbitError as err:
            _call_span_done(req, span, None, err)
            raise
        _call_span_done(req, span, resp, None)
        return resp


class AsyncDeadlineMiddleware(_Builtin):
    """`DeadlineMiddleware` for `AsyncClient`, which also cancels at the deadline."""

    name = "deadline"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Sets `info.deadline` and cancels what is still running when it passes."""
        req, total = _with_deadline(self.e, req)
        left = (req.info.deadline or 0) - time.monotonic()
        try:
            async with asyncio.timeout(max(left, 0)):
                return await call_next(req)
        except TimeoutError:
            raise ApiTimeoutError(self.e.host, total) from None


class AsyncRetryMiddleware(_Builtin):
    """`RetryMiddleware` for `AsyncClient`."""

    name = "retry"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Attempts until success, a final answer, or the budget or deadline says stop."""
        e = self.e
        retries = 0
        last_cost = 0
        while True:
            attempt = _attempt_request(req, retries + 1)
            resp: SdkResponse | None = None
            err: InOrbitError | None = None
            try:
                resp = await call_next(attempt)
            except (ApiTimeoutError, ApiConnectionError) as x:
                err = x
            v = _decide(e, attempt, resp, err, retries)
            if not v.retry:
                if resp is not None and 200 <= resp.status < 300:
                    e.budget.refund(last_cost if retries else 1)
                if err is not None:
                    raise err
                assert resp is not None
                return resp
            if resp is not None:
                await _adiscard(resp)
            _before_retry(e, attempt, v)
            await asyncio.sleep(v.delay)
            retries += 1
            last_cost = v.cost


class AsyncAuthMiddleware(_Builtin):
    """`AuthMiddleware` for `AsyncClient`."""

    name = "auth"

    def __init__(self, e: Engine, provider: AsyncTokenProvider) -> None:
        """Sends `provider`'s token."""
        super().__init__(e)
        self.provider = provider

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Sets the header; resends once with a fresh token after a 401."""
        req.headers["authorization"] = f"Bearer {(await self.provider.token()).access}"
        resp = await call_next(req)
        state = state_of(req)
        if resp.status != 401 or state is None or state.refreshed:
            return resp
        state.refreshed = True
        await _adiscard(resp)
        if isinstance(self.provider, AsyncStaticToken):
            raise _refused(self.provider)
        await self.provider.invalidate()
        again = replace(req, headers=httpx.Headers(req.headers))
        again.headers["authorization"] = f"Bearer {(await self.provider.token()).access}"
        return await call_next(again)


class AsyncRateLimitMiddleware(_Builtin):
    """`RateLimitMiddleware` for `AsyncClient`."""

    name = "rate_limit"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Waits if asked, then keeps the answer's snapshot."""
        wait = _rate_wait(self.e, req)
        if wait > 0:
            await asyncio.sleep(wait)
        resp = await call_next(req)
        _observe_rate(self.e, req, resp)
        return resp


class AsyncAttemptTracingMiddleware(_Builtin):
    """`AttemptTracingMiddleware` for `AsyncClient`."""

    name = "attempt_tracing"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Wraps the attempt in its span."""
        started = time.monotonic()
        span = _attempt_span(self.e, req)
        try:
            resp = await call_next(req)
        except InOrbitError as err:
            _attempt_done(self.e, req, span, started, None, err)
            raise
        _attempt_done(self.e, req, span, started, resp, None)
        return resp


class AsyncLoggingMiddleware(_Builtin):
    """`LoggingMiddleware` for `AsyncClient`."""

    name = "logging"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Logs the attempt and its answer, never a body or a secret."""
        started = time.monotonic()
        _log_request(self.e, req)
        resp = await call_next(req)
        _log_response(self.e, req, resp, started)
        return resp


class AsyncHooksMiddleware(_Builtin):
    """`HooksMiddleware` for `AsyncClient`."""

    name = "hooks"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """`on_request`, then `on_response`."""
        attempt = self.e.attempt_of(req)
        state = state_of(req)
        if state is not None:
            state.last_attempt = attempt
        for h in self.e.hooks:
            h.on_request(attempt)
        resp = await call_next(req)
        if self.e.hooks:
            raw = self.e.raw(req, resp)
            for h in self.e.hooks:
                h.on_response(attempt, raw)
        return resp


class AsyncTimeoutMiddleware(_Builtin):
    """`TimeoutMiddleware` for `AsyncClient`: the attempt is cancelled at its limit."""

    name = "timeout"

    async def __call__(self, req: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        """Cancels the attempt when it runs past its limit."""
        t = _attempt_timeout(self.e, req)
        try:
            async with asyncio.timeout(t):
                return await call_next(req)
        except TimeoutError:
            raise ApiTimeoutError(self.e.host, t) from None


def sync_builtins(e: Engine, provider: TokenProvider) -> Pipeline[Middleware]:
    """A blocking client's default pipeline."""
    ms: list[Middleware] = [
        RequestIdMiddleware(e),
        UserAgentMiddleware(e),
        IdempotencyKeyMiddleware(e),
        CallTracingMiddleware(e),
        DeadlineMiddleware(e),
        RetryMiddleware(e),
        AuthMiddleware(e, provider),
        RateLimitMiddleware(e),
        AttemptTracingMiddleware(e),
        LoggingMiddleware(e),
        HooksMiddleware(e),
        TimeoutMiddleware(e),
    ]
    assert [m.name for m in ms] == list(PIPELINE)
    return Pipeline([(m.name, m) for m in ms])


def async_builtins(e: Engine, provider: AsyncTokenProvider) -> Pipeline[AsyncMiddleware]:
    """An `asyncio` client's default pipeline."""
    ms: list[AsyncMiddleware] = [
        AsyncRequestIdMiddleware(e),
        AsyncUserAgentMiddleware(e),
        AsyncIdempotencyKeyMiddleware(e),
        AsyncCallTracingMiddleware(e),
        AsyncDeadlineMiddleware(e),
        AsyncRetryMiddleware(e),
        AsyncAuthMiddleware(e, provider),
        AsyncRateLimitMiddleware(e),
        AsyncAttemptTracingMiddleware(e),
        AsyncLoggingMiddleware(e),
        AsyncHooksMiddleware(e),
        AsyncTimeoutMiddleware(e),
    ]
    return Pipeline([(m.name, m) for m in ms])


# --- the transport -------------------------------------------------------------------


def _too_large(headers: httpx.Headers) -> bool:
    length = headers.get("content-length", "")
    return length.isdigit() and int(length) > MAX_BODY


def _timeouts(e: Engine, req: SdkRequest) -> tuple[float, httpx.Timeout]:
    state = state_of(req)
    t = state.attempt_timeout if state is not None and state.attempt_timeout else e.timeout
    read = e.idle if req.info.stream else t
    return t, httpx.Timeout(t, connect=min(e.connect_timeout, t), read=read)


def _request(req: SdkRequest, timeout: httpx.Timeout) -> httpx.Request:
    r = httpx.Request(req.method, req.url, headers=req.headers, content=req.body)
    r.extensions["timeout"] = timeout.as_dict()
    return r


def sync_transport(e: Engine, http: httpx.Client) -> CallNext:
    """The innermost step: one HTTP exchange, the body read under the attempt's limit."""

    def send(req: SdkRequest) -> SdkResponse:
        t, timeout = _timeouts(e, req)
        until = time.monotonic() + t
        try:
            resp = http.send(_request(req, timeout), stream=True)
        except httpx.TimeoutException:
            raise ApiTimeoutError(e.host, t) from None
        except httpx.TransportError as x:
            raise ApiConnectionError(e.host, type(x).__name__) from None
        if req.info.stream and 200 <= resp.status_code < 300:
            return SdkResponse(resp.status_code, resp.headers, None, resp)
        try:
            if _too_large(resp.headers):
                raise TooLargeError
            body = bytearray()
            for chunk in resp.iter_bytes():
                body.extend(chunk)
                if len(body) > MAX_BODY:
                    raise TooLargeError
                if time.monotonic() > until:
                    raise ApiTimeoutError(e.host, t)
        except httpx.TimeoutException:
            raise ApiTimeoutError(e.host, t) from None
        except httpx.TransportError as x:
            raise ApiConnectionError(e.host, type(x).__name__) from None
        finally:
            resp.close()
        return SdkResponse(resp.status_code, resp.headers, bytes(body))

    return send


def async_transport(e: Engine, http: httpx.AsyncClient) -> AsyncCallNext:
    """The innermost step for `AsyncClient`."""

    async def send(req: SdkRequest) -> SdkResponse:
        t, timeout = _timeouts(e, req)
        try:
            resp = await http.send(_request(req, timeout), stream=True)
        except httpx.TimeoutException:
            raise ApiTimeoutError(e.host, t) from None
        except httpx.TransportError as x:
            raise ApiConnectionError(e.host, type(x).__name__) from None
        if req.info.stream and 200 <= resp.status_code < 300:
            return SdkResponse(resp.status_code, resp.headers, None, resp)
        try:
            if _too_large(resp.headers):
                raise TooLargeError
            body = bytearray()
            async for chunk in resp.aiter_bytes():
                body.extend(chunk)
                if len(body) > MAX_BODY:
                    raise TooLargeError
        except httpx.TimeoutException:
            raise ApiTimeoutError(e.host, t) from None
        except httpx.TransportError as x:
            raise ApiConnectionError(e.host, type(x).__name__) from None
        finally:
            await resp.aclose()
        return SdkResponse(resp.status_code, resp.headers, bytes(body))

    return send


class _Link:
    """One middleware bound to the rest of the pipeline."""

    def __init__(self, m: Middleware, rest: CallNext) -> None:
        self.m = m
        self.rest = rest

    def __call__(self, req: SdkRequest) -> SdkResponse:
        return self.m(req, self.rest)


class _AsyncLink:
    """One `asyncio` middleware bound to the rest of the pipeline."""

    def __init__(self, m: AsyncMiddleware, rest: AsyncCallNext) -> None:
        self.m = m
        self.rest = rest

    async def __call__(self, req: SdkRequest) -> SdkResponse:
        return await self.m(req, self.rest)


def compose(middlewares: Sequence[Middleware], terminal: CallNext) -> CallNext:
    """The pipeline as one function, outermost first."""
    fn: CallNext = terminal
    for m in reversed(middlewares):
        fn = _Link(m, fn)
    return fn


def acompose(middlewares: Sequence[AsyncMiddleware], terminal: AsyncCallNext) -> AsyncCallNext:
    """The `asyncio` pipeline as one function, outermost first."""
    fn: AsyncCallNext = terminal
    for m in reversed(middlewares):
        fn = _AsyncLink(m, fn)
    return fn
