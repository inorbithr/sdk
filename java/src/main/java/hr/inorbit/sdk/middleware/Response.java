package hr.inorbit.sdk.middleware;

import java.net.http.HttpHeaders;
import java.nio.ByteBuffer;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.Flow;

/**
 * An answer as it comes back through the pipeline: the status, the headers, and the body as
 * bytes, or, when the call is a stream, the open body, which a middleware must not read.
 */
public final class Response {

    private final int status;
    private final HttpHeaders headers;
    private final byte[] body;
    private final Flow.Publisher<List<ByteBuffer>> stream;

    /**
     * An answer with its whole body.
     *
     * @param status the HTTP status
     * @param headers the headers
     * @param body the body
     */
    public Response(int status, HttpHeaders headers, byte[] body) {
        this.status = status;
        this.headers = Objects.requireNonNull(headers, "headers");
        this.body = Objects.requireNonNull(body, "body");
        this.stream = null;
    }

    private Response(int status, HttpHeaders headers, Flow.Publisher<List<ByteBuffer>> stream) {
        this.status = status;
        this.headers = Objects.requireNonNull(headers, "headers");
        this.body = null;
        this.stream = Objects.requireNonNull(stream, "stream");
    }

    /**
     * An open stream's answer.
     *
     * @param status the HTTP status
     * @param headers the headers
     * @param stream the body as it arrives
     * @return the answer
     */
    public static Response ofStream(int status, HttpHeaders headers, Flow.Publisher<List<ByteBuffer>> stream) {
        return new Response(status, headers, stream);
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
     * The headers.
     *
     * @return the headers
     */
    public HttpHeaders headers() {
        return headers;
    }

    /**
     * The body. A middleware must not log it.
     *
     * @return the body, or {@code null} for a stream
     */
    public byte[] body() {
        return body;
    }

    /**
     * The open body of a stream. A middleware must not read it.
     *
     * @return the body, or {@code null} when the answer is not a stream
     */
    public Flow.Publisher<List<ByteBuffer>> stream() {
        return stream;
    }

    /** The status and whether it streams; never the body. */
    @Override
    public String toString() {
        return "Response[" + status + (stream != null ? ", stream" : "") + "]";
    }
}
