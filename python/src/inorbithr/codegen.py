"""What a generated surface imports from the runtime, and nothing else does.

This module is a contract with `iohr sdk generate`: a change that breaks generated code
bumps `VERSION`, and a surface generated for another version refuses to import.
"""

from __future__ import annotations

from collections.abc import AsyncIterator, Awaitable, Callable, Iterator, Sequence
from typing import TypeVar
from urllib.parse import quote

T = TypeVar("T")

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


def pages(fetch: Callable[[str | None], tuple[Sequence[T], str]]) -> Iterator[T]:
    """Every item of a paged list, page after page.

    `fetch` takes a token (`None` for the first page) and answers the page's items and
    the next token. The walk stops after the page whose next token is empty or repeats,
    and fetches nothing more once the loop stops.

    Args:
        fetch: Fetches one page.

    Yields:
        Each item, in order.
    """
    token: str | None = None
    while True:
        items, next_token = fetch(token)
        yield from items
        if not next_token or next_token == token:
            return
        token = next_token


async def apages(
    fetch: Callable[[str | None], Awaitable[tuple[Sequence[T], str]]],
) -> AsyncIterator[T]:
    """Every item of a paged list, page after page, for `async for`.

    As `pages`, with an awaitable `fetch`; a cancelled task stops between pages.

    Args:
        fetch: Fetches one page.

    Yields:
        Each item, in order.
    """
    token: str | None = None
    while True:
        items, next_token = await fetch(token)
        for item in items:
            yield item
        if not next_token or next_token == token:
            return
        token = next_token
