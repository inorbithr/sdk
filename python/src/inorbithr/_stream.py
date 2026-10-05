"""Streams: server-sent events and the multiplexed socket (design.md section 7).

A streaming operation yields its model once per event. `Stream` is what the blocking
client hands back (an iterator and a context manager), `AsyncStream` what the `asyncio`
one does. Both open lazily: the first step of the iteration opens the stream, so an
error from opening is raised there.

Over server-sent events the body is parsed by the WHATWG event-stream rules; over the
socket every stream of a client shares one `/v1/ws` connection, reconnected when the
server ends it and closed when its last stream ends.
"""

from __future__ import annotations

import asyncio
import codecs
import contextlib
import json
import queue
import threading
import time
from collections.abc import AsyncIterator, Iterator, Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Generic, Literal, TypeAlias, TypeVar, cast

import httpx
from pydantic import TypeAdapter, ValidationError
from websockets.asyncio.client import ClientConnection as AsyncConnection
from websockets.asyncio.client import connect as async_connect
from websockets.exceptions import ConnectionClosed, InvalidHandshake, InvalidStatus
from websockets.sync.client import ClientConnection as SyncConnection
from websockets.sync.client import connect as sync_connect

from inorbithr._errors import (
    CODES,
    ApiConnectionError,
    ApiError,
    ApiTimeoutError,
    DecodeError,
    InOrbitError,
    RawResponse,
    TooLargeError,
)
from inorbithr._retry import (
    COST_OTHER,
    COST_THROTTLED,
    RetryBudget,
    backoff,
    request_id,
    retry_after,
    retryable_status,
)

if TYPE_CHECKING:
    import ssl

    from inorbithr._auth import AsyncTokenProvider, TokenProvider
    from inorbithr._transport import ProxyRule

T = TypeVar("T")

#: How a client opens its streams.
StreamTransport: TypeAlias = Literal["sse", "socket"]

#: The largest event (its data) or frame read, 1 MiB.
MAX_EVENT = 1024 * 1024
#: Items waiting for a slow reader, per stream, on the socket.
QUEUE = 64
#: How often the client pings the socket at most; the server pings every 15 s.
PING_EVERY = 15.0

_EMPTY = httpx.Headers()


class Stream(Generic[T]):
    """The items of a stream, for `for`; closed on exit from `with`, or by `close()`.

    Example:
        ```python
        with api.events.stream_events(types="key.created") as events:
            for event in events:
                print(event.type)
        ```
    """

    def __init__(self, items: Iterator[T]) -> None:
        """A stream over `items`, a generator the client made."""
        self._items = items

    def __iter__(self) -> Stream[T]:
        """The stream itself."""
        return self

    def __next__(self) -> T:
        """The next item; the first call opens the stream.

        Raises:
            InOrbitError: The stream could not open, or failed.
        """
        return next(self._items)

    def close(self) -> None:
        """Stops the stream: the connection closes, or the socket call is cancelled."""
        close = getattr(self._items, "close", None)
        if close is not None:
            close()

    def __enter__(self) -> Stream[T]:
        """The stream, closed on exit."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the stream."""
        self.close()


class AsyncStream(Generic[T]):
    """The items of a stream, for `async for`.

    Closed on exit from `async with`, or by `aclose()`.

    Example:
        ```python
        async with api.events.stream_events() as events:
            async for event in events:
                print(event.type)
        ```
    """

    def __init__(self, items: AsyncIterator[T]) -> None:
        """A stream over `items`, an async generator the client made."""
        self._items = items

    def __aiter__(self) -> AsyncStream[T]:
        """The stream itself."""
        return self

    async def __anext__(self) -> T:
        """The next item; the first call opens the stream.

        Raises:
            InOrbitError: The stream could not open, or failed.
        """
        return await self._items.__anext__()

    async def aclose(self) -> None:
        """Stops the stream: the connection closes, or the socket call is cancelled."""
        aclose = getattr(self._items, "aclose", None)
        if aclose is not None:
            await aclose()

    async def __aenter__(self) -> AsyncStream[T]:
        """The stream, closed on exit."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the stream."""
        await self.aclose()


