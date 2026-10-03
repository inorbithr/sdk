"""Who the API thinks you are, and the latest radar digests, with the public surface.

Run with INORBIT_TOKEN set (an API token from the console or `iohr token create`).
"""

import sys

from inorbithr import ApiError, Public


def main() -> None:
    """Prints the caller and the three latest digests."""
    api = Public.from_env()
    try:
        me = api.me().value
        print(f"subject {me.subject}, scopes {' '.join(me.scopes)}")
        page = api.radar.list_digests(limit=3).value
        for d in page.digests:
            print(f"{d.week} {d.language}: {d.summary}")
    except ApiError as e:
        if e.code == "forbidden":
            sys.exit("the token lacks a scope this example needs (identity:read, radar:read)")
        raise


if __name__ == "__main__":
    main()
