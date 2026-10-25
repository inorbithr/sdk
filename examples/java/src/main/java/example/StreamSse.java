package example;

import hr.inorbit.sdk.EventStream;
import hr.inorbit.sdk.generated.Public;
import hr.inorbit.sdk.generated.StreamEventsResponse;

/**
 * A server-sent-events stream: the account's events, as they happen. Each event is typed; the
 * loop ends when the server ends the stream. Needs {@code events:read}.
 */
public final class StreamSse {

    private StreamSse() {}

    /**
     * Prints each event.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Public api = Public.fromEnv(); // streams open as SSE by default
        int n = 0;
        try (EventStream<StreamEventsResponse> events = api.events().streamEvents()) {
            for (StreamEventsResponse e : events) {
                n++;
                System.out.println(e.occurredAt() + " " + e.type() + " " + e.id());
            }
        }
        System.out.println(n + " events");
    }
}
