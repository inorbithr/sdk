"""Auth with a token provider of your own.

Anything with `token()` and `invalidate()` works; here a stand-in for a secret store. The
SDK asks it before every attempt and calls `invalidate()` when the API refused the last
token. Never log the token.
"""

import os

from inorbithr import Client, Public, Token


class Vault:
    """Wherever your tokens live."""

    def token(self) -> Token:
        """A token for the next attempt; set `expires_at` when you know it."""
        return Token(access="custom-token-1")

    def invalidate(self) -> None:
        """The API refused the last token: drop any cached one."""


def main() -> None:
    """Calls the API with the provider."""
    base = os.environ.get("INORBIT_BASE_URL", "https://api.inorbit.hr")
    api = Public(Client(token_provider=Vault(), base_url=base))
    me = api.me().value
    print(f"{me.subject} signed in with a custom provider")


if __name__ == "__main__":
    main()
