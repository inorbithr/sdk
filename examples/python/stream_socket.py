"""The same event stream over one WebSocket (`/v1/ws`) instead of SSE.

`streams="socket"` carries every stream of the client over one connection. Needs `events:read`.
"""

from inorbithr import Client, Public


def main() -> None:
    """Prints each event's time, type and id."""
    api = Public(Client.load(streams="socket"))
    n = 0
    with api.events.stream_events() as events:
        for event in events:
            n += 1
            print(f"{event.occurred_at} {event.type} {event.id}")
    print(f"{n} events")


if __name__ == "__main__":
    main()
