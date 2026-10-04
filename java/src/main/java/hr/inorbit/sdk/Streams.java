package hr.inorbit.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.Code;
import java.net.http.HttpHeaders;
import java.util.Map;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;

/** What the two stream transports share: the error of an envelope, the idle clocks' timer. */
final class Streams {

    /** The largest event's data, and the largest frame read: 1 MiB. */
    static final int MAX_EVENT = 1024 * 1024;

    /** Items waiting per stream on the socket. */
    static final int QUEUE = 64;

    /** One daemon thread for every idle clock. */
    static final ScheduledExecutorService TIMER = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "inorbit-sdk-streams");
        t.setDaemon(true);
        return t;
    });

    private static final HttpHeaders NO_HEADERS = HttpHeaders.of(Map.of(), (a, b) -> true);

    private Streams() {}

    /**
     * The {@link ApiException} an error envelope carries, its status from the code
     * ({@code problem.json}'s {@code x-http-status}).
     */
    static ApiException error(JsonNode envelope, String requestId, int attempts) {
        String code = envelope.path("code").asText("");
        int status = Code.of(code.isEmpty() ? "internal" : code).httpStatus().orElse(500);
        ObjectNode wire = Json.MAPPER.createObjectNode();
        wire.put("code", code);
        wire.put("error", envelope.path("error").asText(""));
        wire.set(
                "details",
                envelope.path("details").isArray() ? envelope.path("details") : Json.MAPPER.createArrayNode());
        byte[] body;
        try {
            body = Json.MAPPER.writeValueAsBytes(wire);
        } catch (com.fasterxml.jackson.core.JsonProcessingException e) {
            body = new byte[0];
        }
        return ApiException.of(new RawResponse(status, NO_HEADERS, body, requestId, attempts));
    }
}
