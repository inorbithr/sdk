package hr.inorbit.sdk.errors;

import com.fasterxml.jackson.databind.JsonNode;
import hr.inorbit.sdk.RawResponse;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.OptionalLong;

/**
 * The API answered with the problem envelope ({@code code}, {@code error}, {@code details}), or the
 * gateway with plain text, which is mapped by status.
 */
public final class ApiException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    private static final Map<Integer, String> GATEWAY = Map.of(
            401, "the token was refused: it is missing, expired or revoked",
            403, "this credential may not call this route: its scopes or role do not allow it",
            404, "no such route or resource",
            429, "too many requests; try again shortly");

    private final int status;
    private final Code code;
    private final String problem;
    private final transient List<Detail> details;
    private final transient RawResponse raw;

    private ApiException(int status, Code code, String problem, List<Detail> details, RawResponse raw, String message) {
        super(message, null);
        this.status = status;
        this.code = code;
        this.problem = problem;
        this.details = List.copyOf(details);
        this.raw = raw;
    }

    /**
     * The error an answer stands for.
     *
     * @param raw the answer
     * @return the error
     */
    public static ApiException of(RawResponse raw) {
        JsonNode wire = raw.json();
        String wireCode = wire == null ? "" : wire.path("code").asText("");
        String wireError = wire == null ? "" : wire.path("error").asText("");
        Code code;
        String message;
        List<Detail> details = new ArrayList<>();
        if (!wireCode.isEmpty() || !wireError.isEmpty()) {
            code = wireCode.isEmpty() ? Code.forStatus(raw.status()) : Code.of(wireCode);
            message = wireError;
            for (JsonNode d : wire.path("details")) {
                details.add(Detail.of(d));
            }
        } else {
            code = Code.forStatus(raw.status());
            message = raw.text().strip();
        }
        if (message.isEmpty()) {
            message = GATEWAY.getOrDefault(
                    raw.status(), raw.status() >= 500 ? "the API failed to answer" : "the request was refused");
        } else if (message.codePointCount(0, message.length()) > 300) {
            message = message.substring(0, message.offsetByCodePoints(0, 300)) + "…";
        }
        String id = raw.serverRequestId().map(r -> ", request id " + r).orElse("");
        String full = message + " (" + code.slug() + ", HTTP " + raw.status() + id + ")";
        return new ApiException(raw.status(), code, message, details, raw, full);
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
     * The error code.
     *
     * @return the code
     */
    public Code code() {
        return code;
    }

    /**
     * What the API said went wrong.
     *
     * @return the problem
     */
    public String problem() {
        return problem;
    }

    /**
     * Typed details.
     *
     * @return the details
     */
    public List<Detail> details() {
        return details;
    }

    /**
     * The answer as it came.
     *
     * @return the raw answer
     */
    public RawResponse raw() {
        return raw;
    }

    /**
     * Seconds the API asked to wait, from a {@code retry} detail or {@code Retry-After}.
     *
     * @return the seconds, or empty when the API did not say
     */
    public OptionalLong retryAfterSeconds() {
        for (Detail d : details) {
            if (d instanceof Detail.Retry r) {
                return OptionalLong.of(r.afterSeconds());
            }
        }
        return raw.header("retry-after")
                .map(String::strip)
                .filter(v -> v.matches("\\d+"))
                .map(v -> OptionalLong.of(Long.parseLong(v)))
                .orElse(OptionalLong.empty());
    }

    @Override
    public String kind() {
        return "api";
    }
}
