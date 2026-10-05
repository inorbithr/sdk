"""Logging through the standard library and, when installed, OpenTelemetry.

`docs/config.md` sections 7.9 and 7.10. Off by default: nothing is logged until `log` is
set, and nothing is sent anywhere by the SDK itself.
"""

from __future__ import annotations

import logging
from collections.abc import Callable, Iterable, Mapping
from typing import Any, TypeAlias
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

import httpx

#: The logger every record goes to unless a `logger` is given.
LOGGER_NAME = "inorbithr"

_LEVELS = {
    "debug": logging.DEBUG,
    "info": logging.INFO,
    "warn": logging.WARNING,
    "error": logging.ERROR,
}

#: Never logged, whatever the settings (section 7.9).
NEVER_LOGGED = frozenset({"authorization", "proxy-authorization", "cookie", "set-cookie"})
REQUEST_HEADERS = frozenset(
    {
        "accept",
        "content-type",
        "content-length",
        "user-agent",
        "x-request-id",
        "traceparent",
        "idempotency-key",
    }
)
RESPONSE_HEADERS = frozenset(
    {
        "content-type",
        "content-length",
        "date",
        "retry-after",
        "x-request-id",
        "idempotency-replayed",
        "x-ratelimit-limit",
        "x-ratelimit-remaining",
        "x-ratelimit-reset",
        "ratelimit",
        "ratelimit-policy",
    }
)

#: A span of the optional OpenTelemetry API (`opentelemetry.trace.Span`), or `None`.
Span: TypeAlias = Any

#: Sees every record last and returns it changed, or `None` to drop it (SR-14).
Redact = Callable[[dict[str, Any]], "dict[str, Any] | None"]


class Log:
    """One client's logging: the level, the header allowlist, the sink and `redact`."""

    def __init__(
        self,
        level: str = "off",
        *,
        logger: logging.Logger | None = None,
        headers: bool = False,
        allow: Iterable[str] = (),
        redact: Redact | None = None,
        profile: str | None = None,
    ) -> None:
        """Records at `level` and above go to `logger` (default: `inorbithr`)."""
        self.threshold = _LEVELS.get(level)
        self.logger = logger or logging.getLogger(LOGGER_NAME)
        self.with_headers = headers
        extra = {h.lower() for h in allow} - NEVER_LOGGED
        self.allow_request = REQUEST_HEADERS | extra
        self.allow_response = RESPONSE_HEADERS | extra
        self.redact = redact
        self.profile = profile

    def on(self, level: str) -> bool:
        """Whether a record at `level` would be written."""
        return self.threshold is not None and _LEVELS[level] >= self.threshold

    def headers(self, headers: httpx.Headers, *, response: bool) -> dict[str, str] | None:
        """The headers as a record holds them: allowlisted values, every other `REDACTED`."""
        if not self.with_headers:
            return None
        allow = self.allow_response if response else self.allow_request
        out: dict[str, str] = {}
        for name, value in headers.multi_items():
            n = name.lower()
            out[n] = value if n in allow and n not in NEVER_LOGGED else "REDACTED"
        return out

    def emit(self, level: str, event: str, fields: Mapping[str, object]) -> None:
        """Writes one record, after `redact`; `None` fields are left out."""
        if not self.on(level):
            return
        record: dict[str, Any] = {"event": event}
        record.update({k: v for k, v in fields.items() if v is not None})
        if self.profile is not None:
            record.setdefault("profile", self.profile)
        if self.redact is not None:
            changed = self.redact(record)
            if changed is None:
                return
            record = changed
        text = " ".join(f"{k}={v}" for k, v in record.items() if k != "event")
        extra = {k: v for k, v in record.items() if k not in _RESERVED}
        extra["inorbithr"] = record
        self.logger.log(_LEVELS[level], "%s %s", event, text, extra=extra)


_RESERVED = frozenset(
    logging.LogRecord("", 0, "", 0, "", (), None).__dict__.keys() | {"message", "asctime"}
)


def path_only(url: str) -> str:
    """A URL's path, without its query string: query values are never logged."""
    return urlsplit(url).path


def redacted_url(url: str) -> str:
    """A URL with every query value replaced by `REDACTED` (for `url.full`)."""
    parts = urlsplit(url)
    if not parts.query:
        return url
    query = urlencode([(k, "REDACTED") for k, _ in parse_qsl(parts.query, keep_blank_values=True)])
    return urlunsplit((parts.scheme, parts.netloc, parts.path, query, ""))


# --- OpenTelemetry -------------------------------------------------------------------


def otel_installed() -> bool:
    """Whether `opentelemetry-api` can be imported (`inorbithr[otel]`)."""
    import importlib.util  # noqa: PLC0415

    try:
        return importlib.util.find_spec("opentelemetry.trace") is not None
    except ImportError:  # the parent package is missing
        return False


