"""What a generated surface imports from the runtime, and nothing else does.

This module is a contract with `iohr sdk generate`: a change that breaks generated code
bumps `VERSION`, and a surface generated for another version refuses to import.
"""

from __future__ import annotations

from urllib.parse import quote

#: The surface contract this runtime implements.
VERSION = 1


def path_segment(value: str) -> str:
    """Percent-encodes `value` as one path segment.

    Every byte but the RFC 3986 unreserved characters is encoded, so a slash or a space
    in an id never changes the route.

    Args:
        value: The parameter's value.

    Returns:
        The encoded segment.
    """
    return quote(value, safe="")


def check(version: int) -> None:
    """Refuses a surface generated for another runtime version.

    Args:
        version: The contract version the surface was generated for.

    Raises:
        ImportError: The surface and this runtime do not match; run `iohr sdk generate`
            again.
    """
    if version != VERSION:
        raise ImportError(
            f"this surface was generated for runtime contract {version}, and this inorbithr "
            f"implements {VERSION}: run `iohr sdk generate` again"
        )