# ------------------------------------------------------------------ server-sent events


class SseParser:
    """The WHATWG event-stream rules, fed bytes as they arrive.

    Lines end with LF, CRLF or CR; a line starting `:` is a comment; `data:` lines join
    with a newline, one leading space dropped; `event:` names the event; a blank line
    dispatches; every other field is ignored. An event's data, and a line, are at most
    `limit` bytes.
    """

    def __init__(self, limit: int = MAX_EVENT) -> None:
        """A parser that refuses an event larger than `limit`."""
        self._limit = limit
        self._decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self._buffer = ""
        self._data: list[str] = []
        self._size = 0
        self._event = ""
        self._first = True

    def feed(self, chunk: bytes) -> list[tuple[str, str]]:
        """The events `chunk` completes, as `(name, data)`.

        Raises:
            TooLargeError: An event or a line is larger than the limit.
        """
        self._buffer += self._decoder.decode(chunk)
        if self._first and self._buffer:
            # A byte-order mark at the start of the stream is not part of the first line.
            self._buffer = self._buffer.removeprefix("﻿")
            self._first = False
        events: list[tuple[str, str]] = []
        while True:
            cut = _line_end(self._buffer)
            if cut is None:
                break
            line, rest = cut
            self._buffer = rest
            event = self._line(line)
            if event is not None:
                events.append(event)
        if len(self._buffer) > self._limit:
            raise TooLargeError(f"a stream event is larger than {self._limit} bytes")
        return events

    def _line(self, line: str) -> tuple[str, str] | None:
        if line == "":
            if not self._data:
                self._event = ""
                return None
            event = (self._event or "message", "\n".join(self._data))
            self._data, self._size, self._event = [], 0, ""
            return event
        if line.startswith(":"):
            return None
        name, _, value = line.partition(":")
        value = value.removeprefix(" ")
        if name == "data":
            self._size += len(value) + 1
            if self._size > self._limit:
                raise TooLargeError(f"a stream event is larger than {self._limit} bytes")
            self._data.append(value)
        elif name == "event":
            self._event = value
        return None


def _line_end(buffer: str) -> tuple[str, str] | None:
    """The first whole line and the rest, or `None` while the line may go on."""
    for i, ch in enumerate(buffer):
        if ch == "\n":
            return buffer[:i], buffer[i + 1 :]
        if ch == "\r":
            if i + 1 == len(buffer):
                return None  # a CR at the end may be the first half of a CRLF
            skip = 2 if buffer[i + 1] == "\n" else 1
            return buffer[:i], buffer[i + skip :]
    return None


def problem_error(
    envelope: bytes, headers: httpx.Headers, request_id: str, attempts: int
) -> ApiError:
    """The `ApiError` an `error` event or frame carries; its status from its code."""
    code: object = None
    with contextlib.suppress(ValueError):
        wire = json.loads(envelope)
        if isinstance(wire, dict):
            code = cast("dict[str, object]", wire).get("code")
    status = CODES.get(code, 500) if isinstance(code, str) else 500
    return ApiError(RawResponse(status, headers, envelope, request_id, attempts))


_ADAPTERS: dict[object, TypeAdapter[Any]] = {}


def decode_item(data: bytes | str, into: object, headers: httpx.Headers, request_id: str) -> Any:  # noqa: ANN401 - the caller's type
    """One event's JSON as `into`.

    Raises:
        DecodeError: The data is not JSON, or not `into`.
    """
    raw_bytes = data.encode() if isinstance(data, str) else data
    adapter = _ADAPTERS.get(into)
    if adapter is None:
        adapter = TypeAdapter(cast("type[Any]", into))
        _ADAPTERS[into] = adapter
    try:
        return adapter.validate_json(raw_bytes)
    except ValidationError as e:
        raise DecodeError(
            e.errors()[0]["msg"], RawResponse(200, headers, raw_bytes, request_id, 1)
        ) from None


