package example;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.EventStream;
import hr.inorbit.sdk.StreamTransport;
import hr.inorbit.sdk.generated.Public;
import hr.inorbit.sdk.generated.StreamEventsResponse;

/**
 * The same event stream over one WebSocket ({@code /v1/ws}) instead of SSE: set {@code
 * streams(StreamTransport.SOCKET)}. Every stream of the client shares the connection. Needs
 * {@code events:read}.
 */
public final class StreamSocket {

    private StreamSocket() {}

    /**
     * Prints each event.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Public api = new Public(Client.builder().streams(StreamTransport.SOCKET).load());
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
