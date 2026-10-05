package hr.inorbit.sdk.middleware;

import java.net.URI;
import java.util.Objects;

/**
 * A request as it goes through the pipeline: method, URL, headers (changed in place), the body as
 * bytes ({@code null} when there is none, never a stream) and the call's {@link CallInfo}.
 */
public final class Request {

    private final String method;
    private final URI uri;
    private final Headers headers;
    private final byte[] body;
    private final CallInfo info;

    /**
     * A request.
     *
     * @param method the HTTP method
     * @param uri the full URL
     * @param headers the headers; this request holds them, changes show
     * @param body the body, or {@code null}
     * @param info the call's metadata
     */
    public Request(String method, URI uri, Headers headers, byte[] body, CallInfo info) {
        this.method = Objects.requireNonNull(method, "method");
        this.uri = Objects.requireNonNull(uri, "uri");
        this.headers = Objects.requireNonNull(headers, "headers");
        this.body = body;
        this.info = Objects.requireNonNull(info, "info");
    }

    /**
     * The HTTP method.
     *
     * @return the method
     */
    public String method() {
        return method;
    }

    /**
     * The full URL.
     *
     * @return the URL
     */
    public URI uri() {
        return uri;
    }

    /**
     * The headers sent; change them in place.
     *
     * @return the headers
     */
    public Headers headers() {
        return headers;
    }

    /**
     * The body. A middleware must not log it.
     *
     * @return the body, or {@code null}
     */
    public byte[] body() {
        return body;
    }

    /**
     * The call's metadata, read only.
     *
     * @return the metadata
     */
    public CallInfo info() {
        return info;
    }

    /**
     * This request with another URL; the headers are copied.
     *
     * @param uri the URL
     * @return the new request
     */
    public Request withUri(URI uri) {
        return new Request(method, uri, new Headers(headers), body, info);
    }

    /**
     * This request with another body; the headers are copied.
     *
     * @param body the body, or {@code null}
     * @return the new request
     */
    public Request withBody(byte[] body) {
        return new Request(method, uri, new Headers(headers), body, info);
    }

    /**
     * This request with other metadata; the headers are copied. The SDK's built-ins use it.
     *
     * @param info the metadata
     * @return the new request
     */
    public Request withInfo(CallInfo info) {
        return new Request(method, uri, new Headers(headers), body, info);
    }

    /** Method, URL and header names; never a value or the body. */
    @Override
    public String toString() {
        return "Request[" + method + " " + uri.getRawPath() + ", " + headers + "]";
    }
}
