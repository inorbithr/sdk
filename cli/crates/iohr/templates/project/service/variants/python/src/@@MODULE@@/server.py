"""The routes, and the one place a request is traced, logged and counted."""

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

from .obs import Logger, Metrics, Span


class Service:
    """What the handlers share. `ready` is False until the server listens and again as soon
    as shutdown starts, so a load balancer stops sending traffic before the process stops."""

    def __init__(self, log: Logger):
        self.log = log
        self.metrics = Metrics()
        self.ready = threading.Event()


def _hello(query: dict[str, list[str]]):
    name = (query.get("name") or ["world"])[0] or "world"
    if len(name) > 64:
        return 400, {"error": "name is at most 64 characters of text"}
    # The name comes from the query string: it is echoed back, never logged.
    return 200, {"message": f"Hello, {name}"}


def make_handler(svc: Service):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "service"

        def log_message(self, *_args):  # the stdlib's access log would print the raw path
            pass

        def do_GET(self):  # noqa: N802 (the name http.server calls)
            span = Span.start(self.headers.get("traceparent"))
            parts = urlsplit(self.path)
            route, status, body, ctype = "unmatched", 404, {"error": "not found"}, "application/json"
            if parts.path == "/healthz":
                route, status, body = "/healthz", 200, {"status": "ok"}
            elif parts.path == "/readyz":
                ok = svc.ready.is_set()
                route, status, body = "/readyz", 200 if ok else 503, {"status": "ready" if ok else "not ready"}
            elif parts.path == "/metrics":
                route, status, body, ctype = "/metrics", 200, svc.metrics.render(), "text/plain; version=0.0.4"
            elif parts.path == "/v1/hello":
                route = "/v1/hello"
                status, body = _hello(parse_qs(parts.query))
            payload = (body if isinstance(body, str) else json.dumps(body) + "\n").encode()
            self.send_response(status)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("traceparent", span.header())
            self.end_headers()
            # Record before the body goes out: this server runs a thread per request, so
            # a caller that has the whole response must also find its log lines and count.
            ms = span.millis()
            fields = {"trace_id": span.trace_id, "route": route, "status": status, "duration_ms": ms}
            extra = {"parent_span_id": span.parent_id} if span.parent_id else {}
            svc.log.info("span", span_id=span.span_id, name=f"GET {route}", **fields, **extra)
            svc.log.info("request", method="GET", **fields)
            svc.metrics.inc(route, status)
            self.wfile.write(payload)

    return Handler


def make_server(host: str, port: int, svc: Service) -> ThreadingHTTPServer:
    server = ThreadingHTTPServer((host, port), make_handler(svc))
    server.daemon_threads = True
    return server
