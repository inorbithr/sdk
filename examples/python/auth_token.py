"""Auth with a token you already have: an API token from the console or `iohr token create`.

The SDK sends it as a Bearer on every call and never exchanges it.
"""

import os

from inorbithr import Client, Public


def main() -> None:
    """Calls the API with the token."""
    api = Public(
        Client(
            token=os.environ.get("INORBIT_TOKEN", ""),
            base_url=os.environ.get("INORBIT_BASE_URL", "https://api.inorbit.hr"),
        )
    )
    me = api.me().value
    print(f"{me.subject} signed in with a token")


if __name__ == "__main__":
    main()
