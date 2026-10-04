"""Prints the account's events as they happen, until interrupted.

Needs a key or token with `events:read`. `streams="socket"` would carry the stream over
the one `/v1/ws` connection instead of server-sent events.
"""

from inorbithr import Client, Public


def main() -> None:
    """Prints each event's type and id."""
    api = Public(Client.from_env())
    with api.events.stream_events() as events:
        for event in events:
            print(f"{event.occurred_at} {event.type} {event.id}")


if __name__ == "__main__":
    main()
