package hr.inorbit.sdk.errors;

import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonValue;
import java.io.Serializable;
import java.util.Map;
import java.util.Objects;
import java.util.OptionalInt;

/**
 * An error code: one of the platform's codes, or a newer one kept as the API wrote it. Compare
 * with the constants ({@code e.code().equals(Code.FORBIDDEN)}); {@link #slug()} is the wire
 * form.
 */
public final class Code implements Serializable {

    private static final long serialVersionUID = 1L;

    /** {@code bad_request}, HTTP 400. */
    public static final Code BAD_REQUEST = new Code("bad_request");
    /** {@code failed_precondition}, HTTP 400. */
    public static final Code FAILED_PRECONDITION = new Code("failed_precondition");
    /** {@code unauthenticated}, HTTP 401. */
    public static final Code UNAUTHENTICATED = new Code("unauthenticated");
    /** {@code forbidden}, HTTP 403. */
    public static final Code FORBIDDEN = new Code("forbidden");
    /** {@code not_found}, HTTP 404. */
    public static final Code NOT_FOUND = new Code("not_found");
    /** {@code method_not_allowed}, HTTP 405. */
    public static final Code METHOD_NOT_ALLOWED = new Code("method_not_allowed");
    /** {@code already_exists}, HTTP 409. */
    public static final Code ALREADY_EXISTS = new Code("already_exists");
    /** {@code conflict}, HTTP 409. */
    public static final Code CONFLICT = new Code("conflict");
    /** {@code payload_too_large}, HTTP 413. */
    public static final Code PAYLOAD_TOO_LARGE = new Code("payload_too_large");
    /** {@code unsupported_media_type}, HTTP 415. */
    public static final Code UNSUPPORTED_MEDIA_TYPE = new Code("unsupported_media_type");
    /** {@code unprocessable}, HTTP 422. */
    public static final Code UNPROCESSABLE = new Code("unprocessable");
    /** {@code rate_limited}, HTTP 429. */
    public static final Code RATE_LIMITED = new Code("rate_limited");
    /** {@code quota_exceeded}, HTTP 429. */
    public static final Code QUOTA_EXCEEDED = new Code("quota_exceeded");
    /** {@code cancelled}, HTTP 499. */
    public static final Code CANCELLED = new Code("cancelled");
    /** {@code internal}, HTTP 500. */
    public static final Code INTERNAL = new Code("internal");
    /** {@code unimplemented}, HTTP 501. */
    public static final Code UNIMPLEMENTED = new Code("unimplemented");
    /** {@code unavailable}, HTTP 503. */
    public static final Code UNAVAILABLE = new Code("unavailable");
    /** {@code timeout}, HTTP 504. */
    public static final Code TIMEOUT = new Code("timeout");

    private static final Map<String, Integer> STATUS = Map.ofEntries(
            Map.entry("bad_request", 400),
            Map.entry("failed_precondition", 400),
            Map.entry("unauthenticated", 401),
            Map.entry("forbidden", 403),
            Map.entry("not_found", 404),
            Map.entry("method_not_allowed", 405),
            Map.entry("already_exists", 409),
            Map.entry("conflict", 409),
            Map.entry("payload_too_large", 413),
            Map.entry("unsupported_media_type", 415),
            Map.entry("unprocessable", 422),
            Map.entry("rate_limited", 429),
            Map.entry("quota_exceeded", 429),
            Map.entry("cancelled", 499),
            Map.entry("internal", 500),
            Map.entry("unimplemented", 501),
            Map.entry("unavailable", 503),
            Map.entry("timeout", 504));

    private static final Map<Integer, String> BY_STATUS = Map.ofEntries(
            Map.entry(400, "bad_request"),
            Map.entry(401, "unauthenticated"),
            Map.entry(403, "forbidden"),
            Map.entry(404, "not_found"),
            Map.entry(405, "method_not_allowed"),
            Map.entry(409, "conflict"),
            Map.entry(413, "payload_too_large"),
            Map.entry(415, "unsupported_media_type"),
            Map.entry(422, "unprocessable"),
            Map.entry(429, "rate_limited"),
            Map.entry(499, "cancelled"),
            Map.entry(501, "unimplemented"),
            Map.entry(503, "unavailable"),
            Map.entry(504, "timeout"));

    private final String slug;

    private Code(String slug) {
        this.slug = slug;
    }

    /**
     * The code with {@code slug}, known or not.
     *
     * @param slug the wire form, such as {@code not_found}
     * @return the code
     */
    @JsonCreator
    public static Code of(String slug) {
        return new Code(Objects.requireNonNull(slug, "slug"));
    }

    /**
     * The code a plain-text answer with {@code status} stands for: {@code internal} for an unlisted
     * 5xx, {@code http_<status>} for anything else unlisted.
     *
     * @param status the HTTP status
     * @return the code
     */
    public static Code forStatus(int status) {
        String slug = BY_STATUS.get(status);
        if (slug != null) {
            return new Code(slug);
        }
        return new Code(status >= 500 ? "internal" : "http_" + status);
    }

    /**
     * The wire form, such as {@code not_found}.
     *
     * @return the slug
     */
    @JsonValue
    public String slug() {
        return slug;
    }

    /**
     * Whether this version of the SDK knows the code.
     *
     * @return {@code true} for one of the platform's codes
     */
    public boolean isKnown() {
        return STATUS.containsKey(slug);
    }

    /**
     * The HTTP status the platform answers this code with, for a known code.
     *
     * @return the status, or empty for an unknown code
     */
    public OptionalInt httpStatus() {
        Integer s = STATUS.get(slug);
        return s == null ? OptionalInt.empty() : OptionalInt.of(s);
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof Code c && c.slug.equals(slug);
    }

    @Override
    public int hashCode() {
        return slug.hashCode();
    }

    @Override
    public String toString() {
        return slug;
    }
}