# ------------------------------------------------------------------ the socket


def socket_url(base: str) -> str:
    """`wss://host/v1/ws` for `https://host` (`ws://` for plain http on loopback)."""
    if base.startswith("https://"):
        return "wss://" + base[len("https://") :] + "/v1/ws"
    return "ws://" + base[len("http://") :] + "/v1/ws"


def call_body(params: Sequence[tuple[str, object]]) -> dict[str, object]:
    """The call frame's body: the parameters by wire name.

    Unset ones are left out and dotted names nested (`a.b` is `{"a": {"b": ...}}`).
    """
    body: dict[str, object] = {}
    for name, given in params:
        if given is None:
            continue
        value = list(cast("tuple[object, ...]", given)) if isinstance(given, tuple) else given
        node = body
        *parents, leaf = name.split(".")
        for p in parents:
            child = node.setdefault(p, {})
            if not isinstance(child, dict):
                break
            node = cast("dict[str, object]", child)
        else:
            node[leaf] = value
    return body


@dataclass
class _Call:
    """One stream on the socket."""

    id: str
    method: str
    body: dict[str, object]
    items: Any = field(repr=False)
    """`queue.Queue` or `asyncio.Queue` of `(kind, value)`."""
    done: bool = False

    def frame(self) -> str:
        return json.dumps({"type": "call", "id": self.id, "method": self.method, "body": self.body})


@dataclass(frozen=True)
class SocketSettings:
    """What a socket needs from its client."""

    base: str
    host: str
    user_agent: str
    timeout: float
    idle: float
    max_retries: int
    tls_context: ssl.SSLContext | None = None
    """The client's TLS context; `None` for the library's default."""
    rule: ProxyRule | None = None
    """The client's proxy rule; `None` lets the library read the environment, as before."""
    budget: RetryBudget | None = None
    """The client's retry budget, which reconnects draw from."""

    def proxy(self) -> str | Literal[True] | None:
        """The proxy for the upgrade, by the client's rule."""
        if self.rule is None:
            return True
        return self.rule.proxy_for(socket_url(self.base).replace("ws", "http", 1))

    def tls(self) -> ssl.SSLContext | None:
        """The TLS context for a `wss://` upgrade."""
        return self.tls_context if self.base.startswith("https") else None

    def draw(self, status: int | None, throttled: bool) -> bool:
        """Whether the budget pays for a retry of the upgrade."""
        if self.budget is None:
            return True
        cost = COST_THROTTLED if status == 429 or (status == 503 and throttled) else COST_OTHER
        return self.budget.draw(cost)


def _upgrade_error(e: InvalidStatus, request_id: str, number: int) -> RawResponse:
    response = e.response
    headers = httpx.Headers(list(response.headers.raw_items()))
    return RawResponse(
        response.status_code, headers, bytes(response.body or b""), request_id, number
    )


def _item(kind: str, frame: dict[str, object], call: _Call | None) -> tuple[str, object] | None:
    """What a stream's frame hands its reader: data, the end, or the error."""
    if call is None:
        return None
    if kind == "data":
        return ("data", json.dumps(frame.get("body")).encode())
    if kind == "end":
        return ("end", None)
    if kind == "error":
        return ("error", problem_error(json.dumps(frame).encode(), _EMPTY, call.id, 1))
    return None


def _frame_kind(raw: str | bytes) -> tuple[str, str | None, dict[str, object]]:
    try:
        frame = json.loads(raw)
    except ValueError:
        return "", None, {}
    if not isinstance(frame, dict):
        return "", None, {}
    f = cast("dict[str, object]", frame)
    kind = f.get("type")
    ident = f.get("id")
    return (
        kind if isinstance(kind, str) else "",
        ident if isinstance(ident, str) and ident else None,
        f,
    )


