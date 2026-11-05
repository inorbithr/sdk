"""Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).

iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
`POST /token {"scopes": [...]}` and gets `{"access_token", "expires_at"}` for scopes its
manifest declares, and a refusal for any other. It is the only credential this project
has: never read IOHR_TOKEN or run `iohr auth token`.
"""

from __future__ import annotations

import json
import os
import socket

MAX_ANSWER = 64 * 1024


class Refused(Exception):
    """iohr refused the request, or the channel could not be reached."""


def _exchange(conn: socket.socket, scopes: list[str]) -> bytes:
    body = json.dumps({"scopes": scopes}).encode()
    request = (
        b"POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\n"
        + b"content-length: " + str(len(body)).encode() + b"\r\nconnection: close\r\n\r\n" + body
    )
    conn.sendall(request)
    chunks: list[bytes] = []
    while sum(len(c) for c in chunks) <= MAX_ANSWER:
        chunk = conn.recv(4096)
        if not chunk:
            break
        chunks.append(chunk)
    return b"".join(chunks)


def token(scopes: list[str]) -> str:
    """An access token for `scopes`; raises Refused with iohr's reason."""
    path = os.environ.get("IOHR_EXT_TOKEN_SOCKET", "").strip()
    if not path:
        raise Refused("IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev")
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(10)
            s.connect(path)
            raw = _exchange(s, scopes)
    except OSError as e:
        raise Refused(f"cannot reach iohr's token channel: {e}") from None
    head, _, body = raw.partition(b"\r\n\r\n")
    status = head.split(b" ", 2)[1] if b" " in head else b""
    try:
        answer = json.loads(body or b"{}")
    except ValueError:
        answer = {}
    if status != b"200":
        raise Refused(f"{status.decode() or 'no answer'} {answer.get('error', '')}: {answer.get('message', '')}")
    if "access_token" not in answer:
        raise Refused("iohr's answer holds no token")
    return answer["access_token"]
