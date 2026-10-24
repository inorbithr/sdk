"""One unary GET: who the API thinks you are.

The answer carries the typed value and the raw response (request id, attempts, rate limit).
Run with INORBIT_TOKEN set (or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES).
"""

from inorbithr import Public


def main() -> None:
    """Prints the caller."""
    api = Public.from_env()
    me = api.me()
    print(f"subject {me.value.subject}, scopes {' '.join(me.value.scopes)}, request {me.request_id}")


if __name__ == "__main__":
    main()