class SyncSocket:
    """One `/v1/ws` connection carrying every stream of a blocking client.

    A reader thread hands each frame to its stream's bounded queue. The connection opens
    with the first stream and closes after the last; when the server ends it (any
    socket-level error but `unauthenticated`, a close, silence past the idle timeout) it
    is opened again and every call that had not ended is issued again.
    """

    def __init__(self, settings: SocketSettings, provider: TokenProvider) -> None:
        """A socket for one client."""
        self._s = settings
        self._provider = provider
        self._lock = threading.Lock()
        self._conn: SyncConnection | None = None
        self._calls: dict[str, _Call] = {}
        self._next = 0

    def items(self, method: str, body: dict[str, object], into: object) -> Iterator[Any]:
        """The items of one stream, a call on the socket.

        Raises:
            InOrbitError: The socket could not open, or the call failed.
        """
        call = self._start(method, body)
        try:
            while True:
                kind, value = call.items.get()
                if kind == "data":
                    yield decode_item(value, into, _EMPTY, call.id)
                elif kind == "end":
                    return
                else:
                    raise cast("InOrbitError", value)
        finally:
            self._finish(call)

    def _start(self, method: str, body: dict[str, object]) -> _Call:
        with self._lock:
            self._next += 1
            call = _Call(str(self._next), method, body, queue.Queue(QUEUE))
            if self._conn is None:
                self._conn = self._connect()
                threading.Thread(
                    target=self._read, args=(self._conn,), name="inorbithr-socket", daemon=True
                ).start()
            self._calls[call.id] = call
            self._conn.send(call.frame())
            return call

    def _finish(self, call: _Call) -> None:
        with self._lock:
            if self._calls.pop(call.id, None) is not None and not call.done and self._conn:
                with contextlib.suppress(ConnectionClosed, OSError):
                    self._conn.send(json.dumps({"type": "cancel", "id": call.id}))
            call.done = True
            if not self._calls and self._conn is not None:
                conn, self._conn = self._conn, None
                conn.close()

    def _connect(self) -> SyncConnection:
        s = self._s
        rid = request_id()
        retries, refreshed, number = 0, False, 0
        while True:
            number += 1
            try:
                return sync_connect(
                    socket_url(s.base),
                    additional_headers={
                        "authorization": f"Bearer {self._provider.token().access}",
                        "x-request-id": rid,
                    },
                    ssl=s.tls(),
                    proxy=s.proxy(),
                    user_agent_header=s.user_agent,
                    compression=None,
                    open_timeout=s.timeout,
                    ping_interval=min(PING_EVERY, s.idle / 3),
                    ping_timeout=s.idle,
                    max_size=MAX_EVENT,
                    max_queue=QUEUE,
                )
            except InvalidStatus as e:
                raw = _upgrade_error(e, rid, number)
                if raw.status == 401 and not refreshed:
                    self._provider.invalidate()
                    refreshed = True
                    continue
                wait = retry_after(raw.headers)
                if (
                    retryable_status(raw.status)
                    and retries < s.max_retries
                    and s.draw(raw.status, wait is not None)
                ):
                    time.sleep(wait if wait is not None else backoff(retries))
                    retries += 1
                    continue
                raise ApiError(raw) from None
            except (OSError, InvalidHandshake, TimeoutError) as e:
                if retries < s.max_retries and s.draw(None, False):
                    time.sleep(backoff(retries))
                    retries += 1
                    continue
                if isinstance(e, TimeoutError):
                    raise ApiTimeoutError(s.host, s.timeout) from None
                raise ApiConnectionError(s.host, type(e).__name__) from None

    def _deliver(self, call: _Call, item: tuple[str, object]) -> None:
        """Hands an item to a stream, waiting while its queue is full, unless it ended."""
        while not call.done:
            try:
                call.items.put(item, timeout=0.1)
            except queue.Full:
                continue
            return

    def _fail_all(self, error: InOrbitError) -> None:
        with self._lock:
            calls = list(self._calls.values())
            self._calls.clear()
            conn, self._conn = self._conn, None
        for c in calls:
            self._deliver(c, ("error", error))
        if conn is not None:
            conn.close()

    def _read(self, conn: SyncConnection) -> None:
        reconnects = 0
        while True:
            try:
                raw = conn.recv()
            except (ConnectionClosed, OSError, TimeoutError):
                raw = None
            verdict = "reopen" if raw is None else self._frame(raw)
            if verdict == "stop":
                return
            if verdict == "frame":
                reconnects = 0
                continue
            fresh = self._reopen(conn, reconnects)
            if fresh is None:
                return
            conn = fresh
            reconnects += 1

    def _frame(self, raw: str | bytes) -> Literal["frame", "reopen", "stop"]:
        """Hands one frame to its stream; says what the connection does next."""
        kind, ident, frame = _frame_kind(raw)
        if kind == "error" and ident is None:
            if frame.get("code") == "unauthenticated":
                envelope = json.dumps(frame).encode()
                self._fail_all(problem_error(envelope, _EMPTY, "socket", 1))
                return "stop"
            return "reopen"  # the server ends the socket: open it again
        with self._lock:
            call = self._calls.get(ident or "")
            if call is not None and kind in ("end", "error"):
                self._calls.pop(call.id, None)
        item = _item(kind, frame, call)
        if call is not None and item is not None:
            self._deliver(call, item)
        return "frame"

    def _reopen(self, conn: SyncConnection, reconnects: int) -> SyncConnection | None:
        """A new connection with every open call issued again, or `None` when done."""
        with contextlib.suppress(Exception):
            conn.close()
        with self._lock:
            if self._conn is not conn or not self._calls:
                return None
        if reconnects > self._s.max_retries:
            self._fail_all(ApiConnectionError(self._s.host, "the socket closed"))
            return None
        time.sleep(backoff(reconnects) if reconnects else 0)
        try:
            fresh = self._connect()
        except InOrbitError as e:
            self._fail_all(e)
            return None
        with self._lock:
            if self._conn is not conn or not self._calls:
                fresh.close()
                return None
            self._conn = fresh
            for c in self._calls.values():
                fresh.send(c.frame())
        return fresh


