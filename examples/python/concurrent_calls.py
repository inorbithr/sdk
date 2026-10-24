"""Async use: one client shared by several calls in flight on `asyncio`.

Callers that need a token at the same moment share one exchange.
"""

import asyncio

from inorbithr import AsyncPublic


async def main() -> None:
    """Makes three calls at once."""
    api = AsyncPublic.from_env()
    answers = await asyncio.gather(api.me(), api.me(), api.me())
    print(f"{len(answers)} calls at once: {', '.join(a.value.subject for a in answers)}")


if __name__ == "__main__":
    asyncio.run(main())
