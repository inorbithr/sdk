"""Reads HOST and PORT from the environment, serves, and stops cleanly on SIGINT or SIGTERM."""

import os
import signal
import sys
import threading

from .obs import Logger
from .server import Service, make_server


def load_config(env) -> tuple[str, int]:
    host = env.get("HOST") or "127.0.0.1"
    raw = env.get("PORT") or "8080"
    if not raw.isdigit() or not 0 <= int(raw) <= 65535:
        raise ValueError("PORT must be a number from 0 to 65535")
    return host, int(raw)


def main() -> int:
    log = Logger(sys.stdout)
    try:
        host, port = load_config(os.environ)
    except ValueError as e:
        log.error("bad configuration", error=str(e))
        return 2
    svc = Service(log)
    server = make_server(host, port, svc)
    stop = threading.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: stop.set())
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    svc.ready.set()
    log.info("listening", addr=f"{server.server_address[0]}:{server.server_address[1]}")
    stop.wait()
    svc.ready.clear()
    log.info("shutting down")
    server.shutdown()
    server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
