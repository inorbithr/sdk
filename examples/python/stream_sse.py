"""A server-sent-events stream: the account's events, as they happen.

Each event is typed; the loop ends when the server ends the stream. Needs `events:read`.
"""

from inorbithr import Public


def main() -> None:
    """Prints each event's time, type and id."""
    api = Public.from_env()  # streams open as SSE by default
    n = 0
    with api.events.stream_events() as events:
        for event in events:
            n += 1
            print(f"{event.occurred_at} {event.type} {event.id}")
    print(f"{n} events")


if __name__ == "__main__":
    main()
