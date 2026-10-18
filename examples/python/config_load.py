"""Configuration by `load`: code, the environment, the iohr config file and the iohr login.

`describe()` says where each value came from, without printing a secret.
"""

from inorbithr import ConfigError, Public


def main() -> None:
    """Prints where the credential came from, then the caller."""
    try:
        api = Public.load(timeout=10)
    except ConfigError as e:
        raise SystemExit(str(e)) from e  # every problem, with its setting
    described = api.client.config().describe()
    print(f"credential from {described['credential']['source']}")
    me = api.me().value
    print(f"{me.subject}, a {me.kind}")


if __name__ == "__main__":
    main()
