"""The same as `whoami.py`, on `asyncio`."""

import asyncio

from inorbithr import AsyncPublic


async def main() -> None:
    """Prints the caller and the three latest digests."""
    api = AsyncPublic.from_env()
    me = (await api.me()).value
    print(f"subject {me.subject}, scopes {' '.join(me.scopes)}")
    page = (await api.radar.list_digests(limit=3)).value
    for d in page.digests:
        print(f"{d.week} {d.language}: {d.summary}")


if __name__ == "__main__":
    asyncio.run(main())
