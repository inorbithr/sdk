"""Timestamps as the API sends them: RFC 3339 strings, "" when unset.

Rule N5 of spec/README.md, settled behaviour. Models keep the string; this reads it.
"""

from __future__ import annotations

import re
from datetime import datetime

_RFC3339 = re.compile(r"\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})")


def parse_timestamp(value: str) -> datetime | None:
    """Read a timestamp field: None for "" (the field is unset), an aware datetime for RFC 3339.

    Raises ValueError when the value is neither "" nor RFC 3339.
    """
    if value == "":
        return None
    if not _RFC3339.fullmatch(value):
        raise ValueError("not an RFC 3339 timestamp")
    text = value.upper().replace("Z", "+00:00")
    # datetime keeps microseconds: a longer fraction is cut, not rounded.
    head, sep, tail = text.partition(".")
    if sep:
        digits = len(tail) - len(tail.lstrip("0123456789"))
        tail = tail[:digits][:6].ljust(6, "0") + tail[digits:]
    return datetime.fromisoformat(head + sep + tail)
