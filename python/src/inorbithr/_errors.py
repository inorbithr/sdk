"""Every way a call fails, as one tree under `InOrbitError`.

`isinstance` and `err.kind` tell them apart; the message says what failed and never
holds a secret.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Annotated, Any, Literal, TypeAlias, cast

import httpx
from pydantic import BeforeValidator

#: The platform's error codes (`spec/problem.json`), with the HTTP status of each.
CODES: dict[str, int] = {
    "bad_request": 400,
    "failed_precondition": 400,
    "unauthenticated": 401,
    "forbidden": 403,
    "not_found": 404,
    "method_not_allowed": 405,
    "already_exists": 409,
    "conflict": 409,
    "payload_too_large": 413,
    "unsupported_media_type": 415,
    "rate_limited": 429,
    "quota_exceeded": 429,
    "cancelled": 499,
    "internal": 500,
    "unimplemented": 501,
    "unavailable": 503,
    "timeout": 504,
}

#: A code this version of the SDK knows.
KnownCode: TypeAlias = Literal[
    "bad_request",
    "failed_precondition",
    "unauthenticated",
    "forbidden",
    "not_found",
    "method_not_allowed",
    "already_exists",
    "conflict",
    "payload_too_large",
    "unsupported_media_type",
    "rate_limited",
    "quota_exceeded",
    "cancelled",
    "internal",
    "unimplemented",
    "unavailable",
    "timeout",
]

#: An error code: a known one, or a newer one kept as the API wrote it.
Code: TypeAlias = str

_BY_STATUS: dict[int, str] = {
    400: "bad_request",
    401: "unauthenticated",
    403: "forbidden",
    404: "not_found",
    405: "method_not_allowed",
    409: "conflict",
    413: "payload_too_large",
    415: "unsupported_media_type",
    429: "rate_limited",
    499: "cancelled",
    501: "unimplemented",
    503: "unavailable",
    504: "timeout",
}

_GATEWAY_MESSAGES: dict[int, str] = {
    401: "the token was refused: it is missing, expired or revoked",
    403: "this credential may not call this route: its scopes or role do not allow it",
    404: "no such route or resource",
    429: "too many requests; try again shortly",
}


def code_for_status(status: int) -> Code:
    """The code a plain-text answer with `status` stands for.

    Args:
        status: The HTTP status.

    Returns:
        A known code, `internal` for another 5xx, or `http_<status>`.
    """
    if status in _BY_STATUS:
        return _BY_STATUS[status]
    return "internal" if status >= 500 else f"http_{status}"


@dataclass(frozen=True)
class FieldDetail:
    """A request field that was not accepted."""

    field: str
    description: str


@dataclass(frozen=True)
class InfoDetail:
    """Why the API refused, in machine-readable form."""

    reason: str
    domain: str
    metadata: dict[str, str]


@dataclass(frozen=True)
class RetryDetail:
    """How long the API asks the caller to wait."""

    after_seconds: int


@dataclass(frozen=True)
class UnknownDetail:
    """A detail type this version does not know, kept as it came."""

    value: Any


def read_detail(value: object) -> FieldDetail | InfoDetail | RetryDetail | UnknownDetail:
    """Reads one detail off the wire; a type this version does not know is kept.

    Args:
        value: The detail as decoded JSON.

    Returns:
        The typed detail.
    """
    if isinstance(value, FieldDetail | InfoDetail | RetryDetail | UnknownDetail):
        return value
    if not isinstance(value, dict):
        return UnknownDetail(value)
    wire: dict[str, Any] = {str(k): v for k, v in value.items()}  # pyright: ignore[reportUnknownVariableType, reportUnknownArgumentType]

    def text(key: str) -> str:
        v = wire.get(key)
        return v if isinstance(v, str) else ""

    kind = wire.get("type")
    if kind == "field":
        return FieldDetail(text("field"), text("description"))
    if kind == "info":
        raw_meta = wire.get("metadata")
        meta: dict[str, str] = {}
        if isinstance(raw_meta, dict):
            pairs = cast("dict[object, object]", raw_meta).items()
            meta = {str(k): str(v) for k, v in pairs}
        return InfoDetail(text("reason"), text("domain"), meta)
    if kind == "retry":
        after = wire.get("after_seconds")
        try:
            seconds = int(after) if isinstance(after, int | str | float) else 0
        except ValueError:
            seconds = 0
        return RetryDetail(seconds)
    return UnknownDetail(value)


#: One entry of an error's `details`; read with `read_detail` wherever a model holds one.
Detail: TypeAlias = Annotated[
    FieldDetail | InfoDetail | RetryDetail | UnknownDetail, BeforeValidator(read_detail)
]


@dataclass(frozen=True)
class RawResponse:
    """An HTTP answer as it came, for anything the typed result does not carry."""

    status: int
    """The HTTP status."""
    headers: httpx.Headers
    """The response headers."""
    body: bytes = field(repr=False)
    """The body, at most 16 MiB."""
    request_id: str
    """The `x-request-id` this SDK sent."""
    attempts: int
    """How many attempts the call took."""

    @property
    def server_request_id(self) -> str | None:
        """The request id the API answered with, if any."""
        value: str | None = self.headers.get("x-request-id")
        return value

    def text(self) -> str:
        """The body as text."""
        return self.body.decode("utf-8", errors="replace")

    def json(self) -> Any:  # noqa: ANN401 - JSON is any shape
        """The body as JSON, or `None` when it is not JSON."""
        try:
            return json.loads(self.body)
        except ValueError:
            return None


class InOrbitError(Exception):
    """The base of every error this SDK raises."""

    kind: str
    """A stable kind: `api`, `connection`, `timeout`, `auth`, `config`, `too_large`, `decode`."""

    def __init__(self, kind: str, message: str) -> None:
        """An error of `kind` with `message`."""
        super().__init__(message)
        self.kind = kind

    @property
    def message(self) -> str:
        """What failed, in words."""
        return str(self)


def _truncate(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[:limit] + "…"


class ApiError(InOrbitError):
    """The API answered with the problem envelope, or a plain-text error from the gateway."""

    status: int
    """The HTTP status."""
    code: Code
    """The error code."""
    problem: str
    """What the API said went wrong."""
    details: tuple[Detail, ...]
    """Typed details."""
    raw: RawResponse
    """The answer as it came."""

    def __init__(self, raw: RawResponse) -> None:
        """The error an answer stands for."""
        wire = raw.json()
        envelope: dict[str, Any] = {}
        if isinstance(wire, dict):
            envelope = {str(k): v for k, v in wire.items()}  # pyright: ignore[reportUnknownVariableType, reportUnknownArgumentType]
        code_value = envelope.get("code")
        error_value = envelope.get("error")
        has_envelope = (isinstance(code_value, str) and code_value != "") or (
            isinstance(error_value, str) and error_value != ""
        )
        details: list[Detail] = []
        if has_envelope:
            code = (
                code_value
                if isinstance(code_value, str) and code_value
                else code_for_status(raw.status)
            )
            message = error_value if isinstance(error_value, str) else ""
            raw_details = envelope.get("details")
            if isinstance(raw_details, list):
                details = [read_detail(d) for d in cast("list[object]", raw_details)]
        else:
            code = code_for_status(raw.status)
            message = raw.text().strip()
        message = (
            _GATEWAY_MESSAGES.get(
                raw.status,
                "the API failed to answer" if raw.status >= 500 else "the request was refused",
            )
            if message == ""
            else _truncate(message, 300)
        )
        server_id = raw.server_request_id
        suffix = "" if server_id is None else f", request id {server_id}"
        super().__init__("api", f"{message} ({code}, HTTP {raw.status}{suffix})")
        self.status = raw.status
        self.code = code
        self.problem = message
        self.details = tuple(details)
        self.raw = raw

    def retry_after_seconds(self) -> int | None:
        """Seconds the API asked to wait, from a `retry` detail or `Retry-After`."""
        for d in self.details:
            if isinstance(d, RetryDetail):
                return d.after_seconds
        header = self.raw.headers.get("retry-after", "").strip()
        return int(header) if header.isdigit() else None


class ApiConnectionError(InOrbitError):
    """The API could not be reached: DNS, TCP, TLS, or a reset before an answer."""

    host: str
    """The unreachable host."""

    def __init__(self, host: str, reason: str) -> None:
        """Cannot reach `host`, because of `reason`."""
        super().__init__("connection", f"cannot reach {host}: {reason}")
        self.host = host


class ApiTimeoutError(InOrbitError):
    """An attempt ran out of time."""

    host: str
    """The host that did not answer."""

    def __init__(self, host: str, seconds: float) -> None:
        """`host` did not answer within `seconds`."""
        super().__init__("timeout", f"{host} did not answer within {seconds:g} s")
        self.host = host


class AuthError(InOrbitError):
    """The token exchange, or a custom token provider, failed."""

    error: str
    """The token endpoint's `error`, or `HTTP <status>`; empty for a transport failure."""

    def __init__(self, message: str, error: str = "") -> None:
        """An auth failure with its message."""
        super().__init__("auth", message)
        self.error = error


class ConfigError(InOrbitError):
    """The client was configured in a way it cannot work with."""

    def __init__(self, message: str) -> None:
        """A configuration problem."""
        super().__init__("config", message)


class TooLargeError(InOrbitError):
    """The answer is larger than 16 MiB."""

    def __init__(self) -> None:
        """An answer over the cap."""
        super().__init__("too_large", "the answer is larger than 16 MiB; refusing to read it")


class DecodeError(InOrbitError):
    """The API answered something this version cannot read."""

    raw: RawResponse
    """The answer as it came."""

    def __init__(self, reason: str, raw: RawResponse) -> None:
        """An answer that does not read."""
        super().__init__(
            "decode",
            f"the API answered something this version of inorbithr cannot read: {reason}",
        )
        self.raw = raw
