"""A paged list.

`all_list_monitors` yields every item and follows `next_page_token` for you; the second
half reads one page by hand with `page_size` and `page_token`.
Run with INORBIT_ORG_ID and a credential with `connections:read`.
"""

import os

from inorbithr import Public


def main() -> None:
    """Prints every monitor, then one page."""
    api = Public.from_env()
    org = os.environ.get("INORBIT_ORG_ID", "")

    n = 0
    for m in api.connections.all_list_monitors(org, page_size=2):
        n += 1
        print(f"{m.monitor_id} {m.name}")
    print(f"{n} monitors")

    page = api.connections.list_monitors(org, page_size=1).value
    print(f'one page: {len(page.monitors)} monitor, more after "{page.next_page_token}"')


if __name__ == "__main__":
    main()