class AsyncSocket:
    """`SyncSocket` for `asyncio`: a reader task instead of a thread."""

    def __init__(self, settings: SocketSettings, provider: AsyncTokenProvider) -> None:
        """A socket for one client."""
        self._s = settings
        self._provider = provider
        self._lock = asyncio.Lock()
        self._conn: AsyncConnection | None = None
        self._calls: dict[str, _Call] = {}
        self._next = 0
        self._reader: asyncio.Task[None] | None = None

    async def items(self, method: str, body: dict[str, object], into: object) -> AsyncIterator[Any]:
        """The items of one stream, a call on the socket.

        Raises:
            InOrbitError: The socket could not open, or the call failed.
        """
        call = await self._start(method, body)
        try:
            while True:
                kind, value = await call.items.get()
                if kind == "data":
                    yield decode_item(value, into, _EMPTY, call.id)
                elif kind == "end":
                    return
                else:
                    raise cast("InOrbitError", value)
        finally:
            await self._finish(call)

    async def _start(self, method: str, body: dict[str, object]) -> _Call:
        async with self._lock:
            self._next += 1
            call = _Call(str(self._next), method, body, asyncio.Queue(QUEUE))
            if self._conn is None:
                self._conn = await self._connect()
                self._reader = asyncio.create_task(self._read(self._conn))
            self._calls[call.id] = call
            await self._conn.send(call.frame())
            return call

    async def _finish(self, call: _Call) -> None:
        async with self._lock:
            if self._calls.pop(call.id, None) is not None and not call.done and self._conn:
                with contextlib.suppress(ConnectionClosed, OSError):
                    await self._conn.send(json.dumps({"type": "cancel", "id": call.id}))
            call.done = True
            if not self._calls and self._conn is not None:
                conn, self._conn = self._conn, None
                await conn.close()

    async def _connect(self) -> AsyncConnection:
        s = self._s
        rid = request_id()
        retries, refreshed, number = 0, False, 0
        while True:
            number += 1
            try:
                return await async_connect(
                    socket_url(s.base),
                    additional_headers={
                        "authorization": f"Bearer {(await self._provider.token()).access}",
                        "x-request-id": rid,
                    },
                    ssl=s.tls(),
                    proxy=s.proxy(),
                    user_agent_header=s.user_agent,
                    compression=None,
                    open_timeout=s.timeout,
                    ping_interval=min(PING_EVERY, s.idle / 3),
                    ping_timeout=s.idle,
                    max_size=MAX_EVENT,
                    max_queue=QUEUE,
                )
            except InvalidStatus as e:
                raw = _upgrade_error(e, rid, number)
                if raw.status == 401 and not refreshed:
                    await self._provider.invalidate()
                    refreshed = True
                    continue
                wait = retry_after(raw.headers)
                if (
                    retryable_status(raw.status)
                    and retries < s.max_retries
                    and s.draw(raw.status, wait is not None)
                ):
                    await asyncio.sleep(wait if wait is not None else backoff(retries))
                    retries += 1
                    continue
                raise ApiError(raw) from None
            except (OSError, InvalidHandshake, TimeoutError) as e:
                if retries < s.max_retries and s.draw(None, False):
                    await asyncio.sleep(backoff(retries))
                    retries += 1
                    continue
                if isinstance(e, TimeoutError):
                    raise ApiTimeoutError(s.host, s.timeout) from None
                raise ApiConnectionError(s.host, type(e).__name__) from None

    @staticmethod
    async def _deliver(call: _Call, item: tuple[str, object]) -> None:
        while not call.done:
            try:
                await asyncio.wait_for(call.items.put(item), timeout=0.1)
            except TimeoutError:
                continue
            return

    async def _fail_all(self, error: InOrbitError) -> None:
        async with self._lock:
            calls = list(self._calls.values())
            self._calls.clear()
            conn, self._conn = self._conn, None
        for c in calls:
            await self._deliver(c, ("error", error))
        if conn is not None:
            await conn.close()

    async def _read(self, conn: AsyncConnection) -> None:
        reconnects = 0
        while True:
            try:
                raw: str | bytes | None = await conn.recv()
            except (ConnectionClosed, OSError, TimeoutError):
                raw = None
            verdict = "reopen" if raw is None else await self._frame(raw)
            if verdict == "stop":
                return
            if verdict == "frame":
                reconnects = 0
                continue
            fresh = await self._reopen(conn, reconnects)
            if fresh is None:
                return
            conn = fresh
            reconnects += 1

    async def _frame(self, raw: str | bytes) -> Literal["frame", "reopen", "stop"]:
        kind, ident, frame = _frame_kind(raw)
        if kind == "error" and ident is None:
            if frame.get("code") == "unauthenticated":
                envelope = json.dumps(frame).encode()
                await self._fail_all(problem_error(envelope, _EMPTY, "socket", 1))
                return "stop"
            return "reopen"
        async with self._lock:
            call = self._calls.get(ident or "")
            if call is not None and kind in ("end", "error"):
                self._calls.pop(call.id, None)
        item = _item(kind, frame, call)
        if call is not None and item is not None:
            await self._deliver(call, item)
        return "frame"

    async def _reopen(self, conn: AsyncConnection, reconnects: int) -> AsyncConnection | None:
        with contextlib.suppress(Exception):
            await conn.close()
        async with self._lock:
            if self._conn is not conn or not self._calls:
                return None
        if reconnects > self._s.max_retries:
            await self._fail_all(ApiConnectionError(self._s.host, "the socket closed"))
            return None
        await asyncio.sleep(backoff(reconnects) if reconnects else 0)
        try:
            fresh = await self._connect()
        except InOrbitError as e:
            await self._fail_all(e)
            return None
        async with self._lock:
            if self._conn is not conn or not self._calls:
                await fresh.close()
                return None
            self._conn = fresh
            for c in self._calls.values():
                await fresh.send(c.frame())
        return fresh
