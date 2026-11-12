"""Observability on the standard library alone: JSON logs, spans that follow the W3C
traceparent header, and request counters in the Prometheus text format.

Nothing here records a header, a query string or an environment value: a log line and a span
name the route that matched, the status, the duration and the trace id.
"""

import json
import re
import secrets
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone

_TRACEPARENT = re.compile(r"^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$")


class Logger:
    """One JSON object per line to `sink` (anything with `write`)."""

    def __init__(self, sink):
        self._sink = sink
        self._lock = threading.Lock()

    def info(self, msg: str, **fields) -> None:
        record = {"ts": datetime.now(timezone.utc).isoformat(), "level": "INFO", "msg": msg}
        record.update(fields)
        line = json.dumps(record, separators=(",", ":")) + "\n"
        with self._lock:
            self._sink.write(line)
            self._sink.flush()

    def error(self, msg: str, **fields) -> None:
        self.info(msg, level="ERROR", **fields)


@dataclass
class Span:
    """One request's trace: the id it joined (or started) and its own id."""

    trace_id: str
    span_id: str = field(default_factory=lambda: secrets.token_hex(8))
    parent_id: str | None = None
    started: float = field(default_factory=time.perf_counter)

    @classmethod
    def start(cls, traceparent: str | None) -> "Span":
        """Joins the trace of a valid traceparent header, or starts a new one."""
        m = _TRACEPARENT.match(traceparent or "")
        if m and set(m[1]) != {"0"} and set(m[2]) != {"0"}:
            return cls(trace_id=m[1], parent_id=m[2])
        return cls(trace_id=secrets.token_hex(16))

    def header(self) -> str:
        """The traceparent to send back (and on to callers): this span is the parent."""
        return f"00-{self.trace_id}-{self.span_id}-01"

    def millis(self) -> float:
        return round((time.perf_counter() - self.started) * 1000, 3)


class Metrics:
    """Request counts by route and status. Routes are matched patterns, never raw paths, so
    the label set is bounded by the code and holds no user input."""

    def __init__(self):
        self._counts: dict[tuple[str, int], int] = {}
        self._lock = threading.Lock()

    def inc(self, route: str, status: int) -> None:
        with self._lock:
            self._counts[(route, status)] = self._counts.get((route, status), 0) + 1

    def render(self) -> str:
        lines = [
            "# HELP http_requests_total Requests served, by route and status.",
            "# TYPE http_requests_total counter",
        ]
        with self._lock:
            for (route, status), n in sorted(self._counts.items()):
                lines.append(f'http_requests_total{{route="{route}",status="{status}"}} {n}')
        return "\n".join(lines) + "\n"