class Telemetry:
    """Spans and metrics through the caller's OpenTelemetry providers, or nothing."""

    def __init__(
        self,
        *,
        tracing: bool | None,
        metrics: bool | None,
        tracer_provider: object = None,
        meter_provider: object = None,
    ) -> None:
        """Uses the global providers unless providers are given; off when not installed."""
        installed = otel_installed()
        self.tracer: Any = None
        self.meter: Any = None
        self.instruments: dict[str, Any] = {}
        if installed and (tracing if tracing is not None else True):
            from opentelemetry import trace  # noqa: PLC0415

            provider: Any = tracer_provider or trace.get_tracer_provider()
            self.tracer = provider.get_tracer("inorbithr", _version())
        want_metrics = (
            metrics if metrics is not None else (tracing if tracing is not None else True)
        )
        if installed and want_metrics:
            from opentelemetry import metrics as m  # noqa: PLC0415

            mp: Any = meter_provider or m.get_meter_provider()
            self.meter = mp.get_meter("inorbithr", _version())
            self.instruments = {
                "attempt": self.meter.create_histogram(
                    "http.client.request.duration", unit="s", description="One HTTP attempt"
                ),
                "call": self.meter.create_histogram(
                    "inorbit.client.call.duration", unit="s", description="One call, every attempt"
                ),
                "retries": self.meter.create_counter(
                    "inorbit.client.retries", unit="{retry}", description="Retries made"
                ),
                "exchanges": self.meter.create_counter(
                    "inorbit.client.token.exchanges", unit="{exchange}", description="Token fetches"
                ),
            }

    @property
    def tracing(self) -> bool:
        """Whether spans are made."""
        return self.tracer is not None

    def record(self, instrument: str, value: float, attributes: Mapping[str, object]) -> None:
        """Adds `value` to a histogram or counter, when metrics are on."""
        found = self.instruments.get(instrument)
        if found is None:
            return
        attrs = {k: v for k, v in attributes.items() if v is not None}
        if instrument in ("retries", "exchanges"):
            found.add(int(value), attrs)
        else:
            found.record(value, attrs)

    def start_call(
        self, name: str, attributes: Mapping[str, object], traceparent: str | None
    ) -> Span:
        """The call's `INTERNAL` span, a child of the caller's `traceparent` when given."""
        if self.tracer is None:
            return None
        from opentelemetry import trace  # noqa: PLC0415
        from opentelemetry.trace.propagation.tracecontext import (  # noqa: PLC0415
            TraceContextTextMapPropagator,
        )

        context = (
            TraceContextTextMapPropagator().extract({"traceparent": traceparent})
            if traceparent
            else None
        )
        return self.tracer.start_span(
            name, context=context, kind=trace.SpanKind.INTERNAL, attributes=dict(attributes)
        )

    def start_attempt(self, parent: Span, name: str, attributes: Mapping[str, object]) -> Span:
        """One attempt's `CLIENT` span, a child of the call's span."""
        if self.tracer is None:
            return None
        from opentelemetry import trace  # noqa: PLC0415

        context = trace.set_span_in_context(parent) if parent is not None else None
        attrs = {k: v for k, v in attributes.items() if v is not None}
        return self.tracer.start_span(
            name, context=context, kind=trace.SpanKind.CLIENT, attributes=attrs
        )

    @staticmethod
    def inject(span: Span, headers: httpx.Headers) -> None:
        """Sends `traceparent` and `tracestate` from `span`."""
        from opentelemetry import trace  # noqa: PLC0415
        from opentelemetry.trace.propagation.tracecontext import (  # noqa: PLC0415
            TraceContextTextMapPropagator,
        )

        carrier: dict[str, str] = {}
        TraceContextTextMapPropagator().inject(carrier, context=trace.set_span_in_context(span))
        for k, v in carrier.items():
            headers[k] = v

    @staticmethod
    def ids(span: Span) -> tuple[str | None, str | None]:
        """The trace id and span id of `span`, as hex, when it is recording."""
        if span is None:
            return None, None
        ctx = span.get_span_context()
        if not ctx.is_valid:
            return None, None
        return f"{ctx.trace_id:032x}", f"{ctx.span_id:016x}"

    @staticmethod
    def end(
        span: Span,
        error_type: str | None = None,
        attributes: Mapping[str, object] | None = None,
    ) -> None:
        """Ends `span`, marking it failed with `error_type` when given."""
        if span is None:
            return
        for k, v in (attributes or {}).items():
            if v is not None:
                span.set_attribute(k, v)
        if error_type is not None:
            from opentelemetry.trace import Status, StatusCode  # noqa: PLC0415

            span.set_attribute("error.type", error_type)
            span.set_status(Status(StatusCode.ERROR))
        span.end()


def _version() -> str:
    from inorbithr._version import SDK_VERSION  # noqa: PLC0415

    return SDK_VERSION
