"""The SDK retries for you.

The API answers 429 with Retry-After; the SDK waits that long and tries again, and the
answer says how many attempts it took. Tune it with `max_retries`.
"""

from inorbithr import Client, Public


def main() -> None:
    """Calls the API and says how many attempts it took."""
    api = Public(Client.load(max_retries=3))
    me = api.me()
    print(f"{me.value.subject} after {me.raw.attempts} attempts")


if __name__ == "__main__":
    main()
