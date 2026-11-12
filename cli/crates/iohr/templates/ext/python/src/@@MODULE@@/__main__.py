"""@@ID@@: writes $STATE/heartbeat.json every minute and asks iohr's token channel for
a token with the scope its manifest declares. Run it with `iohr ext dev`."""

from __future__ import annotations

import json
import os
import signal
import sys
import time

from . import ID, VERSION
from .channel import Refused, token

SCOPES = ["identity:read"]


def heartbeat(state: str) -> None:
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
        "token": tok,
        "dev": os.environ.get("IOHR_EXT_DEV") == "1",
    }
    tmp = os.path.join(state, ".heartbeat.json.tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(beat, f)
    os.replace(tmp, os.path.join(state, "heartbeat.json"))
    print(f"heartbeat at {now} (token {tok})", flush=True)


def main() -> int:
    state = os.environ.get("STATE")
    if not state or not os.environ.get("CACHE"):
        print("STATE and CACHE are not set: run this with iohr ext dev", file=sys.stderr)
        return 2
    every = int(os.environ.get("HEARTBEAT_EVERY", "60"))
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    print(f"{ID} {VERSION} starting; state in {state}", flush=True)
    try:
        while True:
            heartbeat(state)
            if os.environ.get("IOHR_EXT_ONCE"):
                return 0
            time.sleep(every)
    except KeyboardInterrupt:
        print("stopping", flush=True)
        return 0


if __name__ == "__main__":
    sys.exit(main())
