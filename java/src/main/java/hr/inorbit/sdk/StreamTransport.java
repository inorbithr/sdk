package hr.inorbit.sdk;

/** How a client opens its streams (design.md section 7). */
public enum StreamTransport {
    /** Server-sent events, one HTTP response per stream; the default. */
    SSE,
    /**
     * Every stream of the client as a call on one {@code /v1/ws} connection, reconnected and
     * the calls issued again when the server ends the socket.
     */
    SOCKET
}
