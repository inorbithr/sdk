"""64-bit integers: an `int` in Python, a decimal string on the wire (design.md section 4)."""

from __future__ import annotations

from typing import Annotated, TypeAlias

from pydantic import PlainSerializer

#: A 64-bit integer. Reads a decimal string or a JSON number; writes a decimal string.
Int64: TypeAlias = Annotated[int, PlainSerializer(str, return_type=str, when_used="json")]
