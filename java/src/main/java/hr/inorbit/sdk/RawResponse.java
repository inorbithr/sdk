package hr.inorbit.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.io.IOException;
import java.net.http.HttpHeaders;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Optional;

/** An HTTP answer as it came, for anything the typed result does not carry. */
public final class RawResponse {

    private final int status;
    private final HttpHeaders headers;
    private final byte[] body;
    private final String requestId;
    private final int attempts;

    /**
     * An answer read off the wire.
     *
     * @param status the HTTP status
     * @param headers the response headers
     * @param body the body, at most 16 MiB
     * @param requestId the {@code x-request-id} the SDK sent
     * @param attempts how many attempts the call took
     */
    public RawResponse(int status, HttpHeaders headers, byte[] body, String requestId, int attempts) {
        this.status = status;
        this.headers = headers;
        this.body = body.clone();
        this.requestId = requestId;
        this.attempts = attempts;
    }

    /**
     * The HTTP status.
     *
     * @return the status
     */
    public int status() {
        return status;
    }

    /**
     * The response headers.
     *
     * @return the headers
     */
    public HttpHeaders headers() {
        return headers;
    }

    /**
     * The first value of a header.
     *
     * @param name the header, in any case
     * @return its value, if present
     */
    public Optional<String> header(String name) {
        return headers.firstValue(name);
    }

    /**
     * The body.
     *
     * @return a copy of the body
     */
    public byte[] body() {
        return body.clone();
    }

    /**
     * The body as UTF-8 text.
     *
     * @return the text
     */
    public String text() {
        return new String(body, StandardCharsets.UTF_8);
    }

    /**
     * The body as JSON.
     *
     * @return the JSON, or {@code null} when the body is empty or not JSON
     */
    public JsonNode json() {
        if (body.length == 0) {
            return null;
        }
        try {
            return Json.MAPPER.readTree(body);
        } catch (IOException e) {
            return null;
        }
    }

    /**
     * The {@code x-request-id} the SDK sent.
     *
     * @return the request id
     */
    public String requestId() {
        return requestId;
    }

    /**
     * The request id the API answered with.
     *
     * @return the id, if the API sent one
     */
    public Optional<String> serverRequestId() {
        return headers.firstValue("x-request-id");
    }

    /**
     * How many attempts the call took.
     *
     * @return the attempts, 1 or more
     */
    public int attempts() {
        return attempts;
    }

    /** The status, the size and the request id; never the body or a header. */
    @Override
    public String toString() {
        return "RawResponse[status=" + status + ", body_bytes=" + body.length + ", request_id=" + requestId
                + ", attempts=" + attempts + "]";
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof RawResponse r
                && r.status == status
                && Arrays.equals(r.body, body)
                && r.requestId.equals(requestId)
                && r.attempts == attempts;
    }

    @Override
    public int hashCode() {
        return 31 * (31 * status + Arrays.hashCode(body)) + requestId.hashCode();
    }
}
