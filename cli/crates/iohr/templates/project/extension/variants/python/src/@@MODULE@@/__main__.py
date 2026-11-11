"""@@ID@@: the demo project's private macOS service.

Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's
token channel for a token with the one scope its manifest declares. It also serves a tiny
panel on 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.

Run it with `iohr ext dev` (the real token channel, $STATE and $CACHE under
~/.inorbit/ext-dev/@@NAME@@/), or once with IOHR_EXT_ONCE=1 as the self-test does.
"""

from __future__ import annotations

import json
import os
import signal
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from . import ID, VERSION
from .channel import Refused, token

SCOPES = ["identity:read"]
HERE = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PANEL = os.path.join(HERE, "panel", "index.html")


def write_json(state: str, name: str, value: dict) -> None:
    tmp = os.path.join(state, f".{name}.tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(value, f)
    os.replace(tmp, os.path.join(state, name))


def heartbeat(state: str, beats: int) -> dict:
    try:
        token(SCOPES)  # kept in memory only, never printed
        tok = "granted"
    except Refused as e:
        print(f"token refused: {e}", file=sys.stderr)
        tok = "refused"
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    beat = {
        "id": ID,
        "version": VERSION,
        "state": "ok",
        "at": now,
        "beats": beats,
        "token": tok,
        "dev": os.environ.get("IOHR_EXT_DEV") == "1",
    }
    write_json(state, "heartbeat.json", beat)
    print(f"heartbeat {beats} at {now} (token {tok})", flush=True)
    return beat


class Panel(BaseHTTPRequestHandler):
    """The panel: index.html, and /api/heartbeat as JSON. Nothing else, nothing written."""

    latest: dict = {}

    def do_GET(self) -> None:  # noqa: N802 (http.server's name)
        if self.path == "/api/heartbeat":
            body = json.dumps(self.latest).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
        elif self.path in ("/", "/index.html"):
            with open(PANEL, "rb") as f:
                body = f.read()
            self.send_response(200)
            self.send_header("content-type", "text/html; charset=utf-8")
        else:
            body = b"not found"
            self.send_response(404)
            self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_: object) -> None:  # quiet
        pass


def serve(state: str) -> ThreadingHTTPServer:
    """Listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, and it is
    written to $STATE/panel.json so the app knows where the panel is."""
    port = int(os.environ.get("PANEL_PORT", "0"))
    srv = ThreadingHTTPServer(("127.0.0.1", port), Panel)
    write_json(state, "panel.json", {"id": ID, "url": f"http://127.0.0.1:{srv.server_port}/"})
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    print(f"panel at http://127.0.0.1:{srv.server_port}/", flush=True)
    return srv


def main() -> int:
    state = os.environ.get("STATE")
    if not state or not os.environ.get("CACHE"):
        print("STATE and CACHE are not set: run this with iohr ext dev", file=sys.stderr)
        return 2
    every = int(os.environ.get("HEARTBEAT_EVERY", "60"))
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    print(f"{ID} {VERSION} starting; state in {state}", flush=True)
    srv = serve(state) if not os.environ.get("IOHR_EXT_ONCE") else None
    beats = 0
    try:
        while True:
            beats += 1
            Panel.latest = heartbeat(state, beats)
            if os.environ.get("IOHR_EXT_ONCE"):
                return 0
            time.sleep(every)
    except KeyboardInterrupt:
        print("stopping", flush=True)
        return 0
    finally:
        if srv:
            srv.shutdown()


if __name__ == "__main__":
    sys.exit(main())
